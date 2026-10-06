import type { Settings, ValidMods } from "../../@types/settings.js";
import { createSettingsScrollRestoration } from "./settingsScrollRestoration.js";

const {
    plugin: { store },
} = shelter;

export let isRestartRequired = false;

export function setRestartRequired() {
    isRestartRequired = true;
}

export function refreshSettings() {
    store.settings = window.legcord.settings.getConfig();
}

export function refreshThemes() {
    store.themes = window.legcord.themes.getThemes();
}

/** Nearest scrollable ancestor — Discord's settings content scroller. */
function findScrollableAncestor(start: Element | null | undefined): HTMLElement | null {
    let el = start instanceof HTMLElement ? start : (start?.parentElement ?? null);
    while (el) {
        const { overflowY } = getComputedStyle(el);
        if (
            (overflowY === "auto" || overflowY === "scroll" || overflowY === "overlay") &&
            el.scrollHeight > el.clientHeight + 1
        ) {
            return el;
        }
        el = el.parentElement;
    }
    return null;
}

function findSettingsScroller(): HTMLElement | null {
    // Prefer a scroller that currently contains focus / active control.
    const fromFocus = findScrollableAncestor(document.activeElement);
    if (fromFocus) return fromFocus;

    for (const root of document.querySelectorAll("shltr-rroot")) {
        const found = findScrollableAncestor(root.parentElement);
        if (found) return found;
    }
    return null;
}

function findSettingsContentScroller(): HTMLElement | null {
    for (const root of document.querySelectorAll("shltr-rroot")) {
        if (!root.textContent?.trim()) continue;
        const found = findScrollableAncestor(root.parentElement);
        if (found) return found;
    }
    return findSettingsScroller();
}

function findNestedSettingsScroller(start: Element | null | undefined, outer: HTMLElement): HTMLElement | null {
    let el = start instanceof HTMLElement ? start : (start?.parentElement ?? null);
    while (el && el !== outer) {
        const { overflowY } = getComputedStyle(el);
        if (
            (overflowY === "auto" || overflowY === "scroll" || overflowY === "overlay") &&
            el.scrollHeight > el.clientHeight + 1
        ) {
            return el;
        }
        el = el.parentElement;
    }
    return null;
}

const scrollRestoration = createSettingsScrollRestoration<HTMLElement, HTMLElement>({
    findScroller: findSettingsContentScroller,
    resolveScroller(cached) {
        if (cached?.isConnected) {
            const active = document.activeElement;
            if (active instanceof Element && cached.contains(active)) {
                return findNestedSettingsScroller(active, cached) ?? cached;
            }
            if (active instanceof Element) return findSettingsContentScroller();
            return cached;
        }
        return findSettingsScroller();
    },
    getScrollTop: (scroller) => scroller.scrollTop,
    setScrollTop: (scroller, top) => {
        scroller.scrollTop = top;
    },
    getActiveElement: () => (document.activeElement instanceof HTMLElement ? document.activeElement : null),
    contains: (element) => document.contains(element),
    isActive: (element) => document.activeElement === element,
    focus: (element) => element.focus({ preventScroll: true }),
    requestFrame: (callback) => requestAnimationFrame(callback),
    cancelFrame: (id) => cancelAnimationFrame(id),
});

export function mountSettingsScrollRestoration(): () => void {
    const unmount = scrollRestoration.mount();
    const onResize = () => scrollRestoration.invalidate();
    window.addEventListener("resize", onResize);
    return () => {
        window.removeEventListener("resize", onResize);
        unmount();
    };
}

export function disposeSettingsScrollRestoration() {
    scrollRestoration.dispose();
}

/** Keep Discord's settings scroller put across store/DOM updates. */
export function withPreservedSettingsScroll<T>(run: () => T): T {
    return scrollRestoration.preserve(run);
}

export function setConfig<K extends keyof Settings>(key: K, value: Settings[K], shouldRestart?: boolean) {
    withPreservedSettingsScroll(() => {
        // Mutate in place — do NOT replace store.settings (that remounts the page and jumps scroll).
        store.settings[key] = value;
        console.log(key, ":", store.settings[key]);
        if (shouldRestart) {
            isRestartRequired = true;
        }
        window.legcord.settings.setConfig(key, value);
    });
}

/** True when running under the native WebView2 shell (Phase C-3+). */
export function isNativeWebView(): boolean {
    return (window.legcord as unknown as { boot?: { runtime?: unknown } })?.boot?.runtime === "webview2";
}

/** Minimal native settings mutation shape for this caller. */
interface NativeBlockPowerSavingSettings {
    setBlockPowerSavingInVoiceChat: (value: boolean) => Promise<boolean>;
}

/**
 * Native WebView2 path for the ONE allowlisted `blockPowerSavingInVoiceChat`
 * mutation (Phase C-3). Awaits the typed async
 * `settings.set_block_power_saving_in_voice_chat` capability and only then
 * updates local store state from the host-confirmed value, so the renderer
 * never holds stale state. A rejected mutation leaves local state untouched.
 * The Electron path stays in `setConfig` above, unchanged.
 */
export async function setBlockPowerSavingInVoiceChat(value: boolean): Promise<boolean> {
    const nativeSet = (window.legcord as unknown as { settings: NativeBlockPowerSavingSettings }).settings
        .setBlockPowerSavingInVoiceChat;
    const applied = await nativeSet(value);
    withPreservedSettingsScroll(() => {
        // Mutate in place — do NOT replace store.settings (that remounts the page and jumps scroll).
        store.settings.blockPowerSavingInVoiceChat = applied;
        console.log("blockPowerSavingInVoiceChat", ":", store.settings.blockPowerSavingInVoiceChat);
        isRestartRequired = true;
    });
    return applied;
}

/** Closed folder locations the native `folders.open` capability reveals. */
export type NativeFolderLocation = "storage" | "themes" | "plugins";

/** Minimal native folders shape (Phase D Step 2). */
interface NativeFolders {
    open: (location: NativeFolderLocation) => Promise<string>;
    openTheme: (id: string) => Promise<string>;
}

/**
 * Native folders namespace when the WebView2 shell grants it, `undefined`
 * otherwise. Capability-gated: the namespace exists only when the boot
 * snapshot grants `folders.open`/`folders.open_theme`. Uses the existing
 * `isNativeWebView` environment check — no second detection mechanism.
 */
function nativeFolders(): NativeFolders | undefined {
    if (!isNativeWebView()) return undefined;
    const folders = (window.legcord as unknown as { folders?: NativeFolders }).folders;
    if (typeof folders?.open !== "function" || typeof folders?.openTheme !== "function") return undefined;
    return folders;
}

/**
 * Native WebView2 folder reveal for a fixed root. Returns `true` when the
 * native typed capability handled the action; `false` when the caller must
 * use the Electron IPC path. Fire-and-forget like the Electron `send` it
 * replaces: rejections are swallowed after a path-free console diagnostic,
 * so no unhandled rejection can escape a click handler and no filesystem
 * path ever reaches the UI.
 */
export function revealFolderNative(location: NativeFolderLocation): boolean {
    const folders = nativeFolders();
    if (!folders) return false;
    void folders
        .open(location)
        .catch(() => console.warn(`[Legcord] Could not reveal the ${location} folder in the native shell.`));
    return true;
}

/**
 * Native WebView2 open for one theme directory. The id passes through
 * verbatim — validation is owned by the native capability, not recreated
 * here. Same fire-and-forget contract as `revealFolderNative`.
 */
export function openThemeFolderNative(id: string): boolean {
    const folders = nativeFolders();
    if (!folders) return false;
    void folders
        .openTheme(id)
        .catch(() => console.warn("[Legcord] Could not open the theme folder in the native shell."));
    return true;
}

/** Bounded display metadata for one native theme (Phase D Step 3d-1 list contract). */
export interface NativeThemeListEntry {
    id: string;
    name: string;
    author: string;
    description: string;
    version: string;
    enabled: boolean;
}

/** Result summary for the native host-owned theme file picker. */
export interface NativeThemeImportResult {
    canceled: boolean;
    imported: number;
    failed: number;
}

/**
 * Native themes namespace when the WebView2 shell grants it, `undefined`
 * otherwise. Capability-gated per operation at the call site — this helper
 * only reuses the existing `isNativeWebView` environment check, no second
 * detection mechanism. The host remains authoritative for id validation.
 */
function nativeThemes(): Record<string, unknown> | undefined {
    if (!isNativeWebView()) return undefined;
    const themes = (window.legcord as unknown as { themes?: Record<string, unknown> }).themes;
    if (!themes || typeof themes !== "object") return undefined;
    return themes;
}

/** True when the native async theme inventory is the active path. */
export function isNativeThemeListAvailable(): boolean {
    return typeof nativeThemes()?.listThemes === "function";
}

/** True when the native async theme toggle is the active path. */
export function isNativeThemeToggleAvailable(): boolean {
    return typeof nativeThemes()?.setEnabled === "function";
}

/** True when the native host-owned import picker is the active path. */
export function isNativeThemeImportAvailable(): boolean {
    return typeof nativeThemes()?.importFiles === "function";
}

/**
 * True when the Themes UI runs against the native inventory. Deferred theme
 * operations (edit, uninstall, URL install, Quick CSS management) stay
 * hidden/disabled in this mode — no placeholder RPC exists for them.
 */
export function isNativeThemesMode(): boolean {
    return isNativeThemeListAvailable();
}

function isRecord(value: unknown): value is Record<string, unknown> {
    return !!value && typeof value === "object";
}

/**
 * Renderer-side shape check for one native list entry. The host owns id and
 * filesystem validation; this only guards the UI against malformed bridge
 * data. Returns `null` for entries the page must skip.
 */
export function toThemeManifest(entry: unknown): import("../../@types/themeManifest.js").ThemeManifest | null {
    if (!isRecord(entry)) return null;
    if (typeof entry.id !== "string" || entry.id.length === 0) return null;
    if (typeof entry.name !== "string") return null;
    if (typeof entry.enabled !== "boolean") return null;
    const text = (value: unknown): string | undefined =>
        typeof value === "string" ? value : value === undefined ? undefined : undefined;
    const author = text(entry.author);
    const description = text(entry.description);
    const version = text(entry.version);
    if (author === undefined || description === undefined || version === undefined) return null;
    return {
        id: entry.id,
        name: entry.name,
        author,
        description,
        version,
        enabled: entry.enabled,
        // Native list carries no CSS path; the card never consumes `theme`.
        theme: "",
    };
}

/**
 * Native theme inventory. Rejects (path-free Error) when the host call
 * fails so the page can show an error state; never rejects for a missing
 * capability — gate with `isNativeThemeListAvailable` first so exactly one
 * path executes per load.
 */
export async function loadNativeThemes(): Promise<import("../../@types/themeManifest.js").ThemeManifest[]> {
    const listThemes = nativeThemes()?.listThemes as (() => Promise<unknown>) | undefined;
    if (typeof listThemes !== "function") throw new Error("[Legcord] Native theme list is unavailable.");
    let raw: unknown;
    try {
        raw = await listThemes();
    } catch {
        throw new Error("[Legcord] Could not load themes in the native shell.");
    }
    if (!Array.isArray(raw)) throw new Error("[Legcord] Could not load themes in the native shell.");
    const themes: import("../../@types/themeManifest.js").ThemeManifest[] = [];
    for (const entry of raw) {
        const manifest = toThemeManifest(entry);
        if (manifest) themes.push(manifest);
    }
    return themes;
}

/**
 * Native inventory refresh: replaces `store.themes` in place-preserving
 * scroll, mirroring `refreshThemes` for Electron. Rejects on host failure;
 * the caller catches and shows an error state (never an unhandled
 * rejection, never a fallback sync getter that would throw natively).
 */
export async function refreshNativeThemes(): Promise<void> {
    const themes = await loadNativeThemes();
    withPreservedSettingsScroll(() => {
        store.themes = themes;
    });
}

/**
 * Native theme toggle. Exactly one native operation per call; the existing
 * host watcher applies/removes CSS — renderer code never touches styles.
 * Rejects on host failure so the card can revert its optimistic switch.
 */
export async function setNativeThemeEnabled(id: string, enabled: boolean): Promise<void> {
    const setEnabled = nativeThemes()?.setEnabled as ((id: string, enabled: boolean) => Promise<unknown>) | undefined;
    if (typeof setEnabled !== "function") throw new Error("[Legcord] Native theme toggle is unavailable.");
    try {
        await setEnabled(id, enabled);
    } catch {
        throw new Error("[Legcord] Could not update the theme in the native shell.");
    }
}

/**
 * Native host-owned import picker. Cancellation resolves `{ canceled: true }`
 * and stays silent; host failures are swallowed with a path-free warning
 * and reported as a silent cancellation so clicks never produce unhandled
 * rejections. Returns `null` when the capability is absent (caller uses the
 * Electron picker path) — exactly one path per click.
 */
export async function importNativeThemes(): Promise<NativeThemeImportResult | null> {
    const importFiles = nativeThemes()?.importFiles as (() => Promise<unknown>) | undefined;
    if (typeof importFiles !== "function") return null;
    try {
        const result = (await importFiles()) as Partial<NativeThemeImportResult>;
        if (
            !isRecord(result) ||
            typeof result.canceled !== "boolean" ||
            typeof result.imported !== "number" ||
            typeof result.failed !== "number"
        ) {
            throw new Error("invalid import summary");
        }
        return { canceled: result.canceled, imported: result.imported, failed: result.failed };
    } catch {
        console.warn("[Legcord] Could not import themes in the native shell.");
        return { canceled: true, imported: 0, failed: 0 };
    }
}

function removeMod(array: ValidMods[], filter: ValidMods) {
    return array.filter((i) => i !== filter);
}

export function toggleMod(mod: ValidMods, enabled: boolean) {
    isRestartRequired = true;
    const currentMods = store.settings.mods as ValidMods[];
    if (enabled) {
        if (mod === "vencord") {
            setConfig("mods", [...removeMod(currentMods, "equicord"), "vencord"]);
        } else if (mod === "equicord") {
            setConfig("mods", [...removeMod(currentMods, "vencord"), "equicord"]);
        }
    } else {
        setConfig("mods", removeMod(currentMods, mod));
    }
}

export function isMinWindowsVersion(major: number, minor: number, build: number) {
    const [sys_major, sys_minor, sys_build] = window.legcord.osRelease
        .split(".")
        .map((val, _, __) => Number.parseInt(val, 10));
    return sys_major >= major && sys_minor >= minor && sys_build >= build;
}

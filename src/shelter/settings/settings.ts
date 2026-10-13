import { createSignal } from "solid-js";
import type { Settings, ValidMods } from "../../@types/settings.js";
import { createSettingsScrollRestoration } from "./settingsScrollRestoration.js";

const {
    plugin: { store },
} = shelter;

export let isRestartRequired = false;

// Reactive twin of `isRestartRequired` for the in-page restart banner: the
// plain flag only surfaces on the next settings/games pane view (the TRACK
// modal), so a toggle flipped while already on the page showed no
// indication at all. Every restart-requesting mutation funnels through
// `markRestartRequired`, which sets both.
const [restartRequiredSignal, setRestartRequiredSignal] = createSignal(false);

export { restartRequiredSignal as isRestartRequiredSignal };

export function markRestartRequired() {
    isRestartRequired = true;
    setRestartRequiredSignal(true);
}

export function setRestartRequired() {
    markRestartRequired();
}

export function refreshSettings() {
    store.settings = window.remcord.settings.getConfig();
}

export function refreshThemes() {
    store.themes = window.remcord.themes.getThemes();
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
        if (isNativeWebView()) {
            setConfigNative(key, value, shouldRestart);
            return;
        }
        // Mutate in place — do NOT replace store.settings (that remounts the page and jumps scroll).
        store.settings[key] = value;
        console.log(key, ":", store.settings[key]);
        if (shouldRestart) {
            markRestartRequired();
        }
        window.remcord.settings.setConfig(key, value);
    });
}

/**
 * Settings keys the native shell can actually persist, mirroring the closed
 * host `settings.set` allowlist (`native/src/settings.rs`) plus the
 * dedicated voice-chat capability. Every other key is Electron-consumed,
 * unsafe, or deferred with no native writer: natively those controls render
 * disabled (see `isSettingWritableNatively`) instead of throwing.
 */
export const NATIVE_WRITABLE_SETTING_KEYS: ReadonlyArray<string> = [
    "mods",
    "supportBannerDismissed",
    "showExperimentalPluginMenu",
    "extendedPluginAbilities",
    "blockPowerSavingInVoiceChat",
    "csp",
    "quickCss",
];

/** True when `key` has a native persistence path (closed allowlist). */
export function isSettingWritableNatively(key: string): boolean {
    return NATIVE_WRITABLE_SETTING_KEYS.includes(key);
}

/**
 * Native WebView2 branch of `setConfig`. Allowlisted keys delegate to the
 * typed async capabilities and update local state only from the
 * host-confirmed value (reverting the optimistic write when the host
 * rejects, so the UI never shows unpersisted state). Non-allowlisted keys
 * are discarded with an honest warning — no store mutation, no restart
 * flag, no call into a missing bridge member (previously this threw
 * `setConfig is not a function` after flipping the switch, which reverted
 * on restart and looked like a dead toggle).
 */
function setConfigNative<K extends keyof Settings>(key: K, value: Settings[K], shouldRestart?: boolean) {
    if (key === "blockPowerSavingInVoiceChat") {
        const previous = store.settings.blockPowerSavingInVoiceChat;
        store.settings.blockPowerSavingInVoiceChat = value as Settings["blockPowerSavingInVoiceChat"];
        void setBlockPowerSavingInVoiceChat(value as boolean).catch(() => {
            withPreservedSettingsScroll(() => {
                store.settings.blockPowerSavingInVoiceChat = previous;
            });
            console.warn(`[Remcord] Setting "${key}" was rejected by the native shell; change discarded.`);
        });
        return;
    }
    if (
        key === "mods" ||
        key === "supportBannerDismissed" ||
        key === "showExperimentalPluginMenu" ||
        key === "extendedPluginAbilities" ||
        key === "csp" ||
        key === "quickCss"
    ) {
        const previous = store.settings[key];
        (store.settings as Record<string, unknown>)[key as string] = value;
        void setNativeSetting(key, value, shouldRestart).catch(() => {
            withPreservedSettingsScroll(() => {
                (store.settings as Record<string, unknown>)[key as string] = previous;
            });
            console.warn(`[Remcord] Setting "${key}" was rejected by the native shell; change discarded.`);
        });
        return;
    }
    // Unsupported natively: never pretend success. The control stays at its
    // persisted value and no restart is requested.
    console.warn(`[Remcord] Setting "${String(key)}" is not supported by the native app shell; change discarded.`);
}

/** True when running under the native WebView2 shell (Phase C-3+). */
export function isNativeWebView(): boolean {
    return (window.remcord as unknown as { boot?: { runtime?: unknown } })?.boot?.runtime === "webview2";
}

/** Minimal native settings mutation shape for this caller. */
interface NativeBlockPowerSavingSettings {
    setBlockPowerSavingInVoiceChat: (value: boolean) => Promise<boolean>;
}

/**
 * Native WebView2 path for the ONE allowlisted `blockPowerSavingInVoiceChat`
 * mutation (Phase C-3). Updates local state OPTIMISTICALLY so the toggle
 * reacts immediately, then converges to the host-confirmed value; a rejected
 * mutation reverts and rethrows so the caller reports honestly. The Electron
 * path stays in `setConfig` above, unchanged.
 */
export async function setBlockPowerSavingInVoiceChat(value: boolean): Promise<boolean> {
    const previous = store.settings.blockPowerSavingInVoiceChat;
    withPreservedSettingsScroll(() => {
        // Mutate in place — do NOT replace store.settings (that remounts the page and jumps scroll).
        store.settings.blockPowerSavingInVoiceChat = value;
    });
    const nativeSet = (window.remcord as unknown as { settings: NativeBlockPowerSavingSettings }).settings
        .setBlockPowerSavingInVoiceChat;
    let applied: boolean;
    try {
        applied = await nativeSet(value);
    } catch (error) {
        withPreservedSettingsScroll(() => {
            store.settings.blockPowerSavingInVoiceChat = previous;
        });
        throw error;
    }
    withPreservedSettingsScroll(() => {
        store.settings.blockPowerSavingInVoiceChat = applied;
        console.log("blockPowerSavingInVoiceChat", ":", store.settings.blockPowerSavingInVoiceChat);
        markRestartRequired();
    });
    return applied;
}

/** CamelCase settings keys the closed native `settings.set` capability accepts. */
export type NativeSettingsSetKey =
    | "mods"
    | "supportBannerDismissed"
    | "showExperimentalPluginMenu"
    | "extendedPluginAbilities"
    | "csp"
    | "quickCss";

/** Wire keys for the closed native `settings.set` capability. */
const NATIVE_SETTINGS_SET_WIRE_KEY: Record<NativeSettingsSetKey, string> = {
    mods: "mods",
    supportBannerDismissed: "support_banner_dismissed",
    showExperimentalPluginMenu: "show_experimental_plugin_menu",
    extendedPluginAbilities: "extended_plugin_abilities",
    csp: "csp",
    quickCss: "quick_css",
};

/** Minimal native `settings.set` shape (closed allowlist, key echo + value). */
interface NativeSettingsSet {
    set: (
        key: string,
        value: boolean | string[] | string,
    ) => Promise<{ key: string; value: boolean | string[] | string }>;
}

/**
 * True when the native closed `settings.set` capability is the active path.
 * Gated on the existing `isNativeWebView` environment check AND function
 * presence — exactly one path executes per mutation.
 */
export function isNativeSettingsSetAvailable(): boolean {
    if (!isNativeWebView()) return false;
    const settings = (window.remcord as unknown as { settings?: Partial<NativeSettingsSet> }).settings;
    return typeof settings?.set === "function";
}

/**
 * Native WebView2 path for the closed `settings.set` allowlist. Updates local
 * store state OPTIMISTICALLY so every supported toggle reacts immediately,
 * then converges to the host-confirmed value; a rejected mutation reverts to
 * the previous value and rethrows so the caller can report honestly (the
 * caller owns the warning — this helper never warns itself, keeping exactly
 * one warning per failed mutation). The Electron path stays in
 * `setConfig`/`toggleMod` above, unchanged.
 */
export async function setNativeSetting<K extends NativeSettingsSetKey>(
    key: K,
    value: Settings[K],
    shouldRestart?: boolean,
): Promise<Settings[K]> {
    const previous = (store.settings as Record<string, unknown>)[key as string];
    // Immediate reaction: mutate in place — do NOT replace store.settings
    // (that remounts the page and jumps scroll).
    withPreservedSettingsScroll(() => {
        (store.settings as Record<string, unknown>)[key as string] = value;
    });
    const nativeSet = (window.remcord as unknown as { settings: NativeSettingsSet }).settings.set;
    let applied: { key: string; value: boolean | string[] | string };
    try {
        applied = await nativeSet(NATIVE_SETTINGS_SET_WIRE_KEY[key], value as boolean | string[] | string);
    } catch (error) {
        withPreservedSettingsScroll(() => {
            (store.settings as Record<string, unknown>)[key as string] = previous;
        });
        throw error;
    }
    withPreservedSettingsScroll(() => {
        // Converge to the host-confirmed value instead of holding stale data.
        (store.settings as Record<string, unknown>)[key] = applied.value;
        console.log(key, ":", store.settings[key]);
        if (shouldRestart) {
            markRestartRequired();
        }
    });
    return applied.value as Settings[K];
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
    const folders = (window.remcord as unknown as { folders?: NativeFolders }).folders;
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
        .catch(() => console.warn(`[Remcord] Could not reveal the ${location} folder in the native shell.`));
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
        .catch(() => console.warn("[Remcord] Could not open the theme folder in the native shell."));
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
    const themes = (window.remcord as unknown as { themes?: Record<string, unknown> }).themes;
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
    if (typeof listThemes !== "function") throw new Error("[Remcord] Native theme list is unavailable.");
    let raw: unknown;
    try {
        raw = await listThemes();
    } catch {
        throw new Error("[Remcord] Could not load themes in the native shell.");
    }
    if (!Array.isArray(raw)) throw new Error("[Remcord] Could not load themes in the native shell.");
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
    if (typeof setEnabled !== "function") throw new Error("[Remcord] Native theme toggle is unavailable.");
    try {
        await setEnabled(id, enabled);
    } catch {
        throw new Error("[Remcord] Could not update the theme in the native shell.");
    }
}

/**
 * Native host-owned import picker. Cancellation resolves `{ canceled: true }`
 * and stays silent; bridge failures resolve `{ canceled: false, failed: 1 }`
 * so the caller toasts an honest error instead of silence. Returns `null`
 * when the capability is absent (caller uses the Electron picker path) —
 * exactly one path per click.
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
        console.warn("[Remcord] Could not import themes in the native shell.");
        return { canceled: false, imported: 0, failed: 1 };
    }
}

/** One honest outcome per completed theme import: full success, a mix, or
 * total failure. Cancellation never reaches this helper — it stays silent.
 * Shared by the native picker summary and the Electron `themesChanged`
 * event so both shells toast identically. */
export type ThemeImportOutcome = "success" | "partial" | "failed";

export function themeImportOutcome(imported: number, failed: number): ThemeImportOutcome {
    if (failed === 0) return "success";
    return imported > 0 ? "partial" : "failed";
}

/** Bounded restore outcome shared by the native object shape and the parsed
 * Electron string shape. `error` is `"CANCELLED"`, a typed host code,
 * `"UNKNOWN"` (explicit failure without a message — the caller's generic
 * error state), or `"INVALID"` (unparseable/misshapen result — the caller's
 * invalid-file state). Never a path. */
export type BackupRestoreOutcome = { ok: true; clientMods: unknown } | { ok: false; error: string };

/**
 * Minimal native backup namespace (Phase D4-3). `remcordWindow.d.ts` stays
 * Electron-shaped (`restore(): Promise<string>`); the native object result
 * is described only here, at the single restore call site that consumes it.
 * No generic filesystem/dialog API.
 */
interface NativeBackupNamespace {
    save: (data: string) => Promise<{ ok: boolean; error?: string }>;
    restore?: () => Promise<unknown>;
}

/**
 * Native backup namespace when the WebView2 shell grants it, `undefined`
 * otherwise. Gated on BOTH the existing `isNativeWebView` environment check
 * AND function presence: an Electron `window.remcord.backup` also carries
 * `save`/`restore` functions (with the STRING restore shape), so the
 * environment check is what selects the native OBJECT result shape. No
 * second detection mechanism.
 */
function nativeBackup(): NativeBackupNamespace | undefined {
    if (!isNativeWebView()) return undefined;
    const backup = (window.remcord as unknown as { backup?: NativeBackupNamespace }).backup;
    if (!backup || typeof backup.save !== "function" || typeof backup.restore !== "function") {
        return undefined;
    }
    return backup;
}

/** True when the native backup capabilities are the active path. Save shares
 * the Electron signature in both shells; restore needs the object shape, so
 * the restore caller branches on this — exactly one path per click. */
export function isNativeBackupAvailable(): boolean {
    return nativeBackup() !== undefined;
}

/**
 * Backup save for both shells. Exactly one call per invocation: the native
 * facade preserves the Electron signature AND result shape (`{ok:true} |
 * {ok:false, error}`), so no branch is needed — the method on
 * `window.remcord.backup` is the native capability natively and the Electron
 * IPC bridge otherwise. The payload is built by the caller (clientMods stay
 * renderer-built, never move into Rust). Rejections propagate; the caller
 * shows the error state so clicks never produce unhandled rejections.
 */
export async function saveBackup(payload: string): Promise<{ ok: boolean; error?: string }> {
    const backup = (window.remcord as unknown as { backup: NativeBackupNamespace }).backup;
    return await backup.save(payload);
}

/**
 * Native backup restore. Gate with `isNativeBackupAvailable` first so
 * exactly one path executes per restore: the native OBJECT result here, the
 * Electron JSON-string path otherwise. Rejects when the capability is absent
 * (caller falls back); host failures resolve into the result shape via the
 * facade, transport rejections propagate to the caller's error state. The
 * host owns picker/validation/staging/commit; the caller only feeds the
 * returned clientMods into the existing renderer application path.
 */
export async function restoreBackupNative(): Promise<unknown> {
    const restore = nativeBackup()?.restore;
    if (typeof restore !== "function") throw new Error("[Remcord] Native backup restore is unavailable.");
    return await restore();
}

/**
 * Normalize the native object-shaped restore result. Never throws and never
 * surfaces paths: misshapen results become `{ ok: false, error: "INVALID" }`
 * for the caller's invalid-file state. `clientMods` passes through verbatim
 * for the existing renderer application path (bounded host-side already).
 */
export function normalizeNativeRestoreResult(result: unknown): BackupRestoreOutcome {
    if (!isRecord(result)) return { ok: false, error: "INVALID" };
    if (result.ok === true) return { ok: true, clientMods: result.clientMods };
    if (typeof result.error === "string" && result.error.length > 0) {
        return { ok: false, error: result.error };
    }
    return { ok: false, error: "INVALID" };
}

/**
 * Parse the Electron string-shaped restore result into the shared outcome.
 * Never throws. Preserves the existing renderer-visible semantics exactly:
 * unparseable input (or a misshapen document) becomes `"INVALID"` for the
 * caller's invalid-file state; an explicit failure without a message becomes
 * `"UNKNOWN"` for the generic error state; anything else carries its error
 * (including `"CANCELLED"`) or its clientMods verbatim.
 */
export function parseElectronRestoreResult(raw: unknown): BackupRestoreOutcome {
    if (typeof raw !== "string") return { ok: false, error: "INVALID" };
    let parsed: unknown;
    try {
        parsed = JSON.parse(raw) as unknown;
    } catch {
        return { ok: false, error: "INVALID" };
    }
    if (!isRecord(parsed)) return { ok: false, error: "INVALID" };
    if (parsed.ok === true) return { ok: true, clientMods: parsed.clientMods };
    if (typeof parsed.error === "string" && parsed.error.length > 0) {
        return { ok: false, error: parsed.error };
    }
    if (parsed.ok === false) return { ok: false, error: "UNKNOWN" };
    return { ok: false, error: "INVALID" };
}

function removeMod(array: ValidMods[], filter: ValidMods) {
    return array.filter((i) => i !== filter);
}

export function toggleMod(mod: ValidMods, enabled: boolean) {
    const currentMods = (store.settings.mods ?? []) as ValidMods[];
    // Native WebView2 persists through the closed async allowlist (immediate
    // optimistic reaction with revert-on-rejection inside `setNativeSetting`;
    // the restart flag is set only from the host-confirmed value); Electron
    // keeps the original synchronous setConfig.
    const writeMods = (next: ValidMods[]) => {
        if (isNativeSettingsSetAvailable()) {
            void setNativeSetting("mods", next, true).catch(() =>
                console.warn("[Remcord] Could not update mods in the native shell."),
            );
        } else if (isNativeWebView()) {
            // Native shell without the capability: route through the honest
            // setConfigNative path (optimistic with revert, restart flag only
            // on success) instead of setting a bogus restart flag up front.
            setConfig("mods", next, true);
        } else {
            markRestartRequired();
            setConfig("mods", next);
        }
    };
    if (enabled) {
        // Dedupe the id being enabled too: re-enabling an already-enabled
        // mod previously appended a duplicate entry (observed as
        // `["equicord", "equicord"]` in a real profile). Vencord and Equicord
        // stay mutually exclusive (both publish the `Vencord` page global);
        // shelter/custom are independent and dedupe without exclusions.
        if (mod === "vencord") {
            writeMods([...removeMod(removeMod(currentMods, "equicord"), "vencord"), "vencord"]);
        } else if (mod === "equicord") {
            writeMods([...removeMod(removeMod(currentMods, "vencord"), "equicord"), "equicord"]);
        } else {
            writeMods([...removeMod(currentMods, mod), mod]);
        }
    } else {
        writeMods(removeMod(currentMods, mod));
    }
}

export function isMinWindowsVersion(major: number, minor: number, build: number) {
    const [sys_major, sys_minor, sys_build] = window.remcord.osRelease
        .split(".")
        .map((val, _, __) => Number.parseInt(val, 10));
    return sys_major >= major && sys_minor >= minor && sys_build >= build;
}

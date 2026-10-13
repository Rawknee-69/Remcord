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
    "channel",
    "screenshareQuality",
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
        key === "quickCss" ||
        key === "channel" ||
        key === "screenshareQuality"
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
    | "quickCss"
    | "channel"
    | "screenshareQuality";

/** Wire keys for the closed native `settings.set` capability. */
const NATIVE_SETTINGS_SET_WIRE_KEY: Record<NativeSettingsSetKey, string> = {
    mods: "mods",
    supportBannerDismissed: "support_banner_dismissed",
    showExperimentalPluginMenu: "show_experimental_plugin_menu",
    extendedPluginAbilities: "extended_plugin_abilities",
    csp: "csp",
    quickCss: "quick_css",
    channel: "channel",
    screenshareQuality: "screenshare_quality",
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

/** Closed screenshare capture-quality vocabulary (native `screenshare_quality.rs` parity). */
export type ScreenshareQualityMode = "default" | "balanced" | "low";

const VALID_SCREENSHARE_QUALITY_MODES: ReadonlyArray<string> = ["default", "balanced", "low"];

/** Normalize one raw mode to the closed vocabulary (fail-safe to `"default"`). */
export function normalizeScreenshareQualityMode(raw: unknown): ScreenshareQualityMode {
    return typeof raw === "string" && VALID_SCREENSHARE_QUALITY_MODES.includes(raw)
        ? (raw as ScreenshareQualityMode)
        : "default";
}

/** Document-start status published by the native capture script (`window.__remcordScreenshareQuality`). */
export interface ScreenshareQualityStatus {
    mode: string;
    active: boolean;
    width: number | null;
    height: number | null;
    fps: number | null;
}

function isScreenshareQualityStatus(value: unknown): value is ScreenshareQualityStatus {
    if (!isRecord(value)) return false;
    if (typeof value.mode !== "string" || typeof value.active !== "boolean") return false;
    for (const key of ["width", "height", "fps"]) {
        const entry = value[key];
        if (entry !== null && (typeof entry !== "number" || !Number.isSafeInteger(entry))) return false;
    }
    return true;
}

/**
 * Reads the frozen document-start status the native capture script publishes.
 * Returns `null` outside the native shell (or when the script has not run):
 * the absence is reported, never a guessed mode. The `active` flag tells
 * whether this document constrains capture; requested targets are never
 * presented as achieved capture settings (actual `getSettings()` values are
 * logged to the console during sharing — see `screenshare_quality.rs`).
 */
export function readScreenshareQualityStatus(): ScreenshareQualityStatus | null {
    try {
        const status = (window as unknown as Record<string, unknown>).__remcordScreenshareQuality;
        if (status === undefined) return null;
        return isScreenshareQualityStatus(status) ? status : null;
    } catch {
        return null;
    }
}

/**
 * One honest status line for the Settings control: the persisted selection
 * plus what this document actually applies. Never claims an achieved capture
 * setting — only the selected mode and whether constraining is active.
 */
export function describeScreenshareQualityStatus(selected: unknown): string {
    const normalized = normalizeScreenshareQualityMode(selected);
    const status = readScreenshareQualityStatus();
    if (!isNativeWebView()) {
        return `Selected: ${normalized}. The capture control applies in the native Remcord app (needs a restart); this Electron session keeps its own share picker.`;
    }
    if (!status) {
        return `Selected: ${normalized}. Status unavailable in this document — restart the app to apply a changed mode.`;
    }
    if (!status.active) {
        return `Selected: ${normalized}. This document captures with default behavior (no constraint).`;
    }
    return `Selected: ${normalized}. This document targets up to ${status.width ?? "?"}x${status.height ?? "?"} at ${status.fps ?? "?"} FPS when sharing; Discord's stream-quality options are disabled while this target caps capture (clicking one explains how to revert). The actual captured size is logged to the console during sharing and may differ when WebView2 rejects the target.`;
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

/** Host-owned diagnostics facts (read-only `diagnostics.get_info` contract). */
export interface NativeDiagnosticsInfo {
    app_version: string;
    webview2_runtime: string;
    profile_kind: string;
    channel: string;
    csp: string;
    screenshare_quality: string;
    mods: string[];
}

/** Outcome of the native client-mod cache wipe (`mod.clear_cache`). */
export interface NativeModCacheOutcome {
    cleared: string[];
    mod_cache_cleared: boolean;
}

interface NativeDiagnosticsNamespace {
    getInfo: () => Promise<NativeDiagnosticsInfo>;
}

interface NativeModNamespace {
    clearCache: () => Promise<NativeModCacheOutcome>;
}

function nativeDiagnostics(): NativeDiagnosticsNamespace | undefined {
    if (!isNativeWebView()) return undefined;
    const diagnostics = (window.remcord as unknown as { diagnostics?: NativeDiagnosticsNamespace }).diagnostics;
    if (!diagnostics || typeof diagnostics.getInfo !== "function") return undefined;
    return diagnostics;
}

function nativeMod(): NativeModNamespace | undefined {
    if (!isNativeWebView()) return undefined;
    const mod = (window.remcord as unknown as { mod?: NativeModNamespace }).mod;
    if (!mod || typeof mod.clearCache !== "function") return undefined;
    return mod;
}

/** True when the native diagnostics readout is the active path. */
export function isNativeDiagnosticsAvailable(): boolean {
    return nativeDiagnostics() !== undefined;
}

/** True when the native client-mod cache wipe is the active path. */
export function isNativeModCacheAvailable(): boolean {
    return nativeMod() !== undefined;
}

function isDiagnosticsInfo(value: unknown): value is NativeDiagnosticsInfo {
    if (!isRecord(value)) return false;
    return (
        typeof value.app_version === "string" &&
        typeof value.webview2_runtime === "string" &&
        typeof value.profile_kind === "string" &&
        typeof value.channel === "string" &&
        typeof value.csp === "string" &&
        typeof value.screenshare_quality === "string" &&
        Array.isArray(value.mods) &&
        value.mods.every((entry) => typeof entry === "string")
    );
}

/**
 * Native diagnostics readout. Shape-checked renderer-side (the host owns
 * value validation; this only guards the UI against malformed bridge
 * data). Rejects with a path-free Error on host failure or misshapen data.
 */
export async function getNativeDiagnosticsInfo(): Promise<NativeDiagnosticsInfo> {
    const namespace = nativeDiagnostics();
    if (!namespace) throw new Error("[Remcord] Native diagnostics are unavailable.");
    let raw: unknown;
    try {
        raw = await namespace.getInfo();
    } catch {
        throw new Error("[Remcord] Could not load diagnostics in the native shell.");
    }
    if (!isDiagnosticsInfo(raw)) throw new Error("[Remcord] Could not load diagnostics in the native shell.");
    return raw;
}

/** One GPU device from the host CDP report (`null` = not reported). */
export interface NativeGraphicsDevice {
    vendor_id: string | null;
    device_id: string | null;
    vendor_string: string | null;
    device_string: string | null;
    driver_version: string | null;
}

export interface NativeGraphicsFeature {
    name: string;
    status: string;
}

export interface NativeGraphicsBrowser {
    status: "measured" | "unsupported" | "unavailable";
    method: string;
    product: string | null;
    revision: string | null;
    user_agent: string | null;
    js_version: string | null;
    protocol_version: string | null;
    error: string | null;
}

export interface NativeGraphicsSystem {
    status: "measured" | "unsupported" | "unavailable";
    method: string;
    model_name: string | null;
    devices: NativeGraphicsDevice[];
    feature_status: NativeGraphicsFeature[];
    command_line_flags: string[];
    disable_features: string | null;
    enable_features: string | null;
    error: string | null;
}

/** CDP-backed graphics report (read-only `graphics.get_info` contract). */
export interface NativeGraphicsInfo {
    webview2_runtime: string;
    additional_browser_arguments: string;
    environment: string;
    browser: NativeGraphicsBrowser;
    system: NativeGraphicsSystem;
}

interface NativeGraphicsNamespace {
    getInfo: () => Promise<NativeGraphicsInfo>;
}

function nativeGraphics(): NativeGraphicsNamespace | undefined {
    if (!isNativeWebView()) return undefined;
    const graphics = (window.remcord as unknown as { graphics?: NativeGraphicsNamespace }).graphics;
    if (!graphics || typeof graphics.getInfo !== "function") return undefined;
    return graphics;
}

/** True when the CDP-backed native graphics report is the active path. */
export function isNativeGraphicsAvailable(): boolean {
    return nativeGraphics() !== undefined;
}

function isGraphicsStatus(value: unknown): value is NativeGraphicsBrowser["status"] {
    return value === "measured" || value === "unsupported" || value === "unavailable";
}

function isGraphicsDevice(value: unknown): value is NativeGraphicsDevice {
    if (!isRecord(value)) return false;
    for (const key of ["vendor_id", "device_id", "vendor_string", "device_string", "driver_version"]) {
        const entry = value[key];
        if (entry !== null && typeof entry !== "string") return false;
    }
    return true;
}

function isGraphicsInfo(value: unknown): value is NativeGraphicsInfo {
    if (!isRecord(value)) return false;
    if (
        typeof value.webview2_runtime !== "string" ||
        typeof value.additional_browser_arguments !== "string" ||
        typeof value.environment !== "string"
    ) {
        return false;
    }
    const browser = value.browser;
    const system = value.system;
    if (!isRecord(browser) || !isRecord(system)) return false;
    if (!isGraphicsStatus(browser.status) || !isGraphicsStatus(system.status)) return false;
    for (const key of ["product", "revision", "user_agent", "js_version", "protocol_version", "error"]) {
        if (browser[key] !== null && typeof browser[key] !== "string") return false;
    }
    if (system.model_name !== null && typeof system.model_name !== "string") return false;
    if (
        !Array.isArray(system.devices) ||
        !system.devices.every(isGraphicsDevice) ||
        !Array.isArray(system.feature_status) ||
        !system.feature_status.every(
            (entry) => isRecord(entry) && typeof entry.name === "string" && typeof entry.status === "string",
        ) ||
        !Array.isArray(system.command_line_flags) ||
        !system.command_line_flags.every((flag) => typeof flag === "string")
    ) {
        return false;
    }
    for (const key of ["disable_features", "enable_features", "error"]) {
        if (system[key] !== null && typeof system[key] !== "string") return false;
    }
    return true;
}

/**
 * Native CDP-backed graphics report. Shape-checked renderer-side (the host
 * owns value validation; this only guards the UI against malformed bridge
 * data). Rejects with a path-free Error on host failure or misshapen data.
 */
export async function getNativeGraphicsInfo(): Promise<NativeGraphicsInfo> {
    const namespace = nativeGraphics();
    if (!namespace) throw new Error("[Remcord] Native graphics diagnostics are unavailable.");
    let raw: unknown;
    try {
        raw = await namespace.getInfo();
    } catch {
        throw new Error("[Remcord] Could not load graphics diagnostics in the native shell.");
    }
    if (!isGraphicsInfo(raw)) throw new Error("[Remcord] Could not load graphics diagnostics in the native shell.");
    return raw;
}

/** Renderer-observed WebGL adapter string (`null` = unobservable). */
export interface NativeWebGLObservation {
    status: "measured" | "unavailable";
    renderer: string | null;
    error: string | null;
}

interface WebGLProbeContext {
    getParameter: (name: number) => unknown;
    getExtension: (name: string) => { UNMASKED_RENDERER_WEBGL: number } | null;
    RENDERER: number;
}

/**
 * Observes the active WebGL renderer string locally (no host call, no
 * permissions): the UNMASKED_RENDERER_WEBGL adapter name when the debug
 * extension is available, else the masked RENDERER value. An ANGLE string
 * names the active adapter; a SwiftShader/software string would mean
 * software rendering. Never throws: failures report as unavailable.
 */
export function observeWebGLRenderer(): NativeWebGLObservation {
    try {
        const canvas = document.createElement("canvas");
        let context: WebGLProbeContext | null = null;
        for (const kind of ["webgl2", "webgl", "experimental-webgl"] as const) {
            const candidate = canvas.getContext(kind) as unknown as WebGLProbeContext | null;
            if (candidate && typeof candidate.getParameter === "function") {
                context = candidate;
                break;
            }
        }
        if (!context) return { status: "unavailable", renderer: null, error: "no WebGL context" };
        const debugInfo = context.getExtension("WEBGL_debug_renderer_info");
        const unmasked = debugInfo ? context.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL) : null;
        const masked = context.getParameter(context.RENDERER);
        const renderer =
            typeof unmasked === "string" && unmasked.length > 0
                ? unmasked
                : typeof masked === "string" && masked.length > 0
                  ? masked
                  : null;
        if (!renderer) return { status: "unavailable", renderer: null, error: "renderer string not reported" };
        return { status: "measured", renderer: renderer.slice(0, 512), error: null };
    } catch {
        return { status: "unavailable", renderer: null, error: "observation failed" };
    }
}

/**
 * Copies host diagnostics text to the clipboard through
 * `navigator.clipboard` (renderer-side write — no host clipboard surface).
 * Resolves `true` on success, `false` when the clipboard write or the host
 * readout fails. Exactly one path per click: native readout, else Electron.
 */
export async function copyNativeDiagnostics(kind: "debug" | "gpu"): Promise<boolean> {
    let text: string;
    try {
        text = kind === "gpu" ? await buildNativeGraphicsText() : await buildNativeDebugText();
    } catch {
        console.warn("[Remcord] Could not load diagnostics in the native shell.");
        return false;
    }
    try {
        await navigator.clipboard.writeText(text);
        return true;
    } catch {
        console.warn("[Remcord] Could not write diagnostics to the clipboard.");
        return false;
    }
}

async function buildNativeDebugText(): Promise<string> {
    const info = await getNativeDiagnosticsInfo();
    return [
        `Remcord ${info.app_version} (native WebView2 app)`,
        `WebView2 Runtime: ${info.webview2_runtime}`,
        `Profile: ${info.profile_kind}`,
        `Channel: ${info.channel}`,
        `CSP: ${info.csp}`,
        `Screenshare quality: ${info.screenshare_quality}`,
        `Mods: ${info.mods.length > 0 ? info.mods.join(", ") : "(none)"}`,
    ].join("\n");
}

/** Static graphics text: used only when the CDP-backed probe is unavailable. */
function legacyGraphicsText(info: NativeDiagnosticsInfo): string {
    return [
        `Graphics stack: WebView2 Runtime ${info.webview2_runtime}`,
        `Renderer: WebView2 (Chromium, system default settings)`,
        `App: Remcord ${info.app_version} (profile: ${info.profile_kind}, channel: ${info.channel})`,
    ].join("\n");
}

/**
 *Unavailable details for one probe section (`measured` sections never call
 * this): the explicit status plus the bounded host reason when present.
 */
function graphicsSectionStatus(status: string, error: string | null): string {
    return error ? `${status} (${error})` : status;
}

function formatNativeGraphicsText(
    appVersion: string,
    graphics: NativeGraphicsInfo,
    webgl: NativeWebGLObservation,
): string {
    const lines = [
        `Graphics report (Remcord ${appVersion}, WebView2 Runtime ${graphics.webview2_runtime})`,
        `Environment: ${graphics.environment}`,
        `Additional browser arguments: ${graphics.additional_browser_arguments || "(WebView2 defaults)"}`,
    ];
    const browser = graphics.browser;
    if (browser.status === "measured") {
        lines.push(`Browser (measured via Browser.getVersion): ${browser.product ?? "(product not reported)"}`);
        if (browser.js_version) lines.push(`JS version: ${browser.js_version}`);
    } else {
        lines.push(`Browser details: ${graphicsSectionStatus(browser.status, browser.error)}`);
    }
    const system = graphics.system;
    if (system.status === "measured") {
        if (system.devices.length === 0) {
            lines.push("GPUs (measured via System.getInfo): (no devices reported)");
        } else {
            system.devices.forEach((device, index) => {
                const label = device.device_string ?? device.vendor_string ?? "(unnamed adapter)";
                const ids = [device.vendor_id, device.device_id].filter((id) => id !== null).join(":");
                lines.push(
                    `GPU ${index}: ${label}${ids ? ` [${ids}]` : ""}${device.driver_version ? ` (driver ${device.driver_version})` : ""}`,
                );
            });
        }
        lines.push(
            system.feature_status.length > 0
                ? `Feature status: ${system.feature_status.map((entry) => `${entry.name}=${entry.status}`).join(", ")}`
                : "Feature status: (not reported)",
        );
        lines.push(
            `Active command-line flags: ${system.command_line_flags.length > 0 ? system.command_line_flags.join(", ") : "(none of the tracked GPU flags present)"}`,
        );
        if (system.disable_features) lines.push(`disable-features: ${system.disable_features}`);
        if (system.enable_features) lines.push(`enable-features: ${system.enable_features}`);
        lines.push("Note: adapter availability is not active utilization; light workloads need little GPU time.");
    } else {
        lines.push(`GPU details: ${graphicsSectionStatus(system.status, system.error)}`);
    }
    lines.push(
        webgl.status === "measured"
            ? `WebGL renderer (renderer-observed): ${webgl.renderer}`
            : `WebGL renderer: unavailable (${webgl.error ?? "unknown"})`,
    );
    return lines.join("\n");
}

/**
 * CDP-backed GPU copy text: the measured graphics report when the probe
 * succeeds, the legacy static text when the probe or its namespace is
 * unavailable. Never rejects for a probe failure — the click still copies
 * the honest fallback.
 */
async function buildNativeGraphicsText(): Promise<string> {
    const info = await getNativeDiagnosticsInfo();
    const webgl = observeWebGLRenderer();
    let graphics: NativeGraphicsInfo;
    try {
        graphics = await getNativeGraphicsInfo();
    } catch {
        return `${legacyGraphicsText(info)}\n${webgl.status === "measured" ? `WebGL renderer (renderer-observed): ${webgl.renderer}` : `WebGL renderer: unavailable (${webgl.error ?? "unknown"})`}`;
    }
    return formatNativeGraphicsText(info.app_version, graphics, webgl);
}

/**
 * Native client-mod cache wipe. The host deletes its downloaded bundle
 * caches and resets the stored `modCache` map; user-owned custom bundles
 * are never touched. Resolves the host outcome so the caller can report
 * what was cleared; rejects with a path-free Error on host failure.
 */
export async function clearNativeModCache(): Promise<NativeModCacheOutcome> {
    const namespace = nativeMod();
    if (!namespace) throw new Error("[Remcord] Native mod cache clearing is unavailable.");
    try {
        const outcome = await namespace.clearCache();
        if (
            !isRecord(outcome) ||
            !Array.isArray(outcome.cleared) ||
            !outcome.cleared.every((entry) => typeof entry === "string") ||
            typeof outcome.mod_cache_cleared !== "boolean"
        ) {
            throw new Error("invalid outcome");
        }
        return { cleared: [...outcome.cleared], mod_cache_cleared: outcome.mod_cache_cleared };
    } catch {
        throw new Error("[Remcord] Could not clear the client-mod cache in the native shell.");
    }
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
 * Minimal native backup namespace. `remcordWindow.d.ts` stays
 * Electron-shaped (`restore(): Promise<string>`); the native trio result
 * shapes are described only here, at the single restore call site that
 * consumes them. No generic filesystem/dialog API.
 */
interface NativeBackupNamespace {
    save: (data: string) => Promise<{ ok: boolean; error?: string }>;
    inspect?: () => Promise<unknown>;
    commitRestore?: () => Promise<unknown>;
    discardRestore?: () => Promise<unknown>;
}

/**
 * Native backup namespace when the WebView2 shell grants it, `undefined`
 * otherwise. Gated on BOTH the existing `isNativeWebView` environment check
 * AND function presence: an Electron `window.remcord.backup` also carries
 * backup functions (with the STRING restore shape), so the environment
 * check is what selects the native OBJECT result shapes. All three trio
 * members must be present — a partial namespace falls back safely. No
 * second detection mechanism.
 */
function nativeBackup(): NativeBackupNamespace | undefined {
    if (!isNativeWebView()) return undefined;
    const backup = (window.remcord as unknown as { backup?: NativeBackupNamespace }).backup;
    if (
        !backup ||
        typeof backup.save !== "function" ||
        typeof backup.inspect !== "function" ||
        typeof backup.commitRestore !== "function" ||
        typeof backup.discardRestore !== "function"
    ) {
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
 * Native backup inspect: the host-owned picker plus validation and staging.
 * Gate with `isNativeBackupAvailable` first so exactly one path executes
 * per restore. Resolves the raw preview payload (or the canceled shape);
 * the caller normalizes via `normalizeNativeInspectResult`. The staged
 * bytes stay held host-side until `commitRestoreNative` or
 * `discardRestoreNative` resolves the hold. Rejects when the capability is
 * absent (caller falls back); host failures resolve into the result shape
 * via the facade, transport rejections propagate to the caller's error
 * state.
 */
export async function inspectBackupNative(): Promise<unknown> {
    const inspect = nativeBackup()?.inspect;
    if (typeof inspect !== "function") throw new Error("[Remcord] Native backup inspect is unavailable.");
    return await inspect();
}

/**
 * Native backup commit: applies exactly the held staged bytes the preview
 * described. The caller feeds the returned clientMods into the existing
 * renderer application path.
 */
export async function commitRestoreNative(): Promise<unknown> {
    const commit = nativeBackup()?.commitRestore;
    if (typeof commit !== "function") throw new Error("[Remcord] Native backup commit is unavailable.");
    return await commit();
}

/**
 * Native backup discard: drops the held staged restore without touching
 * the profile. Best-effort by design (a failed discard only orphans bounded
 * host temp state); callers never surface it as an error.
 */
export async function discardRestoreNative(): Promise<boolean> {
    try {
        const discard = nativeBackup()?.discardRestore;
        if (typeof discard !== "function") return false;
        const result = (await discard()) as { discarded?: unknown };
        return result?.discarded === true;
    } catch {
        return false;
    }
}

/** User-facing preview of one held staged restore (normalized, never throwing). */
export interface NativeRestorePreview {
    includes: Record<string, boolean>;
    archiveBytes: number;
    uncompressedBytes: number;
    entryCount: number;
    clientModsBytes: number;
    hasVencordModData: boolean;
    hasEquicordModData: boolean;
    hasShelterModData: boolean;
    entries: string[];
    entriesTruncated: boolean;
}

export type NativeInspectOutcome =
    | { canceled: true }
    | { preview: NativeRestorePreview }
    | { error: string }
    | { invalid: true };

function isPreviewIncludes(value: unknown): value is Record<string, boolean> {
    if (!isRecord(value)) return false;
    const keys = [
        "remcordConfig",
        "remcordThemesAndQuickCss",
        "remcordExtensionPlugins",
        "vencordModData",
        "equicordModData",
        "shelterModData",
        "modBundles",
    ];
    return Object.keys(value).length === keys.length && keys.every((key) => typeof value[key] === "boolean");
}

function isNonNegativeInt(value: unknown): value is number {
    return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

/**
 * Normalize the native inspect outcome. Never throws and never surfaces
 * paths: cancellation stays silent (`canceled`), typed host failures carry
 * their code (`error`), and misshapen results become `{ invalid: true }`
 * for the caller's invalid-file state. A valid preview passes through with
 * bounded display fields only (entry names are capped for rendering; the
 * authoritative total rides in `entryCount`).
 */
export function normalizeNativeInspectResult(result: unknown): NativeInspectOutcome {
    if (!isRecord(result)) return { invalid: true };
    if (result.ok === false) {
        if (result.error === "CANCELLED") return { canceled: true };
        if (typeof result.error === "string" && result.error.length > 0) return { error: result.error };
        return { invalid: true };
    }
    if (result.ok !== true) return { invalid: true };
    const preview = result.preview;
    if (
        !isRecord(preview) ||
        Object.keys(preview).length !== 11 ||
        preview.canceled !== false ||
        !isPreviewIncludes(preview.includes) ||
        !isNonNegativeInt(preview.archive_bytes) ||
        !isNonNegativeInt(preview.uncompressed_bytes) ||
        !isNonNegativeInt(preview.entry_count) ||
        !isNonNegativeInt(preview.client_mods_bytes) ||
        typeof preview.has_vencord_mod_data !== "boolean" ||
        typeof preview.has_equicord_mod_data !== "boolean" ||
        typeof preview.has_shelter_mod_data !== "boolean" ||
        !Array.isArray(preview.entries) ||
        preview.entries.length > 128 ||
        !preview.entries.every((entry) => typeof entry === "string" && entry.length > 0 && entry.length <= 512) ||
        typeof preview.entries_truncated !== "boolean"
    ) {
        return { invalid: true };
    }
    return {
        preview: {
            includes: preview.includes as unknown as Record<string, boolean>,
            archiveBytes: preview.archive_bytes as unknown as number,
            uncompressedBytes: preview.uncompressed_bytes as unknown as number,
            entryCount: preview.entry_count as unknown as number,
            clientModsBytes: preview.client_mods_bytes as unknown as number,
            hasVencordModData: preview.has_vencord_mod_data as unknown as boolean,
            hasEquicordModData: preview.has_equicord_mod_data as unknown as boolean,
            hasShelterModData: preview.has_shelter_mod_data as unknown as boolean,
            entries: (preview.entries as unknown as string[]).slice(0, 128),
            entriesTruncated: preview.entries_truncated as unknown as boolean,
        },
    };
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

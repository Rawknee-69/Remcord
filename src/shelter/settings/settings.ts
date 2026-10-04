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

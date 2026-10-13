import type { QuickCssEditorMode } from "./quickCssEditorSource.js";

/** Which CSS owner the editor modal edits. */
export type QuickCssEditorMode = "vencord" | "equicord" | "remcord";

/** Vencord-family QuickCSS storage API, as the web builds publish it. */
export interface ModQuickCssApi {
    get: () => Promise<string>;
    set: (css: string) => Promise<void>;
}

/** Narrow native QuickCSS editor capability pair (fixed file only). */
export interface NativeQuickCssNamespace {
    read: () => Promise<{ css: string; bytes: number }>;
    write: (css: string) => Promise<{ bytes: number }>;
}

/** Editor content source: the initial text plus the owning save function. */
export interface QuickCssEditorSource {
    initialCss: string;
    saveCss: (css: string) => Promise<void>;
}

export function editorTitle(mode: QuickCssEditorMode): string {
    if (mode === "equicord") return "Equicord QuickCSS";
    if (mode === "remcord") return "Remcord QuickCSS";
    return "Vencord QuickCSS";
}

export function modQuickCssApi(): ModQuickCssApi | null {
    const api = (window as unknown as { VencordNative?: { quickCss?: Partial<ModQuickCssApi> } }).VencordNative
        ?.quickCss;
    if (!api || typeof api.get !== "function" || typeof api.set !== "function") return null;
    return { get: api.get.bind(api), set: api.set.bind(api) };
}

export function nativeQuickCssApi(): NativeQuickCssNamespace | null {
    const namespace = (window.remcord as unknown as { quickcss?: Partial<NativeQuickCssNamespace> }).quickcss;
    if (!namespace || typeof namespace.read !== "function" || typeof namespace.write !== "function") return null;
    return {
        read: namespace.read.bind(namespace),
        write: namespace.write.bind(namespace),
    };
}

function honestError(failure: unknown, fallback: string): Error {
    if (failure instanceof Error && failure.message) return failure;
    if (typeof failure === "string" && failure) return new Error(failure);
    return new Error(fallback);
}

/**
 * Resolves the editor content source for one owner. Read failures throw the
 * real error: callers must toast it and never open an empty editor over
 * existing content (a save from an empty buffer would destroy it). A
 * missing storage API throws an explicit unavailability error instead of a
 * TypeError off a missing member.
 */
export async function loadQuickCssEditorSource(mode: QuickCssEditorMode): Promise<QuickCssEditorSource> {
    if (mode === "remcord") {
        const api = nativeQuickCssApi();
        if (!api) throw new Error("The native QuickCSS editor is unavailable in this shell.");
        let initial: string;
        try {
            initial = (await api.read()).css;
        } catch (failure) {
            throw honestError(failure, "Could not read QuickCSS.");
        }
        return {
            initialCss: initial,
            saveCss: (css: string) => api.write(css).then(() => undefined),
        };
    }
    const api = modQuickCssApi();
    if (!api) throw new Error("The mod's QuickCSS storage is unavailable (the mod is not running).");
    let initial: string;
    try {
        initial = (await api.get()) ?? "";
    } catch (failure) {
        throw honestError(failure, "Could not read QuickCSS.");
    }
    return { initialCss: initial, saveCss: (css: string) => api.set(css) };
}

import { createSignal, Show } from "solid-js";
import type { QuickCssEditorMode } from "../quickCssEditorSource.js";

const {
    ui: { Button, ButtonSizes, Header, HeaderTags, Text, TextTags, showToast },
} = shelter;

const MODE_TITLE: Record<QuickCssEditorMode, string> = {
    vencord: "Vencord QuickCSS",
    equicord: "Equicord QuickCSS",
    remcord: "Remcord QuickCSS",
};

/**
 * In-Discord QuickCSS editor modal. The upstream Vencord-family popup editor
 * cannot work in the native shell (the host denies about:blank popups at the
 * WebView2 layer and manages its own child, so the opener's window proxy is
 * dead), and the Remcord editor button had no native path either. This modal
 * edits wherever the CSS actually lives and saves through the owning API —
 * nothing is faked:
 *
 * - vencord/equicord: reads via `VencordNative.quickCss.get()` and persists
 *   via `VencordNative.quickCss.set()`, which live-updates the mod and its
 *   IndexedDB store. Failures keep the modal open with the real error; a
 *   cancelled edit toasts nothing.
 * - remcord: reads/writes the host-owned `quickCss.css` through the narrow
 *   `quickcss.read`/`quickcss.write` capabilities; the host re-pushes the
 *   managed style live on save.
 */
export function QuickCssEditorModal(props: {
    mode: QuickCssEditorMode;
    initialCss: string;
    saveCss: (css: string) => Promise<void>;
    close: () => void;
}) {
    const [css, setCss] = createSignal(props.initialCss);
    const [saving, setSaving] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    async function handleSave() {
        if (saving()) return;
        setSaving(true);
        setError(null);
        try {
            await props.saveCss(css());
            showToast({
                title: MODE_TITLE[props.mode],
                content: "QuickCSS saved.",
                duration: 3000,
            });
            props.close();
        } catch (failure) {
            // Stay open with the real error: closing on failure would look
            // like a successful save of lost edits.
            const message = failure instanceof Error ? failure.message : String(failure);
            setError(message || "Could not save QuickCSS.");
        } finally {
            setSaving(false);
        }
    }

    return (
        <div style={{ display: "flex", "flex-direction": "column", gap: "12px", "min-width": "min(560px, 80vw)" }}>
            <Header tag={HeaderTags.HeadingLG}>{MODE_TITLE[props.mode]}</Header>
            <Text tag={TextTags.textSM}>
                {props.mode === "remcord"
                    ? "Edits the Remcord QuickCSS file. Saving applies immediately."
                    : "Edits the mod's QuickCSS. Saving applies immediately and persists with the mod's settings."}
            </Text>
            <textarea
                value={css()}
                onInput={(event) => setCss(event.currentTarget.value)}
                spellcheck={false}
                rows={18}
                style={{
                    width: "100%",
                    "font-family": "Consolas, monospace",
                    "font-size": "12px",
                    "white-space": "pre",
                    resize: "vertical",
                }}
            />
            <Show when={error()}>
                <Text tag={TextTags.textSM} style={{ color: "#f04747" }}>
                    {error()}
                </Text>
            </Show>
            <div style={{ display: "flex", gap: "8px", "justify-content": "flex-end" }}>
                <Button size={ButtonSizes.MEDIUM} onClick={() => props.close()} disabled={saving()}>
                    Cancel
                </Button>
                <Button size={ButtonSizes.MEDIUM} onClick={() => void handleSave()} disabled={saving()}>
                    {saving() ? "Saving…" : "Save"}
                </Button>
            </div>
        </div>
    );
}

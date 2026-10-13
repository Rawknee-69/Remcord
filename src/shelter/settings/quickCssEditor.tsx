import { QuickCssEditorModal } from "./components/QuickCssEditorModal.jsx";
import {
    editorTitle,
    loadQuickCssEditorSource,
    type QuickCssEditorMode,
    type QuickCssEditorSource,
} from "./quickCssEditorSource.js";

const {
    ui: { openModal, showToast },
} = shelter;

/**
 * Opens the in-Discord QuickCSS editor for one owner. Source-resolution
 * failures toast the real error and never open (see
 * `loadQuickCssEditorSource`); save failures surface inside the modal,
 * which stays open.
 */
export async function openQuickCssEditorModal(mode: QuickCssEditorMode): Promise<void> {
    let source: QuickCssEditorSource;
    try {
        source = await loadQuickCssEditorSource(mode);
    } catch (failure) {
        const message = failure instanceof Error ? failure.message : String(failure);
        showToast({ title: editorTitle(mode), content: message, duration: 5000 });
        return;
    }
    openModal(({ close }: { close: () => void }) => (
        <QuickCssEditorModal mode={mode} initialCss={source.initialCss} saveCss={source.saveCss} close={close} />
    ));
}

// dirty hack to replace Vencord's quick css editor with Remcord's
// fixes the white window bug

if (window.VencordNative) {
    Vencord.Settings.useQuickCss = false;
    let settings = VencordNative.settings.get()
    settings.useQuickCss = false;
    VencordNative.settings.set(settings)
    VencordNative.quickCss.openEditor = function openEditor() {
        shelter.ui.openConfirmationModal({
            header: () => "Vencord QuickCSS is not compatible",
            body: () => `Vencord’s QuickCSS editor is not supported in Remcord. Remcord uses its own Quick CSS editor instead which can be found in it's respective themes section.
                Would you like to import your existing Vencord CSS into Remcord?`,
            type: "danger",
            confirmText: "Import CSS",
            cancelText: "Cancel"
            }).then(    
            async () => {
                const css = await VencordNative.quickCss.get();
                window.remcord.themes.importQuickCss(css);
                window.remcord.themes.openQuickCss()
            },
            () => console.log("Cancel.")
        );
        
    }
}
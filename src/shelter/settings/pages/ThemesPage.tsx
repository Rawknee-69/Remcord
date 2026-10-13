import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { Settings } from "../../../@types/settings.js";
import type { ThemeManifest } from "../../../@types/themeManifest.js";
import { EmptyState } from "../components/EmptyState.jsx";
import { SettingsPageHeader } from "../components/SettingsPageHeader.jsx";
import { ThemesCard } from "../components/ThemesCard.jsx";
import { openQuickCssEditorModal } from "../quickCssEditor.jsx";
import {
    importNativeThemes,
    isNativeThemeImportAvailable,
    isNativeThemeListAvailable,
    isNativeThemesMode,
    refreshNativeThemes,
    refreshThemes,
    revealFolderNative,
    setConfig,
    themeImportOutcome,
} from "../settings.js";
import classes from "./ThemesPages.module.css";

const {
    ui: { Button, Header, HeaderTags, ButtonSizes, TextBox, showToast, SwitchItem, Text, TextTags },
    plugin: { store },
} = shelter;

const BETTERDISCORD_THEMES_URL = "https://betterdiscord.app/themes";

export function ThemesPage() {
    const [downloadUrl, setDownloadUrl] = createSignal("");
    const [nativeLoading, setNativeLoading] = createSignal(false);
    const [nativeError, setNativeError] = createSignal(false);
    const nativeMode = () => isNativeThemesMode();

    async function loadThemesOnce(): Promise<void> {
        if (!isNativeThemeListAvailable()) {
            refreshThemes();
            return;
        }
        setNativeLoading(true);
        setNativeError(false);
        try {
            await refreshNativeThemes();
        } catch {
            console.warn("[Remcord] Could not load themes in the native shell.");
            setNativeError(true);
        } finally {
            setNativeLoading(false);
        }
    }

    onMount(() => {
        void loadThemesOnce();
        // Electron-only completion event (the native facade has no picker
        // callback surface): guard on function presence, not just on the
        // absent native capability, so an older native host can never throw.
        if (!isNativeThemeImportAvailable() && typeof window.remcord.themes.onThemesChanged === "function") {
            const dispose = window.remcord.themes.onThemesChanged((summary) => {
                refreshThemes();
                showImportToasts(summary.imported, summary.failed);
            });
            onCleanup(dispose);
        }
    });
    if (!isNativeThemeListAvailable()) {
        refreshThemes();
    }

    function showImportToasts(imported: number, failed: number) {
        switch (themeImportOutcome(imported, failed)) {
            case "success":
                showToast({
                    title: store.i18n["themes-success"],
                    content: store.i18n["themes-bdInstalled"],
                    duration: 3000,
                });
                break;
            case "partial":
                showToast({
                    title: store.i18n["themes-success"],
                    content: `${store.i18n["themes-bdInstalled"]} (${
                        store.i18n["themes-importPartial"] ??
                        "Some themes failed to import. Valid themes were installed; check the list."
                    })`,
                    duration: 4000,
                });
                break;
            case "failed":
                showToast({
                    title: store.i18n["themes-success"],
                    content:
                        store.i18n["themes-importFailed"] ??
                        "No themes could be imported. The files were not valid BetterDiscord themes.",
                    duration: 4000,
                });
                break;
        }
    }

    function installTheme() {
        // The bridge rejects when the URL fetch or the theme install fails:
        // toast success only on resolve so a failure never looks installed.
        void window.remcord.themes.install(downloadUrl()).then(
            () => {
                setDownloadUrl("");
                refreshThemes();
                showToast({
                    title: store.i18n["themes-success"],
                    content: store.i18n["themes-bdInstalled"],
                    duration: 3000,
                });
            },
            () => {
                setDownloadUrl("");
                refreshThemes();
                showToast({
                    title: store.i18n["themes-success"],
                    content:
                        store.i18n["themes-importFailed"] ??
                        "No themes could be imported. The files were not valid BetterDiscord themes.",
                    duration: 4000,
                });
            },
        );
    }

    async function handleRefresh() {
        if (isNativeThemeListAvailable()) {
            await loadThemesOnce();
            return;
        }
        store.themes = window.remcord.themes.refresh();
    }

    async function handleImport() {
        if (isNativeThemeImportAvailable()) {
            const result = await importNativeThemes();
            // Capability absent is the only `null`; user cancellation stays
            // silent. Any completed pick reports honestly: a full failure
            // toasts an error (previously this returned silently, looking
            // like a broken importer), a partial import names both counts.
            if (!result || result.canceled) return;
            await loadThemesOnce();
            showImportToasts(result.imported, result.failed);
            return;
        }
        window.remcord.themes.openImportPicker();
    }

    const settings = () => store.settings as Settings;
    const t = store.i18n;
    const themes = () => (store.themes as ThemeManifest[]) ?? [];

    function handleOpenQuickCss() {
        // Native WebView2 edits inside Discord through the Remcord editor
        // modal (the upstream about:blank popup flow is dead under the
        // native managed-child popup policy); Electron keeps the original
        // popup call.
        if (nativeMode()) {
            void openQuickCssEditorModal("remcord").catch(() =>
                console.warn("[Remcord] Could not open the QuickCSS editor in the native shell."),
            );
            return;
        }
        window.remcord.themes.openQuickCss();
    }

    return (
        <>
            <SettingsPageHeader title={t["themes-pageTitle"]} description={t["themes-pageDesc"]} divider={true} />
            <SwitchItem
                note={store.i18n["settings-quickCss-desc"]}
                value={settings().quickCss}
                onChange={(e: boolean) => {
                    console.log("Toggled quick CSS", e);
                    // Native WebView2 persists through the closed async
                    // allowlist and the host re-pushes the managed style
                    // live (no restart needed); Electron keeps the original
                    // enable/disable calls plus synchronous setConfig. The
                    // native facade has no enableQuickCss/disableQuickCss,
                    // so calling them first would throw before persisting.
                    if (nativeMode()) {
                        setConfig("quickCss", e);
                        return;
                    }
                    if (e) {
                        window.remcord.themes.enableQuickCss();
                    } else {
                        window.remcord.themes.disableQuickCss();
                    }
                    setConfig("quickCss", e);
                }}
            >
                {store.i18n["settings-quickCss"]}
            </SwitchItem>
            <div class={classes.toolbar}>
                <Button
                    size={ButtonSizes.LARGE}
                    onClick={() => void handleOpenQuickCss()}
                    disabled={!settings().quickCss}
                >
                    {t["themes-openQuickCss"]}
                </Button>
                <Button size={ButtonSizes.LARGE} onClick={() => void handleImport()}>
                    {t["themes-importFromFile"]}
                </Button>
                <Button
                    size={ButtonSizes.LARGE}
                    onClick={() => {
                        // Native WebView2 reveals via the typed folders.open capability;
                        // Electron keeps the original fire-and-forget send.
                        if (!revealFolderNative("themes")) window.remcord.settings.openThemesFolder();
                    }}
                >
                    {t["themes-openThemesFolder"]}
                </Button>
                <Button size={ButtonSizes.LARGE} onClick={() => void handleRefresh()}>
                    {t["themes-refresh"]}
                </Button>
            </div>
            <Show when={!nativeMode()}>
                <div class={classes.addBox}>
                    <TextBox
                        value={downloadUrl()}
                        onInput={setDownloadUrl}
                        placeholder={t["themes-importUrlPlaceholder"]}
                    />
                    <Button size={ButtonSizes.MEDIUM} onClick={installTheme}>
                        {t["themes-import"]}
                    </Button>
                </div>
                <Text tag={TextTags.textSM} class={classes.importNote}>
                    {t["themes-importUrlNote"]}
                </Text>
            </Show>
            <hr class={classes.divider} />
            <Header tag={HeaderTags.HeadingLG}>{t["themes-installed"]}</Header>
            <Show when={nativeLoading()}>
                <Text tag={TextTags.textSM}>{t["themes-empty"]}</Text>
            </Show>
            <Show when={nativeError() && !nativeLoading()}>
                <Text tag={TextTags.textSM}>{t["themes-emptyDesc"]}</Text>
            </Show>
            <Show
                when={themes().length > 0}
                fallback={
                    <EmptyState
                        message={t["themes-empty"]}
                        description={t["themes-emptyDesc"]}
                        linkHref={BETTERDISCORD_THEMES_URL}
                        linkLabel={t["themes-browseThemes"]}
                    />
                }
            >
                <For each={themes()}>{(theme: ThemeManifest) => <ThemesCard theme={theme} />}</For>
            </Show>
        </>
    );
}

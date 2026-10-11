import { createSignal, Show } from "solid-js";
import type { ThemeManifest } from "../../../@types/themeManifest.js";
import {
    isNativeThemesMode,
    isNativeThemeToggleAvailable,
    loadNativeThemes,
    openThemeFolderNative,
    refreshThemes,
    setNativeThemeEnabled,
} from "../settings.js";
import classes from "./ThemesCard.module.css";

const {
    ui: { Header, Switch, HeaderTags, showToast },
    plugin: { store },
} = shelter;

export const ThemesCard = (props: { theme: ThemeManifest }) => {
    const [switchState, setSwitchState] = createSignal(props.theme.enabled);
    const nativeMode = () => isNativeThemesMode();

    async function toggleTheme(state: boolean) {
        // Native WebView2 flips via the typed native enable capability
        // (host watcher owns CSS); Electron keeps the fire-and-forget send.
        // Exactly one path runs per toggle; switches stay independent.
        if (isNativeThemeToggleAvailable() && props.theme.id) {
            const previous = switchState();
            setSwitchState(state);
            try {
                await setNativeThemeEnabled(props.theme.id, state);
                await loadNativeThemes().then((themes) => {
                    const current = themes.find((entry) => entry.id === props.theme.id);
                    if (current) setSwitchState(current.enabled);
                });
            } catch {
                console.warn("[Remcord] Could not update the theme in the native shell.");
                setSwitchState(previous);
            }
            return;
        }
        setSwitchState(state);
        if (props.theme.id) {
            window.remcord.themes.set(props.theme.id, state);
        }
        refreshThemes();
    }
    function removeTheme() {
        if (props.theme.id) {
            window.remcord.themes.uninstall(props.theme.id);
        }
        refreshThemes();
    }
    function editTheme() {
        if (props.theme.id) {
            window.remcord.themes.edit(props.theme.id);
        }
    }
    function updateTheme() {
        if (props.theme.updateSrc) {
            window.remcord.themes.install(props.theme.updateSrc);
        }
        setTimeout(() => {
            refreshThemes();
        }, 1000);
        showToast({
            title: store.i18n["themes-success"],
            content: store.i18n["themes-updated"],
            duration: 3000,
        });
    }
    function openThemesFolder() {
        if (props.theme.id) {
            // Native WebView2 opens via the typed folders.open_theme capability
            // (id passes through verbatim; the host owns validation).
            // Electron keeps the original fire-and-forget send.
            if (!openThemeFolderNative(props.theme.id)) window.remcord.themes.folder(props.theme.id);
        }
    }
    // Native inventory never carries updateSrc, so the update action stays
    // hidden natively without extra gating. Edit/uninstall have no native
    // capability and stay hidden in native mode (functions above preserve
    // the Electron contract for regression tests).
    const showUpdate = () => !!props.theme.updateSrc && !nativeMode();
    const showDeferred = () => !nativeMode();
    return (
        <div class={classes.card}>
            <div class={classes.topRow}>
                <div class={classes.info}>
                    <Header tag={HeaderTags.HeadingLG} class={classes.title}>
                        {props.theme.name}
                    </Header>
                    <div class={classes.author}>
                        {store.i18n["themes-by"]} {props.theme.author}
                    </div>
                </div>
                <Switch checked={switchState()} onChange={(state: boolean) => void toggleTheme(state)} />
            </div>
            <Show when={props.theme.description}>
                <div class={classes.description}>{props.theme.description}</div>
            </Show>
            <div class={classes.actions}>
                <Show when={showDeferred()}>
                    <button
                        title={store.i18n["themes-delete"]}
                        type="button"
                        onClick={removeTheme}
                        class={`${classes.btn} ${classes.btnDanger}`}
                    >
                        <img class={classes.icon} alt={store.i18n["themes-delete"]} src="remcord://assets/Trash.png" />
                    </button>
                    <button title={store.i18n["themes-edit"]} type="button" onClick={editTheme} class={classes.btn}>
                        <img class={classes.icon} alt={store.i18n["themes-edit"]} src="remcord://assets/Edit.png" />
                    </button>
                </Show>
                <Show when={showUpdate()}>
                    <button title={store.i18n["themes-update"]} type="button" onClick={updateTheme} class={classes.btn}>
                        <img
                            class={classes.icon}
                            alt={store.i18n["themes-update"]}
                            src="remcord://assets/UpgradeArrow.png"
                        />
                    </button>
                </Show>
                <button title={store.i18n["themes-open"]} type="button" onClick={openThemesFolder} class={classes.btn}>
                    <img class={classes.icon} alt={store.i18n["themes-open"]} src="remcord://assets/Folder.png" />
                </button>
            </div>
        </div>
    );
};

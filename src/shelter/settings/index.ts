/// <reference path="../../../node_modules/@uwu/shelter-defs/dist/shelter-defs/rootdefs.d.ts" />
import { type Component, createComponent, onCleanup, onMount } from "solid-js";
import {
    AboutSidebarIcon,
    GallerySidebarIcon,
    GamesSidebarIcon,
    KeybindsSidebarIcon,
    PluginsSidebarIcon,
    SettingsSidebarIcon,
    ThemesSidebarIcon,
} from "./components/icons/SettingsNavIcons.jsx";
import { AboutPage } from "./pages/AboutPage.jsx";
import { GalleryPage } from "./pages/GalleryPage.jsx";
import { KeybindsPage } from "./pages/KeybindsPage.jsx";
import { PluginsPage } from "./pages/PluginsPage.jsx";
import { RegisteredGamesPage } from "./pages/RegisteredGamesPage.jsx";
import { SettingsPage } from "./pages/SettingsPage.jsx";
import { ThemesPage } from "./pages/ThemesPage.jsx";
import { openQuickCssRespectingSystem } from "./quickCssEditor.jsx";
import type { QuickCssEditorMode } from "./quickCssEditorSource.js";
import {
    disposeSettingsScrollRestoration,
    isRestartRequired,
    mountSettingsScrollRestoration,
    refreshSettings,
    refreshThemes,
} from "./settings.js";

const {
    plugin: { store },
    settings: { registerSection },
    util: { log },
    ui: { openConfirmationModal },
    flux: { dispatcher, storesFlat },
} = shelter;

let settingsCleanups: (() => void)[] = [];

function withScrollRestoration(Page: Component): Component {
    return () => {
        let unmount: (() => void) | undefined;
        onMount(() => {
            unmount = mountSettingsScrollRestoration();
        });
        onCleanup(() => unmount?.());
        return createComponent(Page, {});
    };
}

function registerSections(): (() => void)[] {
    const array = [
        registerSection("divider"),
        registerSection("header", "Remcord"),
        registerSection("section", "remcord-settings", "Settings", withScrollRestoration(SettingsPage), {
            icon: SettingsSidebarIcon,
        }),
        registerSection("section", "remcord-themes", "Themes", withScrollRestoration(ThemesPage), {
            icon: ThemesSidebarIcon,
        }),
        registerSection("section", "remcord-keybinds", "Keybinds", withScrollRestoration(KeybindsPage), {
            icon: KeybindsSidebarIcon,
        }),
        registerSection("section", "remcord-gallery", "Gallery", withScrollRestoration(GalleryPage), {
            icon: GallerySidebarIcon,
        }),
        registerSection("section", "remcord-games", "Games", withScrollRestoration(RegisteredGamesPage), {
            icon: GamesSidebarIcon,
        }),
        registerSection("divider"),
        registerSection("section", "remcord-about", "About", withScrollRestoration(AboutPage), {
            icon: AboutSidebarIcon,
        }),
    ];
    if (window.remcord.settings.getConfig().showExperimentalPluginMenu) {
        array.push(
            registerSection("section", "remcord-plugins", "Plugins", withScrollRestoration(PluginsPage), {
                icon: PluginsSidebarIcon,
            }),
        );
    }
    return array;
}

function restartRequired(payload: { event: string; properties: { origin_pane: string } }) {
    if (payload.event === "settings_pane_viewed" && typeof payload.properties.origin_pane !== "undefined") {
        const pane = payload.properties.origin_pane;
        if ((pane === "remcord-settings" || pane === "remcord-games") && isRestartRequired) {
            openConfirmationModal({
                header: () => store.i18n?.["settings-restartRequired"] ?? "Restart required",
                body: () => store.i18n?.["settings-restartRequiredBody"] ?? "A restart is required to apply changes.",
                type: "danger",
                confirmText: store.i18n?.["settings-restart"] ?? "Restart",
                cancelText: store.i18n?.["settings-restartLater"] ?? "Later",
            }).then(
                () => window.remcord.restart(),
                () => console.log("restart skipped"),
            );
        }
    }
}

export function onLoad() {
    refreshSettings();
    refreshThemes();
    // used for restart required dialog later
    store.i18n = window.remcord.translations;
    log("Remcord Settings");
    window.remcord.settings.setLang(storesFlat.LocaleStore.locale);
    settingsCleanups = registerSections();
    dispatcher.subscribe("TRACK", restartRequired);
    // Takes the QuickCSS editor handoff dispatched by the native mod
    // adapters (Vencord/Equicord openEditor override): preventDefault
    // signals "handled" back to the opener so it never falls through to
    // the dead popup path.
    window.addEventListener("remcord-native:open-quickcss-editor", quickCssEditorHandoff);
}
export function onUnload() {
    settingsCleanups.forEach((e) => {
        e();
    });
    dispatcher.unsubscribe("TRACK", restartRequired);
    window.removeEventListener("remcord-native:open-quickcss-editor", quickCssEditorHandoff);
    disposeSettingsScrollRestoration();
}

function quickCssEditorHandoff(event: Event) {
    const detail = (event as CustomEvent).detail as { mode?: unknown } | undefined;
    const mode = detail?.mode;
    if (mode !== "vencord" && mode !== "equicord" && mode !== "remcord") return;
    event.preventDefault();
    void openQuickCssRespectingSystem(mode as QuickCssEditorMode);
}

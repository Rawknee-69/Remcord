import { createMemo, createSignal, Show } from "solid-js";
import type { Settings } from "../../../@types/settings.js";
import { BackupSection } from "../components/BackupSection.jsx";
import { DropdownItem } from "../components/DropdownItem.jsx";
import {
    AdvancedPanelIcon,
    BackupPanelIcon,
    BehaviourPanelIcon,
    LookFeelPanelIcon,
    ModsPanelIcon,
    NetworkPanelIcon,
    PowerPanelIcon,
    RpcPanelIcon,
} from "../components/icons/SettingsPanelIcons.jsx";
import { SettingsPanel } from "../components/SettingsPanel.jsx";
import { matchesSettingsQuery, SearchableSetting, SettingsSearchProvider } from "../components/SettingsSearch.jsx";
import { SupportBanner } from "../components/SupportBanner.jsx";
import { TextBoxItem } from "../components/TextBoxItem.jsx";
import {
    isMinWindowsVersion,
    isNativeSettingsSetAvailable,
    isNativeWebView,
    isSettingWritableNatively,
    revealFolderNative,
    setBlockPowerSavingInVoiceChat,
    setConfig,
    setNativeSetting,
    toggleMod,
} from "../settings.js";
import classes from "./SettingsPage.module.css";

const {
    plugin: { store },
    ui: { SwitchItem, Header, HeaderTags, Button, ButtonSizes, TextBox },
} = shelter;

const noBundleUpdates = (settings: Settings) => {
    const value = settings.noBundleUpdates;
    if (Array.isArray(value)) return value;
    return value ? ["shelter", "vencord", "equicord", "custom"] : [];
};

// Native WebView2 shell: only allowlisted settings keys persist (see
// `isSettingWritableNatively` in settings.ts, mirroring the closed host
// `settings.set` allowlist). Every other control renders disabled with an
// honest reason instead of flipping visually, throwing on click, and
// reverting after restart.
const nativeOff = (key: string) => isNativeWebView() && !isSettingWritableNatively(key);

type PanelId = "backup" | "mods" | "lookAndFeel" | "behaviour" | "power" | "rpc" | "networking" | "advanced";

export function SettingsPage() {
    const [query, setQuery] = createSignal("");

    const settings = store.settings as Settings;
    const t = store.i18n;

    // Appends the native-shell reason to a control's note when the control
    // is disabled natively; returns the base note unchanged otherwise (and
    // returns just the reason when there is no base note).
    const nativeNote = (base: string, key: string) => {
        if (!nativeOff(key)) return base;
        const reason = t["settings-nativeUnsupported"] ?? "Not available in the native Remcord app";
        return base ? `${base} (${reason})` : reason;
    };

    if (!settings) {
        return (
            <>
                <Header class={classes.crashTitle} tag={HeaderTags.HeadingXL}>
                    {t?.["settings-firstTimeCrash"] ?? "Setting things up..."}
                </Header>
                <p>
                    {t?.["settings-firstTimeCrash-desc"] ??
                        "Settings are not available on a first-time launch. Please restart."}
                </p>
                <br />
                <Button size={ButtonSizes.MAX} onClick={() => window.remcord.restart()}>
                    Restart Remcord
                </Button>
            </>
        );
    }

    const panelKeywords: Record<PanelId, Array<string | undefined>> = {
        backup: [
            t["settings-category-backup"],
            t["settings-category-backup-desc"],
            t["backup-pageTitle"],
            t["backup-pageSubtitle"],
            t["backup-createBackup"],
            t["backup-restore"],
        ],
        mods: [
            t["settings-category-mods"],
            t["settings-category-mods-desc"],
            t["settings-csp"],
            t["settings-csp-desc"],
            "Vencord",
            "Equicord",
            t["settings-mod-vencord"],
            t["settings-mod-equicord"],
            t["settings-extendedPluginAbilities"],
            t["settings-extendedPluginAbilities-desc"],
        ],
        lookAndFeel: [
            t["settings-category-lookAndFeel"],
            t["settings-category-lookAndFeel-desc"],
            t["settings-theme"],
            t["settings-theme-desc"],
            t["settings-autoHideMenuBar"],
            t["settings-transparency"],
            t["settings-material"],
            t["settings-bounceOnPing"],
            t["settings-trayIcon"],
            t["settings-skipSplash"],
            t["settings-mobileMode"],
        ],
        behaviour: [
            t["settings-category-behaviour"],
            t["settings-category-behaviour-desc"],
            t["settings-channel"],
            t["settings-MultiInstance"],
            t["settings-popoutPiP"],
            t["settings-useSystemCssEditor"],
            t["settings-useMacSystemPicker"],
            t["settings-disableAutogain"],
            t["settings-mintoTray"],
            t["settings-startMinimized"],
            t["settings-startMinimized-off"],
            t["settings-startMinimized-minimized"],
            t["settings-startMinimized-tray"],
            t["settings-smoothScroll"],
            t["settings-autoScroll"],
            t["settings-spellcheck"],
        ],
        power: [
            t["settings-category-powerManagement"],
            t["settings-category-powerManagement-desc"],
            t["settings-prfmMode"],
            t["settings-prfmMode-desc"],
            t["settings-blockPowerSavingInVoiceChat"],
            t["settings-sleepInBackground"],
        ],
        rpc: [
            t["settings-category-arrpc"],
            t["settings-category-arrpc-desc"],
            t["settings-invitewebsocket"],
            t["settings-processScanning"],
            t["settings-windowsLegacyScanning"],
            t["settings-scanInterval"],
            "arRPC",
            "Rich Presence",
        ],
        networking: [
            t["settings-category-networking"],
            t["settings-category-networking-desc"],
            t["settings-proxyMode"],
            t["settings-proxyRules"],
            t["settings-proxyPacScript"],
            t["settings-proxyBypassRules"],
            "proxy",
            "SOCKS",
        ],
        advanced: [
            t["settings-category-advanced"],
            t["settings-category-advanced-desc"],
            t["settings-showExperimentalPluginMenu"],
            t["settings-venmic-deviceSelect"],
            t["settings-venmic-granularSelect"],
            t["settings-venmic-workaround"],
            t["settings-audio"],
            t["settings-hardwareAcceleration"],
            t["settings-sdpH264BaselineRewrite"],
            t["settings-vaapi"],
            t["settings-automaticClientUpdates"],
            t["settings-disableHttpCache"],
            t["settings-additionalArguments"],
            t["settings-noBundleUpdates"],
            t["settings-openCustomIconDialog"],
            t["settings-storageFolder"],
            t["settings-copyDebugInfo"],
            t["settings-copyGPUInfo"],
            t["settings-openWebRTCInternals"],
            t["settings-openGPUInfo"],
            t["settings-clearClientModCache"],
            "venmic",
            "VAAPI",
            "WebRTC",
        ],
    };

    const isSearching = createMemo(() => query().trim().length > 0);

    const panelForceOpen = (id: PanelId) => isSearching() && matchesSettingsQuery(panelKeywords[id], query());

    const panelHidden = (id: PanelId) => {
        if (!isSearching()) return false;
        return !matchesSettingsQuery(panelKeywords[id], query());
    };

    const anyPanelVisible = createMemo(() =>
        (Object.keys(panelKeywords) as PanelId[]).some((id) => matchesSettingsQuery(panelKeywords[id], query())),
    );

    return (
        <SettingsSearchProvider query={query}>
            <Show when={!settings.supportBannerDismissed}>
                <SupportBanner />
            </Show>

            <div class={classes.search}>
                <TextBox value={query()} onInput={(v: string) => setQuery(v)} placeholder={t["settings-search"]} />
            </div>

            <Show when={isSearching() && !anyPanelVisible()}>
                <p class={classes.noResults}>{t["settings-noResults"]}</p>
            </Show>

            <SettingsPanel
                title={t["settings-category-backup"]}
                description={t["settings-category-backup-desc"]}
                icon={BackupPanelIcon}
                id="backup"
                forceOpen={panelForceOpen("backup")}
                hidden={panelHidden("backup")}
            >
                <SearchableSetting keywords={panelKeywords.backup}>
                    <BackupSection embedded />
                </SearchableSetting>
            </SettingsPanel>

            <SettingsPanel
                title={t["settings-category-mods"]}
                description={t["settings-category-mods-desc"]}
                icon={ModsPanelIcon}
                id="mods"
                forceOpen={panelForceOpen("mods")}
                hidden={panelHidden("mods")}
            >
                <SearchableSetting keywords={[t["settings-csp"], t["settings-csp-desc"], "CSP"]}>
                    <DropdownItem
                        value={settings.csp}
                        onChange={(v) => setConfig("csp", v as Settings["csp"], true)}
                        title={t["settings-csp"]}
                        note={nativeNote(t["settings-csp-desc"], "csp")}
                        disabled={nativeOff("csp")}
                        options={[
                            { label: t["settings-csp-none"], value: "none" },
                            { label: t["settings-csp-strict"], value: "strict" },
                            { label: t["settings-csp-vanilla"], value: "vanilla" },
                        ]}
                    />
                </SearchableSetting>
                <SearchableSetting keywords={["Vencord", t["settings-mod-vencord"]]}>
                    <SwitchItem
                        note={t["settings-mod-vencord"]}
                        value={settings.mods.includes("vencord")}
                        onChange={(e: boolean) => toggleMod("vencord", e)}
                    >
                        Vencord
                    </SwitchItem>
                </SearchableSetting>
                <SearchableSetting keywords={["Equicord", t["settings-mod-equicord"]]}>
                    <SwitchItem
                        note={t["settings-mod-equicord"]}
                        value={settings.mods.includes("equicord")}
                        onChange={(e: boolean) => toggleMod("equicord", e)}
                    >
                        Equicord
                    </SwitchItem>
                </SearchableSetting>
                <SearchableSetting
                    keywords={[t["settings-extendedPluginAbilities"], t["settings-extendedPluginAbilities-desc"]]}
                >
                    <SwitchItem
                        note={t["settings-extendedPluginAbilities-desc"]}
                        value={settings.extendedPluginAbilities}
                        onChange={(e: boolean) => {
                            // Native WebView2 uses the closed async allowlist;
                            // Electron keeps the original synchronous setConfig.
                            if (isNativeSettingsSetAvailable()) {
                                void setNativeSetting("extendedPluginAbilities", e).catch(() =>
                                    console.warn("[Remcord] Could not update the setting in the native shell."),
                                );
                            } else {
                                setConfig("extendedPluginAbilities", e);
                            }
                        }}
                    >
                        {t["settings-extendedPluginAbilities"]}
                    </SwitchItem>
                </SearchableSetting>
            </SettingsPanel>

            <SettingsPanel
                title={t["settings-category-lookAndFeel"]}
                description={t["settings-category-lookAndFeel-desc"]}
                icon={LookFeelPanelIcon}
                id="lookAndFeel"
                forceOpen={panelForceOpen("lookAndFeel")}
                hidden={panelHidden("lookAndFeel")}
            >
                <SearchableSetting keywords={[t["settings-theme"], t["settings-theme-desc"], "window"]}>
                    <DropdownItem
                        value={settings.windowStyle}
                        onChange={(v) => setConfig("windowStyle", v as Settings["windowStyle"], true)}
                        title={t["settings-theme"]}
                        note={nativeNote(t["settings-theme-desc"], "windowStyle")}
                        disabled={nativeOff("windowStyle")}
                        options={[
                            { label: t["settings-theme-default"], value: "default" },
                            { label: t["settings-theme-native"], value: "native" },
                            { label: t["settings-theme-overlay"], value: "overlay" },
                            { label: t["settings-theme-legacy"], value: "legacy" },
                        ]}
                    />
                </SearchableSetting>
                <Show when={settings.windowStyle === "native"}>
                    <SearchableSetting keywords={[t["settings-autoHideMenuBar"], t["settings-autoHideMenuBar-desc"]]}>
                        <SwitchItem
                            note={nativeNote(t["settings-autoHideMenuBar-desc"], "autoHideMenuBar")}
                            value={settings.autoHideMenuBar}
                            disabled={nativeOff("autoHideMenuBar")}
                            onChange={(e: boolean) => setConfig("autoHideMenuBar", e, true)}
                        >
                            {t["settings-autoHideMenuBar"]}
                        </SwitchItem>
                    </SearchableSetting>
                </Show>
                <SearchableSetting keywords={[t["settings-transparency"], t["settings-transparency-desc"]]}>
                    <DropdownItem
                        value={store.settings.transparency}
                        onChange={(v) => setConfig("transparency", v as Settings["transparency"], true)}
                        title={t["settings-transparency"]}
                        note={nativeNote(t["settings-transparency-desc"], "transparency")}
                        disabled={nativeOff("transparency")}
                        extraItems={
                            <Show
                                when={
                                    store.settings.transparency !== "none" &&
                                    window.remcord.platform === "darwin" &&
                                    Number.parseInt(window.remcord.osRelease, 10) >= 25
                                }
                            >
                                <div
                                    style={{
                                        background: "rgba(255, 255, 210, 0.85)",
                                        border: "1px solid #e6c200",
                                        color: "#2d2100",
                                        padding: "12px 16px",
                                    }}
                                >
                                    {t["settings-transparency-tahoe-warning"]}
                                </div>
                            </Show>
                        }
                        options={[
                            {
                                label: t["settings-transparency-universal"],
                                value: "universal",
                            },
                            ...(window.remcord.platform === "win32" || window.remcord.platform === "darwin"
                                ? [
                                      {
                                          label: t["settings-transparency-modern"],
                                          value: "modern",
                                      },
                                  ]
                                : []),
                            { label: t["settings-none"], value: "none" },
                        ]}
                    />
                </SearchableSetting>
                <Show
                    when={
                        window.remcord.platform === "win32" &&
                        isMinWindowsVersion(10, 0, 22000) &&
                        store.settings.transparency === "modern"
                    }
                >
                    <SearchableSetting keywords={[t["settings-material"], t["settings-material-desc"]]}>
                        <DropdownItem
                            value={settings.windowMaterial}
                            onChange={(v) => setConfig("windowMaterial", v as Settings["windowMaterial"], true)}
                            title={t["settings-material"]}
                            note={nativeNote(t["settings-material-desc"], "windowMaterial")}
                            disabled={nativeOff("windowMaterial")}
                            options={[
                                { label: t["settings-material-mica"], value: "mica" },
                                { label: t["settings-material-mica-alt"], value: "tabbed" },
                                { label: t["settings-material-acrylic"], value: "acrylic" },
                                { label: t["settings-material-none"], value: "none" },
                            ]}
                        />
                    </SearchableSetting>
                </Show>
                <Show when={window.remcord.platform === "darwin"}>
                    <SearchableSetting keywords={[t["settings-bounceOnPing"], t["settings-bounceOnPing-desc"]]}>
                        <SwitchItem
                            note={nativeNote(t["settings-bounceOnPing-desc"], "bounceOnPing")}
                            value={settings.bounceOnPing}
                            disabled={nativeOff("bounceOnPing")}
                            onChange={(e: boolean) => setConfig("bounceOnPing", e, true)}
                        >
                            {t["settings-bounceOnPing"]}
                        </SwitchItem>
                    </SearchableSetting>
                </Show>
                <SearchableSetting keywords={[t["settings-trayIcon"], t["settings-trayIcon-desc"], "tray"]}>
                    <DropdownItem
                        value={settings.tray}
                        onChange={(v) => setConfig("tray", v as Settings["tray"], true)}
                        title={t["settings-trayIcon"]}
                        note={nativeNote(t["settings-trayIcon-desc"], "tray")}
                        disabled={nativeOff("tray")}
                        options={[
                            { label: t["settings-trayIcon-dynamic"], value: "dynamic" },
                            { label: t["settings-trayIcon-disabled"], value: "disabled" },
                            { label: t["settings-trayIcon-normal"], value: "dsc-tray" },
                            { label: t["settings-trayIcon-classic"], value: "clsc-dsc-tray" },
                            { label: t["settings-trayIcon-colored-plug"], value: "ac_plug_colored" },
                            { label: t["settings-trayIcon-white-plug"], value: "ac_white_plug" },
                            { label: t["settings-trayIcon-white-plug-alt"], value: "ac_white_plug_hollow" },
                            { label: t["settings-trayIcon-black-plug"], value: "ac_black_plug" },
                            { label: t["settings-trayIcon-black-plug-alt"], value: "ac_black_plug_hollow" },
                        ]}
                    />
                </SearchableSetting>
                <SearchableSetting keywords={[t["settings-skipSplash"], t["settings-skipSplash-desc"]]}>
                    <SwitchItem
                        note={nativeNote(t["settings-skipSplash-desc"], "skipSplash")}
                        value={settings.skipSplash}
                        disabled={nativeOff("skipSplash")}
                        onChange={(e: boolean) => setConfig("skipSplash", e)}
                    >
                        {t["settings-skipSplash"]}
                    </SwitchItem>
                </SearchableSetting>
                <SearchableSetting keywords={[t["settings-mobileMode"], t["settings-mobileMode-desc"]]}>
                    <SwitchItem
                        note={nativeNote(t["settings-mobileMode-desc"], "mobileMode")}
                        value={settings.mobileMode}
                        disabled={nativeOff("mobileMode")}
                        onChange={(e: boolean) => setConfig("mobileMode", e, true)}
                    >
                        {t["settings-mobileMode"]}
                    </SwitchItem>
                </SearchableSetting>
            </SettingsPanel>

            <SettingsPanel
                title={t["settings-category-behaviour"]}
                description={t["settings-category-behaviour-desc"]}
                icon={BehaviourPanelIcon}
                id="behaviour"
                forceOpen={panelForceOpen("behaviour")}
                hidden={panelHidden("behaviour")}
            >
                <SearchableSetting keywords={[t["settings-channel"], t["settings-channel-desc"]]}>
                    <DropdownItem
                        value={settings.channel}
                        onChange={(v) => setConfig("channel", v as Settings["channel"], true)}
                        title={t["settings-channel"]}
                        note={nativeNote(t["settings-channel-desc"], "channel")}
                        disabled={nativeOff("channel")}
                        link="https://support.discord.com/hc/en-us/articles/360035675191-Discord-Testing-Clients"
                        options={[
                            { label: t["settings-channel-stable"], value: "stable" },
                            { label: t["settings-channel-canary"], value: "canary" },
                            { label: t["settings-channel-ptb"], value: "ptb" },
                        ]}
                    />
                </SearchableSetting>
                <SearchableSetting keywords={[t["settings-MultiInstance"], t["settings-MultiInstance-desc"]]}>
                    <SwitchItem
                        note={nativeNote(t["settings-MultiInstance-desc"], "multiInstance")}
                        value={settings.multiInstance}
                        disabled={nativeOff("multiInstance")}
                        onChange={(e: boolean) => setConfig("multiInstance", e)}
                    >
                        {t["settings-MultiInstance"]}
                    </SwitchItem>
                </SearchableSetting>
                <SearchableSetting keywords={[t["settings-popoutPiP"], t["settings-popoutPiP-desc"]]}>
                    <SwitchItem
                        note={nativeNote(t["settings-popoutPiP-desc"], "popoutPiP")}
                        value={settings.popoutPiP}
                        disabled={nativeOff("popoutPiP")}
                        onChange={(e: boolean) => setConfig("popoutPiP", e)}
                    >
                        {t["settings-popoutPiP"]}
                    </SwitchItem>
                </SearchableSetting>
                <SearchableSetting keywords={[t["settings-useSystemCssEditor"], t["settings-useSystemCssEditor-desc"]]}>
                    <SwitchItem
                        note={nativeNote(t["settings-useSystemCssEditor-desc"], "useSystemCssEditor")}
                        value={settings.useSystemCssEditor}
                        disabled={nativeOff("useSystemCssEditor")}
                        onChange={(e: boolean) => setConfig("useSystemCssEditor", e)}
                    >
                        {t["settings-useSystemCssEditor"]}
                    </SwitchItem>
                </SearchableSetting>
                <Show when={window.remcord.platform === "darwin"}>
                    <SearchableSetting
                        keywords={[t["settings-useMacSystemPicker"], t["settings-useMacSystemPicker-desc"]]}
                    >
                        <SwitchItem
                            note={nativeNote(t["settings-useMacSystemPicker-desc"], "useMacSystemPicker")}
                            value={settings.useMacSystemPicker}
                            disabled={nativeOff("useMacSystemPicker")}
                            onChange={(e: boolean) => setConfig("useMacSystemPicker", e)}
                        >
                            {t["settings-useMacSystemPicker"]}
                        </SwitchItem>
                    </SearchableSetting>
                </Show>
                <SearchableSetting keywords={[t["settings-disableAutogain"], t["settings-disableAutogain-desc"]]}>
                    <SwitchItem
                        note={nativeNote(t["settings-disableAutogain-desc"], "disableAutogain")}
                        value={settings.disableAutogain}
                        disabled={nativeOff("disableAutogain")}
                        onChange={(e: boolean) => setConfig("disableAutogain", e)}
                    >
                        {t["settings-disableAutogain"]}
                    </SwitchItem>
                </SearchableSetting>
                <SearchableSetting keywords={[t["settings-mintoTray"], t["settings-mintoTray-desc"]]}>
                    <SwitchItem
                        note={nativeNote(t["settings-mintoTray-desc"], "minimizeToTray")}
                        value={settings.minimizeToTray}
                        disabled={nativeOff("minimizeToTray")}
                        onChange={(e: boolean) => setConfig("minimizeToTray", e)}
                    >
                        {t["settings-mintoTray"]}
                    </SwitchItem>
                </SearchableSetting>
                <SearchableSetting
                    keywords={[
                        t["settings-startMinimized"],
                        t["settings-startMinimized-desc"],
                        t["settings-startMinimized-off"],
                        t["settings-startMinimized-minimized"],
                        t["settings-startMinimized-tray"],
                    ]}
                >
                    <DropdownItem
                        value={settings.startMinimized}
                        onChange={(v) => setConfig("startMinimized", v as Settings["startMinimized"], true)}
                        title={t["settings-startMinimized"]}
                        note={nativeNote(t["settings-startMinimized-desc"], "startMinimized")}
                        disabled={nativeOff("startMinimized")}
                        options={[
                            { label: t["settings-startMinimized-off"], value: "off" },
                            { label: t["settings-startMinimized-minimized"], value: "minimized" },
                            { label: t["settings-startMinimized-tray"], value: "tray" },
                        ]}
                    />
                </SearchableSetting>
                <SearchableSetting keywords={[t["settings-smoothScroll"], t["settings-smoothScroll-desc"]]}>
                    <SwitchItem
                        note={nativeNote(t["settings-smoothScroll-desc"], "smoothScroll")}
                        value={settings.smoothScroll}
                        disabled={nativeOff("smoothScroll")}
                        onChange={(e: boolean) => setConfig("smoothScroll", e, true)}
                    >
                        {t["settings-smoothScroll"]}
                    </SwitchItem>
                </SearchableSetting>
                <SearchableSetting keywords={[t["settings-autoScroll"], t["settings-autoScroll-desc"]]}>
                    <SwitchItem
                        note={nativeNote(t["settings-autoScroll-desc"], "autoScroll")}
                        value={settings.autoScroll}
                        disabled={nativeOff("autoScroll")}
                        onChange={(e: boolean) => setConfig("autoScroll", e, true)}
                    >
                        {t["settings-autoScroll"]}
                    </SwitchItem>
                </SearchableSetting>
                <SearchableSetting keywords={[t["settings-spellcheck"], t["settings-spellcheck-desc"]]}>
                    <SwitchItem
                        note={nativeNote(t["settings-spellcheck-desc"], "spellcheck")}
                        value={settings.spellcheck}
                        disabled={nativeOff("spellcheck")}
                        onChange={(e: boolean) => setConfig("spellcheck", e, true)}
                    >
                        {t["settings-spellcheck"]}
                    </SwitchItem>
                </SearchableSetting>
            </SettingsPanel>

            <SettingsPanel
                title={t["settings-category-powerManagement"]}
                description={t["settings-category-powerManagement-desc"]}
                icon={PowerPanelIcon}
                id="power"
                forceOpen={panelForceOpen("power")}
                hidden={panelHidden("power")}
            >
                <SearchableSetting keywords={[t["settings-prfmMode"], t["settings-prfmMode-desc"], "performance"]}>
                    <DropdownItem
                        value={settings.performanceMode}
                        onChange={(v) => setConfig("performanceMode", v as Settings["performanceMode"], true)}
                        title={t["settings-prfmMode"]}
                        note={nativeNote(t["settings-prfmMode-desc"], "performanceMode")}
                        disabled={nativeOff("performanceMode")}
                        options={[
                            { label: t["settings-prfmMode-dynamic"], value: "dynamic" },
                            { label: t["settings-prfmMode-performance"], value: "performance" },
                            { label: t["settings-prfmMode-balanced"], value: "balanced" },
                            { label: t["settings-prfmMode-latency"], value: "latency" },
                            { label: t["settings-prfmMode-smoothScreenshare"], value: "smoothScreenshare" },
                            { label: t["settings-prfmMode-voip"], value: "voip" },
                            { label: t["settings-prfmMode-battery"], value: "battery" },
                            { label: t["settings-prfmMode-memory"], value: "memory" },
                            { label: t["settings-none"], value: "none" },
                        ]}
                    />
                </SearchableSetting>
                <SearchableSetting
                    keywords={[
                        t["settings-blockPowerSavingInVoiceChat"],
                        t["settings-blockPowerSavingInVoiceChat-desc"],
                    ]}
                >
                    <SwitchItem
                        note={t["settings-blockPowerSavingInVoiceChat-desc"]}
                        value={settings.blockPowerSavingInVoiceChat}
                        onChange={(e: boolean) => {
                            // Native WebView2 uses the allowlisted async mutation;
                            // Electron keeps the original synchronous setConfig.
                            if (isNativeWebView()) {
                                void setBlockPowerSavingInVoiceChat(e).catch(() =>
                                    console.warn(
                                        "[Remcord] Could not update blockPowerSavingInVoiceChat in the native shell.",
                                    ),
                                );
                            } else {
                                setConfig("blockPowerSavingInVoiceChat", e, true);
                            }
                        }}
                    >
                        {t["settings-blockPowerSavingInVoiceChat"]}
                    </SwitchItem>
                </SearchableSetting>
                <SearchableSetting keywords={[t["settings-sleepInBackground"], t["settings-sleepInBackground-desc"]]}>
                    <SwitchItem
                        note={nativeNote(t["settings-sleepInBackground-desc"], "sleepInBackground")}
                        value={store.settings.sleepInBackground}
                        disabled={nativeOff("sleepInBackground")}
                        onChange={(e: boolean) => setConfig("sleepInBackground", e, true)}
                    >
                        {t["settings-sleepInBackground"]}
                    </SwitchItem>
                </SearchableSetting>
            </SettingsPanel>

            <SettingsPanel
                title={t["settings-category-arrpc"]}
                description={t["settings-category-arrpc-desc"]}
                icon={RpcPanelIcon}
                id="rpc"
                forceOpen={panelForceOpen("rpc")}
                hidden={panelHidden("rpc")}
            >
                <SearchableSetting
                    keywords={[t["settings-invitewebsocket"], t["settings-invitewebsocket-desc"], "arRPC"]}
                >
                    <SwitchItem
                        note={nativeNote(t["settings-invitewebsocket-desc"], "inviteWebsocket")}
                        value={settings.inviteWebsocket}
                        disabled={nativeOff("inviteWebsocket")}
                        onChange={(e: boolean) => setConfig("inviteWebsocket", e, true)}
                    >
                        {t["settings-invitewebsocket"]}
                    </SwitchItem>
                </SearchableSetting>
                <Show when={settings.inviteWebsocket === true}>
                    <SearchableSetting keywords={[t["settings-processScanning"], t["settings-processScanning-desc"]]}>
                        <SwitchItem
                            note={nativeNote(t["settings-processScanning-desc"], "processScanning")}
                            value={settings.processScanning}
                            disabled={nativeOff("processScanning")}
                            onChange={(e: boolean) => setConfig("processScanning", e, true)}
                        >
                            {t["settings-processScanning"]}
                        </SwitchItem>
                    </SearchableSetting>
                    <Show when={window.remcord.platform === "win32"}>
                        <SearchableSetting
                            keywords={[t["settings-windowsLegacyScanning"], t["settings-windowsLegacyScanning-desc"]]}
                        >
                            <SwitchItem
                                note={nativeNote(t["settings-windowsLegacyScanning-desc"], "windowsLegacyScanning")}
                                value={settings.windowsLegacyScanning}
                                disabled={nativeOff("windowsLegacyScanning")}
                                onChange={(e: boolean) => setConfig("windowsLegacyScanning", e, true)}
                            >
                                {t["settings-windowsLegacyScanning"]}
                            </SwitchItem>
                        </SearchableSetting>
                    </Show>
                    <SearchableSetting keywords={[t["settings-scanInterval"], t["settings-scanInterval-desc"]]}>
                        <TextBoxItem
                            title={t["settings-scanInterval"]}
                            note={nativeNote(t["settings-scanInterval-desc"], "scanInterval")}
                            value={Number(settings.scanInterval).toString()}
                            disabled={nativeOff("scanInterval")}
                            onInput={(v: string) => setConfig("scanInterval", Number(v), true)}
                        />
                    </SearchableSetting>
                </Show>
            </SettingsPanel>

            <SettingsPanel
                title={t["settings-category-networking"]}
                description={t["settings-category-networking-desc"]}
                icon={NetworkPanelIcon}
                id="networking"
                forceOpen={panelForceOpen("networking")}
                hidden={panelHidden("networking")}
            >
                <SearchableSetting keywords={[t["settings-proxyMode"], t["settings-proxyMode-desc"], "proxy"]}>
                    <DropdownItem
                        value={settings.proxyMode ?? "system"}
                        onChange={(v) => setConfig("proxyMode", v as Settings["proxyMode"], true)}
                        title={t["settings-proxyMode"]}
                        note={nativeNote(t["settings-proxyMode-desc"], "proxyMode")}
                        disabled={nativeOff("proxyMode")}
                        link="https://www.electronjs.org/docs/latest/api/structures/proxy-config"
                        options={[
                            { label: t["settings-proxyMode-system"], value: "system" },
                            { label: t["settings-proxyMode-direct"], value: "direct" },
                            { label: t["settings-proxyMode-fixed_servers"], value: "fixed_servers" },
                            { label: t["settings-proxyMode-pac_script"], value: "pac_script" },
                            { label: t["settings-proxyMode-auto_detect"], value: "auto_detect" },
                        ]}
                    />
                </SearchableSetting>
                <Show when={(settings.proxyMode ?? "system") === "fixed_servers"}>
                    <SearchableSetting keywords={[t["settings-proxyRules"], t["settings-proxyRules-desc"]]}>
                        <TextBoxItem
                            title={t["settings-proxyRules"]}
                            note={nativeNote(t["settings-proxyRules-desc"], "proxyRules")}
                            value={settings.proxyRules ?? ""}
                            disabled={nativeOff("proxyRules")}
                            onInput={(v: string) => setConfig("proxyRules", v, true)}
                        />
                    </SearchableSetting>
                </Show>
                <Show when={(settings.proxyMode ?? "system") === "pac_script"}>
                    <SearchableSetting keywords={[t["settings-proxyPacScript"], t["settings-proxyPacScript-desc"]]}>
                        <TextBoxItem
                            title={t["settings-proxyPacScript"]}
                            note={nativeNote(t["settings-proxyPacScript-desc"], "proxyPacScript")}
                            value={settings.proxyPacScript ?? ""}
                            disabled={nativeOff("proxyPacScript")}
                            onInput={(v: string) => setConfig("proxyPacScript", v, true)}
                        />
                    </SearchableSetting>
                </Show>
                <Show when={(settings.proxyMode ?? "system") !== "direct"}>
                    <SearchableSetting keywords={[t["settings-proxyBypassRules"], t["settings-proxyBypassRules-desc"]]}>
                        <TextBoxItem
                            title={t["settings-proxyBypassRules"]}
                            note={nativeNote(t["settings-proxyBypassRules-desc"], "proxyBypassRules")}
                            value={settings.proxyBypassRules ?? "<local>"}
                            disabled={nativeOff("proxyBypassRules")}
                            onInput={(v: string) => setConfig("proxyBypassRules", v, true)}
                        />
                    </SearchableSetting>
                </Show>
            </SettingsPanel>

            <SettingsPanel
                title={t["settings-category-advanced"]}
                description={t["settings-category-advanced-desc"]}
                icon={AdvancedPanelIcon}
                id="advanced"
                forceOpen={panelForceOpen("advanced")}
                hidden={panelHidden("advanced")}
            >
                <SearchableSetting
                    keywords={[t["settings-showExperimentalPluginMenu"], t["settings-showExperimentalPluginMenu-desc"]]}
                >
                    <SwitchItem
                        note={t["settings-showExperimentalPluginMenu-desc"]}
                        value={settings.showExperimentalPluginMenu}
                        onChange={(e: boolean) => {
                            // Native WebView2 uses the closed async allowlist;
                            // Electron keeps the original synchronous setConfig.
                            if (isNativeSettingsSetAvailable()) {
                                void setNativeSetting("showExperimentalPluginMenu", e, true).catch(() =>
                                    console.warn("[Remcord] Could not update the setting in the native shell."),
                                );
                            } else {
                                setConfig("showExperimentalPluginMenu", e, true);
                            }
                        }}
                    >
                        {t["settings-showExperimentalPluginMenu"]}
                    </SwitchItem>
                </SearchableSetting>
                <Show when={window.remcord.platform === "linux"}>
                    <SearchableSetting
                        keywords={[t["settings-venmic-deviceSelect"], t["settings-venmic-deviceSelect-desc"], "venmic"]}
                    >
                        <SwitchItem
                            note={nativeNote(t["settings-venmic-deviceSelect-desc"], "audio")}
                            value={settings.audio.deviceSelect}
                            disabled={nativeOff("audio")}
                            onChange={(e: boolean) => {
                                const audioSettings = structuredClone({ ...settings.audio });
                                audioSettings.deviceSelect = e;
                                setConfig("audio", audioSettings);
                            }}
                        >
                            {t["settings-venmic-deviceSelect"]}
                        </SwitchItem>
                    </SearchableSetting>
                    <SearchableSetting
                        keywords={[t["settings-venmic-granularSelect"], t["settings-venmic-granularSelect-desc"]]}
                    >
                        <SwitchItem
                            note={nativeNote(t["settings-venmic-granularSelect-desc"], "audio")}
                            value={settings.audio.granularSelect}
                            disabled={nativeOff("audio")}
                            onChange={(e: boolean) => {
                                const audioSettings = structuredClone({ ...settings.audio });
                                audioSettings.granularSelect = e;
                                setConfig("audio", audioSettings);
                            }}
                        >
                            {t["settings-venmic-granularSelect"]}
                        </SwitchItem>
                    </SearchableSetting>
                    <SearchableSetting
                        keywords={[t["settings-venmic-workaround"], t["settings-venmic-workaround-desc"]]}
                    >
                        <SwitchItem
                            note={nativeNote(t["settings-venmic-workaround-desc"], "audio")}
                            value={settings.audio.workaround}
                            disabled={nativeOff("audio")}
                            onChange={(e: boolean) => {
                                const audioSettings = structuredClone({ ...settings.audio });
                                audioSettings.workaround = e;
                                setConfig("audio", audioSettings);
                            }}
                        >
                            {t["settings-venmic-workaround"]}
                        </SwitchItem>
                    </SearchableSetting>
                    <SearchableSetting
                        keywords={[t["settings-venmic-ignoreVirtual"], t["settings-venmic-ignoreVirtual-desc"]]}
                    >
                        <SwitchItem
                            note={nativeNote(t["settings-venmic-ignoreVirtual-desc"], "audio")}
                            value={settings.audio.ignoreVirtual}
                            disabled={nativeOff("audio")}
                            onChange={(e: boolean) => {
                                const audioSettings = structuredClone({ ...settings.audio });
                                audioSettings.ignoreVirtual = e;
                                setConfig("audio", audioSettings);
                            }}
                        >
                            {t["settings-venmic-ignoreVirtual"]}
                        </SwitchItem>
                    </SearchableSetting>
                    <SearchableSetting
                        keywords={[t["settings-venmic-ignoreDevices"], t["settings-venmic-ignoreDevices-desc"]]}
                    >
                        <SwitchItem
                            note={nativeNote(t["settings-venmic-ignoreDevices-desc"], "audio")}
                            value={settings.audio.ignoreDevices}
                            disabled={nativeOff("audio")}
                            onChange={(e: boolean) => {
                                const audioSettings = structuredClone({ ...settings.audio });
                                audioSettings.ignoreDevices = e;
                                setConfig("audio", audioSettings);
                            }}
                        >
                            {t["settings-venmic-ignoreDevices"]}
                        </SwitchItem>
                    </SearchableSetting>
                    <SearchableSetting
                        keywords={[t["settings-venmic-ignoreInputMedia"], t["settings-venmic-ignoreInputMedia-desc"]]}
                    >
                        <SwitchItem
                            note={nativeNote(t["settings-venmic-ignoreInputMedia-desc"], "audio")}
                            value={settings.audio.ignoreInputMedia}
                            disabled={nativeOff("audio")}
                            onChange={(e: boolean) => {
                                const audioSettings = structuredClone({ ...settings.audio });
                                audioSettings.ignoreInputMedia = e;
                                setConfig("audio", audioSettings);
                            }}
                        >
                            {t["settings-venmic-ignoreInputMedia"]}
                        </SwitchItem>
                    </SearchableSetting>
                    <SearchableSetting
                        keywords={[t["settings-venmic-onlySpeakers"], t["settings-venmic-onlySpeakers-desc"]]}
                    >
                        <SwitchItem
                            note={nativeNote(t["settings-venmic-onlySpeakers-desc"], "audio")}
                            value={settings.audio.onlySpeakers}
                            disabled={nativeOff("audio")}
                            onChange={(e: boolean) => {
                                const audioSettings = structuredClone({ ...settings.audio });
                                audioSettings.onlySpeakers = e;
                                setConfig("audio", audioSettings);
                            }}
                        >
                            {t["settings-venmic-onlySpeakers"]}
                        </SwitchItem>
                    </SearchableSetting>
                    <SearchableSetting
                        keywords={[
                            t["settings-venmic-onlyDefaultSpeakers"],
                            t["settings-venmic-onlyDefaultSpeakers-desc"],
                        ]}
                    >
                        <SwitchItem
                            note={nativeNote(t["settings-venmic-onlyDefaultSpeakers-desc"], "audio")}
                            value={settings.audio.onlyDefaultSpeakers}
                            disabled={nativeOff("audio")}
                            onChange={(e: boolean) => {
                                const audioSettings = structuredClone({ ...settings.audio });
                                audioSettings.onlyDefaultSpeakers = e;
                                setConfig("audio", audioSettings);
                            }}
                        >
                            {t["settings-venmic-onlyDefaultSpeakers"]}
                        </SwitchItem>
                    </SearchableSetting>
                </Show>
                <SearchableSetting keywords={[t["settings-audio"], t["settings-audio-desc"], "loopback"]}>
                    <DropdownItem
                        value={settings.audio.loopbackType}
                        onChange={(v) => {
                            const audioSettings = structuredClone({ ...settings.audio });
                            audioSettings.loopbackType = v as Settings["audio"]["loopbackType"];
                            setConfig("audio", audioSettings);
                        }}
                        title={t["settings-audio"]}
                        note={nativeNote(t["settings-audio-desc"], "audio")}
                        disabled={nativeOff("audio")}
                        link="https://www.electronjs.org/docs/latest/api/session#sessetdisplaymediarequesthandlerhandler-opts"
                        options={[
                            { label: t["settings-audio-loopback"], value: "loopback" },
                            { label: t["settings-audio-loopbackWithMute"], value: "loopbackWithMute" },
                        ]}
                    />
                </SearchableSetting>
                <SearchableSetting
                    keywords={[t["settings-hardwareAcceleration"], t["settings-hardwareAcceleration-desc"]]}
                >
                    <SwitchItem
                        note={nativeNote(t["settings-hardwareAcceleration-desc"], "hardwareAcceleration")}
                        value={settings.hardwareAcceleration}
                        disabled={nativeOff("hardwareAcceleration")}
                        onChange={(e: boolean) => setConfig("hardwareAcceleration", e, true)}
                    >
                        {t["settings-hardwareAcceleration"]}
                    </SwitchItem>
                </SearchableSetting>
                <Show when={window.remcord.platform === "darwin" || window.remcord.platform === "win32"}>
                    <SearchableSetting
                        keywords={[
                            t["settings-sdpH264BaselineRewrite"],
                            t["settings-sdpH264BaselineRewrite-desc"],
                            "H264",
                            "SDP",
                            "VideoToolbox",
                            "OpenH264",
                        ]}
                    >
                        <SwitchItem
                            note={nativeNote(t["settings-sdpH264BaselineRewrite-desc"], "sdpH264BaselineRewrite")}
                            value={settings.sdpH264BaselineRewrite ?? true}
                            disabled={nativeOff("sdpH264BaselineRewrite")}
                            onChange={(e: boolean) => setConfig("sdpH264BaselineRewrite", e, true)}
                        >
                            {t["settings-sdpH264BaselineRewrite"]}
                        </SwitchItem>
                    </SearchableSetting>
                </Show>
                <Show when={window.remcord.platform === "linux"}>
                    <SearchableSetting keywords={[t["settings-vaapi"], t["settings-vaapi-desc"], "VAAPI"]}>
                        <SwitchItem
                            note={nativeNote(t["settings-vaapi-desc"], "vaapi")}
                            value={settings.vaapi}
                            disabled={nativeOff("vaapi")}
                            onChange={(e: boolean) => setConfig("vaapi", e, true)}
                        >
                            {t["settings-vaapi"]}
                        </SwitchItem>
                    </SearchableSetting>
                </Show>
                <SearchableSetting
                    keywords={[t["settings-automaticClientUpdates"], t["settings-automaticClientUpdates-desc"]]}
                >
                    <SwitchItem
                        note={nativeNote(t["settings-automaticClientUpdates-desc"], "automaticUpdates")}
                        value={settings.automaticUpdates}
                        disabled={nativeOff("automaticUpdates")}
                        onChange={(e: boolean) => setConfig("automaticUpdates", e, true)}
                    >
                        {t["settings-automaticClientUpdates"]}
                    </SwitchItem>
                </SearchableSetting>
                <SearchableSetting keywords={[t["settings-disableHttpCache"], t["settings-disableHttpCache-desc"]]}>
                    <SwitchItem
                        note={nativeNote(t["settings-disableHttpCache-desc"], "disableHttpCache")}
                        value={settings.disableHttpCache}
                        disabled={nativeOff("disableHttpCache")}
                        onChange={(e: boolean) => setConfig("disableHttpCache", e, true)}
                    >
                        {t["settings-disableHttpCache"]}
                    </SwitchItem>
                </SearchableSetting>
                <SearchableSetting
                    keywords={[t["settings-additionalArguments"], t["settings-additionalArguments-desc"]]}
                >
                    <TextBoxItem
                        title={t["settings-additionalArguments"]}
                        note={nativeNote(t["settings-additionalArguments-desc"], "additionalArguments")}
                        value={settings.additionalArguments}
                        disabled={nativeOff("additionalArguments")}
                        onInput={(v: string) => setConfig("additionalArguments", v)}
                    />
                </SearchableSetting>
                <SearchableSetting
                    keywords={[
                        t["settings-noBundleUpdates"],
                        t["settings-noBundleUpdates-desc"],
                        t["settings-mod-shelter"],
                    ]}
                >
                    <SwitchItem
                        note={nativeNote(t["settings-noBundleUpdates-desc"], "noBundleUpdates")}
                        value={noBundleUpdates(settings).includes("shelter")}
                        disabled={nativeOff("noBundleUpdates")}
                        onChange={(e: boolean) => {
                            const next = new Set(noBundleUpdates(settings));
                            if (e) next.add("shelter");
                            else next.delete("shelter");
                            setConfig("noBundleUpdates", Array.from(next) as Settings["noBundleUpdates"], true);
                        }}
                    >
                        {t["settings-mod-shelter"]}
                    </SwitchItem>
                </SearchableSetting>
                <Show when={settings.mods.includes("vencord")}>
                    <SearchableSetting keywords={[t["settings-noBundleUpdates"], "Vencord"]}>
                        <SwitchItem
                            value={noBundleUpdates(settings).includes("vencord")}
                            note={nativeNote("", "noBundleUpdates")}
                            disabled={nativeOff("noBundleUpdates")}
                            onChange={(e: boolean) => {
                                const next = new Set(noBundleUpdates(settings));
                                if (e) next.add("vencord");
                                else next.delete("vencord");
                                setConfig("noBundleUpdates", Array.from(next) as Settings["noBundleUpdates"], true);
                            }}
                        >
                            Vencord
                        </SwitchItem>
                    </SearchableSetting>
                </Show>
                <Show when={settings.mods.includes("equicord")}>
                    <SearchableSetting keywords={[t["settings-noBundleUpdates"], "Equicord"]}>
                        <SwitchItem
                            value={noBundleUpdates(settings).includes("equicord")}
                            note={nativeNote("", "noBundleUpdates")}
                            disabled={nativeOff("noBundleUpdates")}
                            onChange={(e: boolean) => {
                                const next = new Set(noBundleUpdates(settings));
                                if (e) next.add("equicord");
                                else next.delete("equicord");
                                setConfig("noBundleUpdates", Array.from(next) as Settings["noBundleUpdates"], true);
                            }}
                        >
                            Equicord
                        </SwitchItem>
                    </SearchableSetting>
                </Show>
                <Show when={settings.mods.includes("custom")}>
                    <SearchableSetting keywords={[t["settings-noBundleUpdates"], t["settings-mod-custom"]]}>
                        <SwitchItem
                            value={noBundleUpdates(settings).includes("custom")}
                            note={nativeNote("", "noBundleUpdates")}
                            disabled={nativeOff("noBundleUpdates")}
                            onChange={(e: boolean) => {
                                const next = new Set(noBundleUpdates(settings));
                                if (e) next.add("custom");
                                else next.delete("custom");
                                setConfig("noBundleUpdates", Array.from(next) as Settings["noBundleUpdates"], true);
                            }}
                        >
                            {t["settings-mod-custom"]}
                        </SwitchItem>
                    </SearchableSetting>
                </Show>
                <div class={classes.panelActions}>
                    <SearchableSetting keywords={[t["settings-openCustomIconDialog"], "icon"]}>
                        <Button
                            size={ButtonSizes.MAX}
                            disabled={isNativeWebView()}
                            title={
                                isNativeWebView()
                                    ? (t["settings-nativeUnsupported"] ?? "Not available in the native Remcord app")
                                    : undefined
                            }
                            onClick={window.remcord.settings.openCustomIconDialog}
                        >
                            {t["settings-openCustomIconDialog"]}
                        </Button>
                    </SearchableSetting>
                    <SearchableSetting keywords={[t["settings-storageFolder"], "storage"]}>
                        <Button
                            size={ButtonSizes.MAX}
                            onClick={() => {
                                // Native WebView2 reveals via the typed folders.open capability;
                                // Electron keeps the original fire-and-forget send.
                                if (!revealFolderNative("storage")) window.remcord.settings.openStorageFolder();
                            }}
                        >
                            {t["settings-storageFolder"]}
                        </Button>
                    </SearchableSetting>
                    <SearchableSetting keywords={[t["settings-copyDebugInfo"], "debug"]}>
                        <Button
                            size={ButtonSizes.MAX}
                            disabled={isNativeWebView()}
                            title={
                                isNativeWebView()
                                    ? (t["settings-nativeUnsupported"] ?? "Not available in the native Remcord app")
                                    : undefined
                            }
                            onClick={window.remcord.settings.copyDebugInfo}
                        >
                            {t["settings-copyDebugInfo"]}
                        </Button>
                    </SearchableSetting>
                    <SearchableSetting keywords={[t["settings-copyGPUInfo"], "GPU"]}>
                        <Button
                            size={ButtonSizes.MAX}
                            disabled={isNativeWebView()}
                            title={
                                isNativeWebView()
                                    ? (t["settings-nativeUnsupported"] ?? "Not available in the native Remcord app")
                                    : undefined
                            }
                            onClick={window.remcord.settings.copyGPUInfo}
                        >
                            {t["settings-copyGPUInfo"]}
                        </Button>
                    </SearchableSetting>
                    <SearchableSetting keywords={[t["settings-openWebRTCInternals"], "WebRTC", "internals"]}>
                        <Button
                            size={ButtonSizes.MAX}
                            disabled={isNativeWebView()}
                            title={
                                isNativeWebView()
                                    ? (t["settings-nativeUnsupported"] ?? "Not available in the native Remcord app")
                                    : undefined
                            }
                            onClick={window.remcord.settings.openWebRTCInternals}
                        >
                            {t["settings-openWebRTCInternals"]}
                        </Button>
                    </SearchableSetting>
                    <SearchableSetting keywords={[t["settings-openGPUInfo"], "GPU", "chrome"]}>
                        <Button
                            size={ButtonSizes.MAX}
                            disabled={isNativeWebView()}
                            title={
                                isNativeWebView()
                                    ? (t["settings-nativeUnsupported"] ?? "Not available in the native Remcord app")
                                    : undefined
                            }
                            onClick={window.remcord.settings.openGPUInfo}
                        >
                            {t["settings-openGPUInfo"]}
                        </Button>
                    </SearchableSetting>
                    <SearchableSetting keywords={[t["settings-clearClientModCache"], "cache"]}>
                        <Button
                            size={ButtonSizes.MAX}
                            disabled={isNativeWebView()}
                            title={
                                isNativeWebView()
                                    ? (t["settings-nativeUnsupported"] ?? "Not available in the native Remcord app")
                                    : undefined
                            }
                            onClick={() => setConfig("modCache", {} as Settings["modCache"])}
                        >
                            {t["settings-clearClientModCache"]}
                        </Button>
                    </SearchableSetting>
                </div>
            </SettingsPanel>
        </SettingsSearchProvider>
    );
}

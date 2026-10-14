// Regression guard for Phase C/C-2: the Electron settings and localization
// paths must remain unchanged. Native WebView2 capabilities are additional
// async paths (deliberate `isNativeWebView()` branches) and must never
// rewrite these Electron contracts.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const bridge = readFileSync(new URL("../../src/discord/preload/bridge.ts", import.meta.url), "utf8");
const ipc = readFileSync(new URL("../../src/discord/ipc.ts", import.meta.url), "utf8");
const power = readFileSync(new URL("../../src/shelter/power/index.ts", import.meta.url), "utf8");
const shelterSettings = readFileSync(new URL("../../src/shelter/settings/settings.ts", import.meta.url), "utf8");
const settingsPage = readFileSync(
    new URL("../../src/shelter/settings/pages/SettingsPage.tsx", import.meta.url),
    "utf8",
);
const types = readFileSync(new URL("../../src/@types/remcordWindow.d.ts", import.meta.url), "utf8");
const invite = readFileSync(new URL("../../src/discord/preload/inviteBackButton.mts", import.meta.url), "utf8");
const heroUpdater = readFileSync(
    new URL("../../src/shelter/settings/components/HeroUpdater.tsx", import.meta.url),
    "utf8",
);

test("preload keeps the synchronous getEntireConfig bridge", () => {
    assert.match(bridge, /getConfig:\s*\(\)\s*=>\s*ipcRenderer\.sendSync\("getEntireConfig"\)/);
    assert.doesNotMatch(bridge, /settings[\s\S]{0,200}invoke\("getEntireConfig"/);
});

test("main keeps the synchronous getEntireConfig/getConfig handlers", () => {
    assert.match(ipc, /ipcMain\.on\("getEntireConfig"/);
    assert.match(ipc, /event\.returnValue = getEntireConfig\(\)/);
    assert.match(ipc, /ipcMain\.on\("getConfig"/);
});

test("declared window.remcord settings contract stays synchronous", () => {
    assert.match(types, /getConfig:\s*\(\)\s*=>\s*Readonly<Settings>/);
});

test("the Shelter power Electron path stays synchronous", () => {
    // The Electron branch must call the sync getter directly in onLoad.
    assert.match(power, /const settings = window\.remcord\.settings\.getConfig\(\);/);
    assert.match(power, /if \(!settings\.blockPowerSavingInVoiceChat\) return;/);
});

test("the native WebView2 branch is a deliberate separate path", () => {
    assert.match(power, /isNativeWebView/);
    assert.match(power, /onLoadNative/);
    assert.match(power, /await getConfig\(\)/);
    // The native branch must not use Electron synchronous IPC.
    assert.doesNotMatch(power, /sendSync/);
});

test("preload keeps the async getLang bridge", () => {
    assert.match(bridge, /getLang:\s*async\s*\(toGet:\s*string\)/);
    assert.match(bridge, /await ipcRenderer\.invoke\("getLang", toGet\)/);
});

test("main keeps the getLang handler returning the catalog string", () => {
    assert.match(ipc, /ipcMain\.handle\("getLang"/);
    assert.match(ipc, /return getLang\(toGet\)/);
});

test("declared window.remcord getLang contract stays async", () => {
    assert.match(types, /getLang:\s*\(toGet:\s*string\)\s*=>\s*Promise<string>/);
});

test("the invite back-button Electron path keeps its raw async lookup", () => {
    // Electron behavior: direct invoke plus the English fallback.
    assert.match(invite, /ipcRenderer\.invoke\("getLang", "invite-goBackToApp"\)/);
    assert.match(invite, /"Go back to the Discord app"/);
    // JANK-02: no polling interval may be reintroduced.
    assert.doesNotMatch(invite, /setInterval/);
});

test("the invite back-button native path is a deliberate separate branch", () => {
    assert.match(invite, /isNativeWebView/);
    assert.match(invite, /\.getLang\(\s*"invite-goBackToApp"\s*,?\s*\)/);
    assert.doesNotMatch(invite, /sendSync/);
});

test("preload keeps the synchronous setConfig bridge", () => {
    assert.match(
        bridge,
        /setConfig:\s*\(key:\s*string,\s*value:\s*unknown\)\s*=>\s*ipcRenderer\.sendSync\("setConfig", key, value\)/,
    );
    assert.doesNotMatch(bridge, /invoke\("setConfig"/);
});

test("main keeps the synchronous setConfig handler returning undefined", () => {
    assert.match(ipc, /ipcMain\.on\("setConfig"/);
    assert.match(ipc, /setConfig\(key, value\)/);
    assert.match(ipc, /event\.returnValue = undefined/);
});

test("declared window.remcord setConfig contract stays synchronous", () => {
    assert.match(types, /setConfig:\s*<K extends keyof Settings>\(object:\s*K,\s*toSet:\s*Settings\[K\]\)\s*=>\s*void/);
});

test("the Shelter settings Electron mutation path stays synchronous", () => {
    // The shared helper must still persist via the sync bridge call.
    assert.match(shelterSettings, /window\.remcord\.settings\.setConfig\(key, value\)/);
    // The power-panel switch keeps the original Electron call byte-identical.
    assert.match(settingsPage, /setConfig\("blockPowerSavingInVoiceChat", e, true\)/);
});

test("the block-power-saving native mutation is a deliberate separate path", () => {
    assert.match(shelterSettings, /isNativeWebView/);
    assert.match(shelterSettings, /setBlockPowerSavingInVoiceChat/);
    assert.match(shelterSettings, /await nativeSet\(value\)/);
    assert.match(settingsPage, /void setBlockPowerSavingInVoiceChat\(e\)/);
    // The native path must not use Electron synchronous IPC or a generic key.
    assert.doesNotMatch(shelterSettings, /sendSync/);
    assert.doesNotMatch(settingsPage, /sendSync/);
    assert.doesNotMatch(shelterSettings, /setBlockPowerSavingInVoiceChat\([^)]*key/);
});

test("the closed settings.set allowlist is a deliberate separate path", () => {
    // The gate reuses the existing environment check plus function presence.
    assert.match(shelterSettings, /export function isNativeSettingsSetAvailable/);
    assert.match(shelterSettings, /export async function setNativeSetting/);
    // Seven wire keys: mods, three side-effect-free flags, csp, quickCss,
    // and channel (each with a traced native consumer).
    assert.match(shelterSettings, /mods: "mods"/);
    assert.match(shelterSettings, /supportBannerDismissed: "support_banner_dismissed"/);
    assert.match(shelterSettings, /showExperimentalPluginMenu: "show_experimental_plugin_menu"/);
    assert.match(shelterSettings, /extendedPluginAbilities: "extended_plugin_abilities"/);
    assert.match(shelterSettings, /csp: "csp"/);
    assert.match(shelterSettings, /quickCss: "quick_css"/);
    assert.match(shelterSettings, /channel: "channel"/);
    // No Electron-only or unsafe key may enter the map.
    assert.doesNotMatch(shelterSettings, /minimizeToTray|proxyRules|customIcon|additionalArguments|keybinds/);
    // toggleMod keeps the mutual-exclusion shape and branches per shell.
    assert.match(shelterSettings, /setNativeSetting\("mods", next, true\)/);
    assert.match(shelterSettings, /setConfig\("mods", next\)/);
    const supportBanner = readFileSync(
        new URL("../../src/shelter/settings/components/SupportBanner.tsx", import.meta.url),
        "utf8",
    );
    assert.match(supportBanner, /setNativeSetting\("supportBannerDismissed", true\)/);
    assert.match(supportBanner, /setConfig\("supportBannerDismissed", true\)/);
    assert.match(settingsPage, /setNativeSetting\("showExperimentalPluginMenu", e, true\)/);
    assert.match(settingsPage, /setNativeSetting\("extendedPluginAbilities", e\)/);
    assert.match(settingsPage, /setConfig\("showExperimentalPluginMenu", e, true\)/);
    assert.match(settingsPage, /setConfig\("extendedPluginAbilities", e\)/);
    // The new path adds no generic surface: no key/path/URL members, no
    // sync IPC, no eval, no dynamic execution.
    for (const source of [shelterSettings, settingsPage, supportBanner]) {
        assert.doesNotMatch(source, /settings_set_config/);
        assert.doesNotMatch(source, /invoke\(|sendSync|eval\(|new Function/);
    }
});

test("preload keeps the synchronous get-app-version bridge", () => {
    assert.match(bridge, /version:\s*ipcRenderer\.sendSync\("get-app-version", "app-version"\)/);
    assert.doesNotMatch(bridge, /invoke\("get-app-version"/);
});

test("main keeps the get-app-version handler returning the package version", () => {
    assert.match(ipc, /ipcMain\.on\("get-app-version"/);
    assert.match(ipc, /event\.returnValue = getVersion\(\)/);
});

test("declared window.remcord version contract stays a synchronous string", () => {
    assert.match(types, /version:\s*string/);
});

test("the update checker Electron path keeps its synchronous version read", () => {
    // Electron behavior: direct sync read plus the dot-stripped comparison.
    assert.match(heroUpdater, /return window\.remcord\.version;/);
    assert.match(heroUpdater, /await getLocalVersion\(\)/);
});

test("the update checker native path is a deliberate separate branch", () => {
    assert.match(heroUpdater, /boot\?\.runtime === "webview2"/);
    assert.match(heroUpdater, /native\.getVersion\(\)/);
    assert.doesNotMatch(heroUpdater, /sendSync/);
    assert.doesNotMatch(heroUpdater, /invoke\(/);
});

test("preload keeps the scoped plugin-storage fs bridge", () => {
    // Third-party plugin API: scoped (pluginId, relativePath) over the
    // pluginWriteFile/pluginReadFile channels. The native milestone adds a
    // separate typed capability and must not rewrite this contract.
    assert.match(bridge, /writeFile:\s*\(pluginId:\s*string, relativePath:\s*string, data:\s*string\)/);
    assert.match(bridge, /readFile:\s*\(pluginId:\s*string, relativePath:\s*string\)/);
    assert.match(bridge, /ipcRenderer\.invoke\("pluginWriteFile", pluginId, relativePath, data\)/);
    assert.match(bridge, /ipcRenderer\.invoke\("pluginReadFile", pluginId, relativePath\)/);
});

test("main keeps the gated pluginWriteFile/pluginReadFile handlers", () => {
    assert.match(ipc, /ipcMain\.handle\(\s*"pluginWriteFile"/);
    assert.match(ipc, /ipcMain\.handle\(\s*"pluginReadFile"/);
    assert.match(ipc, /if\s*\(!getConfig\("extendedPluginAbilities"\)\)/);
    assert.match(ipc, /EXTENSION_DISABLED/);
    assert.match(ipc, /function sanitizePluginId/);
    assert.match(ipc, /function resolvePluginFilePath/);
});

test("declared window.remcord fs contract stays scoped", () => {
    assert.match(types, /fs:\s*\{/);
    assert.match(types, /writeFile:\s*\(/);
    assert.match(types, /readFile:\s*\(/);
});

test("no src caller is migrated to a native plugin-storage path in this milestone", () => {
    // There are no in-repository window.remcord.fs callers: the contract is
    // a public third-party plugin API. This milestone therefore branches no
    // src/ caller; a DEV-only native smoke probe exercises the contract.
    assert.doesNotMatch(power, /pluginStorage/);
    assert.doesNotMatch(shelterSettings, /pluginStorage/);
    assert.doesNotMatch(heroUpdater, /pluginStorage/);
    assert.doesNotMatch(invite, /pluginStorage/);
});

const themesMain = readFileSync(new URL("../../src/common/themes.ts", import.meta.url), "utf8");
const themesPreload = readFileSync(new URL("../../src/discord/preload/themes.ts", import.meta.url), "utf8");
const nativeFacade = readFileSync(new URL("../web/compatibility.js", import.meta.url), "utf8");
const nativeMain = readFileSync(new URL("../src/main.rs", import.meta.url), "utf8");

test("the loader probe adds no renderer-facing loader API", () => {
    // The Shelter loader probe is host DEV-only: no shipped renderer code
    // sends or handles it, and the frozen bridge facade gains no plugin
    // loading surface (no addLocalPlugin/startPlugin passthrough, no eval,
    // no URL/path/source input).
    for (const source of [bridge, ipc, types]) {
        assert.doesNotMatch(source, /shelter_loader_probe/);
        assert.doesNotMatch(source, /SHELTER_LOADER_PROBE/);
        assert.doesNotMatch(source, /addLocalPlugin|ensureLoaderPlugin/);
    }
    assert.doesNotMatch(nativeFacade, /addLocalPlugin|startPlugin|stopPlugin|removePlugin/);
    assert.doesNotMatch(nativeFacade, /shelter_loader_probe/);
    // The probe script itself is a fixed host string: the only id, source,
    // and manifest in the chain are literals, never renderer input.
    assert.match(nativeMain, /SHELTER_LOADER_PROBE_SCRIPT/);
    assert.match(nativeMain, /remcord-loader-probe/);
    // The probe re-runs after Shelter's own bootstrap marker (its global is
    // async), gated to Shelter so other mods never duplicate it.
    assert.match(nativeMain, /loader_probe_after_marker/);
    assert.match(nativeMain, /descriptor\.id == SHELTER_MOD\.id/);
});

test("Electron keeps the host-push theme injection pipeline", () => {
    // Startup: discover manifests, resolve manifest.theme, cache CSS, push on
    // did-finish-load with Quick CSS first. No renderer pull exists.
    assert.match(themesMain, /export function injectThemesMain/);
    assert.match(themesMain, /buildThemeCssCache\(\)/);
    assert.match(themesMain, /did-finish-load/);
    assert.match(themesMain, /webContents\.send\("addTheme", id, css\)/);
    assert.match(themesMain, /if \(getConfig\("quickCss"\)\) initQuickCss\(browserWindow\)/);
    assert.match(themesPreload, /ipcRenderer\.on\("addTheme"/);
    assert.match(themesPreload, /ipcRenderer\.on\("removeTheme"/);
});

test("Electron keeps the sync theme-list and enable/disable contracts", () => {
    assert.match(bridge, /getThemes:\s*\(\)\s*=>\s*ipcRenderer\.sendSync\("getThemes"\)/);
    assert.match(
        bridge,
        /set:\s*\(id:\s*string, state:\s*boolean\)\s*=>\s*ipcRenderer\.send\("setThemeEnabled", id, state\)/,
    );
    assert.match(ipc, /ipcMain\.on\("getThemes"/);
    assert.match(ipc, /ipcMain\.on\("setThemeEnabled"/);
});

test("no renderer theme CSS pull exists in Electron or native", () => {
    // The investigation result this milestone preserves: themes are
    // host -> renderer pushes with no pull-style reader, so no
    // themes.getCSS() bridge may be introduced on either side.
    assert.doesNotMatch(bridge, /getCSS/);
    assert.doesNotMatch(types, /getCSS/);
    assert.doesNotMatch(themesMain, /getCSS/);
    assert.doesNotMatch(nativeFacade, /getCSS/);
    assert.doesNotMatch(nativeFacade, /addTheme/);
    assert.doesNotMatch(nativeFacade, /removeTheme/);
    assert.doesNotMatch(nativeFacade, /remcord-native-theme/);
});

test("no src caller is migrated to a native theme path in this milestone", () => {
    // Static startup injection is host-driven: no renderer caller changes.
    assert.doesNotMatch(power, /remcord-native-theme/);
    assert.doesNotMatch(shelterSettings, /remcord-native-theme/);
    assert.doesNotMatch(heroUpdater, /remcord-native-theme/);
    assert.doesNotMatch(invite, /remcord-native-theme/);
});

test("Electron keeps the Quick CSS host-push lifecycle", () => {
    // Missing file is created empty, content (even empty) is always pushed
    // as remcord-quick-css BEFORE themes, with a 300 ms-debounced watcher
    // for live updates and removeTheme on disable.
    assert.match(themesMain, /const quickCssPath = path\.join\(userDataPath, "\/quickCss\.css"\)/);
    assert.match(themesMain, /if \(!fs\.existsSync\(quickCssPath\)\)/);
    assert.match(themesMain, /fs\.writeFileSync\(quickCssPath, ""\)/);
    assert.match(themesMain, /webContents\.send\("addTheme", "remcord-quick-css"/);
    assert.match(themesMain, /webContents\.send\(\s*"removeTheme", "remcord-quick-css"\)/);
    assert.match(themesMain, /}, 300\)/);
});

test("Electron keeps the Quick CSS enablement contract", () => {
    // Enabled flag is settings.quickCss (default true); startup guards on
    // it inside injectThemesMain, the Themes page toggles via enable/disable
    // IPC plus setConfig.
    assert.match(themesMain, /if \(getConfig\("quickCss"\)\) initQuickCss\(browserWindow\)/);
    assert.match(ipc, /ipcMain\.on\("enableQuickCss"/);
    assert.match(ipc, /ipcMain\.on\("disableQuickCss"/);
    const config = readFileSync(new URL("../../src/common/config.ts", import.meta.url), "utf8");
    assert.match(config, /quickCss: true,/);
    const themesPage = readFileSync(
        new URL("../../src/shelter/settings/pages/ThemesPage.tsx", import.meta.url),
        "utf8",
    );
    assert.match(themesPage, /window\.remcord\.themes\.enableQuickCss\(\)/);
    assert.match(themesPage, /window\.remcord\.themes\.disableQuickCss\(\)/);
    assert.match(themesPage, /setConfig\("quickCss", e\)/);
});

test("no renderer Quick CSS pull exists in Electron; native exposes only the editor pair", () => {
    // Quick CSS is a host -> renderer push with no pull-style reader: the
    // Electron bridge exposes only push-side members (open/import/enable/
    // disable via fire-and-forget send), never a CSS-returning read. No
    // quickCss.get() shape may be introduced on either side.
    assert.doesNotMatch(bridge, /getQuickCss|readQuickCss|invoke\("getQuickCss"/);
    assert.doesNotMatch(types, /getQuickCss|readQuickCss/);
    assert.doesNotMatch(themesMain, /getQuickCss|quickCss\.get|getCSS/);
    // No quickCss.get() shape may be introduced on either side. The backup
    // include key `remcordThemesAndQuickCss` is Electron's exact save-shape
    // member (not a CSS reader), so the native assertion targets pull-side
    // tokens instead of the bare substring. The native in-Discord editor is
    // the deliberate exception: exactly the fixed-file `quickcss.read` /
    // `quickcss.write` pair (capability-gated, bounded, no path), and no
    // other pull-shaped member.
    assert.doesNotMatch(nativeFacade, /getQuickCss|quickCss\.get|remcord-quick-css/);
    assert.match(nativeFacade, /requireCapability\("quickcss\.read"\)/);
    assert.match(nativeFacade, /requireCapability\("quickcss\.write"\)/);
    for (const token of ["quickcss.delete", "quickcss.list", "quickcss.open", "quickcss.get"]) {
        assert.doesNotMatch(nativeFacade, new RegExp(token.replace(".", "\\.")));
    }
});

test("no src caller is migrated to a native Quick CSS path in this milestone", () => {
    // Static startup injection is host-driven: no renderer caller changes.
    assert.doesNotMatch(power, /remcord-quick-css/);
    assert.doesNotMatch(shelterSettings, /remcord-quick-css/);
    assert.doesNotMatch(heroUpdater, /remcord-quick-css/);
    assert.doesNotMatch(invite, /remcord-quick-css/);
});

test("Electron keeps the four fixed-name bundle handlers byte-identical", () => {
    // Every client-mod bundle handler still reads only fixed userData file
    // names and returns `{ js, css?, enabled }`. The native milestone
    // replaces none of these; it adds a host-controlled registry beside them.
    assert.match(ipc, /ipcMain\.handle\("getShelterBundle"/);
    assert.match(ipc, /ipcMain\.handle\("getVencordBundle"/);
    assert.match(ipc, /ipcMain\.handle\("getEquicordBundle"/);
    assert.match(ipc, /ipcMain\.handle\("getCustomBundle"/);
    assert.match(ipc, /ifExistsRead\(path\.join\(app\.getPath\("userData"\), "shelter\.js"\)\)/);
    assert.match(ipc, /ifExistsRead\(path\.join\(app\.getPath\("userData"\), "vencord\.js"\)\)/);
    assert.match(ipc, /ifExistsRead\(path\.join\(app\.getPath\("userData"\), "vencord\.css"\)\)/);
    assert.match(ipc, /ifExistsRead\(path\.join\(app\.getPath\("userData"\), "equicord\.js"\)\)/);
    assert.match(ipc, /ifExistsRead\(path\.join\(app\.getPath\("userData"\), "custom\.js"\)\)/);
    assert.match(ipc, /enabled: getConfig\("mods"\)\.includes\("vencord"\)/);
    assert.match(ipc, /function ifExistsRead\(path: string\): string \| undefined/);
    // Preload consumers are unchanged: each invokes its own channel with no
    // arguments and executes whatever source the handler returned.
    const vencordPreload = readFileSync(new URL("../../src/discord/preload/mods/vencord.mts", import.meta.url), "utf8");
    assert.match(vencordPreload, /ipcRenderer\.invoke\("getVencordBundle"\)/);
    assert.match(vencordPreload, /webFrame\.executeJavaScript\(bundle\.js\)/);
    assert.match(vencordPreload, /webFrame\.insertCSS\(bundle\.css!\)/);
    const modBundleTypes = readFileSync(new URL("../../src/@types/ModBundle.d.ts", import.meta.url), "utf8");
    assert.match(modBundleTypes, /interface ModBundle/);
    assert.match(modBundleTypes, /js: string/);
    assert.match(modBundleTypes, /css\?: string/);
    assert.match(modBundleTypes, /enabled: boolean/);
    // The declared renderer contract stays a raw-source bundle: the native
    // path deliberately does NOT grow a typed read for it.
    assert.doesNotMatch(types, /ModBundle/);
});

const shelterPreload = readFileSync(new URL("../../src/discord/preload/mods/shelter.mts", import.meta.url), "utf8");
const equicordPreload = readFileSync(new URL("../../src/discord/preload/mods/equicord.mts", import.meta.url), "utf8");
const consts = readFileSync(new URL("../../src/common/consts.ts", import.meta.url), "utf8");

test("Electron keeps the Shelter preload wrapper and its injector plugin map", () => {
    // Shelter's Electron path wraps the bundle in a host-built IIFE that
    // declares SHELTER_INJECTOR_PLUGINS and adds a macOS-only Touch Bar entry.
    // The native milestone reproduces the declaration in local (pinned) form
    // for the servable builtins (see the native injector test below); the
    // Electron file must therefore stay byte-identical.
    assert.match(
        shelterPreload,
        /const requiredPlugins: Record<string, \[string, \{ isVisible: boolean; allowedActions: Record<string, true> \}\]> = \{/,
    );
    assert.match(shelterPreload, /"remcord-arrpc": \["remcord:\/\/plugins\/rpc\/"/);
    assert.match(shelterPreload, /"remcord-settings": \["remcord:\/\/plugins\/settings\/"/);
    assert.match(shelterPreload, /"remcord-power": \["remcord:\/\/plugins\/power\/"/);
    assert.match(shelterPreload, /"remcord-screenshare": \["remcord:\/\/plugins\/screenshare\/"/);
    assert.match(shelterPreload, /"remcord-titlebar": \["remcord:\/\/plugins\/titlebar\/"/);
    // Platform-specific behavior: Touch Bar is Darwin-only.
    assert.match(shelterPreload, /if \(process\.platform === "darwin"\)/);
    assert.match(shelterPreload, /"remcord-touchbar"/);
    // The wrapper itself, and the fact that Shelter gets NO CSS.
    assert.match(shelterPreload, /webFrame\.executeJavaScript\(`\(\(\)=>\{/);
    assert.match(shelterPreload, /const SHELTER_INJECTOR_PLUGINS = \$\{JSON\.stringify\(requiredPlugins\)\};/);
    assert.doesNotMatch(shelterPreload, /insertCSS/);
    // Enabled is hardcoded true at the handler, so Shelter is NOT gated on
    // settings.mods.
    assert.match(
        ipc,
        /ipcMain\.handle\("getShelterBundle", \(\) => \{\s*return \{\s*js: ifExistsRead\(path\.join\(app\.getPath\("userData"\), "shelter\.js"\)\),\s*enabled: true,/,
    );
});

test("native Shelter injector declares the servable builtins in local form", () => {
    // The native host prepends `const SHELTER_INJECTOR_PLUGINS=<map>;` to the
    // Shelter execution script (Electron's wrapper order), built from
    // build-embedded artifacts — never fetched, never renderer-supplied.
    const injector = readFileSync(new URL("../src/shelter_injector.rs", import.meta.url), "utf8");
    const main = readFileSync(new URL("../src/main.rs", import.meta.url), "utf8");
    const build = readFileSync(new URL("../build.rs", import.meta.url), "utf8");
    // Exactly the two servable builtins, keyed by Electron's exact ids.
    // (The exclusions — host-owned settings/titlebar, Electron-only
    // screenshare, darwin-only touchbar — are asserted in the Rust
    // `injector_ids_are_exactly_the_two_servable_builtins` test, since this
    // module documents them in comments.)
    assert.match(injector, /&\["remcord-arrpc", "remcord-power"\]/);
    // Local form: inline source plus manifest, pinned, with Electron's exact
    // visibility values — no source URL to fetch, no update checks.
    assert.match(injector, /"update": false/);
    assert.match(injector, /"isVisible": false/);
    assert.match(injector, /"allowedActions": \{\}/);
    // Embedded at compile time from the lune pipeline output (same contract
    // as the settings/titlebar artifacts), with name pins and markers.
    assert.match(build, /injector_id: "remcord-arrpc"/);
    assert.match(build, /injector_id: "remcord-power"/);
    assert.match(build, /manifest_name: "Remcord RPC"/);
    assert.match(build, /manifest_name: "Remcord Power Management"/);
    // The declaration leads the Shelter execution script only: other mods
    // keep their exact previous shape.
    assert.match(main, /execution_script_for\(&descriptor, &bundle\.js, &shelter_injector_decl\)/);
    assert.match(main, /if descriptor\.id == SHELTER_MOD\.id/);
});

test("Electron keeps the Equicord preload CSS requirement", () => {
    // Equicord explicitly requires CSS; Vencord has the same note. Both must
    // stay byte-identical so the native CSS stage keeps its parity target.
    assert.match(equicordPreload, /ipcRenderer\.invoke\("getEquicordBundle"\)/);
    assert.match(equicordPreload, /webFrame\.executeJavaScript\(bundle\.js\)/);
    assert.match(equicordPreload, /webFrame\.insertCSS\(bundle\.css!\); \/\/NOTE - Equicord requires CSS\./);
    assert.match(ipc, /enabled: getConfig\("mods"\)\.includes\("equicord"\)/);
    // Both mods still download from the same fixed upstream URLs.
    assert.match(consts, /js: "https:\/\/github\.com\/Vendicated\/Vencord\/releases\/download\/devbuild\/browser\.js"/);
    assert.match(consts, /js: "https:\/\/github\.com\/Equicord\/Equicord\/releases\/download\/latest\/browser\.js"/);
    assert.match(consts, /css: "https:\/\/github\.com\/Equicord\/Equicord\/releases\/download\/latest\/browser\.css"/);
    assert.match(consts, /js: "https:\/\/raw\.githubusercontent\.com\/uwu\/shelter-builds\/main\/shelter\.js"/);
    // Shelter has no CSS entry upstream at all: a native descriptor must not
    // invent one.
    assert.doesNotMatch(consts, /shelter: \{[\s\S]{0,400}?css:/);
});

test("the native registry serves Shelter and Equicord from those exact fixed sources", () => {
    const main = readFileSync(new URL("../src/main.rs", import.meta.url), "utf8");
    // Fixed upstream sources: the same values as src/common/consts.ts.
    assert.match(main, /js_url: "https:\/\/raw\.githubusercontent\.com\/uwu\/shelter-builds\/main\/shelter\.js"/);
    assert.match(main, /js_url: "https:\/\/github\.com\/Equicord\/Equicord\/releases\/download\/latest\/browser\.js"/);
    assert.match(main, /url: "https:\/\/github\.com\/Equicord\/Equicord\/releases\/download\/latest\/browser\.css"/);
    // Fixed cache names under the DEV profile, and fixed host allowlists.
    assert.match(main, /js_file: "shelter\.js"/);
    assert.match(main, /file: "equicord\.css"/);
    assert.match(main, /allowed_hosts: &\["raw\.githubusercontent\.com"\]/);
});

test("native Shelter and Equicord adapters add no Electron or Node surface", () => {
    const strip = (source) =>
        source
            .replace(/\/\*[\s\S]*?\*\//g, "")
            .replace(/^\s*\/\/.*$/gm, "")
            .replace(/biome-ignore[^\n]*\n/g, "");
    for (const name of ["shelter", "equicord", "custom"]) {
        const adapter = strip(readFileSync(new URL(`../web/mods/${name}/adapter.js`, import.meta.url), "utf8"));
        // Neither adapter reaches for an Electron/Node global.
        assert.doesNotMatch(adapter, /require\(|process\.|webFrame|ipcRenderer|contextBridge/, name);
        // The only host call is the shared logical-id bundle request.
        assert.match(adapter, /native\.getModBundle\("[a-z]+"\)/, name);
        assert.doesNotMatch(adapter, /readFile|writeFile|loadScript|executeSource|executeScript/, name);
        assert.doesNotMatch(adapter, /eval\(|new Function/, name);
        // And it names no URL beyond the three entry origins of the gate.
        assert.deepEqual(
            adapter.match(/https?:\/\/[^\s"'`)]+/g) ?? [],
            ["https://discord.com", "https://canary.discord.com", "https://ptb.discord.com"],
            name,
        );
    }
});

test("native plugin loader adds no Electron surface beyond the documented new Function primitive", () => {
    const strip = (source) =>
        source
            .replace(/\/\*[\s\S]*?\*\//g, "")
            .replace(/^\s*\/\/.*$/gm, "")
            .replace(/biome-ignore[^\n]*\n/g, "");
    const loader = strip(readFileSync(new URL("../web/plugins/loader.js", import.meta.url), "utf8"));
    // No Electron/Node globals: the loader talks only to window.remcord and
    // window.ipc, like every other document-start script.
    assert.doesNotMatch(loader, /require\(|process\.|webFrame|ipcRenderer|contextBridge/);
    assert.doesNotMatch(loader, /readFile|writeFile|loadScript|executeSource|executeScript/);
    // The single documented exception: plugin sources execute through
    // `new Function` with the closed api argument (Electron parity for the
    // execution primitive); no other eval surface may exist.
    assert.match(loader, /new Function\("api"/);
    assert.doesNotMatch(loader, /eval\(/);
    // Only the closed runtime capabilities plus the three entry origins.
    assert.match(loader, /getPluginEntries/);
    assert.match(loader, /getPluginScript/);
    assert.doesNotMatch(loader, /shelter-builds|githubusercontent|example\.invalid/);
    assert.deepEqual(loader.match(/https?:\/\/[^\s"'`)]+/g) ?? [], [
        "https://discord.com",
        "https://canary.discord.com",
        "https://ptb.discord.com",
    ]);
});

test("native bundle retrieval adds no renderer filesystem or exec surface", () => {
    // The bundle facade member is a scoped logical-id request only: no path,
    // URL, filename, source, generic read, or generic invocation. (The
    // plugin-storage `readFile` member is the already-validated Phase D
    // capability and is unrelated to bundles.)
    const bundleFacade = nativeFacade.slice(
        nativeFacade.indexOf("function getModBundle"),
        nativeFacade.indexOf("function isSettingsSnapshot"),
    );
    assert.ok(bundleFacade.length > 0);
    assert.doesNotMatch(bundleFacade, /readFile|loadScript|executeSource|eval\(|new Function/);
    assert.doesNotMatch(bundleFacade, /\bpath\b|\burl\b|\bfile\b|\bsource\b/);
    assert.match(bundleFacade, /requireCapability\("mod\.get_bundle"\)/);
    assert.match(bundleFacade, /mod_id: modId/);
    // The facade exposes exactly one bundle entry point plus the thin
    // Vencord convenience wrapper; there is no generic bundle reader.
    assert.equal((nativeFacade.match(/function get\w*Bundle\(/g) ?? []).length, 2);
    assert.doesNotMatch(nativeFacade, /function readBundle\(/);
});

test("Electron keeps the Quick CSS live file-watcher contract", () => {
    // initQuickCss watches the single file, reacts only to change events,
    // debounces 300 ms, then re-reads and pushes removeTheme + addTheme.
    // disableQuickCss and window close tear the watcher down.
    assert.match(themesMain, /quickCssWatcher = fs\.watch\(quickCssPath/);
    assert.match(themesMain, /if \(eventType === "change"\)/);
    assert.match(themesMain, /fs\.readFileSync\(quickCssPath, "utf-8"\)/);
    assert.match(themesMain, /quickCssWatcher\.close\(\)/);
});

test("Electron theme watching covers only the manifest list cache", () => {
    // startThemeWatcher watches the themes folder recursively, filters to
    // manifest.json/rename events, debounces 500 ms, and refreshes ONLY the
    // list cache. Per-theme CSS edits have no live update path: they need
    // an enable toggle or a reload.
    assert.match(themesMain, /themeWatcher = fs\.watch\(themesFolder, \{ recursive: true \}/);
    assert.match(themesMain, /filename\.endsWith\("manifest\.json"\)/);
    assert.match(themesMain, /}, 500\)/);
    assert.match(themesMain, /refreshThemeListCache\(\)/);
    assert.doesNotMatch(themesMain, /fs\.watch\(.*\.css/);
});

test("native live watching exposes no renderer filesystem-watch surface", () => {
    // Rust owns both watchers; the renderer must never watch the
    // filesystem itself (no fs.watch bridge, no mtime polling hook, no DOM
    // observer standing in for filesystem state).
    assert.doesNotMatch(nativeFacade, /fs\.watch/);
    assert.doesNotMatch(nativeFacade, /watchFile/);
    assert.doesNotMatch(nativeFacade, /MutationObserver/);
});

test("Electron keeps the backup save/restore handlers and bundle allowlist", () => {
    // backupSave takes one JSON string payload, builds the zip from the
    // host profile paths, and writes only through a host-mediated save
    // dialog; backupRestore takes no arguments and reads only through a
    // host-mediated open dialog.
    const backupMain = readFileSync(new URL("../../src/common/backup.ts", import.meta.url), "utf8");
    assert.match(ipc, /ipcMain\.handle\(\s*"backupSave"/);
    assert.match(ipc, /ipcMain\.handle\("backupRestore"/);
    assert.match(ipc, /buildBackupZipBuffer\(payload,/);
    assert.match(ipc, /applyBackupFromMap\(map,/);
    assert.match(ipc, /dialog\.showSaveDialog/);
    assert.match(ipc, /dialog\.showOpenDialog/);
    assert.match(backupMain, /const BUNDLE_FILES = \[/);
    // The declared renderer contract stays the Electron save/restore pair.
    assert.match(bridge, /ipcRenderer\.invoke\("backupSave", data\)/);
    assert.match(bridge, /ipcRenderer\.invoke\("backupRestore"\)/);
    assert.match(types, /save\(data: string\): Promise</);
    assert.match(types, /restore\(\): Promise<string>/);
});

test("the native bundle allowlist is the same 7 names in the same order", () => {
    // Semantic parity with the canonical Electron definition: extract the
    // quoted names from BUNDLE_FILES in src/common/backup.ts and compare
    // element-wise against native/src/bundles.rs. This is a literal
    // comparison of the canonical definitions, not a paraphrase.
    const backupMain = readFileSync(new URL("../../src/common/backup.ts", import.meta.url), "utf8");
    const nativeBundles = readFileSync(new URL("../src/bundles.rs", import.meta.url), "utf8");
    const electronBlock = backupMain.match(/const BUNDLE_FILES = \[([\s\S]*?)\];/);
    assert.ok(electronBlock);
    const electronNames = [...electronBlock[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]);
    const nativeBlock = nativeBundles.match(/pub const BUNDLE_FILES[^=]*=\s*&\[([\s\S]*?)\];/);
    assert.ok(nativeBlock);
    const nativeNames = [...nativeBlock[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]);
    assert.deepEqual(nativeNames, electronNames);
    assert.deepEqual(nativeNames, [
        "vencord.js",
        "vencord.css",
        "equicord.js",
        "equicord.css",
        "shelter.js",
        "custom.js",
        "custom.css",
    ]);
});

test("no src caller uses the native bundle discovery path", () => {
    // This slice migrates discovery only and adds no UI caller: a native
    // branch in src/ would be fabrication. The Electron backup UI keeps
    // calling save/restore, and nothing references the native capability.
    const backupSection = readFileSync(
        new URL("../../src/shelter/settings/components/BackupSection.tsx", import.meta.url),
        "utf8",
    );
    for (const source of [bridge, ipc, types, backupSection]) {
        assert.doesNotMatch(source, /listBundles/);
        assert.doesNotMatch(source, /backup\.list_bundles/);
        assert.doesNotMatch(source, /backup_list_bundles/);
    }
    assert.match(backupSection, /saveBackup\(JSON\.stringify\(payload\)\)/);
    assert.match(backupSection, /window\.remcord\.backup\.restore\(\)/);
});

test("custom bundles execute from the host-controlled local source natively", () => {
    // F1: the authoritative host-controlled source is the host-built
    // profile-root path (`custom_bundle.rs`), so presence in the inventory
    // means servability when `settings.mods` names `custom` — the renderer
    // still supplies only the logical id, never a path or URL.
    const nativeBundles = readFileSync(new URL("../src/bundles.rs", import.meta.url), "utf8");
    const nativeCustom = readFileSync(new URL("../src/custom_bundle.rs", import.meta.url), "utf8");
    const nativeBackup = readFileSync(new URL("../src/backup.rs", import.meta.url), "utf8");
    const nativeModCache = readFileSync(new URL("../src/mod_cache.rs", import.meta.url), "utf8");
    const main = readFileSync(new URL("../src/main.rs", import.meta.url), "utf8");
    const customAdapter = readFileSync(new URL("../web/mods/custom/adapter.js", import.meta.url), "utf8");
    // The inventory and the execution path reason about the same fixed names.
    assert.match(nativeCustom, /CUSTOM_JS_FILE: &str = "custom\.js"/);
    assert.match(nativeCustom, /CUSTOM_CSS_FILE: &str = "custom\.css"/);
    // Retrieval is local-file only: links never followed, size caps enforced,
    // no network, no renderer-supplied path.
    assert.match(nativeCustom, /symlink_metadata/);
    assert.match(nativeCustom, /MAX_CUSTOM_JS_BYTES/);
    assert.match(nativeCustom, /MAX_CUSTOM_CSS_BYTES/);
    assert.doesNotMatch(nativeCustom, /ureq|https?:\/\/|getModBundle\("custom"\)/);
    // A descriptor registers the custom id on the shared capability, with a
    // user-owned identity (no upstream banner or sentinel to prove).
    assert.match(main, /id: "custom"/);
    assert.match(main, /CUSTOM_MOD/);
    assert.match(main, /CUSTOM_ADAPTER_SCRIPT/);
    assert.match(main, /ModArtifactIdentity::UserOwned/);
    assert.match(main, /load_custom_bundle/);
    // The adapter carries identity only: the logical id, never a path/URL.
    assert.match(customAdapter, /getModBundle\("custom"\)/);
    assert.doesNotMatch(customAdapter, /custom\.js|custom\.css/);
    assert.doesNotMatch(
        customAdapter,
        /example\.invalid|https?:\/\/(?!discord\.com|canary\.discord\.com|ptb\.discord\.com)/,
    );
    // User-owned files are still user data: the cache-clear allowlist must
    // never name them (docs/tests may mention them as must-survive), and
    // restores still land them as DATA ONLY (execution happens from the
    // profile files at the next launch, never from the archive).
    const cacheBlock = nativeModCache.match(/pub const MOD_CACHE_FILES[^=]*=\s*&\[([\s\S]*?)\];/);
    assert.ok(cacheBlock);
    assert.doesNotMatch(cacheBlock[1], /custom/);
    assert.match(nativeBackup, /custom\.js.*DATA ONLY|DATA ONLY/);
    // Electron parity reference is untouched: the local userData files stay
    // the source of truth there too.
    assert.match(ipc, /ifExistsRead\(path\.join\(app\.getPath\("userData"\), "custom\.js"\)\)/);
    assert.match(nativeBundles, /"custom\.js"/);
    assert.match(nativeBundles, /"custom\.css"/);
});

test("the native backup facade exposes only discovery plus save plus the inspect/commit/discard trio", () => {
    // No Electron channel name, dialog send, zip/generic member, or bundle
    // content may exist alongside them.
    assert.match(nativeFacade, /function listBundles\(/);
    assert.match(nativeFacade, /function saveBackup\(/);
    assert.match(nativeFacade, /function inspectBackup\(/);
    assert.match(nativeFacade, /function commitRestore\(/);
    assert.match(nativeFacade, /function discardRestore\(/);
    assert.match(nativeFacade, /backup_saved/);
    assert.match(nativeFacade, /backup_inspected/);
    assert.match(nativeFacade, /backup_restored/);
    assert.match(nativeFacade, /backup_restore_discarded/);
    assert.doesNotMatch(nativeFacade, /function restoreBackup\(/);
    assert.doesNotMatch(nativeFacade, /backupSave/);
    assert.doesNotMatch(nativeFacade, /backupRestore/);
    assert.doesNotMatch(nativeFacade, /showSaveDialog/);
    assert.doesNotMatch(nativeFacade, /showOpenDialog/);
    assert.doesNotMatch(nativeFacade, /AdmZip|readBackupZipToMap|buildBackupZipBuffer/);
});

test("the native backup save preserves the Electron archive and dialog contract", () => {
    // Save keeps the Electron layout/manifest semantics: manifest first,
    // the exact data/ + bundles/ prefixes, version 1, UTC ISO-8601 dates,
    // the translated save title, the ZIP filter, the dated default filename,
    // and .zip enforcement — while the renderer never supplies a path.
    const nativeBackup = readFileSync(new URL("../src/backup.rs", import.meta.url), "utf8");
    assert.match(nativeBackup, /manifest\.json/);
    assert.match(nativeBackup, /data\/storage\/settings\.json/);
    assert.match(nativeBackup, /data\/quickCss\.css/);
    assert.match(nativeBackup, /data\/themes/);
    assert.match(nativeBackup, /data\/plugins/);
    assert.match(nativeBackup, /data\/runtime-plugins/);
    assert.match(nativeBackup, /data\/plugin-storage/);
    assert.match(nativeBackup, /bundles\//);
    assert.match(nativeBackup, /BACKUP_VERSION: u8 = 1/);
    assert.match(nativeBackup, /backup-dialogSave-title/);
    assert.match(nativeBackup, /remcord-backup-/);
    assert.match(nativeBackup, /BUNDLE_FILES/);
    // Bounds and link policy are explicit and stricter than Electron.
    assert.match(nativeBackup, /MAX_BACKUP_FILE_BYTES/);
    assert.match(nativeBackup, /MAX_BACKUP_BUNDLE_BYTES/);
    assert.match(nativeBackup, /MAX_BACKUP_TOTAL_BYTES/);
    assert.match(nativeBackup, /MAX_BACKUP_ENTRIES/);
    assert.match(nativeBackup, /BACKUP_TOO_LARGE/);
    assert.match(nativeBackup, /symlink_metadata/);
    // The save dialog owns its configuration: the host never sets a folder
    // (no renderer defaultPath exists anywhere in the capability).
    assert.doesNotMatch(nativeBackup, /SetFolder/);
});

test("the native backup restore preserves the Electron restore contract", () => {
    // Restore keeps the Electron section layout and dialog semantics while
    // hardening every unsafe behavior: the exact data/ + bundles/ prefixes,
    // settings-only storage routing, the fixed Quick CSS name, the bundle
    // allowlist, the translated OPEN title, and no renderer-supplied path.
    const nativeBackup = readFileSync(new URL("../src/backup.rs", import.meta.url), "utf8");
    const nativeProtocol = readFileSync(new URL("../src/protocol.rs", import.meta.url), "utf8");
    const nativeFacade = readFileSync(new URL("../web/compatibility.js", import.meta.url), "utf8");
    const nativeMain = readFileSync(new URL("../src/main.rs", import.meta.url), "utf8");
    // Section routing parity.
    assert.match(nativeBackup, /data\/storage\/settings\.json/);
    assert.match(nativeBackup, /data\/quickCss\.css/);
    assert.match(nativeBackup, /"themes"/);
    assert.match(nativeBackup, /"plugins"/);
    assert.match(nativeBackup, /"runtime-plugins"/);
    assert.match(nativeBackup, /"plugin-storage"/);
    assert.match(nativeBackup, /BUNDLE_FILES/);
    // The dialog is host-owned with the translated open title; every trio
    // request is exactly empty.
    assert.match(nativeBackup, /backup-dialogOpen-title/);
    assert.match(nativeBackup, /Open Remcord backup/);
    assert.match(nativeProtocol, /BackupInspect/);
    assert.match(nativeProtocol, /BackupCommitRestore/);
    assert.match(nativeProtocol, /BackupDiscardRestore/);
    assert.match(nativeProtocol, /backup_inspect/);
    assert.match(nativeProtocol, /backup_commit_restore/);
    assert.match(nativeProtocol, /backup_discard_restore/);
    assert.match(nativeProtocol, /BackupInspected/);
    assert.match(nativeProtocol, /BackupRestored/);
    assert.match(nativeProtocol, /BackupRestoreDiscarded/);
    assert.match(nativeFacade, /function inspectBackup\(/);
    assert.match(nativeFacade, /function commitRestore\(/);
    assert.match(nativeFacade, /function discardRestore\(/);
    // Hardening markers: closed routing (no generic extract), whole-archive
    // duplicate/collision rejection, staging outside the profile, commit
    // with rollback, existing section validators, and bounded clientMods.
    assert.match(nativeBackup, /validate_and_stage/);
    assert.match(nativeBackup, /reject_duplicate_names/);
    assert.match(nativeBackup, /create_staging_dir/);
    assert.match(nativeBackup, /commit_staged/);
    assert.match(nativeBackup, /validate_theme_id/);
    assert.match(nativeBackup, /validate_plugin_id/);
    assert.match(nativeBackup, /validate_relative_path/);
    assert.match(nativeBackup, /INVALID_ARCHIVE/);
    assert.match(nativeBackup, /merge_settings_blob/);
    assert.match(nativeBackup, /custom\.js.*DATA ONLY|DATA ONLY/);
    // The restore worker is dedicated and STA-owned, like the save worker.
    assert.match(nativeMain, /remcord-backup-restore-sta/);
    assert.match(nativeMain, /BackupRestoreReady/);
    // Electron behavior stays untouched: the Map-buffered apply path, the
    // shallow-merge settings restore, and the renderer clientMods applier.
    const backupMain = readFileSync(new URL("../../src/common/backup.ts", import.meta.url), "utf8");
    assert.match(backupMain, /readBackupZipToMap/);
    assert.match(backupMain, /applyBackupFromMap/);
    assert.match(backupMain, /setConfigBulk/);
    const backupSection = readFileSync(
        new URL("../../src/shelter/settings/components/BackupSection.tsx", import.meta.url),
        "utf8",
    );
    assert.match(backupSection, /applyClientModsFromRestore/);
    assert.match(backupSection, /addRemotePlugin/);
    // Native trio wiring lives in src/ deliberately: inspect stages and
    // holds host-side, the preview modal confirms, commit applies the held
    // bytes, and any other dismissal discards the hold. The Electron
    // `window.remcord.backup.restore()` call stays byte-identical, so the
    // check targets the native trio and native branches only.
    assert.match(backupSection, /inspectBackupNative\(\)/);
    assert.match(backupSection, /commitRestoreNative\(\)/);
    assert.match(backupSection, /discardRestoreNative\(\)/);
    assert.match(backupSection, /BackupRestorePreviewModal/);
    assert.match(backupSection, /normalizeNativeInspectResult\(inspected\)/);
    for (const source of [bridge, ipc, types]) {
        assert.doesNotMatch(source, /backup_inspect/);
        assert.doesNotMatch(source, /backup_commit_restore/);
        assert.doesNotMatch(source, /backup_discard_restore/);
    }
    assert.doesNotMatch(backupSection, /backup_inspect/);
    assert.doesNotMatch(backupSection, /backup_commit_restore/);
    assert.doesNotMatch(backupSection, /backup_discard_restore/);
    assert.doesNotMatch(backupSection, /isNativeWebView/);
    assert.match(backupSection, /window\.remcord\.backup\.restore\(\)/);
});

test("BackupSection dispatches native vs Electron backup without a generic surface", () => {
    // Trio wiring: the existing section calls the shared save helper (one
    // call site, exact payload preserved) and previews-then-commits restore
    // on the native capability gate — inspect stages and holds, the preview
    // modal confirms, commit applies the held bytes, discard drops the hold.
    // Electron keeps the JSON-string restore. No generic filesystem, dialog,
    // IPC, or ZIP surface may appear in either file, and the declared
    // Electron contract stays untouched.
    const backupSection = readFileSync(
        new URL("../../src/shelter/settings/components/BackupSection.tsx", import.meta.url),
        "utf8",
    );
    const settingsHelpers = readFileSync(new URL("../../src/shelter/settings/settings.ts", import.meta.url), "utf8");
    // Save: one helper call site with the exact payload expression.
    assert.match(backupSection, /saveBackup\(JSON\.stringify\(payload\)\)/);
    // Restore: capability-gated branch, preview modal, exactly one commit
    // path plus the best-effort discard on every non-commit dismissal.
    assert.match(backupSection, /isNativeBackupAvailable\(\)/);
    assert.match(backupSection, /inspectBackupNative\(\)/);
    assert.match(backupSection, /normalizeNativeInspectResult\(inspected\)/);
    assert.match(backupSection, /BackupRestorePreviewModal/);
    assert.match(backupSection, /commitRestoreNative\(\)/);
    assert.match(backupSection, /discardRestoreNative\(\)/);
    assert.match(backupSection, /normalizeNativeRestoreResult\(await commitRestoreNative\(\)\)/);
    assert.match(backupSection, /parseElectronRestoreResult\(await window\.remcord\.backup\.restore\(\)\)/);
    // The existing renderer clientMods path is preserved verbatim.
    assert.match(backupSection, /applyClientModsFromRestore\(outcome\.clientMods\)/);
    // Async UI state: busy guard on both buttons and every exit path.
    assert.match(backupSection, /disabled=\{busy\(\)\}/);
    assert.match(backupSection, /if \(busy\(\)\) return;/);
    // Helpers: gate on environment + trio presence, shared outcomes, no
    // generic surface. remcordWindow.d.ts keeps the Electron string shape.
    assert.match(settingsHelpers, /export function isNativeBackupAvailable/);
    assert.match(settingsHelpers, /export async function saveBackup/);
    assert.match(settingsHelpers, /export async function inspectBackupNative/);
    assert.match(settingsHelpers, /export async function commitRestoreNative/);
    assert.match(settingsHelpers, /export async function discardRestoreNative/);
    assert.match(settingsHelpers, /export function normalizeNativeInspectResult/);
    assert.match(settingsHelpers, /export function normalizeNativeRestoreResult/);
    assert.match(settingsHelpers, /export function parseElectronRestoreResult/);
    assert.match(types, /restore\(\): Promise<string>/);
    for (const source of [backupSection, settingsHelpers]) {
        assert.doesNotMatch(source, /backup_inspect/);
        assert.doesNotMatch(source, /backup_commit_restore/);
        assert.doesNotMatch(source, /backup_discard_restore/);
        assert.doesNotMatch(source, /showOpenDialog|showSaveDialog/);
        assert.doesNotMatch(source, /AdmZip|readBackupZipToMap|buildBackupZipBuffer/);
        assert.doesNotMatch(source, /fetch\(/);
    }
    // The new helpers add no storage/plugin execution of their own: applying
    // clientMods stays exclusively inside the pre-existing
    // applyClientModsFromRestore renderer path (asserted above).
    assert.doesNotMatch(settingsHelpers, /localStorage\.setItem/);
    assert.doesNotMatch(settingsHelpers, /addRemotePlugin/);
});

test("Electron keeps every folder and file-open IPC handler", () => {
    // Fixed roots use showItemInFolder (reveal); the per-theme directory
    // uses openPath (open). File-in-editor and picker paths are separate
    // milestones and must not be mistaken for folder opening.
    assert.match(ipc, /ipcMain\.on\("openStorageFolder"/);
    assert.match(ipc, /shell\.showItemInFolder\(storagePath\)/);
    assert.match(ipc, /ipcMain\.on\("openThemesFolder"/);
    assert.match(ipc, /shell\.showItemInFolder\(themesPath\)/);
    assert.match(ipc, /ipcMain\.on\("openPluginsFolder"/);
    assert.match(ipc, /shell\.showItemInFolder\(extensionsPath\)/);
    assert.match(ipc, /ipcMain\.on\("openRuntimePluginsFolder"/);
    assert.match(ipc, /shell\.showItemInFolder\(pluginsPath\)/);
    assert.match(ipc, /ipcMain\.on\("openCrashesFolder"/);
    assert.match(ipc, /ipcMain\.on\("openThemeFolder", \(_event, id: string\)/);
    assert.match(ipc, /shell\.openPath\(path\.join\(themesPath, id\)\)/);
    // The renderer bridge keeps the same fire-and-forget sends.
    assert.match(bridge, /openStorageFolder: \(\) => ipcRenderer\.send\("openStorageFolder"\)/);
    assert.match(bridge, /openThemesFolder: \(\) => ipcRenderer\.send\("openThemesFolder"\)/);
    assert.match(bridge, /folder: \(id: string\) => ipcRenderer\.send\("openThemeFolder", id\)/);
    assert.match(bridge, /openFolder: \(\) => ipcRenderer\.send\("openRuntimePluginsFolder"\)/);
    // Declared contracts stay void-returning.
    assert.match(types, /openStorageFolder: \(\) => void/);
    assert.match(types, /openThemesFolder: \(\) => void/);
    assert.match(types, /folder: \(id: string\) => void/);
    assert.match(types, /openFolder: \(\) => void/);
});

test("Electron joins the raw theme id with no validation", () => {
    // Documents WHY the native side validates: path.join honors absolute
    // segments and ../, so a hostile id escapes the themes root here.
    assert.match(ipc, /shell\.openPath\(path\.join\(themesPath, id\)\)/);
    assert.doesNotMatch(ipc, /validate.*id.*openThemeFolder|openThemeFolder.*validate/);
});

test("only the four wired UI actions use the native folder opening path", () => {
    // The bridge, main handlers, and declared contracts stay Electron-only:
    // native dispatch lives in the four settings-UI call sites, each gated
    // behind the shared isNativeWebView check with the Electron send intact.
    for (const source of [bridge, ipc, types]) {
        assert.doesNotMatch(source, /folders\.open/);
        assert.doesNotMatch(source, /folders_open/);
        assert.doesNotMatch(source, /remcord\.folders/);
    }
    const settingsPage = readFileSync(
        new URL("../../src/shelter/settings/pages/SettingsPage.tsx", import.meta.url),
        "utf8",
    );
    const themesPage = readFileSync(
        new URL("../../src/shelter/settings/pages/ThemesPage.tsx", import.meta.url),
        "utf8",
    );
    const pluginsPage = readFileSync(
        new URL("../../src/shelter/settings/pages/PluginsPage.tsx", import.meta.url),
        "utf8",
    );
    const themesCard = readFileSync(
        new URL("../../src/shelter/settings/components/ThemesCard.tsx", import.meta.url),
        "utf8",
    );
    for (const source of [settingsPage, themesPage, pluginsPage, themesCard]) {
        assert.match(source, /Native WebView2/);
        assert.match(source, /Electron keeps the original fire-and-forget send/);
    }
    assert.match(themesCard, /window\.remcord\.themes\.folder\(props\.theme\.id\)/);
});

test("native folder opening preserves the reveal versus open distinction", () => {
    // Both operations may show Explorer, but they are different shell
    // operations: reveal selects the fixed root in its parent, open enters
    // the theme directory itself. The module documents and implements both
    // with direct shell APIs, never a constructed command line.
    const nativeFolders = readFileSync(new URL("../src/folders.rs", import.meta.url), "utf8");
    assert.match(nativeFolders, /SHOpenFolderAndSelectItems/);
    assert.match(nativeFolders, /ShellExecuteW/);
    assert.match(nativeFolders, /showItemInFolder/);
    assert.match(nativeFolders, /openPath/);
    assert.doesNotMatch(nativeFolders, /Command::new|cmd \/C|create_process|system\(/);
});

test("native folder facade exposes identifiers only, no shell surface", () => {
    assert.match(nativeFacade, /function openFolder\(/);
    assert.match(nativeFacade, /function openThemeFolder\(/);
    assert.match(nativeFacade, /requireCapability\("folders\.open"\)/);
    assert.match(nativeFacade, /requireCapability\("folders\.open_theme"\)/);
    assert.doesNotMatch(nativeFacade, /openPath\(/);
    assert.doesNotMatch(nativeFacade, /showItemInFolder/);
    assert.doesNotMatch(nativeFacade, /ShellExecute/);
    assert.doesNotMatch(nativeFacade, /openExternal/);
    assert.doesNotMatch(nativeFacade, /showOpenDialog|showSaveDialog/);
});

test("Electron keeps the theme install/edit/picker implementation", () => {
    // installTheme fetches remote URLs and reads local files, then
    // installThemeFromCode derives `<name>-BD` (first space only) and
    // overwrites manifest.json + src.css. parseBDManifest requires `/**`,
    // defaults name "null" with enabled false, and stops at `import`.
    const themesMain = readFileSync(new URL("../../src/common/themes.ts", import.meta.url), "utf8");
    assert.match(themesMain, /export async function installTheme\(linkOrPath: string\)/);
    assert.match(themesMain, /code = await \(await fetch\(linkOrPath\)\)\.text\(\);/);
    assert.match(themesMain, /code = fs\.readFileSync\(linkOrPath, "utf8"\);/);
    assert.match(themesMain, /function installThemeFromCode\(code: string, linkOrPath\?: string\)/);
    assert.match(themesMain, /manifest\.name\?\.replace\(" ", "-"\)\}-BD/);
    assert.match(
        themesMain,
        /fs\.writeFileSync\(path\.join\(themePath, "manifest\.json"\), JSON\.stringify\(manifest\)\)/,
    );
    assert.match(themesMain, /fs\.writeFileSync\(path\.join\(themePath, "src\.css"\), code\)/);
    assert.match(themesMain, /if \(!content\.startsWith\("\/\*\*"\)\) \{\s*throw new Error\("Not a manifest\."\);/);
    assert.match(themesMain, /theme: "src\.css",\s*name: "null",\s*enabled: false,/);
    assert.match(themesMain, /if \(key === "import"\) break;/);
    assert.match(themesMain, /manifest\.updateSrc = linkOrPath;/);
    assert.match(themesMain, /manifest\.supportsRemcordTitlebar = (true|false);/);
    // editTheme reads the manifest with string interpolation and opens the
    // manifest-named file in the system editor or the internal CSS editor.
    assert.match(ipc, /ipcMain\.on\("editTheme", \(_event, id: string\)/);
    assert.match(ipc, /readFileSync\(`\$\{themesPath\}\/\$\{id\}\/manifest\.json`/);
    assert.match(ipc, /shell\.openPath\(`\$\{themesPath\}\/\$\{id\}\/\$\{manifest\.theme\}`\)/);
    // Import picking is a filtered host file dialog feeding installTheme.
    assert.match(ipc, /ipcMain\.on\("openImportPicker"/);
    assert.match(ipc, /properties: \["openFile", "multiSelections"\]/);
    // installBDTheme bridges a renderer URL string to installTheme.
    assert.match(ipc, /ipcMain\.handle\("installBDTheme", async \(_event, link: string\)/);
    // The renderer bridge keeps URL install, id edit, and picker sends.
    assert.match(bridge, /install: async \(url: string\) => ipcRenderer\.invoke\("installBDTheme", url\)/);
    assert.match(bridge, /edit: \(id: string\) => ipcRenderer\.send\("editTheme", id\)/);
    assert.match(bridge, /openImportPicker: \(\) => ipcRenderer\.send\("openImportPicker"\)/);
});

test("no src caller uses the native theme install path", () => {
    // Step 3a ships the capability without UI wiring: a native branch in
    // src/ would be fabrication. The Electron theme UI keeps calling
    // install/edit/picker, and nothing references the native surface.
    const themesPage = readFileSync(
        new URL("../../src/shelter/settings/pages/ThemesPage.tsx", import.meta.url),
        "utf8",
    );
    const themesCard = readFileSync(
        new URL("../../src/shelter/settings/components/ThemesCard.tsx", import.meta.url),
        "utf8",
    );
    for (const source of [bridge, ipc, types, themesPage, themesCard]) {
        assert.doesNotMatch(source, /installFromCode/);
        assert.doesNotMatch(source, /themes\.install_code/);
        assert.doesNotMatch(source, /theme_install_code/);
    }
    assert.match(themesPage, /window\.remcord\.themes\.install\(downloadUrl\(\)\)/);
    assert.match(themesCard, /window\.remcord\.themes\.install\(props\.theme\.updateSrc\)/);
    assert.match(themesCard, /window\.remcord\.themes\.edit\(props\.theme\.id\)/);
});

test("native theme install is CSS-text-only with no network or dialog surface", () => {
    // The native module derives everything from CSS text: no fetch, no
    // dialog, no picker, no editor launch, no shell execution, no URL
    // source anywhere near the install path.
    const nativeThemes = readFileSync(new URL("../src/themes.rs", import.meta.url), "utf8");
    assert.match(nativeThemes, /pub fn install_theme_from_code/);
    assert.match(nativeThemes, /pub fn parse_bd_manifest/);
    // Test fixtures contain an inert example URL for the import-break case;
    // the shipped install path itself must have no network, dialog, picker,
    // editor-launch, or shell surface.
    const shippedThemes = nativeThemes.slice(0, nativeThemes.indexOf("#[cfg(test)]"));
    assert.doesNotMatch(shippedThemes, /fetch\(|ureq|Dialog|ShellExecute|openPath|Command::new/);
    assert.doesNotMatch(shippedThemes, /https?:\/\//);
    // The facade takes CSS text and returns metadata: no URL/path/id
    // member, no install-from-URL shape.
    const installFacade = nativeFacade.slice(
        nativeFacade.indexOf("function installThemeFromCode"),
        nativeFacade.indexOf('window.addEventListener("remcord-native:response"'),
    );
    assert.ok(installFacade.length > 0);
    assert.match(installFacade, /requireCapability\("themes\.install_code"\)/);
    assert.match(installFacade, /\{\s*type:\s*"theme_install_code",\s*payload:\s*\{\s*css\s*\}\s*\}/);
    assert.doesNotMatch(installFacade, /fetch\(|installBDTheme|showOpenDialog|openPath/);
});

test("Electron keeps the theme edit and CSS editor implementation", () => {
    // editTheme reads the manifest with raw interpolation and opens the
    // manifest-named file in the system editor or the internal Monaco
    // editor window, which reads/writes the file directly.
    assert.match(ipc, /ipcMain\.on\("editTheme", \(_event, id: string\)/);
    assert.match(ipc, /readFileSync\(`\$\{themesPath\}\/\$\{id\}\/manifest\.json`/);
    assert.match(ipc, /shell\.openPath\(`\$\{themesPath\}\/\$\{id\}\/\$\{manifest\.theme\}`\)/);
    const cssEditor = readFileSync(new URL("../../src/cssEditor/main.ts", import.meta.url), "utf8");
    assert.match(cssEditor, /export function openCssEditor\(file: string\)/);
    assert.match(cssEditor, /fs\.writeFileSync\(file, css\)/);
    assert.match(cssEditor, /fs\.readFileSync\(file, "utf-8"\)/);
    // The renderer bridge keeps the id-only edit send.
    assert.match(bridge, /edit: \(id: string\) => ipcRenderer\.send\("editTheme", id\)/);
});

test("no src caller uses the native theme write path", () => {
    // Step 3b ships the capability without UI wiring: a native branch in
    // src/ would be fabrication. The Electron theme UI keeps calling edit,
    // and nothing references the native write surface.
    const themesCard = readFileSync(
        new URL("../../src/shelter/settings/components/ThemesCard.tsx", import.meta.url),
        "utf8",
    );
    for (const source of [bridge, ipc, types, themesCard]) {
        assert.doesNotMatch(source, /writeCss/);
        assert.doesNotMatch(source, /themes\.write_css/);
        assert.doesNotMatch(source, /theme_write_css/);
    }
    assert.match(themesCard, /window\.remcord\.themes\.edit\(props\.theme\.id\)/);
});

test("native theme write resolves the manifest file with no editor surface", () => {
    // The native module follows manifest.theme through the filename
    // allowlist (Electron interpolates it raw): no editor window, no
    // Monaco round-trip, no system-editor launch, no shell execution.
    const nativeThemes = readFileSync(new URL("../src/themes.rs", import.meta.url), "utf8");
    assert.match(nativeThemes, /pub fn write_theme_css/);
    assert.match(nativeThemes, /validate_theme_css_filename\(&manifest\.theme\)/);
    const shippedThemes = nativeThemes.slice(0, nativeThemes.indexOf("#[cfg(test)]"));
    assert.doesNotMatch(shippedThemes, /openCssEditor|monaco|Monaco|ShellExecute|openPath|Command::new/);
    // The facade takes a validated id plus CSS text and returns metadata:
    // no path, filename, URL, destination, or enablement member.
    const writeFacade = nativeFacade.slice(
        nativeFacade.indexOf("function writeThemeCss"),
        nativeFacade.indexOf('window.addEventListener("remcord-native:response"'),
    );
    assert.ok(writeFacade.length > 0);
    assert.match(writeFacade, /requireCapability\("themes\.write_css"\)/);
    assert.match(writeFacade, /\{\s*type:\s*"theme_write_css",\s*payload:\s*\{\s*id,\s*css\s*\}\s*\}/);
    assert.doesNotMatch(writeFacade, /fetch\(|installBDTheme|showOpenDialog|openPath/);
});

test("Electron keeps the import picker dialog configuration", () => {
    // The picker is a host file dialog with no defaultPath, translated
    // title/button/filter names, open-file multi-selection, the SCSS/CSS
    // filter first and the all-files filter second. Cancellation returns
    // silently; each selected file installs sequentially with per-file
    // continue-after-failure, and one `themesChanged` summary event carries
    // the exact counts so the renderer refreshes from host truth.
    // `installTheme` reads local files unbounded as UTF-8.
    assert.match(ipc, /ipcMain\.on\("openImportPicker"/);
    assert.match(ipc, /title: getLang\("dialog-importTheme-title"\)/);
    assert.match(ipc, /buttonLabel: getLang\("dialog-importTheme-button"\)/);
    assert.match(ipc, /properties: \["openFile", "multiSelections"\]/);
    assert.match(ipc, /name: getLang\("dialog-importTheme-discordStyles"\)/);
    assert.match(ipc, /extensions: \["scss", "css"\]/);
    assert.match(ipc, /name: getLang\("dialog-importTheme-allFiles"\)/);
    assert.match(ipc, /if \(result\.canceled\) return;/);
    assert.match(ipc, /for \(const file of result\.filePaths\) \{[\s\S]{0,400}?await installTheme\(file\);/);
    assert.match(ipc, /webContents\.send\("themesChanged", \{ imported, failed \}\)/);
    assert.match(bridge, /openImportPicker: \(\) => ipcRenderer\.send\("openImportPicker"\)/);
    const lang = readFileSync(new URL("../../assets/lang/en-US.json", import.meta.url), "utf8");
    assert.match(lang, /"dialog-importTheme-title": "Select a theme you want to import"/);
    assert.match(lang, /"dialog-importTheme-button": "Import"/);
    assert.match(lang, /"dialog-importTheme-discordStyles": "Discord styles"/);
    assert.match(lang, /"dialog-importTheme-allFiles": "All Files"/);
    const themesPage = readFileSync(
        new URL("../../src/shelter/settings/pages/ThemesPage.tsx", import.meta.url),
        "utf8",
    );
    assert.match(themesPage, /window\.remcord\.themes\.openImportPicker/);
});

test("no src caller uses the native theme import path", () => {
    // Step 3d-2 wires the ThemesPage import button: native importFiles()
    // first, Electron openImportPicker only when the capability is absent.
    // Exactly one path executes per click; remote URL install stays
    // Electron-only (hidden in native mode).
    const themesPage = readFileSync(
        new URL("../../src/shelter/settings/pages/ThemesPage.tsx", import.meta.url),
        "utf8",
    );
    const themesCard = readFileSync(
        new URL("../../src/shelter/settings/components/ThemesCard.tsx", import.meta.url),
        "utf8",
    );
    for (const source of [bridge, ipc, types, themesCard]) {
        assert.doesNotMatch(source, /importFiles/);
        assert.doesNotMatch(source, /themes\.import_files/);
        assert.doesNotMatch(source, /theme_import_files/);
    }
    assert.match(themesPage, /importNativeThemes\(\)/);
    assert.match(themesPage, /isNativeThemeImportAvailable\(\)/);
    assert.match(themesPage, /window\.remcord\.themes\.openImportPicker/);
    assert.doesNotMatch(themesPage, /themes\.import_files/);
    assert.doesNotMatch(themesPage, /theme_import_files/);
});

test("native theme import shares the installer with a Common Item Dialog and no generic surface", () => {
    // Exactly one installation implementation lives underneath both
    // install_code and import_files: the import path parses with the native
    // BD parser and installs through the shared helper.
    const nativeThemes = readFileSync(new URL("../src/themes.rs", import.meta.url), "utf8");
    assert.match(nativeThemes, /pub fn install_parsed_theme/);
    const nativeImport = readFileSync(new URL("../src/theme_import.rs", import.meta.url), "utf8");
    assert.match(nativeImport, /install_parsed_theme\(profile_dir, &css, parsed\)/);
    assert.match(nativeImport, /parse_bd_manifest\(&css\)/);
    // Modern dialog path with multi-select, translated title/button, and
    // file-type filters — never the legacy GetOpenFileNameW API and never
    // a third-party dialog dependency.
    assert.match(nativeImport, /IFileOpenDialog/);
    assert.match(nativeImport, /SetOkButtonLabel/);
    assert.match(nativeImport, /SetTitle/);
    assert.match(nativeImport, /SetFileTypes/);
    assert.match(nativeImport, /FOS_ALLOWMULTISELECT/);
    assert.match(nativeImport, /GetResults/);
    // The module docs name the legacy API only to reject it: assert the
    // call site itself never appears (with paren), not the bare word.
    assert.doesNotMatch(nativeImport, /GetOpenFileNameW?\(/);
    assert.doesNotMatch(nativeImport, /\brfd\b/);
    // Theme-specific only: no network fetch, no remote URL, no generic
    // dialog/filesystem/read/write primitive, no shell execution.
    const shippedImport = nativeImport.slice(0, nativeImport.indexOf("#[cfg(test)]"));
    assert.doesNotMatch(shippedImport, /fetch\(|ureq|https?:\/\//);
    assert.doesNotMatch(shippedImport, /dialog\.openFile|filesystem\.(read|write)/);
    assert.doesNotMatch(shippedImport, /ShellExecute|openPath|Command::new|showOpenDialog/);
    // The facade sends an exact empty payload with the long-operation
    // timeout and validates the bounded summary: no path/URL/filter member.
    const importFacade = nativeFacade.slice(
        nativeFacade.indexOf("function importFiles"),
        nativeFacade.indexOf("function listBundles"),
    );
    assert.ok(importFacade.length > 0);
    assert.match(importFacade, /requireCapability\("themes\.import_files"\)/);
    assert.match(importFacade, /function importFiles\(timeoutMs = 120_000\)/);
    assert.match(importFacade, /\{\s*type:\s*"theme_import_files",\s*payload:\s*\{\s*\}\s*\}/);
    assert.doesNotMatch(importFacade, /fetch\(|showOpenDialog|openPath|path:|url:/);
});

test("Electron keeps the sync theme-list and fire-and-forget enable contracts", () => {
    // Toggle path: ThemesCard calls themes.set(id, state); the bridge sends
    // setThemeEnabled fire-and-forget; the host reads the manifest, pushes
    // addTheme/removeTheme per window only when the state differs, persists
    // ONLY manifest.enabled, and never touches another theme. Refresh path:
    // getThemes/refresh are synchronous manifest-list reads.
    const themesMain = readFileSync(new URL("../../src/common/themes.ts", import.meta.url), "utf8");
    const themesCard = readFileSync(
        new URL("../../src/shelter/settings/components/ThemesCard.tsx", import.meta.url),
        "utf8",
    );
    assert.match(
        bridge,
        /set:\s*\(id:\s*string, state:\s*boolean\)\s*=>\s*ipcRenderer\.send\("setThemeEnabled", id, state\)/,
    );
    assert.match(ipc, /ipcMain\.on\("setThemeEnabled", \(_event, name: string, enabled: boolean\)/);
    assert.match(ipc, /setThemeEnabled\(name, enabled\)/);
    assert.match(themesMain, /manifest\.enabled = enabled;/);
    assert.match(themesMain, /passedWindow\.webContents\.send\("addTheme", id, css\)/);
    assert.match(themesMain, /passedWindow\.webContents\.send\("removeTheme", id\)/);
    assert.match(themesCard, /window\.remcord\.themes\.set\(props\.theme\.id, state\)/);
});

test("no src caller uses the native theme enable path", () => {
    // Step 3d-2 wires the ThemesCard toggle: native setEnabled first,
    // Electron set only when the capability is absent. Exactly one path
    // executes per toggle; switches stay independent (no mutual exclusion).
    const themesPage = readFileSync(
        new URL("../../src/shelter/settings/pages/ThemesPage.tsx", import.meta.url),
        "utf8",
    );
    const themesCard = readFileSync(
        new URL("../../src/shelter/settings/components/ThemesCard.tsx", import.meta.url),
        "utf8",
    );
    for (const source of [bridge, ipc, types]) {
        // Scoped to the theme-enable shapes: the unrelated plugins
        // setEnabled API legitimately contains the bare word.
        assert.doesNotMatch(source, /themes\.setEnabled/);
        assert.doesNotMatch(source, /themes\.set_enabled/);
        assert.doesNotMatch(source, /theme_set_enabled/);
    }
    assert.match(themesCard, /setNativeThemeEnabled\(props\.theme\.id, state\)/);
    assert.match(themesCard, /isNativeThemeToggleAvailable\(\)/);
    assert.match(themesCard, /window\.remcord\.themes\.set\(props\.theme\.id, state\)/);
    assert.doesNotMatch(themesCard, /themes\.set_enabled/);
    assert.doesNotMatch(themesCard, /theme_set_enabled/);
    assert.doesNotMatch(themesPage, /themes\.setEnabled/);
});

test("native theme enable flips only the manifest flag with no CSS machinery", () => {
    // The host derives the manifest path from the validated id and edits
    // the parsed JSON document in place: no path/filename input, no style
    // element construction, no script evaluation near the flip.
    const nativeThemes = readFileSync(new URL("../src/themes.rs", import.meta.url), "utf8");
    assert.match(nativeThemes, /pub fn set_theme_enabled/);
    assert.match(nativeThemes, /resolve_theme_dir\(profile_dir, theme_id\)/);
    assert.match(nativeThemes, /read_manifest_text\(&manifest_path\)/);
    const shippedThemes = nativeThemes.slice(0, nativeThemes.indexOf("#[cfg(test)]"));
    const flipBody = shippedThemes.slice(shippedThemes.indexOf("pub fn set_theme_enabled"));
    assert.ok(flipBody.length > 0);
    assert.doesNotMatch(flipBody, /evaluate_script|managed_css_script|apply_managed/);
    assert.doesNotMatch(flipBody, /fetch\(|ureq|openPath|ShellExecute|Command::new/);
    // The facade sends id plus boolean and validates the echo: no path,
    // filename, manifest, CSS, URL, or destination member.
    const enableFacade = nativeFacade.slice(
        nativeFacade.indexOf("function setThemeEnabled"),
        nativeFacade.indexOf("function importFiles"),
    );
    assert.ok(enableFacade.length > 0);
    assert.match(enableFacade, /requireCapability\("themes\.set_enabled"\)/);
    assert.match(enableFacade, /\{\s*type:\s*"theme_set_enabled",\s*payload:\s*\{\s*id,\s*enabled\s*\}\s*\}/);
    // No path/URL/filename member may be constructed or read anywhere near
    // the flip (the comment above documents the rejection in prose, so the
    // assertion targets member shapes, not bare words).
    assert.doesNotMatch(enableFacade, /fetch\(|showOpenDialog|openPath|"path"|"filename"|"url"|path:|url:/);
});

test("Electron keeps the metadata-only theme inventory contract", () => {
    // getThemes returns one manifest-metadata entry per directory WITH a
    // readable manifest (id = directory name spread onto the manifest):
    // no CSS is read, disabled themes are included, missing/malformed
    // manifests are silently skipped, and ordering is filesystem discovery
    // order. refresh only invalidates the list cache and re-reads.
    const themesMain = readFileSync(new URL("../../src/common/themes.ts", import.meta.url), "utf8");
    assert.match(themesMain, /themes\.push\(\{\s*\.\.\.manifest,\s*id:\s*entry\s*\}\);/);
    assert.match(themesMain, /if \(manifest\) \{\s*themes\.push/);
    assert.doesNotMatch(
        themesMain,
        /getCachedThemeList[\s\S]{0,400}readFileSync\(`\$\{themePath\}\/\$\{themeFile\.theme\}`/,
    );
    assert.match(ipc, /ipcMain\.on\("getThemes", \(event\) => \{/);
    assert.match(ipc, /event\.returnValue = getCachedThemeList\(\);/);
    assert.match(ipc, /ipcMain\.on\("refreshThemesCache", \(event\) => \{/);
    assert.match(bridge, /getThemes:\s*\(\)\s*=>\s*ipcRenderer\.sendSync\("getThemes"\)/);
    assert.match(bridge, /refresh:\s*\(\)\s*=>\s*ipcRenderer\.sendSync\("refreshThemesCache"\)/);
});

test("no src caller uses the native theme list path yet", () => {
    // Step 3d-2 wires the ThemesPage inventory: native listThemes() first,
    // Electron getThemes/refresh only when the capability is absent.
    const themesPage = readFileSync(
        new URL("../../src/shelter/settings/pages/ThemesPage.tsx", import.meta.url),
        "utf8",
    );
    for (const source of [bridge, ipc, types]) {
        assert.doesNotMatch(source, /themes\.listThemes/);
        assert.doesNotMatch(source, /themes\.list\b/);
        assert.doesNotMatch(source, /themes_list/);
    }
    assert.match(themesPage, /refreshNativeThemes\(\)/);
    assert.match(themesPage, /isNativeThemeListAvailable\(\)/);
    assert.match(themesPage, /window\.remcord\.themes\.refresh\(\)/);
    assert.doesNotMatch(themesPage, /themes\.list\b/);
    assert.doesNotMatch(themesPage, /themes_list/);
});

test("native theme list is metadata-only with bounded entries", () => {
    // The host enumerates its own root in sorted-id order and returns the
    // UI-consumed display subset: no CSS contents, no paths, no update
    // URLs, no filesystem surface.
    const nativeThemes = readFileSync(new URL("../src/themes.rs", import.meta.url), "utf8");
    assert.match(nativeThemes, /pub fn list_themes/);
    assert.match(nativeThemes, /pub const MAX_LISTED_THEMES: usize = 256/);
    assert.match(nativeThemes, /pub const MAX_LIST_FIELD_BYTES: usize = 4096/);
    const shippedThemes = nativeThemes.slice(0, nativeThemes.indexOf("#[cfg(test)]"));
    const listBody = shippedThemes.slice(
        shippedThemes.indexOf("pub fn list_themes"),
        shippedThemes.indexOf("/// Deterministic DOM id"),
    );
    assert.ok(listBody.length > 0);
    assert.doesNotMatch(listBody, /read_to_string\(&css|css_bytes|updateSrc/);
    assert.doesNotMatch(listBody, /evaluate_script|fetch\(|ureq|openPath|ShellExecute/);
    // The facade sends an exact empty payload and validates the bounded
    // display array: no path, root, selector, URL, or filter member.
    const listFacade = nativeFacade.slice(
        nativeFacade.indexOf("function listThemes"),
        nativeFacade.indexOf("function listBundles"),
    );
    assert.ok(listFacade.length > 0);
    assert.match(listFacade, /requireCapability\("themes\.list"\)/);
    assert.match(listFacade, /\{\s*type:\s*"themes_list",\s*payload:\s*\{\s*\}\s*\}/);
    assert.doesNotMatch(listFacade, /fetch\(|showOpenDialog|openPath|"path"|"url"|path:|url:/);
});

test("settings UI reveals the storage folder natively and sends otherwise", () => {
    const settingsPage = readFileSync(
        new URL("../../src/shelter/settings/pages/SettingsPage.tsx", import.meta.url),
        "utf8",
    );
    // Native branch calls the typed capability with the exact location;
    // Electron branch keeps the original fire-and-forget send.
    assert.match(settingsPage, /revealFolderNative\("storage"\)/);
    assert.match(settingsPage, /window\.remcord\.settings\.openStorageFolder\(\)/);
    // Exactly one dispatch: native success short-circuits the Electron call.
    assert.equal((settingsPage.match(/revealFolderNative\("storage"\)/g) ?? []).length, 1);
});

test("themes UI reveals the themes folder natively and sends otherwise", () => {
    const themesPage = readFileSync(
        new URL("../../src/shelter/settings/pages/ThemesPage.tsx", import.meta.url),
        "utf8",
    );
    assert.match(themesPage, /revealFolderNative\("themes"\)/);
    assert.match(themesPage, /window\.remcord\.settings\.openThemesFolder\(\)/);
    assert.equal((themesPage.match(/revealFolderNative\("themes"\)/g) ?? []).length, 1);
});

test("plugins UI reveals the plugins folder natively and sends otherwise", () => {
    const pluginsPage = readFileSync(
        new URL("../../src/shelter/settings/pages/PluginsPage.tsx", import.meta.url),
        "utf8",
    );
    assert.match(pluginsPage, /revealFolderNative\("plugins"\)/);
    assert.match(pluginsPage, /window\.remcord\.plugins\.openFolder\(\)/);
    // The toolbar button and the empty-state action each dispatch once.
    assert.equal((pluginsPage.match(/revealFolderNative\("plugins"\)/g) ?? []).length, 2);
});

test("plugins UI manages plugins through the native capabilities with no environment branch", () => {
    const pluginsPage = readFileSync(
        new URL("../../src/shelter/settings/pages/PluginsPage.tsx", import.meta.url),
        "utf8",
    );
    // List, enable flip, and reload all run natively now: no native guard,
    // no unavailable-state fallback.
    assert.match(pluginsPage, /window\.remcord\.plugins\.list\(\)/);
    assert.match(pluginsPage, /window\.remcord\.plugins\.setEnabled\(plugin\.id, enabled\)/);
    assert.match(pluginsPage, /window\.remcord\.plugins\.reload\(plugin\.id\)/);
    assert.doesNotMatch(pluginsPage, /isNativeWebView/);
    assert.doesNotMatch(pluginsPage, /nativeUnavailable/);
});

test("theme cards open the theme folder natively with the verbatim id", () => {
    const themesCard = readFileSync(
        new URL("../../src/shelter/settings/components/ThemesCard.tsx", import.meta.url),
        "utf8",
    );
    assert.match(themesCard, /openThemeFolderNative\(props\.theme\.id\)/);
    assert.match(themesCard, /window\.remcord\.themes\.folder\(props\.theme\.id\)/);
    // No renderer-side id filtering: validation is owned by the host.
    assert.doesNotMatch(themesCard, /RegExp|new RegExp|\.test\(props\.theme\.id\)|\.match\(props\.theme\.id\)/);
});

test("folder UI wiring introduces no generic filesystem surface", () => {
    const settingsHelper = readFileSync(new URL("../../src/shelter/settings/settings.ts", import.meta.url), "utf8");
    const settingsPage = readFileSync(
        new URL("../../src/shelter/settings/pages/SettingsPage.tsx", import.meta.url),
        "utf8",
    );
    const themesPage = readFileSync(
        new URL("../../src/shelter/settings/pages/ThemesPage.tsx", import.meta.url),
        "utf8",
    );
    const pluginsPage = readFileSync(
        new URL("../../src/shelter/settings/pages/PluginsPage.tsx", import.meta.url),
        "utf8",
    );
    const themesCard = readFileSync(
        new URL("../../src/shelter/settings/components/ThemesCard.tsx", import.meta.url),
        "utf8",
    );
    for (const source of [settingsHelper, settingsPage, themesPage, pluginsPage, themesCard]) {
        assert.doesNotMatch(source, /openPath|showItemInFolder|ShellExecute|openExternal/);
        assert.doesNotMatch(source, /invoke\(|sendSync|eval\(|new Function/);
        assert.doesNotMatch(source, /\bany\b/);
    }
    // One environment check, reused: no second detection mechanism.
    assert.match(settingsHelper, /export function isNativeWebView/);
    for (const source of [settingsPage, themesPage, pluginsPage, themesCard]) {
        assert.doesNotMatch(source, /function isNativeWebView/);
    }
});

test("Electron keeps the synchronous settings-plugin boot contracts", () => {
    // getConfig/getThemes are sendSync reads, translations is a preload-time
    // sendSync value, setLang/restart are fire-and-forget sends.
    assert.match(bridge, /getConfig:\s*\(\)\s*=>\s*ipcRenderer\.sendSync\("getEntireConfig"\)/);
    assert.match(bridge, /getThemes:\s*\(\)\s*=>\s*ipcRenderer\.sendSync\("getThemes"\)/);
    assert.match(bridge, /translations:\s*ipcRenderer\.sendSync\("getTranslations"\)/);
    assert.match(bridge, /setLang:\s*\(lang:\s*string\)\s*=>\s*ipcRenderer\.send\("setLang", lang\)/);
    assert.match(bridge, /restart:\s*\(\)\s*=>\s*ipcRenderer\.send\("restart"\)/);
    assert.match(ipc, /ipcMain\.on\("getEntireConfig"/);
    assert.match(ipc, /ipcMain\.on\("getThemes"/);
    assert.match(ipc, /ipcMain\.on\("getTranslations"/);
    assert.match(ipc, /ipcMain\.on\("setLang"/);
    assert.match(ipc, /ipcMain\.on\("restart"/);
    // The real plugin reads all five synchronously during onLoad.
    const settingsIndex = readFileSync(new URL("../../src/shelter/settings/index.ts", import.meta.url), "utf8");
    assert.match(settingsIndex, /export function onLoad\(\)/);
    assert.match(settingsIndex, /refreshSettings\(\)/);
    assert.match(settingsIndex, /refreshThemes\(\)/);
    assert.match(settingsIndex, /store\.i18n = window\.remcord\.translations/);
    assert.match(settingsIndex, /window\.remcord\.settings\.setLang\(storesFlat\.LocaleStore\.locale\)/);
    assert.doesNotMatch(settingsIndex, /await/);
});

test("no src caller uses the native settingsBoot surface yet", () => {
    // This milestone stops before full settings-plugin execution: no src/
    // file may reference the native boot surface, and the Electron contracts
    // above must be the only ones the plugin reads.
    const settingsIndex = readFileSync(new URL("../../src/shelter/settings/index.ts", import.meta.url), "utf8");
    for (const source of [bridge, ipc, types, shelterSettings, settingsIndex]) {
        assert.doesNotMatch(source, /settingsBoot/);
        assert.doesNotMatch(source, /settings_boot/);
    }
});

test("native settingsBoot is host-built data plus two narrow senders", () => {
    // The snapshot is built from the actual capability owners only.
    const nativeBoot = readFileSync(new URL("../src/settings_boot.rs", import.meta.url), "utf8");
    assert.match(nativeBoot, /crate::settings::load_settings/);
    assert.match(nativeBoot, /crate::themes::raw_theme_manifests/);
    assert.match(nativeBoot, /crate::localization::load_raw_lang/);
    const shippedBoot = nativeBoot.slice(0, nativeBoot.indexOf("#[cfg(test)]"));
    assert.doesNotMatch(shippedBoot, /sendSync|evaluate_script|fetch\(|ureq|openPath|ShellExecute/);
    // The facade exposes exactly seven members: five data plus two
    // fire-and-forget senders over closed one-way events.
    const bootFacade = nativeFacade.slice(nativeFacade.indexOf("const LOCALE_PATTERN"));
    assert.ok(bootFacade.length > 0);
    assert.match(bootFacade, /\{\s*type:\s*"settings_boot_set_lang",\s*payload:\s*\{\s*locale\s*\}\s*\}/);
    assert.match(bootFacade, /\{\s*type:\s*"settings_boot_restart",\s*payload:\s*\{\s*\}\s*\}/);
    assert.doesNotMatch(bootFacade, /sendSync|new Function|[^a-zA-Z]eval\(|require\(/);
});

test("Electron keeps the synchronous platform and release bridges", () => {
    assert.match(bridge, /platform:\s*ipcRenderer\.sendSync\("getOS"\)/);
    assert.match(bridge, /osRelease:\s*ipcRenderer\.sendSync\("getOSRelease"\)/);
    assert.match(ipc, /ipcMain\.on\("getOS"/);
    assert.match(ipc, /event\.returnValue\s*=\s*process\.platform/);
    assert.match(ipc, /ipcMain\.on\("getOSRelease"/);
    assert.match(ipc, /event\.returnValue\s*=\s*os\.release\(\)/);
});

test("Electron keeps the optional version read for the about popup", () => {
    const aboutPopup = readFileSync(
        new URL("../../src/shelter/settings/components/AboutPopup.tsx", import.meta.url),
        "utf8",
    );
    assert.match(aboutPopup, /window\.remcord\?\.version/);
});

test("no src caller uses the native shim surface", () => {
    const settingsIndex = readFileSync(new URL("../../src/shelter/settings/index.ts", import.meta.url), "utf8");
    for (const source of [bridge, ipc, types, shelterSettings, settingsIndex]) {
        assert.doesNotMatch(source, /settingsBoot/);
        assert.doesNotMatch(source, /settings_boot/);
        assert.doesNotMatch(source, /hostPlatform/);
        assert.doesNotMatch(source, /getThemesSync/);
    }
});

test("native shim is static data plus closed narrow senders", () => {
    // platform/osRelease are spliced launch-time strings (process.platform /
    // os.release() parity), never capabilities, requests, or reads.
    const platformFacade = nativeFacade.slice(
        nativeFacade.indexOf("const hostPlatform = freezeDeep"),
        nativeFacade.indexOf("Object.defineProperty(window"),
    );
    assert.ok(platformFacade.length > 0);
    // The top-level assignments carry the frozen host strings (asserted on
    // the whole facade: the slice above ends where window.remcord is built).
    assert.match(nativeFacade, /platform:\s*hostPlatform\.platform/);
    assert.match(nativeFacade, /osRelease:\s*hostPlatform\.osRelease/);
    assert.doesNotMatch(platformFacade, /requireCapability|request\(|sendSync|fetch\(/);
    // The dual getConfig keeps the capability gate and the live request for
    // awaited callers; the synchronous value is a snapshot working copy.
    const dualGetConfig = nativeFacade.slice(
        nativeFacade.indexOf("function getConfig"),
        nativeFacade.indexOf("function getLang"),
    );
    assert.match(dualGetConfig, /requireCapability\("settings\.get_config"\)/);
    assert.match(dualGetConfig, /\{\s*type:\s*"settings_get_config",\s*payload:\s*\{\s*\}\s*\}/);
    assert.match(dualGetConfig, /settingsWorkingCopy/);
    assert.doesNotMatch(dualGetConfig, /sendSync|new Function|[^a-zA-Z]eval\(/);
});

test("Electron keeps the remcord:// protocol namespaces and root mapping", () => {
    // P0.2 parity pin: the native resolver serves exactly the six Electron
    // namespaces with the same bundle-relative roots. Electron behavior is
    // the reference; native hardening (canonical containment, no `..`
    // substring check) is a deliberate, documented difference, not a drift
    // in coverage. Product difference: Electron registers `remcord://`;
    // Remcord registers only `remcord://` and rewrites legacy `remcord://`
    // URLs through the same resolver (no second surface).
    const electronProtocol = readFileSync(new URL("../../src/protocol.ts", import.meta.url), "utf8");
    for (const namespace of ["plugins", "html", "js", "assets", "css", "local"]) {
        assert.match(electronProtocol, new RegExp(`remcord://${namespace}/`));
    }
    // Root mapping parity: bundle-relative dirs plus userAssets under userData.
    assert.match(electronProtocol, /path\.join\(import\.meta\.dirname, "plugins"/);
    assert.match(electronProtocol, /path\.join\(import\.meta\.dirname, "html"/);
    assert.match(electronProtocol, /path\.join\(import\.meta\.dirname, "js"/);
    assert.match(electronProtocol, /path\.join\(import\.meta\.dirname, "assets", "app"/);
    assert.match(electronProtocol, /path\.join\(import\.meta\.dirname, "css"/);
    assert.match(electronProtocol, /path\.join\(app\.getPath\("userData"\), "userAssets"\)/);
    const nativeScheme = readFileSync(new URL("../src/remcord_protocol/scheme.rs", import.meta.url), "utf8");
    const nativeHandler = readFileSync(new URL("../src/remcord_protocol/handler.rs", import.meta.url), "utf8");
    assert.match(nativeHandler, /plugins.*bundle_dir\.join\("plugins"\)/s);
    assert.match(nativeHandler, /html.*bundle_dir\.join\("html"\)/s);
    assert.match(nativeHandler, /assets.*bundle_dir\.join\("assets"\)\.join\("app"\)/s);
    assert.match(nativeHandler, /local.*user_data_dir\.join\("userAssets"\)/s);
    assert.match(nativeScheme, /REMCORD_ALLOWED_ORIGINS.*https:\/\/discord\.com/);
    // Canonical scheme is remcord, registered exactly once at the
    // environment level; no second scheme, alias, or rewrite exists.
    assert.match(nativeScheme, /REMCORD_SCHEME_NAME.*=.*"remcord"/);
    assert.doesNotMatch(nativeScheme, /CoreWebView2CustomSchemeRegistration::new\(LEGACY/);
    assert.match(nativeHandler, /remcord:\/\/\*/);
    const nativeCsp = readFileSync(new URL("../src/remcord_protocol/csp.rs", import.meta.url), "utf8");
    assert.match(nativeCsp, /extend_csp_for_remcord/);
    assert.doesNotMatch(nativeCsp, /LEGACY_CSP_SOURCE/);
    // Hardening markers: canonical containment, symlink refusal, no path
    // leakage, and no Electron-style substring check in executable code
    // (the doc comment names Electron's weak check for contrast — strip
    // comment lines before asserting).
    const nativeResolver = readFileSync(new URL("../src/remcord_protocol/resolver.rs", import.meta.url), "utf8");
    assert.match(nativeResolver, /canonicalize/);
    assert.match(nativeResolver, /SymlinkEscape/);
    const shippedResolver = nativeResolver
        .slice(0, nativeResolver.indexOf("#[cfg(test)]"))
        .split("\n")
        .filter((line) => !line.trim().startsWith("//"))
        .join("\n");
    assert.doesNotMatch(shippedResolver, /includes\(\s*"\.\."\s*\)/);
});

test("the known-broken disableAutogain caller is left untouched", () => {
    // remcord://assets/js/disableAutogain.js is referenced by
    // src/discord/preload/patches.mts but the file does not exist in the
    // repository. P0.2 must neither fabricate it nor silently rewrite the
    // caller: resolution reports it missing (404) like any absent asset.
    const patches = readFileSync(new URL("../../src/discord/preload/patches.mts", import.meta.url), "utf8");
    assert.match(patches, /remcord:\/\/assets\/js\/disableAutogain\.js/);
    assert.throws(() => readFileSync(new URL("../../ts-out/assets/app/js/disableAutogain.js", import.meta.url)));
});

test("F5 gallery keeps the Electron store contract with a natively-gated page", () => {
    // The native F5 core mirrors the Electron store operation-for-operation
    // (gallery.ts) plus the bridge signatures (bridge.ts); the Gallery page
    // calls the same window.remcord.gallery methods natively and gates only
    // the dialog-backed buttons (import/export/root-pick, F5b slice).
    const store = readFileSync(new URL("../../src/common/gallery.ts", import.meta.url), "utf8");
    for (const fn of [
        "listGalleryItems",
        "addGalleryUrl",
        "addGalleryLink",
        "removeGalleryItem",
        "bulkRemoveGalleryItems",
        "updateGalleryItem",
        "listGalleryCategories",
        "addGalleryCategory",
        "renameGalleryCategory",
        "removeGalleryCategory",
        "getGalleryDiskUsage",
        "resolveGalleryFile",
    ]) {
        assert.match(store, new RegExp(`export (async )?function ${fn}`), `${fn} must stay exported`);
    }
    assert.match(store, /gallery-index\.json/);
    const ipc = readFileSync(new URL("../../src/discord/ipc.ts", import.meta.url), "utf8");
    assert.match(ipc, /48 \* 1024 \* 1024/);
    assert.match(ipc, /TOO_LARGE/);
    const bridge = readFileSync(new URL("../../src/discord/preload/bridge.ts", import.meta.url), "utf8");
    for (const method of [
        "gallery:list",
        "gallery:add-url",
        "gallery:add-link",
        "gallery:remove",
        "gallery:bulk-remove",
        "gallery:update",
        "gallery:categories",
        "gallery:category-add",
        "gallery:category-rename",
        "gallery:category-remove",
        "gallery:usage",
        "gallery:root",
        "gallery:file",
    ]) {
        assert.match(bridge, new RegExp(`"${method}"`), `${method} must stay bridged`);
    }
    const page = readFileSync(new URL("../../src/shelter/settings/pages/GalleryPage.tsx", import.meta.url), "utf8");
    // The full-page unavailable gate is gone; only dialog-backed actions
    // stay gated behind isNativeWebView.
    assert.doesNotMatch(page, /does not provide the Gallery disk cache yet/);
    assert.match(page, /window\.remcord\.gallery/);
    assert.match(page, /\.list\(\)/);
    assert.match(page, /\.addUrl\(/);
    assert.match(page, /F5b dialog slice/);
    // The native contract pins the same shape from the host side.
    const nativeGallery = readFileSync(new URL("../src/gallery.rs", import.meta.url), "utf8");
    assert.match(nativeGallery, /INDEX_VERSION/);
    assert.match(nativeGallery, /GALLERY_UNAVAILABLE|GALLERY_FULL/);
    assert.match(nativeGallery, /resolve_gallery_file/);
    const compat = readFileSync(new URL("../web/compatibility.js", import.meta.url), "utf8");
    assert.match(compat, /const gallery = Object\.freeze/);
    assert.match(compat, /list: listGalleryItems/);
    assert.match(compat, /getFile: getGalleryFile/);
});

test("F8 keybind execution runs validated entries with runJavascript structurally excluded", () => {
    // Electron registers enabled+global entries as system hotkeys and
    // serves enabled non-global ones as menu accelerators; runAction runs
    // nine actions including arbitrary-JS execution. Natively the same
    // entries register through RegisterHotKey (global) and the in-page
    // matcher (non-global), actions run through fixed page scripts, and
    // runJavascript has no wire shape anywhere.
    const keybindsMain = readFileSync(new URL("../../src/discord/globalKeybinds.ts", import.meta.url), "utf8");
    assert.match(keybindsMain, /globalShortcut\.register\(keybind\.accelerator/);
    assert.match(keybindsMain, /globalShortcut\.unregisterAll\(\)/);
    assert.match(ipc, /ipcMain\.on\("addKeybind"/);
    assert.match(ipc, /ipcMain\.on\("toggleKeybind"/);
    assert.match(ipc, /ipcMain\.on\("removeKeybind"/);
    assert.match(ipc, /refreshGlobalKeybinds\(\)/);
    // The Keybinds page manages entries through the same member names
    // natively (async capabilities) and offers no runJavascript action
    // on that path; the card toggles/removes through the same members.
    const keybindsPage = readFileSync(
        new URL("../../src/shelter/settings/pages/KeybindsPage.tsx", import.meta.url),
        "utf8",
    );
    assert.doesNotMatch(keybindsPage, /not available in the native Remcord app yet/);
    assert.match(keybindsPage, /KeybindMaker/);
    const keybindMaker = readFileSync(
        new URL("../../src/shelter/settings/components/KeybindMaker.tsx", import.meta.url),
        "utf8",
    );
    assert.match(keybindMaker, /isNativeKeybindAvailable\(\)/);
    assert.match(keybindMaker, /addKeybindNative\(/);
    const keybindCard = readFileSync(
        new URL("../../src/shelter/settings/components/KeybindCard.tsx", import.meta.url),
        "utf8",
    );
    assert.match(keybindCard, /toggleKeybindNative\(/);
    assert.match(keybindCard, /removeKeybindNative\(/);
    // The native contract pins the same shape from the host side. Doc
    // comments name the forbidden primitives for contrast — strip comment
    // lines before asserting, like the resolver test does.
    const nativeKeybinds = readFileSync(new URL("../src/keybinds.rs", import.meta.url), "utf8");
    assert.match(nativeKeybinds, /STRUCTURALLY EXCLUDED/);
    const shippedKeybinds = nativeKeybinds
        .split("\n")
        .filter((line) => !line.trim().startsWith("//"))
        .join("\n");
    assert.doesNotMatch(shippedKeybinds, /SendInput|SetWindowsHookEx|WH_KEYBOARD/);
    // Global registration lives in the main-thread worker owner; the
    // matched event shape lives in the protocol plus the matcher script.
    const nativeMain = readFileSync(new URL("../src/main.rs", import.meta.url), "utf8");
    assert.match(nativeMain, /RegisterHotKey/);
    assert.match(nativeMain, /KeybindFired/);
    const nativeProtocol = readFileSync(new URL("../src/protocol.rs", import.meta.url), "utf8");
    assert.match(nativeProtocol, /KeybindMatched/);
    const matcher = readFileSync(new URL("../web/keybinds/matcher.js", import.meta.url), "utf8");
    assert.match(matcher, /keybind_matched/);
    assert.match(matcher, /preventDefault/);
});

test("F7 settings import stays copy-only with no Electron auto-migration", () => {
    // The Electron app never migrates settings between profiles: no
    // cross-profile import call may exist on its side (the internal
    // startMinimized boolean migration is unrelated and stays), and the
    // native copy-only path must not add one either. The Electron settings
    // shape (config.ts defaults + settings.d.ts keys) stays the import
    // reference; the native UI triggers exactly one explicit capability.
    const config = readFileSync(new URL("../../src/common/config.ts", import.meta.url), "utf8");
    assert.doesNotMatch(config, /importElectron|import.*profile|migrat.*profile|profile.*migrat/i);
    const settingsPage = readFileSync(
        new URL("../../src/shelter/settings/pages/SettingsPage.tsx", import.meta.url),
        "utf8",
    );
    assert.match(settingsPage, /isNativeSettingsImportAvailable\(\)/);
    assert.match(settingsPage, /importElectronSettingsNative\(\)/);
    assert.match(settingsPage, /markRestartRequired\(\)/);
    // Explicit button + confirm only: no import call outside the guarded
    // button component, no startup/timer import anywhere in src/.
    assert.doesNotMatch(config, /settings_import_electron/);
    const settings = readFileSync(new URL("../../src/shelter/settings/settings.ts", import.meta.url), "utf8");
    assert.match(settings, /export function isNativeSettingsImportAvailable/);
    assert.match(settings, /export async function importElectronSettingsNative/);
    // The native contract pins the same shape from the host side.
    const nativeImport = readFileSync(new URL("../src/settings_import.rs", import.meta.url), "utf8");
    assert.match(nativeImport, /IMPORTABLE_KEYS/);
    assert.match(nativeImport, /RESTART_REQUIRED_KEYS/);
    assert.match(nativeImport, /copy-only|Copy-only/);
    assert.doesNotMatch(nativeImport, /std::fs::write\([^)]*appdata|electron.*write/i);
});

test("F6 backup UI drives the native inspect/commit/discard flow with a working preview modal", () => {
    // D4-3: BackupSection routes save through the shared saveBackup helper
    // (same signature/shape both shells) and restore through the native
    // inspect → preview-modal → commit/discard flow when
    // isNativeBackupAvailable(), keeping the Electron JSON-string path
    // otherwise. The preview modal must render: every component it uses
    // has to be imported (Text/TextTags were missing and crashed it).
    const backupSection = readFileSync(
        new URL("../../src/shelter/settings/components/BackupSection.tsx", import.meta.url),
        "utf8",
    );
    assert.match(backupSection, /isNativeBackupAvailable\(\)/);
    assert.match(backupSection, /inspectBackupNative\(\)/);
    assert.match(backupSection, /commitRestoreNative\(\)/);
    assert.match(backupSection, /discardRestoreNative\(\)/);
    assert.match(backupSection, /normalizeNativeInspectResult/);
    assert.match(backupSection, /normalizeNativeRestoreResult/);
    assert.match(backupSection, /BackupRestorePreviewModal/);
    // Regression: the preview modal renders <Text tag={TextTags.*}> — both
    // must be destructured from shelter.ui or the modal throws at open.
    const destructure = backupSection.match(/ui:\s*\{([\s\S]*?)\},\s*plugin:/);
    assert.ok(destructure, "shelter.ui destructure must exist");
    assert.match(backupSection, /<Text[ >]/, "<Text> must be used");
    assert.match(backupSection, /TextTags\./, "TextTags.* must be used");
    for (const name of ["Text", "TextTags"]) {
        assert.ok(new RegExp(`\\b${name}\\b`).test(destructure[1]), `${name} must be destructured from shelter.ui`);
    }
    // The Electron path stays intact alongside the native branches.
    assert.match(backupSection, /window\.remcord\.backup\.restore\(\)/);
    assert.match(backupSection, /parseElectronRestoreResult/);
    // Helpers + normalizers behind the UI path.
    const settings = readFileSync(new URL("../../src/shelter/settings/settings.ts", import.meta.url), "utf8");
    for (const fn of [
        "isNativeBackupAvailable",
        "saveBackup",
        "inspectBackupNative",
        "commitRestoreNative",
        "discardRestoreNative",
        "normalizeNativeInspectResult",
        "normalizeNativeRestoreResult",
        "parseElectronRestoreResult",
    ]) {
        assert.match(settings, new RegExp(`export (async )?function ${fn}`), `${fn} must stay exported`);
    }
});

test("F4 in-mod settings UI stays Electron-owned with no native src branch", () => {
    // Vencord/Equicord persist page-side in localStorage; Shelter owns its
    // plugins through its own registrar. F4 adds no host surface, so the
    // Electron sources must still own these paths and src/ must carry no
    // native settings-page branch.
    const backupSection = readFileSync(
        new URL("../../src/shelter/settings/components/BackupSection.tsx", import.meta.url),
        "utf8",
    );
    assert.match(backupSection, /localStorage\.getItem\("VencordSettings"\)/);
    assert.match(backupSection, /localStorage\.getItem\("EquicordSettings"\)/);
    assert.match(backupSection, /localStorage\.setItem\("VencordSettings"/);
    assert.match(backupSection, /localStorage\.setItem\("EquicordSettings"/);
    assert.match(backupSection, /shelter\.plugins\.installedPlugins/);
    assert.match(backupSection, /shelter\.plugins\.addRemotePlugin/);
    const editorSource = readFileSync(
        new URL("../../src/shelter/settings/quickCssEditorSource.ts", import.meta.url),
        "utf8",
    );
    assert.match(editorSource, /"vencord" \| "equicord" \| "remcord"/);
    assert.match(editorSource, /VencordNative\?/);
    assert.match(editorSource, /quickCss/);
    const settingsIndex = readFileSync(new URL("../../src/shelter/settings/index.ts", import.meta.url), "utf8");
    assert.match(settingsIndex, /remcord-native:open-quickcss-editor/);
    // No native settings-page branch may appear in the Electron sources.
    for (const source of [backupSection, editorSource, settingsIndex]) {
        assert.doesNotMatch(source, /mod_settings|plugin_settings|settings_page/);
    }
    // The native contract pins the same shape from the host side.
    const modSettings = readFileSync(new URL("../src/mod_settings.rs", import.meta.url), "utf8");
    assert.match(modSettings, /VENCORD_SETTINGS_KEY/);
    assert.match(modSettings, /EQUICORD_SETTINGS_KEY/);
    assert.match(modSettings, /QUICKCSS_HANDOFF_EVENT/);
    assert.match(modSettings, /FORBIDDEN_F4_CAPABILITY_FRAGMENTS/);
});

test("F16 window-state persistence keeps Electron semantics with host-owned placement", () => {
    // Electron owns the save/restore contract: window.json under
    // <userData>/storage, debounced saves on move/resize, monitor-fitted
    // restore with a primary-monitor fallback.
    const windowState = readFileSync(new URL("../../src/common/windowState.ts", import.meta.url), "utf8");
    assert.match(windowState, /window\.json/);
    assert.match(windowState, /setWindowState/);
    assert.match(windowState, /getWindowState/);
    const windowBounds = readFileSync(new URL("../../src/common/windowBounds.ts", import.meta.url), "utf8");
    assert.match(windowBounds, /sanitizeWindowBounds/);
    assert.match(windowBounds, /MIN_VISIBLE_PX/);
    const discordWindow = readFileSync(new URL("../../src/discord/window.ts", import.meta.url), "utf8");
    assert.match(discordWindow, /saveWindowState/);
    assert.match(discordWindow, /getStoredWindowBounds/);
    assert.match(discordWindow, /queueSave/);
    // The native host mirrors the file shape and the fit math, and adds
    // exactly one parameter-free read capability. Placement stays
    // host-owned: no setter/move/resize wire exists on either side of the
    // bridge (Electron never exposed renderer-driven placement either).
    const nativeWindowState = readFileSync(new URL("../src/window_state.rs", import.meta.url), "utf8");
    assert.match(nativeWindowState, /WINDOW_FILE_NAME/);
    assert.match(nativeWindowState, /STORAGE_DIR_NAME/);
    assert.match(nativeWindowState, /SAVE_DEBOUNCE/);
    assert.match(nativeWindowState, /MIN_VISIBLE_PX/);
    assert.match(nativeWindowState, /resolve_placement/);
    const protocol = readFileSync(new URL("../src/protocol.rs", import.meta.url), "utf8");
    assert.match(protocol, /WindowGetState/);
    assert.match(protocol, /window_get_state/);
    // No setter/move/resize command variant exists (the rejection tests
    // name the forbidden wires, so the check pins constructors, not
    // mentions).
    for (const variant of ["WindowSetState(", "WindowMove(", "WindowResize(", "WindowSetBounds("]) {
        assert.equal(protocol.includes(variant), false);
    }
});
test("F19 discord:// links stay Electron-shaped with a host-owned native mirror", () => {
    // Electron owns the contract: scheme registration, second-instance /
    // open-url handling, and the pushState navigation with its exact
    // sanitize class. The native mirror (`native/src/discord_protocol.rs`)
    // ports each piece host-side with no renderer surface.
    const mainSource = readFileSync(new URL("../../src/main.ts", import.meta.url), "utf8");
    assert.match(mainSource, /setAsDefaultProtocolClient\("discord"\)/);
    const windowSource = readFileSync(new URL("../../src/discord/window.ts", import.meta.url), "utf8");
    assert.match(windowSource, /second-instance/);
    assert.match(windowSource, /open-url/);
    assert.match(windowSource, /discord:\/\/-/);
    const domSource = readFileSync(new URL("../../src/common/dom.ts", import.meta.url), "utf8");
    assert.match(domSource, /history\.pushState/);
    assert.match(domSource, /popstate/);
    assert.match(domSource, /\[\^a-zA-Z0-9\/_\\-@\.\]/);
    // The native host mirror ports the same shapes with no bridge surface.
    const deeplink = readFileSync(new URL("../src/discord_protocol.rs", import.meta.url), "utf8");
    assert.match(deeplink, /DISCORD_LINK_PREFIX/);
    assert.match(deeplink, /history\.pushState/);
    assert.match(deeplink, /setAsDefaultProtocolClient/);
    assert.doesNotMatch(deeplink, /BridgeCommand|BridgeResponsePayload/);
});

test("F11 keeps the Electron rpc bridge, worker, and stores intact", () => {
    // The preload rpc surface stays exactly as the Games page and Shelter
    // consume it: fire-and-forget sends plus synchronous reads. The native
    // shell adds async paths alongside (never rewrites these members).
    assert.match(bridge, /refreshProcessList:\s*\(\)\s*=>\s*ipcRenderer\.send\("refreshProcessList"\)/);
    assert.match(bridge, /getProcessList:\s*\(\)\s*=>\s*ipcRenderer\.sendSync\("getProcessList"\)/);
    assert.match(bridge, /addDetectable:\s*\(detectable:\s*Game\)\s*=>\s*ipcRenderer\.send\("addDetectable", detectable\)/);
    assert.match(bridge, /removeDetectable:\s*\(id:\s*string\)\s*=>\s*ipcRenderer\.send\("removeDetectable", id\)/);
    assert.match(bridge, /getDetectables:\s*\(\)\s*=>\s*ipcRenderer\.sendSync\("getDetectables"\)/);
    assert.match(bridge, /getBlacklist:\s*\(\)\s*=>\s*rpcBlacklistCache\.get\(\)/);
    assert.match(bridge, /blacklistGame:\s*\(name:\s*string,\s*id:\s*number\)\s*=>\s*\{/);
    assert.match(bridge, /unblacklistGame:\s*\(id:\s*number\)\s*=>\s*\{/);
    // The declared contract keeps the synchronous rpc reads.
    assert.match(types, /getProcessList:\s*\(\)\s*=>\s*ProcessInfo\[\]/);
    assert.match(types, /refreshProcessList:\s*\(\)\s*=>\s*void/);
    assert.match(types, /getDetectables:\s*\(\)\s*=>\s*GameList/);
    assert.match(types, /getBlacklist:\s*\(\)\s*=>\s*DetectedGame\[\]/);
    // The main handlers stay: sync reads, mutations returning the new list
    // plus the rpcBlacklistUpdated broadcast, and the detectable controls.
    assert.match(ipc, /ipcMain\.on\("getRpcBlacklist"/);
    assert.match(ipc, /ipcMain\.on\("blacklistGame"/);
    assert.match(ipc, /ipcMain\.on\("unblacklistGame"/);
    assert.match(ipc, /passedWindow\.webContents\.send\("rpcBlacklistUpdated", blacklist\)/);
    assert.match(ipc, /ipcMain\.on\("getProcessList"/);
    assert.match(ipc, /ipcMain\.on\("refreshProcessList"/);
    assert.match(ipc, /ipcMain\.on\("getDetectables"/);
    assert.match(ipc, /ipcMain\.on\("addDetectable"/);
    assert.match(ipc, /ipcMain\.on\("removeDetectable"/);
});

test("F11 keeps the arRPC worker protocol and scan-settings resolution", () => {
    // The worker keeps its defaults, its activity/invite protocol, and its
    // on-demand process-list answer. The native shell resolves the same
    // settings host-side (never edits this worker).
    const rpcWorker = readFileSync(new URL("../../src/rpc.ts", import.meta.url), "utf8");
    assert.match(rpcWorker, /processScanning:\s*true/);
    assert.match(rpcWorker, /windowsLegacyScanning:\s*false/);
    assert.match(rpcWorker, /scanInterval:\s*5000/);
    assert.match(rpcWorker, /RPC\.on\("activity"/);
    assert.match(rpcWorker, /RPC\.on\("invite"/);
    assert.match(rpcWorker, /refreshProcessList/);
    assert.match(rpcWorker, /type:\s*"processList"/);
    const rpcProcess = readFileSync(new URL("../../src/discord/rpcProcess.ts", import.meta.url), "utf8");
    assert.match(rpcProcess, /MIN_SCAN_INTERVAL_MS\s*=\s*1000/);
    assert.match(rpcProcess, /DEFAULT_SCAN_INTERVAL_MS\s*=\s*5000/);
    assert.match(rpcProcess, /rawInterval\s*>=\s*MIN_SCAN_INTERVAL_MS/);
    assert.match(rpcProcess, /export function startRPC/);
    assert.match(rpcProcess, /export function stopRPC/);
    assert.match(rpcProcess, /export function refreshProcessList/);
    // The filesystem stores keep their exact semantics: tolerant blacklist
    // reads with id dedupe, file-backed detectables.
    const blacklist = readFileSync(new URL("../../src/common/blacklistGame.ts", import.meta.url), "utf8");
    assert.match(blacklist, /blacklist\.json/);
    assert.match(blacklist, /BLACKLIST_CACHE_TTL/);
    assert.match(blacklist, /if \(list\.some\(\(e\) => e\.id === entryId\)\) return;/);
    const detectables = readFileSync(new URL("../../src/common/detectables.ts", import.meta.url), "utf8");
    assert.match(detectables, /detectables\.json/);
    assert.match(detectables, /export function addDetectable/);
    assert.match(detectables, /export function removeDetectable/);
    // Shelter's hot presence path keeps the optional sync blacklist lookup
    // (the native facade serves it from a seeded cache, so this line works
    // unchanged under both shells).
    const shelterRpc = readFileSync(new URL("../../src/shelter/rpc/index.ts", import.meta.url), "utf8");
    assert.match(shelterRpc, /window\.remcord\.rpc\?\.getBlacklist\?\.\(\) \?\? \[\]/);
});

test("F11 keeps the Games page Electron-owned with a native EmptyState gate", () => {
    // The page stays synchronous Electron calls behind isNativeWebView()
    // guards; natively it renders the EmptyState. The native shell exposes
    // async rpc capabilities for the F11b rewire (never edits this page).
    const gamesPage = readFileSync(
        new URL("../../src/shelter/settings/pages/RegisteredGamesPage.tsx", import.meta.url),
        "utf8",
    );
    assert.match(gamesPage, /window\.remcord\.rpc\.refreshProcessList\(\)/);
    assert.match(gamesPage, /window\.remcord\.rpc\.getDetectables\(\)/);
    assert.match(gamesPage, /window\.remcord\.rpc\.getProcessList\(\)/);
    assert.match(gamesPage, /window\.remcord\.rpc\.getBlacklist\(\)/);
    assert.match(gamesPage, /isNativeWebView\(\)/);
    assert.match(gamesPage, /games-empty/);
    // The native host surface behind the future rewire exists with the
    // Electron-named capabilities and no socket surface.
    const arrpc = readFileSync(new URL("../src/arrpc.rs", import.meta.url), "utf8");
    assert.match(arrpc, /pub fn load_blacklist/);
    assert.match(arrpc, /pub fn add_blacklist_game/);
    assert.match(arrpc, /pub fn remove_blacklist_game/);
    assert.match(arrpc, /pub fn load_detectables/);
    assert.match(arrpc, /pub fn add_detectable/);
    assert.match(arrpc, /pub fn remove_detectable/);
    assert.match(arrpc, /pub fn enumerate_processes/);
    assert.match(arrpc, /pub fn resolve_rpc_settings/);
    assert.doesNotMatch(arrpc, /TcpListener|TcpStream|tungstenite|tokio|std::net/);
});

test("F11 enables the four scanner keys through the native settings path", () => {
    // The host allowlist, the facade allowlist, and the TS writable list
    // agree on the four arRPC keys (restart-required model, like channel).
    const settingsRs = readFileSync(new URL("../src/settings.rs", import.meta.url), "utf8");
    for (const key of ["InviteWebsocket", "ProcessScanning", "WindowsLegacyScanning", "ScanInterval"]) {
        assert.match(settingsRs, new RegExp(`\\b${key}\\b`), `${key} must be allowlisted host-side`);
    }
    assert.match(settingsRs, /ScanInterval\(u64\)/);
    const compat = readFileSync(new URL("../web/compatibility.js", import.meta.url), "utf8");
    for (const key of ["invite_websocket", "process_scanning", "windows_legacy_scanning", "scan_interval"]) {
        assert.ok(compat.includes(`"${key}"`), `${key} must be allowlisted facade-side`);
    }
    const settingsTs = readFileSync(new URL("../../src/shelter/settings/settings.ts", import.meta.url), "utf8");
    for (const key of ["inviteWebsocket", "processScanning", "windowsLegacyScanning", "scanInterval"]) {
        assert.ok(settingsTs.includes(`"${key}"`), `${key} must be writable natively`);
    }
});

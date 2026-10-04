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
const types = readFileSync(new URL("../../src/@types/legcordWindow.d.ts", import.meta.url), "utf8");
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

test("declared window.legcord settings contract stays synchronous", () => {
    assert.match(types, /getConfig:\s*\(\)\s*=>\s*Readonly<Settings>/);
});

test("the Shelter power Electron path stays synchronous", () => {
    // The Electron branch must call the sync getter directly in onLoad.
    assert.match(power, /const settings = window\.legcord\.settings\.getConfig\(\);/);
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

test("declared window.legcord getLang contract stays async", () => {
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

test("declared window.legcord setConfig contract stays synchronous", () => {
    assert.match(types, /setConfig:\s*<K extends keyof Settings>\(object:\s*K,\s*toSet:\s*Settings\[K\]\)\s*=>\s*void/);
});

test("the Shelter settings Electron mutation path stays synchronous", () => {
    // The shared helper must still persist via the sync bridge call.
    assert.match(shelterSettings, /window\.legcord\.settings\.setConfig\(key, value\)/);
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

test("preload keeps the synchronous get-app-version bridge", () => {
    assert.match(bridge, /version:\s*ipcRenderer\.sendSync\("get-app-version", "app-version"\)/);
    assert.doesNotMatch(bridge, /invoke\("get-app-version"/);
});

test("main keeps the get-app-version handler returning the package version", () => {
    assert.match(ipc, /ipcMain\.on\("get-app-version"/);
    assert.match(ipc, /event\.returnValue = getVersion\(\)/);
});

test("declared window.legcord version contract stays a synchronous string", () => {
    assert.match(types, /version:\s*string/);
});

test("the update checker Electron path keeps its synchronous version read", () => {
    // Electron behavior: direct sync read plus the dot-stripped comparison.
    assert.match(heroUpdater, /return window\.legcord\.version;/);
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

test("declared window.legcord fs contract stays scoped", () => {
    assert.match(types, /fs:\s*\{/);
    assert.match(types, /writeFile:\s*\(/);
    assert.match(types, /readFile:\s*\(/);
});

test("no src caller is migrated to a native plugin-storage path in this milestone", () => {
    // There are no in-repository window.legcord.fs callers: the contract is
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
    assert.doesNotMatch(nativeFacade, /legcord-native-theme/);
});

test("no src caller is migrated to a native theme path in this milestone", () => {
    // Static startup injection is host-driven: no renderer caller changes.
    assert.doesNotMatch(power, /legcord-native-theme/);
    assert.doesNotMatch(shelterSettings, /legcord-native-theme/);
    assert.doesNotMatch(heroUpdater, /legcord-native-theme/);
    assert.doesNotMatch(invite, /legcord-native-theme/);
});

test("Electron keeps the Quick CSS host-push lifecycle", () => {
    // Missing file is created empty, content (even empty) is always pushed
    // as legcord-quick-css BEFORE themes, with a 300 ms-debounced watcher
    // for live updates and removeTheme on disable.
    assert.match(themesMain, /const quickCssPath = path\.join\(userDataPath, "\/quickCss\.css"\)/);
    assert.match(themesMain, /if \(!fs\.existsSync\(quickCssPath\)\)/);
    assert.match(themesMain, /fs\.writeFileSync\(quickCssPath, ""\)/);
    assert.match(themesMain, /webContents\.send\("addTheme", "legcord-quick-css"/);
    assert.match(themesMain, /webContents\.send\(\s*"removeTheme", "legcord-quick-css"\)/);
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
    assert.match(themesPage, /window\.legcord\.themes\.enableQuickCss\(\)/);
    assert.match(themesPage, /window\.legcord\.themes\.disableQuickCss\(\)/);
    assert.match(themesPage, /setConfig\("quickCss", e\)/);
});

test("no renderer Quick CSS pull exists in Electron or native", () => {
    // Quick CSS is a host -> renderer push with no pull-style reader: the
    // bridge exposes only push-side members (open/import/enable/disable via
    // fire-and-forget send), never a CSS-returning read. No quickCss.get()
    // shape may be introduced on either side.
    assert.doesNotMatch(bridge, /getQuickCss|readQuickCss|invoke\("getQuickCss"/);
    assert.doesNotMatch(types, /getQuickCss|readQuickCss/);
    assert.doesNotMatch(themesMain, /getQuickCss|quickCss\.get|getCSS/);
    assert.doesNotMatch(nativeFacade, /quickCss/i);
    assert.doesNotMatch(nativeFacade, /legcord-quick-css/);
});

test("no src caller is migrated to a native Quick CSS path in this milestone", () => {
    // Static startup injection is host-driven: no renderer caller changes.
    assert.doesNotMatch(power, /legcord-quick-css/);
    assert.doesNotMatch(shelterSettings, /legcord-quick-css/);
    assert.doesNotMatch(heroUpdater, /legcord-quick-css/);
    assert.doesNotMatch(invite, /legcord-quick-css/);
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

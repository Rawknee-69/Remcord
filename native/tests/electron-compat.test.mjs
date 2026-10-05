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

const shelterPreload = readFileSync(new URL("../../src/discord/preload/mods/shelter.mts", import.meta.url), "utf8");
const equicordPreload = readFileSync(new URL("../../src/discord/preload/mods/equicord.mts", import.meta.url), "utf8");
const consts = readFileSync(new URL("../../src/common/consts.ts", import.meta.url), "utf8");

test("Electron keeps the Shelter preload wrapper and its injector plugin map", () => {
    // Shelter's Electron path wraps the bundle in a host-built IIFE that
    // declares SHELTER_INJECTOR_PLUGINS and adds a macOS-only Touch Bar entry.
    // The native milestone deliberately does not reproduce this wrapper: the
    // legcord:// protocol and preload plugin loader are unimplemented natively,
    // and Shelter skips the injector entirely when the global is absent. The
    // Electron file must therefore stay byte-identical.
    assert.match(
        shelterPreload,
        /const requiredPlugins: Record<string, \[string, \{ isVisible: boolean; allowedActions: Record<string, true> \}\]> = \{/,
    );
    assert.match(shelterPreload, /"legcord-arrpc": \["legcord:\/\/plugins\/rpc\/"/);
    assert.match(shelterPreload, /"legcord-settings": \["legcord:\/\/plugins\/settings\/"/);
    assert.match(shelterPreload, /"legcord-power": \["legcord:\/\/plugins\/power\/"/);
    assert.match(shelterPreload, /"legcord-screenshare": \["legcord:\/\/plugins\/screenshare\/"/);
    assert.match(shelterPreload, /"legcord-titlebar": \["legcord:\/\/plugins\/titlebar\/"/);
    // Platform-specific behavior: Touch Bar is Darwin-only.
    assert.match(shelterPreload, /if \(process\.platform === "darwin"\)/);
    assert.match(shelterPreload, /"legcord-touchbar"/);
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
    for (const name of ["shelter", "equicord"]) {
        const adapter = strip(readFileSync(new URL(`../web/mods/${name}/adapter.js`, import.meta.url), "utf8"));
        // Neither adapter reaches for an Electron/Node global.
        assert.doesNotMatch(adapter, /require\(|process\.|webFrame|ipcRenderer|contextBridge/, name);
        // The only host call is the shared logical-id bundle request.
        assert.match(adapter, /native\.getModBundle\("[a-z]+"\)/, name);
        assert.doesNotMatch(adapter, /readFile|writeFile|loadScript|executeSource|executeScript/, name);
        assert.doesNotMatch(adapter, /eval\(|new Function/, name);
        // And it names no URL beyond the exact origin gate.
        assert.deepEqual(adapter.match(/https?:\/\/[^\s"'`)]+/g) ?? [], ["https://discord.com"], name);
    }
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
    assert.match(backupSection, /window\.legcord\.backup\.save\(JSON\.stringify\(payload\)\)/);
    assert.match(backupSection, /window\.legcord\.backup\.restore\(\)/);
});

test("custom bundles stay inventory-only and non-executable natively", () => {
    // No authoritative host-controlled source exists for custom artifacts,
    // so presence in the inventory must never imply servability: the mod
    // registry serves only registered host-controlled descriptors, and the
    // discovery module documents the restriction.
    const nativeBundles = readFileSync(new URL("../src/bundles.rs", import.meta.url), "utf8");
    const main = readFileSync(new URL("../src/main.rs", import.meta.url), "utf8");
    assert.match(nativeBundles, /inventory-only and non-executable/);
    // No descriptor registers the custom id, so inventory presence can
    // never become execution (the pre-existing "custom" settings-gate test
    // below only proves an unregistered id enables nothing).
    assert.doesNotMatch(main, /id: "custom"/);
    // The native facade exposes discovery only: no save, restore, zip,
    // dialog, or bundle-content member may exist alongside it.
    assert.match(nativeFacade, /function listBundles\(/);
    assert.doesNotMatch(nativeFacade, /backupSave/);
    assert.doesNotMatch(nativeFacade, /backupRestore/);
    assert.doesNotMatch(nativeFacade, /showSaveDialog/);
    assert.doesNotMatch(nativeFacade, /AdmZip|readBackupZipToMap|buildBackupZipBuffer/);
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
        assert.doesNotMatch(source, /legcord\.folders/);
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
    assert.match(themesCard, /window\.legcord\.themes\.folder\(props\.theme\.id\)/);
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
    assert.match(themesMain, /manifest\.supportsLegcordTitlebar = (true|false);/);
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
    assert.match(themesPage, /window\.legcord\.themes\.install\(downloadUrl\(\)\)/);
    assert.match(themesCard, /window\.legcord\.themes\.install\(props\.theme\.updateSrc\)/);
    assert.match(themesCard, /window\.legcord\.themes\.edit\(props\.theme\.id\)/);
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
        nativeFacade.indexOf("window.addEventListener(\"legcord-native:response\""),
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
    assert.match(themesCard, /window\.legcord\.themes\.edit\(props\.theme\.id\)/);
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
        nativeFacade.indexOf('window.addEventListener("legcord-native:response"'),
    );
    assert.ok(writeFacade.length > 0);
    assert.match(writeFacade, /requireCapability\("themes\.write_css"\)/);
    assert.match(writeFacade, /\{\s*type:\s*"theme_write_css",\s*payload:\s*\{\s*id,\s*css\s*\}\s*\}/);
    assert.doesNotMatch(writeFacade, /fetch\(|installBDTheme|showOpenDialog|openPath/);
});

test("settings UI reveals the storage folder natively and sends otherwise", () => {
    const settingsPage = readFileSync(
        new URL("../../src/shelter/settings/pages/SettingsPage.tsx", import.meta.url),
        "utf8",
    );
    // Native branch calls the typed capability with the exact location;
    // Electron branch keeps the original fire-and-forget send.
    assert.match(settingsPage, /revealFolderNative\("storage"\)/);
    assert.match(settingsPage, /window\.legcord\.settings\.openStorageFolder\(\)/);
    // Exactly one dispatch: native success short-circuits the Electron call.
    assert.equal((settingsPage.match(/revealFolderNative\("storage"\)/g) ?? []).length, 1);
});

test("themes UI reveals the themes folder natively and sends otherwise", () => {
    const themesPage = readFileSync(
        new URL("../../src/shelter/settings/pages/ThemesPage.tsx", import.meta.url),
        "utf8",
    );
    assert.match(themesPage, /revealFolderNative\("themes"\)/);
    assert.match(themesPage, /window\.legcord\.settings\.openThemesFolder\(\)/);
    assert.equal((themesPage.match(/revealFolderNative\("themes"\)/g) ?? []).length, 1);
});

test("plugins UI reveals the plugins folder natively and sends otherwise", () => {
    const pluginsPage = readFileSync(
        new URL("../../src/shelter/settings/pages/PluginsPage.tsx", import.meta.url),
        "utf8",
    );
    assert.match(pluginsPage, /revealFolderNative\("plugins"\)/);
    assert.match(pluginsPage, /window\.legcord\.plugins\.openFolder\(\)/);
    // Both the toolbar button and the empty-state action dispatch once.
    assert.equal((pluginsPage.match(/revealFolderNative\("plugins"\)/g) ?? []).length, 2);
});

test("theme cards open the theme folder natively with the verbatim id", () => {
    const themesCard = readFileSync(
        new URL("../../src/shelter/settings/components/ThemesCard.tsx", import.meta.url),
        "utf8",
    );
    assert.match(themesCard, /openThemeFolderNative\(props\.theme\.id\)/);
    assert.match(themesCard, /window\.legcord\.themes\.folder\(props\.theme\.id\)/);
    // No renderer-side id filtering: validation is owned by the host.
    assert.doesNotMatch(themesCard, /RegExp|new RegExp|\.test\(props\.theme\.id\)|\.match\(props\.theme\.id\)/);
});

test("folder UI wiring introduces no generic filesystem surface", () => {
    const settingsHelper = readFileSync(
        new URL("../../src/shelter/settings/settings.ts", import.meta.url),
        "utf8",
    );
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

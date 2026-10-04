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

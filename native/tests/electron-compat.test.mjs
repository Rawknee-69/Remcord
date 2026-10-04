// Regression guard for Phase C: the Electron synchronous settings path must
// remain byte-for-byte synchronous. The native WebView2 capability is an
// additional async path (Shelter power plugin `isNativeWebView()` branch) and
// must never rewrite these Electron contracts.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const bridge = readFileSync(new URL("../../src/discord/preload/bridge.ts", import.meta.url), "utf8");
const ipc = readFileSync(new URL("../../src/discord/ipc.ts", import.meta.url), "utf8");
const power = readFileSync(new URL("../../src/shelter/power/index.ts", import.meta.url), "utf8");
const types = readFileSync(new URL("../../src/@types/legcordWindow.d.ts", import.meta.url), "utf8");

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

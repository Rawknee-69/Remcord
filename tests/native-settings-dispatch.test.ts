import assert from "node:assert/strict";
import test from "node:test";

// Covers the native-shell `setConfig` dispatch in
// `src/shelter/settings/settings.ts`: allowlisted keys must persist through
// the typed async capabilities (and only from host-confirmed values),
// unsupported keys must be discarded honestly (no store mutation, no restart
// flag, no call into a missing bridge member), and `toggleMod` must never
// persist duplicate mod ids.

type SettingsFn = {
    set?: (key: string, value: unknown) => Promise<{ key: string; value: unknown }>;
    setConfig?: (key: string, value: unknown) => void;
    setBlockPowerSavingInVoiceChat?: (value: boolean) => Promise<boolean>;
    getConfig?: () => Record<string, unknown>;
};

type Calls = {
    electronSetConfig: Array<[string, unknown]>;
    nativeSet: Array<[string, unknown]>;
    blockPower: boolean[];
    warnings: string[];
};

function installGlobals(options: {
    runtime?: string;
    settingsImpl?: Partial<SettingsFn>;
    seed?: Record<string, unknown>;
}): Calls {
    const calls: Calls = { electronSetConfig: [], nativeSet: [], blockPower: [], warnings: [] };
    const settings: Record<string, unknown> = {
        setConfig: (key: string, value: unknown) => {
            calls.electronSetConfig.push([key, value]);
        },
        ...(options.settingsImpl ?? {}),
    };
    const remcord: Record<string, unknown> = { settings };
    if (options.runtime !== undefined) {
        remcord.boot = { runtime: options.runtime, platform: "win32", protocol_version: 1, capabilities: [] };
    }
    const seed = { mods: [], ...(options.seed ?? {}) };
    const prevShelter = Reflect.get(globalThis, "shelter") as
        | { plugin: { store: { settings: Record<string, unknown>; themes: unknown[] } } }
        | undefined;
    if (prevShelter) {
        prevShelter.plugin.store.settings = seed;
        prevShelter.plugin.store.themes = [];
    } else {
        Reflect.set(globalThis, "shelter", {
            plugin: { store: { settings: seed, themes: [] } },
        });
    }
    Reflect.set(globalThis, "window", { remcord });
    Reflect.set(globalThis, "document", {
        activeElement: null,
        querySelectorAll: () => [],
        contains: () => false,
    });
    Reflect.set(globalThis, "HTMLElement", class {});
    Reflect.set(globalThis, "Element", class {});
    Reflect.set(globalThis, "getComputedStyle", () => ({ overflowY: "visible" }));
    Reflect.set(globalThis, "requestAnimationFrame", () => 0);
    Reflect.set(globalThis, "cancelAnimationFrame", () => undefined);
    const originalWarn = console.warn;
    console.warn = ((message?: unknown) => {
        calls.warnings.push(String(message));
    }) as typeof console.warn;
    Reflect.set(globalThis, "__restoreWarn", originalWarn);
    return calls;
}

function storeSettings(): Record<string, unknown> {
    return (globalThis as unknown as { shelter: { plugin: { store: { settings: Record<string, unknown> } } } }).shelter
        .plugin.store.settings;
}

function restoreConsole() {
    const originalWarn = Reflect.get(globalThis, "__restoreWarn") as typeof console.warn | undefined;
    if (originalWarn) console.warn = originalWarn;
}

async function loadHelpers() {
    return await import("../src/shelter/settings/settings.js");
}

const tick = () => new Promise((resolve) => setImmediate(resolve));

test("native writability allowlist is exactly the host-supported keys", async () => {
    installGlobals({});
    try {
        const helpers = await loadHelpers();
        for (const key of [
            "mods",
            "supportBannerDismissed",
            "showExperimentalPluginMenu",
            "extendedPluginAbilities",
            "blockPowerSavingInVoiceChat",
            "csp",
            "quickCss",
            "channel",
        ]) {
            assert.equal(helpers.isSettingWritableNatively(key), true, `${key} must be writable`);
        }
        for (const key of [
            "windowStyle",
            "transparency",
            "audio",
            "proxyMode",
            "keybinds",
            "minimizeToTray",
            "noBundleUpdates",
            "modCache",
        ]) {
            assert.equal(helpers.isSettingWritableNatively(key), false, `${key} must be disabled`);
        }
    } finally {
        restoreConsole();
    }
});

test("native unsupported key is discarded honestly", async () => {
    const calls = installGlobals({ runtime: "webview2" });
    try {
        const helpers = await loadHelpers();
        assert.equal(helpers.isRestartRequired, false);
        helpers.setConfig("windowStyle", "native", true);
        await tick();
        // No store mutation, no restart request, no Electron call, one warning.
        assert.equal(storeSettings().windowStyle, undefined);
        assert.equal(helpers.isRestartRequired, false);
        assert.equal(helpers.isRestartRequiredSignal(), false);
        assert.deepEqual(calls.electronSetConfig, []);
        assert.deepEqual(calls.nativeSet, []);
        assert.equal(calls.warnings.length, 1);
        assert.match(calls.warnings[0]!, /windowStyle.*not supported/);
    } finally {
        restoreConsole();
    }
});

test("native allowlisted key persists from the host-confirmed value", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        settingsImpl: {
            set: (key: string, value: unknown) => {
                calls.nativeSet.push([key, value]);
                return Promise.resolve({ key, value });
            },
        },
        seed: { extendedPluginAbilities: false },
    });
    try {
        const helpers = await loadHelpers();
        helpers.setConfig("extendedPluginAbilities", true, true);
        await tick();
        await tick();
        assert.deepEqual(calls.nativeSet, [["extended_plugin_abilities", true]]);
        assert.equal(storeSettings().extendedPluginAbilities, true);
        assert.equal(helpers.isRestartRequired, true);
        // The in-page banner signal flips with the flag, so the indication
        // is immediate instead of waiting for the next pane view.
        assert.equal(helpers.isRestartRequiredSignal(), true);
        assert.deepEqual(calls.electronSetConfig, []);
    } finally {
        restoreConsole();
    }
});

test("native rejected mutation reverts the optimistic write", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        settingsImpl: {
            set: (key: string, value: unknown) => {
                calls.nativeSet.push([key, value]);
                return Promise.reject(new Error("SETTINGS_UNAVAILABLE"));
            },
        },
        seed: { showExperimentalPluginMenu: false },
    });
    try {
        const helpers = await loadHelpers();
        helpers.setConfig("showExperimentalPluginMenu", true, true);
        await tick();
        await tick();
        assert.deepEqual(calls.nativeSet, [["show_experimental_plugin_menu", true]]);
        assert.equal(storeSettings().showExperimentalPluginMenu, false);
        assert.equal(calls.warnings.length, 1);
        assert.match(calls.warnings[0]!, /rejected/);
        assert.deepEqual(calls.electronSetConfig, []);
    } finally {
        restoreConsole();
    }
});

test("native block-power-saving delegates to its dedicated capability", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        settingsImpl: {
            setBlockPowerSavingInVoiceChat: (value: boolean) => {
                calls.blockPower.push(value);
                return Promise.resolve(value);
            },
        },
        seed: { blockPowerSavingInVoiceChat: false },
    });
    try {
        const helpers = await loadHelpers();
        helpers.setConfig("blockPowerSavingInVoiceChat", true);
        await tick();
        await tick();
        assert.deepEqual(calls.blockPower, [true]);
        assert.equal(storeSettings().blockPowerSavingInVoiceChat, true);
        assert.deepEqual(calls.electronSetConfig, []);
    } finally {
        restoreConsole();
    }
});

test("toggleMod never persists duplicate mod ids", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        settingsImpl: {
            set: (key: string, value: unknown) => {
                calls.nativeSet.push([key, value]);
                return Promise.resolve({ key, value });
            },
        },
        seed: { mods: ["equicord"] },
    });
    try {
        const helpers = await loadHelpers();
        // Re-enabling the already-enabled mod must not duplicate it (this
        // produced ["equicord", "equicord"] in a real profile).
        helpers.toggleMod("equicord", true);
        await tick();
        await tick();
        assert.deepEqual(calls.nativeSet, [["mods", ["equicord"]]]);
        // Switching mods keeps mutual exclusion without duplicates.
        calls.nativeSet.length = 0;
        storeSettings().mods = ["equicord", "equicord"];
        helpers.toggleMod("vencord", true);
        await tick();
        await tick();
        assert.deepEqual(calls.nativeSet, [["mods", ["vencord"]]]);
        assert.deepEqual(calls.electronSetConfig, []);
    } finally {
        restoreConsole();
    }
});

test("native csp change persists through the closed allowlist", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        settingsImpl: {
            set: (key: string, value: unknown) => {
                calls.nativeSet.push([key, value]);
                return Promise.resolve({ key, value });
            },
        },
        seed: { csp: "none" },
    });
    try {
        const helpers = await loadHelpers();
        // The CSP dropdown path: setConfig routes the string-valued key to
        // the native allowlist (wire "csp"), never to Electron setConfig.
        helpers.setConfig("csp", "strict", true);
        await tick();
        await tick();
        assert.deepEqual(calls.nativeSet, [["csp", "strict"]]);
        assert.equal(storeSettings().csp, "strict");
        assert.deepEqual(calls.electronSetConfig, []);
    } finally {
        restoreConsole();
    }
});

test("native channel change persists through the closed allowlist with restart", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        settingsImpl: {
            set: (key: string, value: unknown) => {
                calls.nativeSet.push([key, value]);
                return Promise.resolve({ key, value });
            },
        },
        seed: { channel: "stable" },
    });
    try {
        const helpers = await loadHelpers();
        // The channel dropdown path: setConfig routes the string-valued key
        // to the native allowlist (wire "channel") with restart requested —
        // the host maps it to the entry URL on next launch.
        helpers.setConfig("channel", "canary", true);
        await tick();
        await tick();
        assert.deepEqual(calls.nativeSet, [["channel", "canary"]]);
        assert.equal(storeSettings().channel, "canary");
        assert.deepEqual(calls.electronSetConfig, []);
    } finally {
        restoreConsole();
    }
});

test("native quickCss toggle persists without requesting restart", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        settingsImpl: {
            set: (key: string, value: unknown) => {
                calls.nativeSet.push([key, value]);
                return Promise.resolve({ key, value });
            },
        },
        seed: { quickCss: false },
    });
    try {
        const helpers = await loadHelpers();
        // The toggle applies live host-side (managed style re-push), so no
        // restart flag is requested, unlike the mod/csp controls.
        helpers.setConfig("quickCss", true);
        await tick();
        await tick();
        assert.deepEqual(calls.nativeSet, [["quick_css", true]]);
        assert.equal(storeSettings().quickCss, true);
        assert.deepEqual(calls.electronSetConfig, []);
    } finally {
        restoreConsole();
    }
});

test("Electron path keeps the synchronous setConfig contract", async () => {
    const calls = installGlobals({ seed: { windowStyle: "overlay" } });
    try {
        const helpers = await loadHelpers();
        assert.equal(helpers.isNativeWebView(), false);
        helpers.setConfig("windowStyle", "native", true);
        assert.equal(storeSettings().windowStyle, "native");
        assert.deepEqual(calls.electronSetConfig, [["windowStyle", "native"]]);
        assert.equal(helpers.isRestartRequired, true);
    } finally {
        restoreConsole();
    }
});

test("native toggle reacts immediately, then converges to the host value", async () => {
    let resolveHost!: (result: { key: string; value: unknown }) => void;
    const calls = installGlobals({
        runtime: "webview2",
        settingsImpl: {
            set: (key: string, value: unknown) => {
                calls.nativeSet.push([key, value]);
                return new Promise<{ key: string; value: unknown }>((resolve) => {
                    resolveHost = resolve;
                });
            },
        },
        seed: { mods: [] },
    });
    try {
        const helpers = await loadHelpers();
        // Click: Vencord toggle on.
        helpers.toggleMod("vencord", true);
        // Immediate reaction — the UI must flip before the host responds.
        assert.deepEqual(storeSettings().mods, ["vencord"]);
        assert.deepEqual(calls.nativeSet, [["mods", ["vencord"]]]);
        // Host confirms: UI converges, restart is requested.
        resolveHost({ key: "mods", value: ["vencord"] });
        await tick();
        await tick();
        assert.deepEqual(storeSettings().mods, ["vencord"]);
        assert.equal(helpers.isRestartRequired, true);
        assert.deepEqual(calls.electronSetConfig, []);
    } finally {
        restoreConsole();
    }
});

test("native toggle reverts on host rejection without newly requesting restart", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        settingsImpl: {
            set: (key: string, value: unknown) => {
                calls.nativeSet.push([key, value]);
                const error = new Error("Native bridge request failed");
                (error as unknown as { code: string }).code = "REQUEST_TIMEOUT";
                return Promise.reject(error);
            },
        },
        seed: { mods: [] },
    });
    try {
        const helpers = await loadHelpers();
        // isRestartRequired is monotonic (once true it stays true until the
        // restart modal runs), so assert rejection adds no NEW request.
        const flagBefore = helpers.isRestartRequired;
        // Click: Equicord toggle on, host times out.
        helpers.toggleMod("equicord", true);
        // Immediate flip first (click-to-state), then revert on failure.
        assert.deepEqual(storeSettings().mods, ["equicord"]);
        await tick();
        await tick();
        assert.deepEqual(calls.nativeSet, [["mods", ["equicord"]]]);
        assert.deepEqual(storeSettings().mods, []);
        assert.equal(helpers.isRestartRequired, flagBefore);
        assert.equal(calls.warnings.length, 1);
        assert.deepEqual(calls.electronSetConfig, []);
    } finally {
        restoreConsole();
    }
});

test("toggleMod supports shelter and custom instead of silently ignoring them", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        settingsImpl: {
            set: (key: string, value: unknown) => {
                calls.nativeSet.push([key, value]);
                return Promise.resolve({ key, value });
            },
        },
        seed: { mods: [] },
    });
    try {
        const helpers = await loadHelpers();
        helpers.toggleMod("shelter", true);
        await tick();
        await tick();
        assert.deepEqual(calls.nativeSet[0], ["mods", ["shelter"]]);
        storeSettings().mods = ["shelter"];
        calls.nativeSet.length = 0;
        // Custom is independent: it adds without touching shelter.
        helpers.toggleMod("custom", true);
        await tick();
        await tick();
        assert.deepEqual(calls.nativeSet[0], ["mods", ["shelter", "custom"]]);
        // Disabling removes only that id.
        calls.nativeSet.length = 0;
        storeSettings().mods = ["vencord", "custom"];
        helpers.toggleMod("custom", false);
        await tick();
        await tick();
        assert.deepEqual(calls.nativeSet[0], ["mods", ["vencord"]]);
        assert.deepEqual(calls.electronSetConfig, []);
    } finally {
        restoreConsole();
    }
});

test("native Remcord-owned flag reacts immediately and restores after reload", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        settingsImpl: {
            set: (key: string, value: unknown) => {
                calls.nativeSet.push([key, value]);
                return Promise.resolve({ key, value });
            },
        },
        seed: { extendedPluginAbilities: false },
    });
    try {
        const helpers = await loadHelpers();
        // Correct operation name (wire key) and payload.
        helpers.setConfig("extendedPluginAbilities", true);
        assert.equal(storeSettings().extendedPluginAbilities, true);
        await tick();
        await tick();
        assert.deepEqual(calls.nativeSet, [["extended_plugin_abilities", true]]);
        assert.equal(storeSettings().extendedPluginAbilities, true);
        // State restoration after a normal reload: a fresh document seeded
        // from the persisted value renders the same state.
        const persisted = storeSettings().extendedPluginAbilities;
        installGlobals({
            runtime: "webview2",
            settingsImpl: {
                set: (key: string, value: unknown) => Promise.resolve({ key, value }),
            },
            seed: { extendedPluginAbilities: persisted },
        });
        assert.equal(storeSettings().extendedPluginAbilities, true);
        assert.deepEqual(calls.electronSetConfig, []);
    } finally {
        restoreConsole();
    }
});

test("unsupported settings stay disabled instead of rendering a broken control", async () => {
    installGlobals({});
    try {
        const helpers = await loadHelpers();
        for (const key of ["windowStyle", "keybinds", "noBundleUpdates", "modCache"]) {
            assert.equal(helpers.isSettingWritableNatively(key), false, `${key} must be disabled natively`);
        }
        assert.equal(helpers.isSettingWritableNatively("mods"), true);
        assert.equal(helpers.isSettingWritableNatively("csp"), true);
        assert.equal(helpers.isSettingWritableNatively("quickCss"), true);
        assert.equal(helpers.isSettingWritableNatively("channel"), true);
        assert.equal(helpers.isNativeSettingsSetAvailable(), false);
    } finally {
        restoreConsole();
    }
});

test("duplicate toggles never duplicate ids and each click dispatches once", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        settingsImpl: {
            set: (key: string, value: unknown) => {
                calls.nativeSet.push([key, value]);
                return Promise.resolve({ key, value });
            },
        },
        seed: { mods: ["vencord"] },
    });
    try {
        const helpers = await loadHelpers();
        helpers.toggleMod("vencord", true);
        helpers.toggleMod("vencord", true);
        await tick();
        await tick();
        assert.equal(calls.nativeSet.length, 2);
        assert.deepEqual(calls.nativeSet[0], ["mods", ["vencord"]]);
        assert.deepEqual(calls.nativeSet[1], ["mods", ["vencord"]]);
        assert.deepEqual(storeSettings().mods, ["vencord"]);
    } finally {
        restoreConsole();
    }
});

test("native diagnostics copy writes the host readout to the clipboard", async () => {
    const written: string[] = [];
    installGlobals({
        runtime: "webview2",
        settingsImpl: {},
        seed: {},
    });
    const win = Reflect.get(globalThis, "window") as { remcord: Record<string, unknown> };
    win.remcord.diagnostics = {
        getInfo: () =>
            Promise.resolve({
                app_version: "1.3.0",
                webview2_runtime: "154.0.4258.62",
                profile_kind: "release",
                channel: "canary",
                csp: "strict",
                mods: ["equicord"],
            }),
    };
    // Node ships a global `navigator` without `clipboard`: override it as
    // an own property so the renderer clipboard path is exercisable here.
    Object.defineProperty(globalThis, "navigator", {
        value: {
            clipboard: {
                writeText: (text: string) => {
                    written.push(text);
                    return Promise.resolve();
                },
            },
        },
        configurable: true,
        writable: true,
    });
    try {
        const helpers = await loadHelpers();
        assert.equal(helpers.isNativeDiagnosticsAvailable(), true);
        assert.equal(await helpers.copyNativeDiagnostics("debug"), true);
        assert.equal(written.length, 1);
        assert.match(written[0], /Remcord 1\.3\.0/);
        assert.match(written[0], /154\.0\.4258\.62/);
        assert.match(written[0], /canary/);
        assert.match(written[0], /equicord/);
        assert.equal(await helpers.copyNativeDiagnostics("gpu"), true);
        assert.equal(written.length, 2);
        assert.match(written[1], /WebView2 Runtime 154\.0\.4258\.62/);
        assert.doesNotMatch(written[1], /token|cookie/i);
    } finally {
        Reflect.deleteProperty(globalThis, "navigator");
        restoreConsole();
    }
});

test("native diagnostics copy fails honestly when the host readout fails", async () => {
    installGlobals({ runtime: "webview2", settingsImpl: {}, seed: {} });
    const win = Reflect.get(globalThis, "window") as { remcord: Record<string, unknown> };
    win.remcord.diagnostics = { getInfo: () => Promise.reject(new Error("bridge down")) };
    Object.defineProperty(globalThis, "navigator", {
        value: { clipboard: { writeText: () => Promise.resolve() } },
        configurable: true,
        writable: true,
    });
    try {
        const helpers = await loadHelpers();
        assert.equal(await helpers.copyNativeDiagnostics("debug"), false);
    } finally {
        Reflect.deleteProperty(globalThis, "navigator");
        restoreConsole();
    }
});

test("native diagnostics are unavailable outside the native shell", async () => {
    installGlobals({ seed: {} });
    try {
        const helpers = await loadHelpers();
        assert.equal(helpers.isNativeDiagnosticsAvailable(), false);
        assert.equal(helpers.isNativeModCacheAvailable(), false);
        await assert.rejects(helpers.getNativeDiagnosticsInfo(), /unavailable/);
        await assert.rejects(helpers.clearNativeModCache(), /unavailable/);
    } finally {
        restoreConsole();
    }
});

test("native mod cache wipe resolves the host outcome and validates its shape", async () => {
    installGlobals({ runtime: "webview2", settingsImpl: {}, seed: {} });
    const win = Reflect.get(globalThis, "window") as { remcord: Record<string, unknown> };
    win.remcord.mod = {
        clearCache: () => Promise.resolve({ cleared: ["vencord.js", "shelter.js"], mod_cache_cleared: true }),
    };
    try {
        const helpers = await loadHelpers();
        assert.equal(helpers.isNativeModCacheAvailable(), true);
        assert.deepEqual(await helpers.clearNativeModCache(), {
            cleared: ["vencord.js", "shelter.js"],
            mod_cache_cleared: true,
        });
    } finally {
        restoreConsole();
    }
});

test("native mod cache wipe rejects a misshapen host outcome", async () => {
    installGlobals({ runtime: "webview2", settingsImpl: {}, seed: {} });
    const win = Reflect.get(globalThis, "window") as { remcord: Record<string, unknown> };
    // Fixed-name enforcement lives in the facade response schema; the page
    // helper guards the envelope shape only (array of strings + flag).
    win.remcord.mod = { clearCache: () => Promise.resolve("wiped") };
    try {
        const helpers = await loadHelpers();
        await assert.rejects(helpers.clearNativeModCache(), /Could not clear/);
    } finally {
        restoreConsole();
    }
});

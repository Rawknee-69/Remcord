import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const bootstrapTemplate = readFileSync(new URL("../web/compatibility.js", import.meta.url), "utf8");
const modSubstrate = readFileSync(new URL("../web/mod-substrate/mod-bootstrap.js", import.meta.url), "utf8");
const storagePreservation = readFileSync(
    new URL("../web/mod-substrate/storage-preservation.js", import.meta.url),
    "utf8",
);
const vencordAdapter = readFileSync(new URL("../web/mods/vencord/adapter.js", import.meta.url), "utf8");
const shelterAdapter = readFileSync(new URL("../web/mods/shelter/adapter.js", import.meta.url), "utf8");
const equicordAdapter = readFileSync(new URL("../web/mods/equicord/adapter.js", import.meta.url), "utf8");
const customAdapter = readFileSync(new URL("../web/mods/custom/adapter.js", import.meta.url), "utf8");
const keybindMatcher = readFileSync(new URL("../web/keybinds/matcher.js", import.meta.url), "utf8");
const pluginLoader = readFileSync(new URL("../web/plugins/loader.js", import.meta.url), "utf8");
const testModAdapter = readFileSync(new URL("../web/mods/testmod/adapter.js", import.meta.url), "utf8");

/**
 * Strips comments from a JS source file so security assertions inspect only
 * executable/configuration content.
 *
 * Adapters legitimately DOCUMENT what they deliberately do not do (e.g.
 * "Shelter ships no stylesheet"), and a bare regex over the whole file would
 * flag those comments. Stripping comments keeps the assertion meaningful — it
 * still fails on any real URL, path, or eval surface in code — without
 * weakening it.
 */
function stripComments(source) {
    let out = "";
    let index = 0;
    let inString = null;
    while (index < source.length) {
        const char = source[index];
        const next = source[index + 1];
        if (inString) {
            out += char;
            if (char === "\\") {
                out += next ?? "";
                index += 2;
                continue;
            }
            if (char === inString) inString = null;
            index += 1;
            continue;
        }
        if (char === '"' || char === "'" || char === "`") {
            inString = char;
            out += char;
            index += 1;
            continue;
        }
        if (char === "/" && next === "/") {
            while (index < source.length && source[index] !== "\n") index += 1;
            continue;
        }
        if (char === "/" && next === "*") {
            index += 2;
            while (index < source.length && !(source[index] === "*" && source[index + 1] === "/")) {
                index += 1;
            }
            index += 2;
            continue;
        }
        out += char;
        index += 1;
    }
    return out;
}

/** Executable/configuration content only, with comments removed. */
const shelterAdapterCode = stripComments(shelterAdapter);
const equicordAdapterCode = stripComments(equicordAdapter);
const customAdapterCode = stripComments(customAdapter);

function createBootstrap(capabilities, settingsBoot = defaultSettingsBoot(), hostPlatform = defaultHostPlatform()) {
    return bootstrapTemplate
        .replace(
            "__REMCORD_BOOT_SNAPSHOT__",
            JSON.stringify({
                runtime: "webview2",
                platform: "win32",
                protocol_version: 1,
                capabilities,
            }),
        )
        .replace("__REMCORD_SETTINGS_BOOT_SNAPSHOT__", JSON.stringify(settingsBoot))
        .replace("__REMCORD_HOST_PLATFORM__", JSON.stringify(hostPlatform));
}

/** Minimal host platform fixture, mirroring the host builder. */
function defaultHostPlatform() {
    return { platform: "win32", osRelease: "10.0.22631" };
}

/** Minimal schema-valid settings boot snapshot, mirroring the host builder. */
function defaultSettingsBoot() {
    return {
        settingsSource: "defaults",
        settings: { mods: [], blockPowerSavingInVoiceChat: false },
        themes: [],
        lang: "en-US",
        i18n: { "invite-goBackToApp": "Go back to the Discord app" },
    };
}

class FakeCustomEvent {
    constructor(type, options = {}) {
        this.type = type;
        this.detail = options.detail;
    }
}

class FakeWindow {
    constructor(origin = "https://discord.com") {
        this.location = { origin };
        this.listeners = new Map();
        this.ipc = {
            postMessage: (message) => this.messages.push(JSON.parse(message)),
        };
        this.messages = [];
    }

    addEventListener(type, listener, options = {}) {
        const listeners = this.listeners.get(type) ?? [];
        listeners.push({ listener, once: options.once === true });
        this.listeners.set(type, listeners);
    }

    dispatchEvent(event) {
        const listeners = this.listeners.get(event.type) ?? [];
        this.listeners.set(
            event.type,
            listeners.filter(({ once }) => !once),
        );
        for (const { listener } of listeners) listener(event);
        return true;
    }
}

function createRuntime(origin = "https://discord.com", capabilities = ["bridge.ping", "mod.get_bundle"], settingsBoot) {
    const window = new FakeWindow(origin);
    const context = vm.createContext({
        window,
        performance,
        TextEncoder,
        console: { error() {}, info() {} },
        CustomEvent: FakeCustomEvent,
        setTimeout,
        clearTimeout,
        Date,
        JSON,
        Object,
        Map,
        Set,
        Promise,
        Error,
        TypeError,
        String,
    });
    vm.runInContext(createBootstrap(capabilities, settingsBoot), context);
    vm.runInContext(modSubstrate, context);
    return { window, context };
}

/**
 * Minimal runtime for the synthetic second mod. It uses a hand-built facade
 * double so the test adapter can exercise the factory without a real
 * TestMod host capability or touching Discord state.
 */
function createSyntheticModRuntime(capabilities = ["mod.test.get_bundle"]) {
    const window = new FakeWindow("https://discord.com");
    let hostResolve;
    const context = vm.createContext({
        window,
        performance,
        TextEncoder,
        console: { error() {}, info() {} },
        CustomEvent: FakeCustomEvent,
        setTimeout,
        clearTimeout,
        Date,
        JSON,
        Object,
        Map,
        Set,
        Promise,
        Error,
        TypeError,
        String,
    });
    window.remcord = {
        native: {
            requireCapability(name) {
                if (!capabilities.includes(name)) {
                    const error = new Error(`Native capability is not available: ${name}`);
                    error.code = "UNSUPPORTED_CAPABILITY";
                    throw error;
                }
                return true;
            },
            getTestModBundle() {
                const pending = new Promise((resolve, reject) => {
                    hostResolve = { resolve, reject };
                });
                Object.defineProperty(pending, "requestId", { value: "testmod-1" });
                return pending;
            },
        },
    };
    vm.runInContext(modSubstrate, context);
    vm.runInContext(testModAdapter, context);
    return { window, context, host: () => hostResolve };
}

function respondPing(window, request, payload) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "ping", payload },
                error: null,
            },
        }),
    );
}

function respondVencord(window, request, payload = {}) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: {
                    type: "mod_bundle",
                    payload: {
                        mod_id: "vencord",
                        bundle_version: "b52ed36",
                        bundle_bytes: 1000,
                        css_bytes: 250,
                        request_latency_ms: 20,
                        retrieval_ms: 10,
                        execution_ms: 3,
                        css_injection_ms: 2,
                        marker: "vencord-bootstrap-executed",
                        document_execution_count: 1,
                        ...payload,
                    },
                },
                error: null,
            },
        }),
    );
}

/** The per-mod wire identity each adapter must use, keyed by logical id. */
const MOD_WIRE = {
    vencord: {
        stateName: "__remcordNativeVencord",
        adapterFlag: "__remcordNativeVencordAdapter",
        domEvent: "remcord-native:vencord-bootstrap",
        marker: "vencord-bootstrap-executed",
        version: "b52ed36",
        // Legacy Vencord-named event family (byte-identical to Phase A).
        resultEvent: "vencord_bootstrap_result",
        ackEvent: "vencord_response_ack",
        errorEvent: "vencord_adapter_error",
        echoesModId: false,
        cssBytes: 250,
        cssInjectionMs: 2,
    },
    shelter: {
        stateName: "__remcordNativeShelter",
        adapterFlag: "__remcordNativeShelterAdapter",
        domEvent: "remcord-native:shelter-bootstrap",
        marker: "shelter-bootstrap-executed",
        // Header-less artifact: the host reports a content digest, not a banner.
        version: "sha256-ba858e9cfb83503f",
        resultEvent: "mod_bootstrap_result",
        ackEvent: "mod_response_ack",
        errorEvent: "mod_adapter_error",
        echoesModId: true,
        // Shelter ships no stylesheet: both CSS fields are null together.
        cssBytes: null,
        cssInjectionMs: null,
    },
    equicord: {
        stateName: "__remcordNativeEquicord",
        adapterFlag: "__remcordNativeEquicordAdapter",
        domEvent: "remcord-native:equicord-bootstrap",
        marker: "equicord-bootstrap-executed",
        version: "1489c0e2a4435d854d7343a5cfb0abb894772318",
        resultEvent: "mod_bootstrap_result",
        ackEvent: "mod_response_ack",
        errorEvent: "mod_adapter_error",
        echoesModId: true,
        cssBytes: 217092,
        cssInjectionMs: 5,
    },
    custom: {
        stateName: "__remcordNativeCustom",
        adapterFlag: "__remcordNativeCustomAdapter",
        domEvent: "remcord-native:custom-bootstrap",
        marker: "custom-bootstrap-executed",
        // User-owned bytes: the host reports a content digest, never a banner.
        version: "sha256-0123456789abcdef",
        resultEvent: "mod_bootstrap_result",
        ackEvent: "mod_response_ack",
        errorEvent: "mod_adapter_error",
        echoesModId: true,
        cssBytes: 512,
        cssInjectionMs: 2,
    },
};

function respondMod(window, request, modId, payload = {}) {
    const wire = MOD_WIRE[modId];
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: {
                    type: "mod_bundle",
                    payload: {
                        mod_id: modId,
                        bundle_version: wire.version,
                        bundle_bytes: 1000,
                        css_bytes: wire.cssBytes,
                        request_latency_ms: 20,
                        retrieval_ms: 10,
                        execution_ms: 3,
                        css_injection_ms: wire.cssInjectionMs,
                        marker: wire.marker,
                        document_execution_count: 1,
                        ...payload,
                    },
                },
                error: null,
            },
        }),
    );
}

/** Delivers a host bootstrap verdict on the mod's document-local DOM event. */
function dispatchBootstrap(window, request, modId, overrides = {}) {
    const wire = MOD_WIRE[modId];
    window.dispatchEvent(
        new FakeCustomEvent(wire.domEvent, {
            detail: {
                request_id: request.request_id,
                ok: true,
                marker: wire.marker,
                execution_ms: 3,
                local_storage_api: "available",
                settings_state: "missing",
                ...overrides,
            },
        }),
    );
}

function respondSettings(window, request, settings = {}, extra = {}) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: {
                    type: "settings_config",
                    payload: {
                        settings: {
                            blockPowerSavingInVoiceChat: false,
                            minimizeToTray: true,
                            windowStyle: "overlay",
                            transparency: "none",
                            mods: [],
                            audio: { granularSelect: true },
                            ...settings,
                        },
                        source: "file",
                        payload_bytes: 2048,
                        ...extra,
                    },
                },
                error: null,
            },
        }),
    );
}

function createSettingsRuntime(capabilities = ["bridge.ping", "settings.get_config"]) {
    return createRuntime("https://discord.com", capabilities);
}

test("the facade exposes an asynchronous settings.getConfig capability", () => {
    const { window } = createSettingsRuntime();
    assert.equal(typeof window.remcord.settings.getConfig, "function");
    assert.equal(Object.isFrozen(window.remcord.settings), true);
    assert.equal(window.remcord.native.supports("settings.get_config"), true);
});

test("settings.getConfig sends a parameter-free typed request and resolves the snapshot", async () => {
    const { window } = createSettingsRuntime();
    const pending = window.remcord.settings.getConfig();
    // Dual contract: a synchronous snapshot value that stays awaitable for
    // the live async capability (thenable, not a Promise instance).
    assert.equal(typeof pending?.then, "function");
    assert.equal(pending.blockPowerSavingInVoiceChat, false);
    const message = window.messages.find((entry) => entry.command?.type === "settings_get_config");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "settings_get_config", payload: {} });

    respondSettings(window, message, { blockPowerSavingInVoiceChat: true });
    const settings = await pending;
    assert.equal(settings.blockPowerSavingInVoiceChat, true);
    assert.equal(settings.windowStyle, "overlay");
});

test("settings.getConfig throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createSettingsRuntime(["bridge.ping"]);
    assert.equal(window.remcord.native.supports("settings.get_config"), false);
    assert.throws(
        () => window.remcord.settings.getConfig(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "settings_get_config"),
        false,
    );
});

test("settings.getConfig rejects malformed snapshots", async () => {
    const { window } = createSettingsRuntime();
    const pending = window.remcord.settings.getConfig();
    const message = window.messages.find((entry) => entry.command?.type === "settings_get_config");
    respondSettings(window, message, { blockPowerSavingInVoiceChat: "yes" });
    await assert.rejects(Promise.resolve(pending), /settings response schema is invalid/);
});

test("settings.getConfig rejects host failures without a snapshot", async () => {
    const { window } = createSettingsRuntime();
    const pending = window.remcord.settings.getConfig();
    const message = window.messages.find((entry) => entry.command?.type === "settings_get_config");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: false,
                payload: null,
                error: { code: "SETTINGS_UNAVAILABLE", message: "settings file is invalid" },
            },
        }),
    );
    await assert.rejects(Promise.resolve(pending), /settings file is invalid/);
});

test("settings.getConfig rejects after its timeout", async () => {
    const { window } = createSettingsRuntime();
    await assert.rejects(Promise.resolve(window.remcord.settings.getConfig(50)), /timed out/);
});

test("settings.getConfig rejects pending requests when the document is disposed", async () => {
    const { window } = createSettingsRuntime();
    const pending = window.remcord.settings.getConfig();
    const rejection = assert.rejects(Promise.resolve(pending), /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh settings facade with no stale promises", async () => {
    const firstDocument = createSettingsRuntime();
    const stale = firstDocument.window.remcord.settings.getConfig();
    const staleRejection = assert.rejects(Promise.resolve(stale), /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createSettingsRuntime();
    assert.notEqual(firstDocument.window.remcord, nextDocument.window.remcord);
    const pending = nextDocument.window.remcord.settings.getConfig();
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "settings_get_config");
    respondSettings(nextDocument.window, message, { blockPowerSavingInVoiceChat: false });
    assert.equal((await pending).blockPowerSavingInVoiceChat, false);
});

function createMutationRuntime(
    capabilities = ["bridge.ping", "settings.get_config", "settings.set_block_power_saving_in_voice_chat"],
) {
    return createRuntime("https://discord.com", capabilities);
}

function respondMutation(window, request, value = true, extra = {}) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: {
                    type: "settings_mutation",
                    payload: {
                        block_power_saving_in_voice_chat: value,
                        source: "file",
                        payload_bytes: 2048,
                        ...extra,
                    },
                },
                error: null,
            },
        }),
    );
}

test("the facade exposes an asynchronous block-power-saving setter and no generic writer", () => {
    const { window } = createMutationRuntime();
    assert.equal(typeof window.remcord.settings.setBlockPowerSavingInVoiceChat, "function");
    assert.equal(Object.isFrozen(window.remcord.settings), true);
    assert.equal(window.remcord.native.supports("settings.set_block_power_saving_in_voice_chat"), true);
    // No generic key/value RPC may exist.
    assert.equal(window.remcord.settings.setConfig, undefined);
});

test("the mutation setter sends a bool-only typed request and resolves the applied value", async () => {
    const { window } = createMutationRuntime();
    const pending = window.remcord.settings.setBlockPowerSavingInVoiceChat(true);
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "settings_set_block_power_saving");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "settings_set_block_power_saving", payload: { value: true } });

    respondMutation(window, message, true);
    assert.equal(await pending, true);
});

test("the mutation setter throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createMutationRuntime(["bridge.ping", "settings.get_config"]);
    assert.equal(window.remcord.native.supports("settings.set_block_power_saving_in_voice_chat"), false);
    assert.throws(
        () => window.remcord.settings.setBlockPowerSavingInVoiceChat(true),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "settings_set_block_power_saving"),
        false,
    );
});

test("the mutation setter rejects non-boolean values without sending", async () => {
    const { window } = createMutationRuntime();
    await assert.rejects(window.remcord.settings.setBlockPowerSavingInVoiceChat("yes"), /must be a boolean/);
    await assert.rejects(window.remcord.settings.setBlockPowerSavingInVoiceChat(1), /must be a boolean/);
    await assert.rejects(window.remcord.settings.setBlockPowerSavingInVoiceChat(undefined), /must be a boolean/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "settings_set_block_power_saving"),
        false,
    );
});

test("the mutation setter rejects malformed mutation responses", async () => {
    const { window } = createMutationRuntime();
    const pending = window.remcord.settings.setBlockPowerSavingInVoiceChat(true);
    const message = window.messages.find((entry) => entry.command?.type === "settings_set_block_power_saving");
    respondMutation(window, message, "yes");
    await assert.rejects(pending, /mutation response schema is invalid/);

    const second = createMutationRuntime();
    const pending2 = second.window.remcord.settings.setBlockPowerSavingInVoiceChat(false);
    const message2 = second.window.messages.find((entry) => entry.command?.type === "settings_set_block_power_saving");
    respondMutation(second.window, message2, false, { source: "defaults" });
    await assert.rejects(pending2, /mutation response schema is invalid/);
});

test("the mutation setter rejects host failures without applying state", async () => {
    const { window } = createMutationRuntime();
    const pending = window.remcord.settings.setBlockPowerSavingInVoiceChat(true);
    const message = window.messages.find((entry) => entry.command?.type === "settings_set_block_power_saving");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: false,
                payload: null,
                error: { code: "SETTINGS_UNAVAILABLE", message: "settings file is invalid" },
            },
        }),
    );
    await assert.rejects(pending, /settings file is invalid/);
});

test("the mutation setter rejects after its timeout", async () => {
    const { window } = createMutationRuntime();
    await assert.rejects(window.remcord.settings.setBlockPowerSavingInVoiceChat(true, 50), /timed out/);
});

test("the mutation setter rejects pending requests when the document is disposed", async () => {
    const { window } = createMutationRuntime();
    const pending = window.remcord.settings.setBlockPowerSavingInVoiceChat(true);
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh mutation facade with no stale promises", async () => {
    const firstDocument = createMutationRuntime();
    const stale = firstDocument.window.remcord.settings.setBlockPowerSavingInVoiceChat(true);
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createMutationRuntime();
    const pending = nextDocument.window.remcord.settings.setBlockPowerSavingInVoiceChat(false);
    const message = nextDocument.window.messages.find(
        (entry) => entry.command?.type === "settings_set_block_power_saving",
    );
    respondMutation(nextDocument.window, message, false);
    assert.equal(await pending, false);
});

test("the migrated caller updates local state only from the host-confirmed value", async () => {
    const { window } = createMutationRuntime();
    // Mirrors src/shelter/settings/settings.ts: local store first holds the
    // old value and is assigned only after the capability resolves.
    const store = { blockPowerSavingInVoiceChat: false };
    const pending = window.remcord.settings.setBlockPowerSavingInVoiceChat(true);
    const message = window.messages.find((entry) => entry.command?.type === "settings_set_block_power_saving");
    assert.equal(store.blockPowerSavingInVoiceChat, false);
    respondMutation(window, message, true);
    store.blockPowerSavingInVoiceChat = await pending;
    assert.equal(store.blockPowerSavingInVoiceChat, true);

    const failing = createMutationRuntime();
    const rejected = failing.window.remcord.settings.setBlockPowerSavingInVoiceChat(true);
    const failedMessage = failing.window.messages.find(
        (entry) => entry.command?.type === "settings_set_block_power_saving",
    );
    failing.window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: failedMessage.request_id,
                ok: false,
                payload: null,
                error: { code: "SETTINGS_UNAVAILABLE", message: "settings file is invalid" },
            },
        }),
    );
    await assert.rejects(rejected, /settings file is invalid/);
    assert.equal(store.blockPowerSavingInVoiceChat, true);
});

function createSettingsSetRuntime(capabilities = ["bridge.ping", "settings.get_config", "settings.set"]) {
    return createRuntime("https://discord.com", capabilities);
}

function respondSettingsSet(window, request, key, value, extra = {}) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: {
                    type: "settings_set",
                    payload: {
                        key,
                        value,
                        source: "file",
                        payload_bytes: 2048,
                        ...extra,
                    },
                },
                error: null,
            },
        }),
    );
}

test("the facade exposes a closed settings.set and no generic writer", () => {
    const { window } = createSettingsSetRuntime();
    assert.equal(typeof window.remcord.settings.set, "function");
    assert.equal(Object.isFrozen(window.remcord.settings), true);
    assert.equal(window.remcord.native.supports("settings.set"), true);
    // No generic key/value RPC may exist.
    assert.equal(window.remcord.settings.setConfig, undefined);
});

test("settings.set sends key plus bounded value and resolves the key echo", async () => {
    const { window } = createSettingsSetRuntime();
    const pending = window.remcord.settings.set("support_banner_dismissed", true);
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "settings_set");
    assert.ok(message);
    assert.deepEqual(message.command, {
        type: "settings_set",
        payload: { key: "support_banner_dismissed", value: true },
    });
    respondSettingsSet(window, message, "support_banner_dismissed", true);
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), {
        key: "support_banner_dismissed",
        value: true,
    });

    const modsPending = window.remcord.settings.set("mods", ["vencord"]);
    const modsMessage = window.messages.find(
        (entry) => entry.command?.type === "settings_set" && entry.request_id !== message.request_id,
    );
    assert.deepEqual(modsMessage.command, {
        type: "settings_set",
        payload: { key: "mods", value: ["vencord"] },
    });
    respondSettingsSet(window, modsMessage, "mods", ["vencord"]);
    assert.deepEqual(JSON.parse(JSON.stringify(await modsPending)), { key: "mods", value: ["vencord"] });

    const cspPending = window.remcord.settings.set("csp", "strict");
    const cspMessage = window.messages.find(
        (entry) =>
            entry.command?.type === "settings_set" &&
            entry.request_id !== message.request_id &&
            entry.request_id !== modsMessage.request_id,
    );
    assert.ok(cspMessage);
    assert.deepEqual(cspMessage.command, {
        type: "settings_set",
        payload: { key: "csp", value: "strict" },
    });
    respondSettingsSet(window, cspMessage, "csp", "strict");
    assert.deepEqual(JSON.parse(JSON.stringify(await cspPending)), { key: "csp", value: "strict" });
});

test("settings.set carries the channel and quick_css keys end to end", async () => {
    const { window } = createSettingsSetRuntime();
    const channelPending = window.remcord.settings.set("channel", "canary");
    const channelMessage = window.messages.find((entry) => entry.command?.type === "settings_set");
    assert.ok(channelMessage);
    assert.deepEqual(channelMessage.command, {
        type: "settings_set",
        payload: { key: "channel", value: "canary" },
    });
    respondSettingsSet(window, channelMessage, "channel", "canary");
    assert.deepEqual(JSON.parse(JSON.stringify(await channelPending)), { key: "channel", value: "canary" });

    const quickCssPending = window.remcord.settings.set("quick_css", false);
    const quickCssMessage = window.messages.find(
        (entry) => entry.command?.type === "settings_set" && entry.request_id !== channelMessage.request_id,
    );
    assert.ok(quickCssMessage);
    assert.deepEqual(quickCssMessage.command, {
        type: "settings_set",
        payload: { key: "quick_css", value: false },
    });
    respondSettingsSet(window, quickCssMessage, "quick_css", false);
    assert.deepEqual(JSON.parse(JSON.stringify(await quickCssPending)), { key: "quick_css", value: false });
});

test("settings.set throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createSettingsSetRuntime(["bridge.ping", "settings.get_config"]);
    assert.equal(window.remcord.native.supports("settings.set"), false);
    assert.throws(
        () => window.remcord.settings.set("support_banner_dismissed", true),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "settings_set"),
        false,
    );
});

test("settings.set rejects unknown keys and mismatched types without sending", async () => {
    const { window } = createSettingsSetRuntime();
    // Unknown keys: Electron-only or unsafe settings have no wire shape.
    await assert.rejects(window.remcord.settings.set("tray", true), /allowlisted/);
    await assert.rejects(window.remcord.settings.set("proxyRules", "http://127.0.0.1:8080"), /allowlisted/);
    await assert.rejects(window.remcord.settings.set("customIcon", "C:\\secret.png"), /allowlisted/);
    // Key/variant mismatches: bool keys refuse arrays, mods refuses a bool,
    // and csp refuses both (plus unknown modes).
    await assert.rejects(window.remcord.settings.set("support_banner_dismissed", ["vencord"]), /boolean/);
    await assert.rejects(window.remcord.settings.set("mods", true), /known mod ids/);
    await assert.rejects(window.remcord.settings.set("csp", true), /CSP modes/);
    await assert.rejects(window.remcord.settings.set("csp", ["none"]), /CSP modes/);
    await assert.rejects(window.remcord.settings.set("csp", "permissive"), /CSP modes/);
    await assert.rejects(window.remcord.settings.set("csp", "none; script-src *"), /CSP modes/);
    // Channel values pair only with the channel key (and only exact modes
    // there); a channel string through the csp key, or vice versa, is
    // refused, and bool/mods keys refuse channel strings.
    await assert.rejects(window.remcord.settings.set("channel", "beta"), /release channels/);
    await assert.rejects(window.remcord.settings.set("channel", "canary.discord.com"), /release channels/);
    await assert.rejects(window.remcord.settings.set("channel", true), /release channels/);
    await assert.rejects(window.remcord.settings.set("channel", ["stable"]), /release channels/);
    await assert.rejects(window.remcord.settings.set("csp", "canary"), /CSP modes/);
    await assert.rejects(window.remcord.settings.set("channel", "strict"), /release channels/);
    await assert.rejects(window.remcord.settings.set("mods", "canary"), /known mod ids/);
    await assert.rejects(window.remcord.settings.set("support_banner_dismissed", "stable"), /boolean/);
    await assert.rejects(window.remcord.settings.set("mods", "none"), /known mod ids/);
    await assert.rejects(window.remcord.settings.set("support_banner_dismissed", "none"), /boolean/);
    // Strings are never valid values; unknown mod ids are refused.
    await assert.rejects(window.remcord.settings.set("support_banner_dismissed", "yes"), /boolean/);
    await assert.rejects(window.remcord.settings.set("mods", ["Vencord"]), /known mod ids/);
    await assert.rejects(window.remcord.settings.set("mods", ["../escape"]), /known mod ids/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "settings_set"),
        false,
    );
});

test("settings.set rejects malformed set responses", async () => {
    const { window } = createSettingsSetRuntime();
    const pending = window.remcord.settings.set("support_banner_dismissed", true);
    const message = window.messages.find((entry) => entry.command?.type === "settings_set");
    // Key echo must equal the request: one key's write can never resolve
    // another key's call.
    respondSettingsSet(window, message, "show_experimental_plugin_menu", true);
    await assert.rejects(pending, /settings set response schema is invalid/);

    const second = createSettingsSetRuntime();
    const pending2 = second.window.remcord.settings.set("mods", ["vencord"]);
    const message2 = second.window.messages.find((entry) => entry.command?.type === "settings_set");
    respondSettingsSet(second.window, message2, "mods", ["vencord"], { source: "defaults" });
    await assert.rejects(pending2, /settings set response schema is invalid/);

    const third = createSettingsSetRuntime();
    const pending3 = third.window.remcord.settings.set("mods", ["vencord"]);
    const message3 = third.window.messages.find((entry) => entry.command?.type === "settings_set");
    respondSettingsSet(third.window, message3, "mods", ["vencord"], { leaked: "path" });
    await assert.rejects(pending3, /settings set response schema is invalid/);
});

// ---------------------------------------------------------------------------
// Settings import from the Electron profile (F7 core).
//
// Copy-only, never auto-migrate: the facade sends a parameter-free typed
// request (the host builds the fixed Electron source path itself), and
// resolves key-name lists plus whether a restart-requiring key applied.
// ---------------------------------------------------------------------------

function createSettingsImportRuntime(
    capabilities = ["bridge.ping", "settings.get_config", "settings.import_electron"],
) {
    return createRuntime("https://discord.com", capabilities);
}

function respondSettingsImported(window, request, payload) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "settings_imported", payload },
                error: null,
            },
        }),
    );
}

test("the facade exposes a parameter-free settings import and no path surface", () => {
    const { window } = createSettingsImportRuntime();
    assert.equal(typeof window.remcord.settings.importElectron, "function");
    assert.equal(Object.isFrozen(window.remcord.settings), true);
    assert.equal(window.remcord.native.supports("settings.import_electron"), true);
    // No path, URL, key-list, or value-taking member may exist alongside.
    assert.equal(window.remcord.settings.importPath, undefined);
    assert.equal(window.remcord.settings.importFile, undefined);
});

test("settings.import_electron sends an exact empty payload and resolves key lists", async () => {
    const { window } = createSettingsImportRuntime();
    const pending = window.remcord.settings.importElectron();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "settings_import_electron");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "settings_import_electron", payload: {} });
    respondSettingsImported(window, message, { applied: ["mods", "csp"], skipped: ["channel"] });
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), {
        applied: ["mods", "csp"],
        skipped: ["channel"],
        restartRequired: true,
    });
    // No restart-requiring key: no restart flag.
    const second = createSettingsImportRuntime();
    const pending2 = second.window.remcord.settings.importElectron();
    const message2 = second.window.messages.find((entry) => entry.command?.type === "settings_import_electron");
    respondSettingsImported(second.window, message2, { applied: ["spellcheck"], skipped: [] });
    assert.deepEqual(JSON.parse(JSON.stringify(await pending2)), {
        applied: ["spellcheck"],
        skipped: [],
        restartRequired: false,
    });
});

test("settings.import_electron throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createSettingsImportRuntime(["bridge.ping", "settings.get_config"]);
    assert.equal(window.remcord.native.supports("settings.import_electron"), false);
    assert.throws(
        () => window.remcord.settings.importElectron(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "settings_import_electron"),
        false,
    );
});

test("settings.import_electron rejects hostile key lists without paths", async () => {
    // Unknown keys, duplicates across lists, values, and paths are all
    // refused: only disjoint subsets of the allowlist resolve.
    for (const payload of [
        { applied: ["windowStyle"], skipped: [] },
        { applied: ["mods"], skipped: ["mods"] },
        { applied: ["mods", "channel"], skipped: [], values: { mods: ["vencord"] } },
        { applied: ["mods"], skipped: [], path: "C:\\x" },
        { applied: "mods", skipped: [] },
        { applied: [], skipped: ["additionalArguments"] },
    ]) {
        const runtime = createSettingsImportRuntime();
        const pending = runtime.window.remcord.settings.importElectron();
        const message = runtime.window.messages.find((entry) => entry.command?.type === "settings_import_electron");
        respondSettingsImported(runtime.window, message, payload);
        await assert.rejects(pending, /settings import response schema is invalid/);
    }
});

// ---------------------------------------------------------------------------
// Keybind execution (F8 core).
//
// Same member names as the Electron bridge (which are fire-and-forget
// sends), but async with typed outcomes. runJavascript has no wire shape:
// the maker refuses it before dispatch and the host refuses it at
// dispatch; stored code can never cross.
// ---------------------------------------------------------------------------

const KEYBIND_CAPS = ["bridge.ping", "keybinds.list", "keybinds.add", "keybinds.toggle", "keybinds.remove"];

function createKeybindsRuntime(capabilities = KEYBIND_CAPS) {
    return createRuntime("https://discord.com", capabilities);
}

function respondKeybinds(window, request, type, payload) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type, payload },
                error: null,
            },
        }),
    );
}

test("the facade exposes Electron-named keybind members and no code surface", () => {
    const { window } = createKeybindsRuntime();
    assert.equal(typeof window.remcord.settings.addKeybind, "function");
    assert.equal(typeof window.remcord.settings.toggleKeybind, "function");
    assert.equal(typeof window.remcord.settings.removeKeybind, "function");
    assert.equal(typeof window.remcord.settings.listKeybinds, "function");
    for (const capability of KEYBIND_CAPS.slice(1)) {
        assert.equal(window.remcord.native.supports(capability), true);
    }
    assert.equal(window.remcord.settings.runJavascript, undefined);
    assert.equal(window.remcord.settings.registerHotkey, undefined);
});

test("keybinds.list sends an exact empty payload and resolves metadata only", async () => {
    const { window } = createKeybindsRuntime();
    const pending = window.remcord.settings.listKeybinds();
    const message = window.messages.find((entry) => entry.command?.type === "keybinds_list");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "keybinds_list", payload: {} });
    respondKeybinds(window, message, "keybinds_list", {
        keybinds: [{ id: "a-1", accelerator: "Ctrl+M", action: "mute", global: true, enabled: true }],
    });
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), [
        { id: "a-1", accelerator: "Ctrl+M", action: "mute", global: true, enabled: true },
    ]);
});

test("keybinds.list rejects code-bearing inventories", async () => {
    for (const payload of [
        { keybinds: [{ id: "a-1", accelerator: "Ctrl+M", action: "runJavascript", global: true, enabled: true }] },
        {
            keybinds: [
                { id: "a-1", accelerator: "Ctrl+M", action: "mute", global: true, enabled: true, js: "alert(1)" },
            ],
        },
        { keybinds: [{ id: "../x", accelerator: "Ctrl+M", action: "mute", global: true, enabled: true }] },
        { keybinds: "nope" },
    ]) {
        const runtime = createKeybindsRuntime();
        const pending = runtime.window.remcord.settings.listKeybinds();
        const message = runtime.window.messages.find((entry) => entry.command?.type === "keybinds_list");
        respondKeybinds(runtime.window, message, "keybinds_list", payload);
        await assert.rejects(pending, /keybind list response schema is invalid/);
    }
});

test("settings.addKeybind sends the closed shape and echoes the id", async () => {
    const { window } = createKeybindsRuntime();
    const entry = { id: "a-1", accelerator: "Ctrl+Shift+M", action: "mute", global: true, enabled: true };
    const pending = window.remcord.settings.addKeybind(entry);
    const message = window.messages.find((entry) => entry.command?.type === "keybinds_add");
    assert.ok(message);
    assert.deepEqual(message.command, {
        type: "keybinds_add",
        payload: { id: "a-1", accelerator: "Ctrl+Shift+M", action: "mute", global: true, enabled: true },
    });
    respondKeybinds(window, message, "keybinds_add", { ok: true, id: "a-1" });
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), { ok: true, id: "a-1" });
    // A mismatched id echo is rejected: one entry's save can never
    // resolve another entry's call.
    const second = createKeybindsRuntime();
    const pending2 = second.window.remcord.settings.addKeybind(entry);
    const message2 = second.window.messages.find((entry) => entry.command?.type === "keybinds_add");
    respondKeybinds(second.window, message2, "keybinds_add", { ok: true, id: "other" });
    await assert.rejects(pending2, /keybind save response schema is invalid/);
    // Typed failures resolve for the maker toast.
    const third = createKeybindsRuntime();
    const pending3 = third.window.remcord.settings.addKeybind(entry);
    const message3 = third.window.messages.find((entry) => entry.command?.type === "keybinds_add");
    respondKeybinds(third.window, message3, "keybinds_add", { ok: false, error: "INVALID_KEYBIND" });
    assert.deepEqual(JSON.parse(JSON.stringify(await pending3)), { ok: false, error: "INVALID_KEYBIND" });
});

test("settings.addKeybind refuses code and hostile shapes before dispatch", async () => {
    const { window } = createKeybindsRuntime();
    const good = { id: "a-1", accelerator: "Ctrl+M", action: "mute", global: true, enabled: true };
    await assert.rejects(
        window.remcord.settings.addKeybind({ ...good, action: "runJavascript", js: "alert(1)" }),
        /executable/,
    );
    await assert.rejects(window.remcord.settings.addKeybind({ ...good, js: "alert(1)" }), /executable code/);
    await assert.rejects(window.remcord.settings.addKeybind({ ...good, id: "../x" }), /id must match/);
    await assert.rejects(window.remcord.settings.addKeybind({ ...good, accelerator: "" }), /accelerator/);
    await assert.rejects(
        window.remcord.settings.addKeybind({ ...good, action: "launchMissiles" }),
        /executable actions/,
    );
    await assert.rejects(window.remcord.settings.addKeybind({ ...good, global: "yes" }), /booleans/);
    await assert.rejects(window.remcord.settings.addKeybind("nope"), /object/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "keybinds_add"),
        false,
    );
});

test("settings.toggleKeybind/removeKeybind send id-only requests with typed outcomes", async () => {
    const { window } = createKeybindsRuntime();
    const toggled = window.remcord.settings.toggleKeybind("a-1");
    const toggleMessage = window.messages.find((entry) => entry.command?.type === "keybinds_toggle");
    assert.deepEqual(toggleMessage.command, { type: "keybinds_toggle", payload: { id: "a-1" } });
    respondKeybinds(window, toggleMessage, "keybinds_toggle", { ok: true, enabled: false });
    assert.deepEqual(JSON.parse(JSON.stringify(await toggled)), { ok: true, enabled: false });

    const removed = window.remcord.settings.removeKeybind("a-1");
    const removeMessage = window.messages.find((entry) => entry.command?.type === "keybinds_remove");
    assert.deepEqual(removeMessage.command, { type: "keybinds_remove", payload: { id: "a-1" } });
    respondKeybinds(window, removeMessage, "keybinds_remove", { ok: true });
    assert.deepEqual(JSON.parse(JSON.stringify(await removed)), { ok: true });

    await assert.rejects(window.remcord.settings.toggleKeybind("../x"), /id must match/);
    await assert.rejects(window.remcord.settings.removeKeybind(""), /id must match/);
});

test("keybind capabilities throw UNSUPPORTED_CAPABILITY when the snapshot denies them", () => {
    const { window } = createKeybindsRuntime(["bridge.ping"]);
    for (const capability of KEYBIND_CAPS.slice(1)) {
        assert.equal(window.remcord.native.supports(capability), false);
    }
    assert.throws(
        () => window.remcord.settings.listKeybinds(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.throws(
        () =>
            window.remcord.settings.addKeybind({
                id: "a",
                accelerator: "Ctrl+M",
                action: "mute",
                global: true,
                enabled: true,
            }),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.throws(
        () => window.remcord.settings.toggleKeybind("a"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.throws(
        () => window.remcord.settings.removeKeybind("a"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type?.startsWith("keybinds_")),
        false,
    );
});

// ---------------------------------------------------------------------------
// Non-global keybind matcher (F8 core, native/web/keybinds/matcher.js).
//
// The matcher observes keydown against the enabled non-global inventory
// and reports matches as id-only one-way events. These tests drive the
// shipped script in a VM: inventory fetch, canonical matching across
// maker release-order noise, scope filtering, and the shadow behavior.
// ---------------------------------------------------------------------------

function createMatcherRuntime() {
    const runtime = createKeybindsRuntime();
    vm.runInContext(keybindMatcher, runtime.context);
    return runtime;
}

function respondKeybindsList(window, entries) {
    const message = window.messages.find((entry) => entry.command?.type === "keybinds_list");
    assert.ok(message, "matcher must fetch the inventory");
    respondKeybinds(window, message, "keybinds_list", { keybinds: entries });
}

function fakeKeydown(overrides = {}) {
    return {
        type: "keydown",
        ctrlKey: false,
        altKey: false,
        shiftKey: false,
        metaKey: false,
        key: "",
        defaultPrevented: false,
        preventDefault() {
            this.defaultPrevented = true;
        },
        stopPropagation() {},
        ...overrides,
    };
}

function matchedIds(window) {
    return window.messages
        .filter((message) => message.event?.type === "keybind_matched")
        .map((message) => message.event.payload.id);
}

// The inventory fetch resolves through the facade promise chain; one
// microtask is not enough ticks in every case, so flush briefly.
function flushInventory() {
    return new Promise((resolve) => setTimeout(resolve, 10));
}

test("the matcher reports canonical matches across maker ordering noise", async () => {
    const { window } = createMatcherRuntime();
    respondKeybindsList(window, [
        { id: "a-1", accelerator: "Shift+Ctrl+M", action: "mute", global: false, enabled: true },
        { id: "off", accelerator: "Ctrl+O", action: "deafen", global: false, enabled: false },
        { id: "glob", accelerator: "Ctrl+G", action: "mute", global: true, enabled: true },
    ]);
    await flushInventory();
    // Canonical Ctrl+Shift+M regardless of stored token order.
    window.dispatchEvent(fakeKeydown({ ctrlKey: true, shiftKey: true, key: "m" }));
    assert.deepEqual(matchedIds(window), ["a-1"]);
    // Disabled and global entries never match in-page.
    window.dispatchEvent(fakeKeydown({ ctrlKey: true, key: "o" }));
    window.dispatchEvent(fakeKeydown({ ctrlKey: true, key: "g" }));
    assert.deepEqual(matchedIds(window), ["a-1"]);
    // A near-miss (missing Shift) matches nothing.
    window.dispatchEvent(fakeKeydown({ ctrlKey: true, key: "m" }));
    assert.deepEqual(matchedIds(window), ["a-1"]);
});

test("the matcher shadows the page on match and ignores default-prevented keys", async () => {
    const { window } = createMatcherRuntime();
    respondKeybindsList(window, [{ id: "a-1", accelerator: "Ctrl+M", action: "mute", global: false, enabled: true }]);
    await flushInventory();
    let prevented = false;
    let stopped = false;
    window.dispatchEvent({
        ...fakeKeydown({ ctrlKey: true, key: "M" }),
        preventDefault() {
            prevented = true;
        },
        stopPropagation() {
            stopped = true;
        },
    });
    assert.deepEqual(matchedIds(window), ["a-1"]);
    assert.equal(prevented, true);
    assert.equal(stopped, true);
    // Already-handled keys are left alone.
    window.dispatchEvent(fakeKeydown({ ctrlKey: true, key: "M", defaultPrevented: true }));
    assert.deepEqual(matchedIds(window), ["a-1"]);
});

test("the matcher never breaks page input and carries ids only", async () => {
    const { window } = createMatcherRuntime();
    respondKeybindsList(window, [
        { id: "a-1", accelerator: "Not+A+Keybind", action: "mute", global: false, enabled: true },
    ]);
    await flushInventory();
    // Garbage inventory matches nothing and throws nothing.
    window.dispatchEvent(fakeKeydown({ ctrlKey: true, key: "a" }));
    assert.deepEqual(matchedIds(window), []);
    // Match reports carry the id and nothing else.
    const serialized = JSON.stringify(window.messages.filter((m) => m.event?.type === "keybind_matched"));
    assert.doesNotMatch(serialized, /accelerator|action|javascript|url|path/i);
});

test("initializes a frozen compatibility facade with the boot snapshot", () => {
    const { window } = createRuntime();
    assert.ok(window.remcord);
    assert.equal(window.remcord.boot.runtime, "webview2");
    assert.equal(window.remcord.boot.platform, "win32");
    assert.equal(Object.isFrozen(window.remcord), true);
    assert.equal(Object.isFrozen(window.remcord.boot), true);
    assert.equal(window.remcord.native.supports("mod.get_bundle"), true);
    assert.equal(
        window.messages.some((message) => message.event?.type === "bootstrap_metrics"),
        true,
    );
});

test("rejects capabilities that are not in the immutable snapshot", () => {
    const { window } = createRuntime();
    assert.equal(window.remcord.native.supports("bridge.ping"), true);
    assert.equal(window.remcord.native.supports("filesystem"), false);
    assert.throws(
        () => window.remcord.native.requireCapability("filesystem"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
});

test("window-state events converge local reads and body attributes without polling", () => {
    const { window, context } = createRuntime();
    const attributes = new Map();
    context.document = {
        body: {
            setAttribute: (name, value) => attributes.set(name, value),
            removeAttribute: (name) => attributes.delete(name),
        },
    };
    // Seeded from the boot snapshot: unfocused, unmaximized.
    assert.equal(window.remcord.native.window.maximized(), false);
    assert.equal(window.remcord.native.window.isNormal(), true);
    assert.equal(window.remcord.window.maximized(), false);
    assert.equal(window.remcord.window.isNormal(), true);

    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:window-state", {
            detail: { version: 1, focused: true, maximized: true },
        }),
    );
    assert.equal(window.remcord.native.window.maximized(), true);
    assert.equal(window.remcord.native.window.isNormal(), false);
    assert.equal(window.remcord.window.maximized(), true);
    assert.equal(attributes.get("isMaximized"), "");
    assert.equal(attributes.has("unFocused"), false);

    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:window-state", {
            detail: { version: 1, focused: false, maximized: false },
        }),
    );
    assert.equal(window.remcord.native.window.maximized(), false);
    assert.equal(window.remcord.native.window.isNormal(), true);
    assert.equal(attributes.has("isMaximized"), false);
    assert.equal(attributes.get("unFocused"), "");

    // Malformed details are ignored: no listener throw, no state change.
    window.dispatchEvent(new FakeCustomEvent("remcord-native:window-state", { detail: null }));
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:window-state", {
            detail: { version: 2, focused: true, maximized: true },
        }),
    );
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:window-state", {
            detail: { version: 1, focused: "yes", maximized: true },
        }),
    );
    assert.equal(window.remcord.native.window.maximized(), false);
    assert.equal(attributes.has("isMaximized"), false);
    assert.equal(attributes.get("unFocused"), "");
});

test("correlates asynchronous responses independently of their arrival order", async () => {
    const { window } = createRuntime();
    const first = window.remcord.native.ping("first");
    const second = window.remcord.native.ping("second");
    const requests = window.messages.filter((message) => message.command?.type === "ping");
    assert.equal(requests.length, 2);

    respondPing(window, requests[1], { nonce: "second", message: "pong" });
    respondPing(window, requests[0], { nonce: "first", message: "pong" });
    assert.deepEqual(await Promise.all([first, second]), [
        { nonce: "first", message: "pong" },
        { nonce: "second", message: "pong" },
    ]);
    assert.equal(window.messages.filter((message) => message.event?.type === "round_trip_ack").length, 2);
});

test("enforces UTF-8 byte limits before sending requests", async () => {
    const { window } = createRuntime();
    await assert.rejects(window.remcord.native.ping("é".repeat(65)), /128 UTF-8 bytes/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "ping"),
        false,
    );
});

test("disposes pending requests on navigation and initializes fresh document state", async () => {
    const firstDocument = createRuntime();
    const pending = firstDocument.window.remcord.native.ping("leaving");
    const rejection = assert.rejects(pending, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;

    const nextDocument = createRuntime();
    assert.ok(nextDocument.window.remcord);
    const request = nextDocument.window.remcord.native.ping("reloaded");
    const nextPing = nextDocument.window.messages.find((message) => message.command?.type === "ping");
    respondPing(nextDocument.window, nextPing, { nonce: "reloaded", message: "pong" });
    assert.deepEqual(await request, { nonce: "reloaded", message: "pong" });
    assert.equal(firstDocument.window.remcord === nextDocument.window.remcord, false);
});

test("does not expose the facade outside the entry origins", () => {
    for (const origin of [
        "https://evil.discord.com",
        "https://deep.sub.discord.com",
        "https://discord.com.attacker.invalid",
        "https://example.com",
        "http://discord.com",
    ]) {
        const { window } = createRuntime(origin);
        assert.equal(window.remcord, undefined, origin);
        assert.equal(window.messages.length, 0, origin);
    }
});

test("exposes the facade on every release-channel entry origin", () => {
    for (const origin of ["https://discord.com", "https://canary.discord.com", "https://ptb.discord.com"]) {
        const { window } = createRuntime(origin, ["bridge.ping"]);
        assert.ok(window.remcord, origin);
        assert.equal(typeof window.remcord.native.ping, "function", origin);
    }
});

test("reports renderer exception stacks and source locations to the native log", () => {
    const { window } = createRuntime();
    window.dispatchEvent({
        type: "error",
        message: "script failed",
        filename: "https://discord.com/assets/app.js",
        lineno: 12,
        colno: 4,
    });
    window.dispatchEvent({
        type: "unhandledrejection",
        reason: new Error("settings access failed"),
    });
    const reports = window.messages.filter((message) => message.event?.type === "renderer_error");
    assert.match(reports[0].event.payload.message, /assets\/app\.js:12:4/);
    assert.match(reports[1].event.payload.message, /settings access failed/);
});

test("window acknowledgements resolve by exact command echo", async () => {
    const { window } = createRuntime("https://discord.com", ["bridge.ping", "window.minimize", "window.drag"]);
    const minimize = window.remcord.native.window.minimize();
    const drag = window.remcord.native.window.drag();
    // Top-level Electron-shaped mirror dispatches the same typed operations.
    const topDrag = window.remcord.window.drag();
    const commands = window.messages.filter((message) => message.command).map((message) => message.command.type);
    assert.deepEqual(commands, ["window_minimize", "window_drag", "window_drag"]);
    const requests = window.messages.filter((message) => message.command);
    assert.equal(requests.length, 3);
    for (const [index, [pending, type]] of [
        [minimize, "window_minimize"],
        [drag, "window_drag"],
        [topDrag, "window_drag"],
    ].entries()) {
        const request = requests[index];
        window.dispatchEvent(
            new FakeCustomEvent("remcord-native:response", {
                detail: {
                    version: 1,
                    request_id: request.request_id,
                    ok: true,
                    payload: { type: "window_ack", payload: { command: type } },
                    error: null,
                },
            }),
        );
        assert.deepEqual(await pending, { command: type });
    }
    // A wrong or extra echo never resolves a window call.
    const bad = window.remcord.native.window.minimize();
    const badRequest = window.messages.filter((message) => message.command?.type === "window_minimize").at(-1);
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: badRequest.request_id,
                ok: true,
                payload: { type: "window_ack", payload: { command: "window_maximize", extra: 1 } },
                error: null,
            },
        }),
    );
    await assert.rejects(bad, /window acknowledgement response schema is invalid/);
});

test("window operations require their capability", () => {
    const { window } = createRuntime("https://discord.com", ["bridge.ping"]);
    assert.throws(
        () => window.remcord.native.window.drag(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.throws(
        () => window.remcord.native.window.minimize(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command),
        false,
    );
});

test("F11 dispatches exactly the fixed fullscreen toggle", () => {
    const { window } = createRuntime("https://discord.com", ["bridge.ping", "window.toggle_fullscreen"]);
    const keydown = (key) =>
        window.dispatchEvent({ type: "keydown", key, defaultPrevented: false, preventDefault() {} });
    keydown("F11");
    const commands = window.messages.filter((message) => message.command).map((message) => message.command.type);
    assert.deepEqual(commands, ["window_toggle_fullscreen"]);
    // No other key is intercepted.
    keydown("F12");
    keydown("a");
    assert.equal(window.messages.filter((message) => message.command).length, 1);
    // Without the capability the key is swallowed silently: no message,
    // no listener throw, no unhandled rejection.
    const denied = createRuntime("https://discord.com", ["bridge.ping"]);
    denied.window.dispatchEvent({ type: "keydown", key: "F11", defaultPrevented: false, preventDefault() {} });
    assert.equal(
        denied.window.messages.some((message) => message.command),
        false,
    );
});

test("identical consecutive renderer errors collapse with exact counts", () => {
    const { window, context } = createRuntime();
    vm.runInContext(
        "console.error('same failure');console.error('same failure');console.error('same failure');",
        context,
    );
    let reports = window.messages.filter((message) => message.event?.type === "renderer_error");
    assert.equal(reports.length, 1);
    assert.match(reports[0].event.payload.message, /same failure/);
    // A distinct message flushes the collapsed run as a summary first.
    vm.runInContext("console.error('different failure');", context);
    reports = window.messages.filter((message) => message.event?.type === "renderer_error");
    assert.equal(reports.length, 3);
    assert.equal(reports[1].event.payload.kind, "error_flood_summary");
    assert.match(reports[1].event.payload.message, /collapsed 2 duplicate\(s\)/);
    assert.match(reports[2].event.payload.message, /different failure/);
});

test("requests only the shared bundle capability with the registered mod id", async () => {
    const { window } = createRuntime();
    const request = window.remcord.native.getVencordBundle();
    const message = window.messages.find((entry) => entry.command?.type === "mod_get_bundle");
    assert.ok(message);
    // Exactly one logical id. No path, URL, filename, or source member.
    assert.deepEqual(message.command, { type: "mod_get_bundle", payload: { mod_id: "vencord" } });
    assert.equal(Object.keys(message.command.payload).length, 1);
    assert.equal(request.requestId, message.request_id);

    respondVencord(window, message);
    const result = await request;
    assert.equal(result.marker, "vencord-bootstrap-executed");
    assert.equal(result.mod_id, "vencord");
});

test("the shared bundle request carries a logical id and nothing else", async () => {
    const { window } = createRuntime();
    const request = window.remcord.native.getModBundle("vencord");
    const message = window.messages.find((entry) => entry.command?.type === "mod_get_bundle");
    assert.deepEqual(message.command.payload, { mod_id: "vencord" });
    for (const forbidden of ["path", "url", "file", "source", "js_url", "channel", "method"]) {
        assert.equal(forbidden in message.command.payload, false);
    }
    respondVencord(window, message);
    assert.equal((await request).bundle_version, "b52ed36");
});

test("rejects bundle ids that could select a path, URL, or source", async () => {
    const { window } = createRuntime();
    for (const hostile of [
        "../vencord",
        "..\\vencord",
        "C:\\vencord.js",
        "//server/share/vencord.js",
        "https://attacker.invalid/vencord.js",
        "javascript:alert(1)",
        "",
        "ven cord",
        "a".repeat(65),
        123,
        null,
        { modId: "vencord" },
    ]) {
        await assert.rejects(
            () => window.remcord.native.getModBundle(hostile),
            TypeError,
            `must reject ${JSON.stringify(hostile)}`,
        );
    }
    // Client-side rejection means nothing reached the wire.
    assert.equal(
        window.messages.some((message) => message.command?.type === "mod_get_bundle"),
        false,
    );
});

test("an unregistered but well-formed id is refused by the host registry", async () => {
    // The charset bound cannot know the registry, so a plausible-looking id
    // (including a file name) is dispatched and must come back as a typed
    // UNKNOWN_MOD failure. Nothing is read, and no source is substituted.
    // The facade's propagation is id-agnostic: the host decides servability
    // (`custom` is served since F1, so a never-registered `myplugin` stands
    // in for the refused shape here).
    const { window } = createRuntime();
    for (const unregistered of ["vencord.js", "shelter", "equicord", "myplugin", "Vencord"]) {
        const request = window.remcord.native.getModBundle(unregistered);
        const message = window.messages.filter((entry) => entry.command?.type === "mod_get_bundle").pop();
        assert.deepEqual(message.command.payload, { mod_id: unregistered });
        window.dispatchEvent(
            new FakeCustomEvent("remcord-native:response", {
                detail: {
                    version: 1,
                    request_id: message.request_id,
                    ok: false,
                    payload: null,
                    error: {
                        code: "UNKNOWN_MOD",
                        message: `mod '${unregistered}' is not registered in the native host`,
                    },
                },
            }),
        );
        await assert.rejects(
            request,
            (error) => {
                assert.equal(error.code, "UNKNOWN_MOD");
                return true;
            },
            unregistered,
        );
    }
});

test("denies the shared bundle capability when the boot snapshot does not grant it", () => {
    const { window } = createRuntime("https://discord.com", ["bridge.ping"]);
    assert.throws(
        () => window.remcord.native.getModBundle("vencord"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.throws(
        () => window.remcord.native.getVencordBundle(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "mod_get_bundle"),
        false,
    );
});

test("rejects malformed bundle responses", async () => {
    const { window } = createRuntime();
    for (const malformed of [
        { document_execution_count: 2 },
        { mod_id: "" },
        { mod_id: 7 },
        { bundle_version: 42 },
        { bundle_bytes: 1.5 },
        { marker: "" },
        { marker: 1 },
        { request_latency_ms: "fast" },
        { mod_id: undefined },
    ]) {
        const request = window.remcord.native.getModBundle("vencord");
        const message = window.messages.filter((entry) => entry.command?.type === "mod_get_bundle").pop();
        respondVencord(window, message, malformed);
        await assert.rejects(request, /response schema is invalid/, JSON.stringify(malformed));
    }
});

test("times out a bundle request that never resolves", async () => {
    const { window } = createRuntime();
    // A short explicit timeout keeps the test fast without touching the
    // documented 120 s bundle default.
    await assert.rejects(
        () => window.remcord.native.getModBundle("vencord", 5),
        (error) => {
            assert.equal(error.code, "REQUEST_TIMEOUT");
            return true;
        },
    );
});

test("propagates a typed bundle failure without reclassifying it as adapter initialization", async () => {
    const { window, context } = createRuntime();
    vm.runInContext(vencordAdapter, context);
    const request = window.messages.find((message) => message.command?.type === "mod_get_bundle");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: false,
                payload: null,
                error: { code: "bundle_retrieval", message: "fixed Vencord asset unavailable" },
            },
        }),
    );
    await Promise.resolve();
    await Promise.resolve();
    assert.equal(window.__remcordNativeVencord.status, "failed");
    assert.equal(
        window.messages.some((message) => message.event?.type === "vencord_adapter_error"),
        false,
    );
});

test("denies Vencord startup when the boot snapshot does not grant the capability", () => {
    const { window, context } = createRuntime("https://discord.com", ["bridge.ping"]);
    vm.runInContext(vencordAdapter, context);
    assert.equal(
        window.messages.some((message) => message.command?.type === "mod_get_bundle"),
        false,
    );
    assert.equal(
        window.messages.some((message) => message.event?.type === "vencord_adapter_error"),
        true,
    );
    assert.equal(window.__remcordNativeVencord.status, "failed");
});

test("Vencord adapter executes once and marks only the bootstrap result", async () => {
    const { window, context } = createRuntime();
    vm.runInContext(vencordAdapter, context);
    vm.runInContext(vencordAdapter, context);
    const request = window.messages.find((message) => message.command?.type === "mod_get_bundle");
    assert.ok(request);
    assert.equal(window.messages.filter((message) => message.command?.type === "mod_get_bundle").length, 1);
    assert.equal(window.__remcordNativeVencord.status, "pending");

    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:vencord-bootstrap", {
            detail: {
                request_id: request.request_id,
                ok: true,
                marker: "vencord-bootstrap-executed",
                execution_ms: 3,
                local_storage_api: "available",
                settings_state: "missing",
            },
        }),
    );
    respondVencord(window, request);
    await Promise.resolve();
    await Promise.resolve();
    assert.equal(window.__remcordNativeVencord.status, "executed");
    assert.equal(window.__remcordNativeVencord.marker, "vencord-bootstrap-executed");
    assert.equal(window.__remcordNativeVencord.executionCount, 1);
    const bootstrapResult = window.messages.find((message) => message.event?.type === "vencord_bootstrap_result");
    assert.equal(bootstrapResult.event.payload.local_storage_api, "available");
    assert.equal(bootstrapResult.event.payload.settings_state, "missing");
    assert.equal(
        window.messages.some((message) => message.event?.type === "vencord_response_ack"),
        true,
    );
});

test("a fresh document gets independent Vencord bootstrap state", () => {
    const firstDocument = createRuntime();
    vm.runInContext(vencordAdapter, firstDocument.context);
    const secondDocument = createRuntime();
    vm.runInContext(vencordAdapter, secondDocument.context);
    assert.notEqual(firstDocument.window.__remcordNativeVencord, secondDocument.window.__remcordNativeVencord);
    assert.equal(secondDocument.window.__remcordNativeVencord.status, "pending");
    assert.equal(
        secondDocument.window.messages.filter((message) => message.command?.type === "mod_get_bundle").length,
        1,
    );
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    secondDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
});

test("a reloaded document re-requests the known bundle through the shared capability", async () => {
    const firstDocument = createRuntime();
    vm.runInContext(vencordAdapter, firstDocument.context);
    const firstRequest = firstDocument.window.messages.find((message) => message.command?.type === "mod_get_bundle");
    respondVencord(firstDocument.window, firstRequest);
    assert.ok(firstRequest.request_id.length > 0);

    // Reload: a fresh document starts from clean per-document state and the
    // adapter asks for the SAME registry id again — never a path or URL.
    const secondDocument = createRuntime();
    vm.runInContext(vencordAdapter, secondDocument.context);
    const secondRequest = secondDocument.window.messages.find((message) => message.command?.type === "mod_get_bundle");
    assert.deepEqual(secondRequest.command.payload, { mod_id: "vencord" });
    assert.notEqual(firstDocument.window.remcord, secondDocument.window.remcord);

    // Disposing the previous document clears its pending bundle request, so
    // a late host answer can no longer resolve anything.
    const lateRequest = firstDocument.window.remcord.native.getModBundle("vencord");
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await assert.rejects(lateRequest, /document was disposed/);
    assert.equal(secondDocument.window.__remcordNativeVencord.status, "pending");
    respondVencord(secondDocument.window, secondRequest, { mod_id: "equicord" });
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    // The adapter's own identity check rejects a response whose echoed
    // registry id is not Vencord: a cross-mod bundle can never be
    // acknowledged as a Vencord bootstrap even with a perfect marker.
    secondDocument.window.dispatchEvent(
        new FakeCustomEvent("remcord-native:vencord-bootstrap", {
            detail: {
                request_id: secondRequest.request_id,
                ok: true,
                marker: "vencord-bootstrap-executed",
                execution_ms: 3,
                local_storage_api: "available",
                settings_state: "valid_json",
            },
        }),
    );
    assert.equal(secondDocument.window.__remcordNativeVencord.status, "executed");
    respondVencord(secondDocument.window, secondRequest, { mod_id: "equicord" });
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    assert.match(secondDocument.window.__remcordNativeVencord.error, /not for Vencord/);
    assert.equal(
        secondDocument.window.messages.some((message) => message.event?.type === "vencord_response_ack"),
        false,
    );
    secondDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
});

test("the substrate factory is idempotent and shared by every adapter", () => {
    const { window, context } = createRuntime();
    const factory = window.__remcordNativeModSubstrate;
    assert.ok(factory);
    assert.equal(typeof factory.createModAdapter, "function");
    assert.equal(typeof factory.overrideQuickCssEditor, "function");
    assert.equal(Object.isFrozen(factory), true);
    vm.runInContext(modSubstrate, context);
    assert.equal(window.__remcordNativeModSubstrate, factory);
});

function installVencordNative(window, calls) {
    window.VencordNative = {
        quickCss: {
            get: () => Promise.resolve(":root{}"),
            set: (css) => {
                calls.set.push(css);
                return Promise.resolve();
            },
            openEditor() {
                calls.original += 1;
            },
        },
    };
}

test("overrideQuickCssEditor leaves a missing bundle alone and never throws", () => {
    const { window } = createRuntime();
    const substrate = window.__remcordNativeModSubstrate;
    assert.equal(substrate.overrideQuickCssEditor("vencord"), false);
    window.VencordNative = {};
    assert.equal(substrate.overrideQuickCssEditor("vencord"), false);
    window.VencordNative = { quickCss: {} };
    assert.equal(substrate.overrideQuickCssEditor("vencord"), false);
});

test("overrideQuickCssEditor is idempotent and patches exactly once", () => {
    const { window } = createRuntime();
    const calls = { set: [], original: 0 };
    installVencordNative(window, calls);
    const substrate = window.__remcordNativeModSubstrate;
    assert.equal(substrate.overrideQuickCssEditor("vencord"), true);
    const patched = window.VencordNative.quickCss.openEditor;
    assert.equal(patched.__remcordNativePatched, true);
    assert.equal(substrate.overrideQuickCssEditor("vencord"), false);
    assert.equal(window.VencordNative.quickCss.openEditor, patched);
});

test("patched openEditor hands off to the Remcord editor when handled", () => {
    const { window } = createRuntime();
    const calls = { set: [], original: 0 };
    installVencordNative(window, calls);
    window.__remcordNativeModSubstrate.overrideQuickCssEditor("vencord");
    // The Remcord settings plugin takes the handoff by canceling the event.
    const originalDispatch = window.dispatchEvent.bind(window);
    let handoff = null;
    window.dispatchEvent = (event) => {
        if (event.type === "remcord-native:open-quickcss-editor") {
            handoff = event;
            return false;
        }
        return originalDispatch(event);
    };
    window.VencordNative.quickCss.openEditor();
    assert.ok(handoff);
    assert.equal(handoff.detail.mode, "vencord");
    assert.equal(calls.original, 0);
});

test("patched openEditor reports honestly when nothing handles the handoff", () => {
    const { window, context } = createRuntime();
    const calls = { set: [], original: 0 };
    installVencordNative(window, calls);
    window.__remcordNativeModSubstrate.overrideQuickCssEditor("equicord");
    const alerts = [];
    context.alert = (message) => alerts.push(message);
    // No listener cancels: dispatchEvent reports unhandled.
    window.VencordNative.quickCss.openEditor();
    assert.equal(calls.original, 0);
    assert.equal(alerts.length, 1);
    assert.match(alerts[0], /Remcord settings plugin is not running/);
});

for (const [modId, adapter, markerEvent] of [
    ["vencord", vencordAdapter, "remcord-native:vencord-bootstrap"],
    ["equicord", equicordAdapter, "remcord-native:equicord-bootstrap"],
]) {
    test(`${modId} adapter hands its QuickCSS editor to Remcord after bootstrap`, async () => {
        const { window, context } = createRuntime();
        const calls = { set: [], original: 0 };
        installVencordNative(window, calls);
        vm.runInContext(adapter, context);
        const request = window.messages.find((message) => message.command?.type === "mod_get_bundle");
        assert.ok(request);
        window.dispatchEvent(
            new FakeCustomEvent(markerEvent, {
                detail: {
                    request_id: request.request_id,
                    ok: true,
                    marker: modId === "vencord" ? "vencord-bootstrap-executed" : "equicord-bootstrap-executed",
                    execution_ms: 3,
                    local_storage_api: "available",
                    settings_state: "missing",
                },
            }),
        );
        const patched = window.VencordNative.quickCss.openEditor;
        assert.equal(patched.__remcordNativePatched, true);
        // The handoff carries the owning mod so the editor titles itself.
        const originalDispatch = window.dispatchEvent.bind(window);
        let handoff = null;
        window.dispatchEvent = (event) => {
            if (event.type === "remcord-native:open-quickcss-editor") {
                handoff = event;
                return false;
            }
            return originalDispatch(event);
        };
        window.VencordNative.quickCss.openEditor();
        assert.equal(handoff.detail.mode, modId);
        assert.equal(calls.original, 0);
        window.dispatchEvent(new FakeCustomEvent("pagehide"));
    });
}

// ---------------------------------------------------------------------------
// Registered mod consumers (Shelter, Equicord, Custom) share this lifecycle
// verbatim. Every assertion below runs for ALL THREE mods, which is what
// makes the shared substrate's generality a tested property rather than a
// claim.
// ---------------------------------------------------------------------------

const REGISTERED_ADAPTERS = { shelter: shelterAdapter, equicord: equicordAdapter, custom: customAdapter };

for (const [modId, adapter] of Object.entries(REGISTERED_ADAPTERS)) {
    const wire = MOD_WIRE[modId];

    test(`${modId} requests only the shared bundle capability with its own registry id`, () => {
        const { window, context } = createRuntime();
        vm.runInContext(adapter, context);
        vm.runInContext(adapter, context);
        const requests = window.messages.filter((message) => message.command?.type === "mod_get_bundle");
        assert.equal(requests.length, 1, "one start per document");
        // Exactly one logical parameter: the id. No URL, path, or source.
        assert.deepEqual(requests[0].command.payload, { mod_id: modId });
        assert.equal(requests[0].command.type, "mod_get_bundle");
        assert.deepEqual(Object.keys(requests[0].command.payload), ["mod_id"]);
        assert.equal(window[wire.stateName].status, "pending");
    });

    test(`${modId} executes once, marks the bootstrap, and acknowledges exactly once`, async () => {
        const { window, context } = createRuntime();
        vm.runInContext(adapter, context);
        const request = window.messages.find((message) => message.command?.type === "mod_get_bundle");

        dispatchBootstrap(window, request, modId);
        respondMod(window, request, modId);
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();

        assert.equal(window[wire.stateName].status, "executed");
        assert.equal(window[wire.stateName].marker, wire.marker);
        assert.equal(window[wire.stateName].executionCount, 1);
        assert.equal(window[wire.stateName].modId, modId);
        assert.equal(window[wire.stateName].bundleVersion, wire.version);

        const results = window.messages.filter((message) => message.event?.type === wire.resultEvent);
        assert.equal(results.length, 1, "one bootstrap result per document");
        assert.equal(results[0].event.payload.ok, true);
        assert.equal(results[0].event.payload.marker, wire.marker);
        assert.equal(results[0].event.payload.request_id, request.request_id);
        assert.equal(results[0].event.payload.local_storage_api, "available");
        // The mod-agnostic event family must carry the mod id so the host can
        // refuse a verdict claimed for another registered mod.
        if (wire.echoesModId) {
            assert.equal(results[0].event.payload.mod_id, modId);
        }
        // No source, URL, path, or filename ever crosses the bridge.
        const serialized = JSON.stringify(results[0]);
        assert.doesNotMatch(serialized, /source|js_url|browser\.js|shelter-builds|githubusercontent/);

        const acks = window.messages.filter((message) => message.event?.type === wire.ackEvent);
        assert.equal(acks.length, 1);
        if (wire.echoesModId) {
            assert.equal(acks[0].event.payload.mod_id, modId);
        }
    });

    test(`${modId} rejects a response the host echoed for a different registered mod`, async () => {
        const { window, context } = createRuntime();
        vm.runInContext(adapter, context);
        const request = window.messages.find((message) => message.command?.type === "mod_get_bundle");

        // A perfect marker for the right mod, but the registry echo says
        // another mod: a cross-mod bundle can never be acknowledged.
        dispatchBootstrap(window, request, modId);
        const other = modId === "shelter" ? "equicord" : "shelter";
        respondMod(window, request, other);
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();

        assert.equal(window[wire.stateName].status, "failed");
        assert.match(window[wire.stateName].error, /not for/);
        assert.equal(
            window.messages.some((message) => message.event?.type === wire.ackEvent),
            false,
        );
    });

    test(`${modId} rejects a wrong execution marker from the host`, async () => {
        const { window, context } = createRuntime();
        vm.runInContext(adapter, context);
        const request = window.messages.find((message) => message.command?.type === "mod_get_bundle");

        dispatchBootstrap(window, request, modId, { marker: "some-other-mod-executed" });
        respondMod(window, request, modId, { marker: "some-other-mod-executed" });
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();

        assert.equal(window[wire.stateName].status, "failed");
        assert.equal(window[wire.stateName].marker, null);
        assert.equal(
            window.messages.some((message) => message.event?.type === wire.ackEvent),
            false,
        );
    });

    test(`${modId} re-requests its own registry id after a reload with clean state`, async () => {
        const first = createRuntime();
        vm.runInContext(adapter, first.context);
        const firstRequest = first.window.messages.find((message) => message.command?.type === "mod_get_bundle");
        dispatchBootstrap(first.window, firstRequest, modId);
        respondMod(first.window, firstRequest, modId);
        await Promise.resolve();
        await Promise.resolve();
        assert.equal(first.window[wire.stateName].status, "executed");

        // Reload: a fresh document starts clean and asks for the SAME id.
        const second = createRuntime();
        vm.runInContext(adapter, second.context);
        const secondRequest = second.window.messages.find((message) => message.command?.type === "mod_get_bundle");
        assert.deepEqual(secondRequest.command.payload, { mod_id: modId });
        assert.notEqual(first.window.remcord, second.window.remcord);
        assert.equal(second.window[wire.stateName].status, "pending");
        assert.equal(second.window[wire.stateName].executionCount, 0);

        // The previous document's pending request is disposed, so a late host
        // answer cannot resolve anything in the new document.
        const late = first.window.remcord.native.getModBundle(modId);
        first.window.dispatchEvent(new FakeCustomEvent("pagehide"));
        await assert.rejects(late, /document was disposed/);

        dispatchBootstrap(second.window, secondRequest, modId);
        respondMod(second.window, secondRequest, modId);
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();
        assert.equal(second.window[wire.stateName].status, "executed");
        assert.equal(second.window[wire.stateName].executionCount, 1);
        // Exactly one execution in the new document: no duplicate bootstrap.
        assert.equal(second.window.messages.filter((message) => message.event?.type === wire.resultEvent).length, 1);
    });

    test(`${modId} is denied when the boot snapshot does not grant the shared capability`, () => {
        const { window, context } = createRuntime("https://discord.com", ["bridge.ping"]);
        vm.runInContext(adapter, context);
        assert.equal(
            window.messages.some((message) => message.command?.type === "mod_get_bundle"),
            false,
        );
        assert.equal(
            window.messages.some((message) => message.event?.type === wire.errorEvent),
            true,
        );
        assert.equal(window[wire.stateName].status, "failed");
        assert.match(window[wire.stateName].error, /capability is not available/);
    });

    test(`${modId} reports a typed host failure without reclassifying it`, async () => {
        const { window, context } = createRuntime();
        vm.runInContext(adapter, context);
        const request = window.messages.find((message) => message.command?.type === "mod_get_bundle");
        window.dispatchEvent(
            new FakeCustomEvent("remcord-native:response", {
                detail: {
                    version: 1,
                    request_id: request.request_id,
                    ok: false,
                    payload: null,
                    error: { code: "bundle_retrieval", message: "[shelter] JS bundle was empty" },
                },
            }),
        );
        await Promise.resolve();
        await Promise.resolve();
        assert.equal(window[wire.stateName].status, "failed");
        assert.match(window[wire.stateName].error, /JS bundle was empty/);
        // A typed host failure keeps its code instead of being reported as an
        // adapter-initialization problem.
        assert.equal(
            window.messages.some((message) => message.event?.type === wire.errorEvent),
            false,
        );
    });

    test(`${modId} does not expose the facade or request anything off the entry origins`, () => {
        for (const origin of [
            "https://evil.discord.com",
            "https://deep.sub.discord.com",
            "https://example.com",
            "https://discord.com.attacker.invalid",
        ]) {
            const { window, context } = createRuntime(origin);
            vm.runInContext(adapter, context);
            assert.equal(window.remcord, undefined, origin);
            assert.deepEqual(window.messages, [], origin);
        }
    });

    test(`${modId} initializes on every release-channel entry origin`, () => {
        for (const origin of ["https://discord.com", "https://canary.discord.com", "https://ptb.discord.com"]) {
            const { window, context } = createRuntime(origin);
            vm.runInContext(adapter, context);
            assert.ok(
                window.messages.some((message) => message.command?.type === "mod_get_bundle"),
                `${modId} on ${origin}`,
            );
        }
    });

    test(`${modId} adapter carries no artifact URL, path, source, or eval surface`, () => {
        // Assertions run on executable/configuration content only: comments
        // legitimately document what the adapter deliberately omits.
        const code = stripComments(adapter);
        // The ONLY URLs an adapter may name are the three entry origins it
        // gates on. No bundle source, no upstream host, no cache file.
        const urls = code.match(/https?:\/\/[^\s"'`)]+/g) ?? [];
        assert.deepEqual(
            urls,
            ["https://discord.com", "https://canary.discord.com", "https://ptb.discord.com"],
            `${modId} names only the entry-origin gate`,
        );
        assert.doesNotMatch(code, /shelter-builds|githubusercontent|github\.com|releases\/download/);
        assert.doesNotMatch(code, /\.css["'`]/, "an adapter names no cache file");
        assert.doesNotMatch(code, /readFile|writeFile|loadScript|executeSource|executeScript/);
        assert.doesNotMatch(code, /eval\(|new Function/);
        assert.doesNotMatch(code, /process\.|require\(|webFrame|ipcRenderer|window\.remcord\.fs/);
        // The only request it can make is the shared bundle capability.
        assert.match(code, /requireCapability|capability: "mod\.get_bundle"/);
        assert.match(code, new RegExp(`getModBundle\\("${modId}"\\)`));
    });
}

test("Shelter's descriptor needs no CSS stage and reports null CSS metadata", () => {
    const { window, context } = createRuntime();
    vm.runInContext(shelterAdapter, context);
    const request = window.messages.find((message) => message.command?.type === "mod_get_bundle");
    dispatchBootstrap(window, request, "shelter");
    respondMod(window, request, "shelter");
    // The host answers with both CSS fields null, which the facade accepts.
    const [result] = window.messages.filter((message) => message.event?.type === "mod_bootstrap_result");
    assert.equal(result.event.payload.ok, true);
    assert.equal(window.__remcordNativeShelter.status, "executed");
    // Shelter's own adapter declares no CSS vocabulary at all: there is no
    // stylesheet to inject and no empty style element to manufacture.
    assert.doesNotMatch(shelterAdapterCode, /cssResultEvent/);
    assert.doesNotMatch(shelterAdapterCode, /insertCSS|css_result/);
    // But it still READS the host's CSS metadata, which is null for this mod.
    assert.match(shelterAdapterCode, /cssBytes: result\.css_bytes/);
});

test("Custom's adapter runs the shared cycle with a content-digest version", async () => {
    // User-owned bytes carry no banner: the host versions them by content
    // digest, and the adapter validates the digest-shaped version plus its
    // own marker exactly like the download-published mods.
    const { window, context } = createRuntime();
    vm.runInContext(customAdapter, context);
    const request = window.messages.find((message) => message.command?.type === "mod_get_bundle");
    assert.deepEqual(request.command.payload, { mod_id: "custom" });
    dispatchBootstrap(window, request, "custom");
    respondMod(window, request, "custom");
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    const state = window.__remcordNativeCustom;
    assert.equal(state.status, "executed");
    assert.equal(state.marker, "custom-bootstrap-executed");
    assert.equal(state.bundleVersion, "sha256-0123456789abcdef");
    assert.equal(state.cssBytes, 512);
    // The host owns the CSS result, so the adapter declares no CSS event —
    // it reports only the bootstrap verdict and the ack, like Equicord.
    assert.doesNotMatch(customAdapterCode, /cssResultEvent/);
    assert.doesNotMatch(customAdapterCode, /_css_result/);
});

test("Custom propagates the typed not-installed failure when its files are absent", async () => {
    // Enabling `custom` in settings without dropping `custom.js`/`custom.css`
    // into the profile is an honest typed failure, not a hang and not silent
    // success: the adapter hears CUSTOM_NOT_INSTALLED from the host.
    const { window, context } = createRuntime();
    vm.runInContext(customAdapter, context);
    const request = window.messages.find((message) => message.command?.type === "mod_get_bundle");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: false,
                payload: null,
                error: {
                    code: "bundle_retrieval",
                    message: "[custom:CUSTOM_NOT_INSTALLED] custom bundles are not installed",
                },
            },
        }),
    );
    await Promise.resolve();
    await Promise.resolve();
    assert.equal(window.__remcordNativeCustom.status, "failed");
    assert.match(window.__remcordNativeCustom.error, /not installed/);
});

test("Equicord's descriptor carries a real CSS stage and stylesheet size", async () => {
    const { window, context } = createRuntime();
    vm.runInContext(equicordAdapter, context);
    const request = window.messages.find((message) => message.command?.type === "mod_get_bundle");
    dispatchBootstrap(window, request, "equicord");
    respondMod(window, request, "equicord");
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    const state = window.__remcordNativeEquicord;
    assert.equal(state.status, "executed");
    assert.equal(state.cssBytes, 217092);
    assert.equal(state.cssInjectionMs, 5);
    assert.notEqual(state.cssBytes, null);
    // The host owns the CSS result, so the adapter declares no CSS event at
    // all — Equicord reports only the bootstrap verdict and the ack.
    assert.doesNotMatch(equicordAdapterCode, /cssResultEvent/);
    assert.doesNotMatch(equicordAdapterCode, /_css_result/);
});

test("the mod_bundle response schema distinguishes no-CSS from CSS", async () => {
    // Both CSS fields null (a CSS-less mod) is accepted.
    const noCss = createRuntime();
    const noCssRequest = noCss.window.remcord.native.getModBundle("shelter");
    respondMod(noCss.window, { request_id: noCssRequest.requestId }, "shelter");
    assert.equal((await noCssRequest).css_bytes, null);

    // A half-populated pair is rejected: "no CSS" must be unambiguous.
    for (const payload of [
        { css_bytes: 100, css_injection_ms: null },
        { css_bytes: null, css_injection_ms: 3 },
        { css_bytes: -1, css_injection_ms: 3 },
        { css_bytes: 1.5, css_injection_ms: 3 },
        { css_bytes: null, css_injection_ms: -1 },
    ]) {
        const runtime = createRuntime();
        const pending = runtime.window.remcord.native.getModBundle("shelter");
        respondMod(runtime.window, { request_id: pending.requestId }, "shelter", payload);
        await assert.rejects(pending, /schema is invalid/, JSON.stringify(payload));
    }
});

test("the shared factory serves Shelter and Equicord without Vencord identifiers", () => {
    for (const code of [shelterAdapterCode, equicordAdapterCode]) {
        assert.match(code, /__remcordNativeModSubstrate/);
        assert.match(code, /substrate\.createModAdapter\(/);
        // Neither adapter reaches into Vencord's identity or state: they use
        // only the shared factory and their own names.
        assert.doesNotMatch(code, /__remcordNativeVencord/);
        assert.doesNotMatch(code, /vencord-bootstrap-executed/);
        assert.doesNotMatch(code, /getModBundle\("vencord"\)/);
        assert.doesNotMatch(code, /remcord-native-vencord/);
        assert.doesNotMatch(code, /vencord_css_result/);
    }
});

test("a synthetic second mod runs the full substrate cycle without Vencord", async () => {
    const { window, host } = createSyntheticModRuntime();
    assert.equal(window.__remcordNativeTestMod.status, "pending");
    assert.equal(window.__remcordNativeVencord, undefined);

    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:testmod-bootstrap", {
            detail: {
                request_id: "testmod-1",
                ok: true,
                marker: "testmod-bootstrap-executed",
                execution_ms: 2,
            },
        }),
    );
    host().resolve({ marker: "testmod-bootstrap-executed", bundle_version: "9.9.9" });
    await Promise.resolve();
    await Promise.resolve();
    assert.equal(window.__remcordNativeTestMod.status, "executed");
    assert.equal(window.__remcordNativeTestMod.marker, "testmod-bootstrap-executed");
    assert.equal(window.__remcordNativeTestMod.executionCount, 1);
    assert.equal(window.__remcordNativeTestMod.bundleVersion, "9.9.9");
    const result = window.messages.find((message) => message.event?.type === "testmod_bootstrap_result");
    assert.equal(result.event.payload.ok, true);
    assert.equal(result.event.payload.marker, "testmod-bootstrap-executed");
    assert.equal(
        window.messages.some((message) => message.event?.type === "testmod_response_ack"),
        true,
    );
});

test("the synthetic adapter loads once and rejects a wrong execution marker", async () => {
    const { window, context, host } = createSyntheticModRuntime();
    vm.runInContext(testModAdapter, context);
    assert.equal(window.__remcordNativeTestMod.status, "pending");

    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:testmod-bootstrap", {
            detail: { request_id: "testmod-1", ok: true, marker: "testmod-bootstrap-executed", execution_ms: 1 },
        }),
    );
    host().resolve({ marker: "wrong-marker", bundle_version: "9.9.9" });
    await Promise.resolve();
    await Promise.resolve();
    assert.equal(window.__remcordNativeTestMod.status, "failed");
    assert.equal(
        window.messages.some((message) => message.event?.type === "testmod_response_ack"),
        false,
    );
});

function createLangRuntime(capabilities = ["bridge.ping", "localization.get_lang"]) {
    return createRuntime("https://discord.com", capabilities);
}

function respondLang(window, request, value = "Go back to the Discord app", lang = "en-US") {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "localization_string", payload: { value, lang } },
                error: null,
            },
        }),
    );
}

test("the facade exposes getLang at the declared window.remcord location", () => {
    const { window } = createLangRuntime();
    assert.equal(typeof window.remcord.getLang, "function");
    assert.equal(window.remcord.native.supports("localization.get_lang"), true);
});

test("getLang sends one typed key-only request and resolves the string", async () => {
    const { window } = createLangRuntime();
    const pending = window.remcord.getLang("invite-goBackToApp");
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "localization_get_lang");
    assert.ok(message);
    assert.deepEqual(message.command, {
        type: "localization_get_lang",
        payload: { key: "invite-goBackToApp" },
    });
    respondLang(window, message);
    assert.equal(await pending, "Go back to the Discord app");
});

test("getLang resolves missing keys to an empty string like Electron", async () => {
    const { window } = createLangRuntime();
    const pending = window.remcord.getLang("remcord-native-no-such-key");
    const message = window.messages.find((entry) => entry.command?.type === "localization_get_lang");
    respondLang(window, message, "");
    assert.equal(await pending, "");
});

test("getLang throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createLangRuntime(["bridge.ping"]);
    assert.throws(
        () => window.remcord.getLang("invite-goBackToApp"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "localization_get_lang"),
        false,
    );
});

test("getLang rejects empty, non-string, and oversized keys without sending", async () => {
    const { window } = createLangRuntime();
    await assert.rejects(window.remcord.getLang(""), /non-empty string/);
    await assert.rejects(window.remcord.getLang(42), /non-empty string/);
    await assert.rejects(window.remcord.getLang("k".repeat(129)), /128 UTF-8 bytes/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "localization_get_lang"),
        false,
    );
});

test("getLang rejects malformed localization responses", async () => {
    const { window } = createLangRuntime();
    const pending = window.remcord.getLang("invite-goBackToApp");
    const message = window.messages.find((entry) => entry.command?.type === "localization_get_lang");
    respondLang(window, message, 42);
    await assert.rejects(pending, /localization response schema is invalid/);
});

test("getLang rejects host failures with the typed error", async () => {
    const { window } = createLangRuntime();
    const pending = window.remcord.getLang("invite-goBackToApp");
    const message = window.messages.find((entry) => entry.command?.type === "localization_get_lang");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: false,
                payload: null,
                error: { code: "INVALID_LANG_KEY", message: "localization key must not be empty" },
            },
        }),
    );
    await assert.rejects(pending, /must not be empty/);
});

test("getLang rejects after its timeout", async () => {
    const { window } = createLangRuntime();
    await assert.rejects(window.remcord.getLang("invite-goBackToApp", 50), /timed out/);
});

test("getLang rejects pending requests when the document is disposed", async () => {
    const { window } = createLangRuntime();
    const pending = window.remcord.getLang("invite-goBackToApp");
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh getLang facade with no stale promises", async () => {
    const firstDocument = createLangRuntime();
    const stale = firstDocument.window.remcord.getLang("invite-goBackToApp");
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createLangRuntime();
    assert.notEqual(firstDocument.window.remcord, nextDocument.window.remcord);
    const pending = nextDocument.window.remcord.getLang("invite-goBackToApp");
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "localization_get_lang");
    respondLang(nextDocument.window, message);
    assert.equal(await pending, "Go back to the Discord app");
});

function createAppRuntime(capabilities = ["bridge.ping", "app.get_version"]) {
    return createRuntime("https://discord.com", capabilities);
}

function respondAppVersion(window, request, version = "1.3.0") {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "app_version", payload: { version } },
                error: null,
            },
        }),
    );
}

test("the facade exposes an asynchronous app.get_version capability", () => {
    const { window } = createAppRuntime();
    assert.equal(typeof window.remcord.native.getVersion, "function");
    assert.equal(Object.isFrozen(window.remcord.native), true);
    assert.equal(window.remcord.native.supports("app.get_version"), true);
    // No synchronous version field or generic RPC may exist.
    assert.equal(window.remcord.version, undefined);
});

test("app.get_version sends a parameter-free typed request and resolves the version", async () => {
    const { window } = createAppRuntime();
    const pending = window.remcord.native.getVersion();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "app_get_version");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "app_get_version", payload: {} });

    respondAppVersion(window, message);
    assert.equal(await pending, "1.3.0");
});

test("app.get_version resolves the dev version exactly like Electron isDev", async () => {
    const { window } = createAppRuntime();
    const pending = window.remcord.native.getVersion();
    const message = window.messages.find((entry) => entry.command?.type === "app_get_version");
    respondAppVersion(window, message, "0.0.0");
    assert.equal(await pending, "0.0.0");
});

test("app.get_version throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createAppRuntime(["bridge.ping"]);
    assert.equal(window.remcord.native.supports("app.get_version"), false);
    assert.throws(
        () => window.remcord.native.getVersion(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "app_get_version"),
        false,
    );
});

test("app.get_version rejects malformed version responses", async () => {
    const { window } = createAppRuntime();
    const pending = window.remcord.native.getVersion();
    const message = window.messages.find((entry) => entry.command?.type === "app_get_version");
    respondAppVersion(window, message, 42);
    await assert.rejects(pending, /app version response schema is invalid/);

    const second = createAppRuntime();
    const pendingEmpty = second.window.remcord.native.getVersion();
    const messageEmpty = second.window.messages.find((entry) => entry.command?.type === "app_get_version");
    respondAppVersion(second.window, messageEmpty, "");
    await assert.rejects(pendingEmpty, /app version response schema is invalid/);
});

test("app.get_version rejects host failures with the typed error", async () => {
    const { window } = createAppRuntime();
    const pending = window.remcord.native.getVersion();
    const message = window.messages.find((entry) => entry.command?.type === "app_get_version");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: false,
                payload: null,
                error: { code: "APP_VERSION_UNAVAILABLE", message: "app version is invalid" },
            },
        }),
    );
    await assert.rejects(pending, /app version is invalid/);
});

test("app.get_version rejects after its timeout", async () => {
    const { window } = createAppRuntime();
    await assert.rejects(window.remcord.native.getVersion(50), /timed out/);
});

test("app.get_version rejects pending requests when the document is disposed", async () => {
    const { window } = createAppRuntime();
    const pending = window.remcord.native.getVersion();
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh app version facade with no stale promises", async () => {
    const firstDocument = createAppRuntime();
    const stale = firstDocument.window.remcord.native.getVersion();
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createAppRuntime();
    assert.notEqual(firstDocument.window.remcord, nextDocument.window.remcord);
    const pending = nextDocument.window.remcord.native.getVersion();
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "app_get_version");
    respondAppVersion(nextDocument.window, message);
    assert.equal(await pending, "1.3.0");
});

test("the migrated update checker compares the host-confirmed version", async () => {
    const { window } = createAppRuntime();
    // Mirrors src/shelter/settings/components/HeroUpdater.tsx: the remote
    // dotted version is compared against the capability-resolved local
    // version with dots stripped, never against a sync field.
    const pending = window.remcord.native.getVersion();
    const message = window.messages.find((entry) => entry.command?.type === "app_get_version");
    respondAppVersion(window, message, "1.3.0");
    const localVersion = (await pending).replace(/\./g, "");
    assert.equal(localVersion, "130");
    assert.equal("140" > localVersion, true);
    assert.equal("130" > localVersion, false);
});

test("the synthetic adapter reports a failed bootstrap verdict without crashing", async () => {
    const { window, host } = createSyntheticModRuntime();
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:testmod-bootstrap", {
            detail: { request_id: "testmod-1", ok: false, marker: "", execution_ms: 1, error: "global missing" },
        }),
    );
    host().resolve({ marker: "testmod-bootstrap-executed", bundle_version: "9.9.9" });
    await Promise.resolve();
    await Promise.resolve();
    assert.equal(window.__remcordNativeTestMod.status, "failed");
    const result = window.messages.find((message) => message.event?.type === "testmod_bootstrap_result");
    assert.equal(result.event.payload.ok, false);
});

test("the synthetic adapter is denied when its capability is not granted", async () => {
    const { window } = createSyntheticModRuntime(["bridge.ping"]);
    await Promise.resolve();
    assert.equal(window.__remcordNativeTestMod.status, "failed");
    assert.equal(
        window.messages.some((message) => message.event?.type === "testmod_adapter_error"),
        true,
    );
});

test("document-start storage preservation survives Discord deleting window.localStorage", () => {
    const storage = { getItem: () => null };
    const window = {
        location: { origin: "https://discord.com" },
        localStorage: storage,
    };
    function FakeStorageCtor() {}
    const context = vm.createContext({
        window,
        location: window.location,
        Window: FakeStorageCtor,
        Object,
        Storage: FakeStorageCtor,
        console: { error() {} },
    });
    vm.runInContext(storagePreservation, context);
    assert.equal(window.__remcordNativeStorageAtDocumentStart.localStorageApi, "function");
    delete window.localStorage;
    assert.equal(window.localStorage, undefined);
    assert.equal(FakeStorageCtor.prototype.localStorage, storage);
    assert.equal(typeof FakeStorageCtor.prototype.localStorage.getItem, "function");
});

function createBundlesRuntime(capabilities = ["bridge.ping", "backup.list_bundles"]) {
    return createRuntime("https://discord.com", capabilities);
}

const BUNDLE_FIXTURE = [
    { name: "vencord.js", present: true, bytes: 698272 },
    { name: "vencord.css", present: true, bytes: 41335 },
    { name: "equicord.js", present: true, bytes: 2516168 },
    { name: "equicord.css", present: true, bytes: 217092 },
    { name: "shelter.js", present: true, bytes: 235850 },
    { name: "custom.js", present: false, bytes: 0 },
    { name: "custom.css", present: false, bytes: 0 },
];

function respondBundlesList(window, request, bundles = BUNDLE_FIXTURE) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "bundles_list", payload: { bundles } },
                error: null,
            },
        }),
    );
}

test("the facade exposes backup save, inspect/commit/discard, plus discovery", () => {
    const { window } = createBundlesRuntime([
        "bridge.ping",
        "backup.list_bundles",
        "backup.save",
        "backup.inspect",
        "backup.commit_restore",
        "backup.discard_restore",
    ]);
    assert.equal(typeof window.remcord.backup.listBundles, "function");
    assert.equal(typeof window.remcord.backup.save, "function");
    assert.equal(typeof window.remcord.backup.inspect, "function");
    assert.equal(typeof window.remcord.backup.commitRestore, "function");
    assert.equal(typeof window.remcord.backup.discardRestore, "function");
    assert.equal(window.remcord.backup.restore, undefined);
    assert.equal(Object.isFrozen(window.remcord.backup), true);
    assert.equal(window.remcord.native.supports("backup.list_bundles"), true);
    assert.equal(window.remcord.native.supports("backup.save"), true);
    assert.equal(window.remcord.native.supports("backup.inspect"), true);
    assert.equal(window.remcord.native.supports("backup.commit_restore"), true);
    assert.equal(window.remcord.native.supports("backup.discard_restore"), true);
    assert.equal(window.remcord.native.supports("backup.restore"), false);
    // No generic restore/filesystem surface may ride along: the Electron
    // channel names and dialog sends stay out of the native facade.
    assert.doesNotMatch(bootstrapTemplate, /backupRestore/);
    assert.doesNotMatch(bootstrapTemplate, /showOpenDialog/);
    assert.doesNotMatch(bootstrapTemplate, /showSaveDialog/);
});

test("backup.list_bundles sends a parameter-free typed request and resolves the inventory", async () => {
    const { window } = createBundlesRuntime();
    const pending = window.remcord.backup.listBundles();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "bundles_list");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "bundles_list", payload: {} });

    respondBundlesList(window, message);
    assert.deepEqual(await pending, BUNDLE_FIXTURE);
});

test("backup.list_bundles throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createBundlesRuntime(["bridge.ping"]);
    assert.equal(window.remcord.native.supports("backup.list_bundles"), false);
    assert.throws(
        () => window.remcord.backup.listBundles(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "bundles_list"),
        false,
    );
});

test("backup.list_bundles rejects malformed inventories", async () => {
    // Wrong entry count.
    const short = createBundlesRuntime();
    const pendingShort = short.window.remcord.backup.listBundles();
    const messageShort = short.window.messages.find((entry) => entry.command?.type === "bundles_list");
    respondBundlesList(short.window, messageShort, BUNDLE_FIXTURE.slice(0, 6));
    await assert.rejects(pendingShort, /bundle inventory response schema is invalid/);

    // Wrong name or order.
    const renamed = createBundlesRuntime();
    const pendingRenamed = renamed.window.remcord.backup.listBundles();
    const messageRenamed = renamed.window.messages.find((entry) => entry.command?.type === "bundles_list");
    respondBundlesList(renamed.window, messageRenamed, [
        ...BUNDLE_FIXTURE.slice(0, 5),
        { name: "custom.css", present: false, bytes: 0 },
        { name: "custom.js", present: false, bytes: 0 },
    ]);
    await assert.rejects(pendingRenamed, /bundle inventory response schema is invalid/);

    // Absent entry carrying bytes.
    const absentBytes = createBundlesRuntime();
    const pendingAbsent = absentBytes.window.remcord.backup.listBundles();
    const messageAbsent = absentBytes.window.messages.find((entry) => entry.command?.type === "bundles_list");
    respondBundlesList(absentBytes.window, messageAbsent, [
        ...BUNDLE_FIXTURE.slice(0, 6),
        { name: "custom.css", present: false, bytes: 12 },
    ]);
    await assert.rejects(pendingAbsent, /bundle inventory response schema is invalid/);

    // Metadata-only: a path riding along is rejected.
    const withPath = createBundlesRuntime();
    const pendingPath = withPath.window.remcord.backup.listBundles();
    const messagePath = withPath.window.messages.find((entry) => entry.command?.type === "bundles_list");
    respondBundlesList(withPath.window, messagePath, [
        ...BUNDLE_FIXTURE.slice(0, 6),
        { name: "custom.css", present: false, bytes: 0, path: "C:\\secret" },
    ]);
    await assert.rejects(pendingPath, /bundle inventory response schema is invalid/);
});

test("backup.list_bundles rejects host failures with the typed error", async () => {
    const { window } = createBundlesRuntime();
    const pending = window.remcord.backup.listBundles();
    const message = window.messages.find((entry) => entry.command?.type === "bundles_list");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: false,
                payload: null,
                error: { code: "BUNDLES_UNAVAILABLE", message: "bundle inventory is unavailable" },
            },
        }),
    );
    await assert.rejects(pending, /bundle inventory is unavailable/);
});

test("backup.list_bundles rejects after its timeout", async () => {
    const { window } = createBundlesRuntime();
    await assert.rejects(window.remcord.backup.listBundles(50), /timed out/);
});

test("backup.list_bundles rejects pending requests when the document is disposed", async () => {
    const { window } = createBundlesRuntime();
    const pending = window.remcord.backup.listBundles();
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh bundle inventory facade with no stale promises", async () => {
    const firstDocument = createBundlesRuntime();
    const stale = firstDocument.window.remcord.backup.listBundles();
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createBundlesRuntime();
    assert.notEqual(firstDocument.window.remcord, nextDocument.window.remcord);
    const pending = nextDocument.window.remcord.backup.listBundles();
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "bundles_list");
    respondBundlesList(nextDocument.window, message);
    assert.deepEqual(await pending, BUNDLE_FIXTURE);
});

function createBackupSaveRuntime(capabilities = ["bridge.ping", "backup.list_bundles", "backup.save"]) {
    return createRuntime("https://discord.com", capabilities);
}

const BACKUP_SAVE_PAYLOAD = JSON.stringify({
    includes: {
        remcordConfig: true,
        remcordThemesAndQuickCss: true,
        remcordExtensionPlugins: true,
        vencordModData: true,
        equicordModData: true,
        shelterModData: true,
        modBundles: true,
    },
    clientMods: {
        vencordLocalStorage: '{"probe":1}',
        equicordLocalStorage: null,
        shelter: { plugins: {}, enabledPlugins: [] },
    },
});

function respondBackupSaved(window, request, payload = { canceled: false, bytes: 2048 }) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "backup_saved", payload },
                error: null,
            },
        }),
    );
}

test("backup.save sends the exact includes/clientMods shape with the long timeout", async () => {
    const { window } = createBackupSaveRuntime();
    const pending = window.remcord.backup.save(BACKUP_SAVE_PAYLOAD);
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "backup_save");
    assert.ok(message);
    assert.deepEqual(message.command, {
        type: "backup_save",
        payload: JSON.parse(BACKUP_SAVE_PAYLOAD),
    });

    respondBackupSaved(window, message);
    const saved = await pending;
    assert.equal(saved.ok, true);
});

test("backup.save resolves cancellation as the silent Electron shape", async () => {
    const { window } = createBackupSaveRuntime();
    const pending = window.remcord.backup.save(BACKUP_SAVE_PAYLOAD);
    const message = window.messages.find((entry) => entry.command?.type === "backup_save");
    respondBackupSaved(window, message, { canceled: true, bytes: 0 });
    const canceled = await pending;
    assert.equal(canceled.ok, false);
    assert.equal(canceled.error, "CANCELLED");
});

test("backup.save maps typed host failures to the Electron error shape", async () => {
    const { window } = createBackupSaveRuntime();
    const pending = window.remcord.backup.save(BACKUP_SAVE_PAYLOAD);
    const message = window.messages.find((entry) => entry.command?.type === "backup_save");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: false,
                payload: null,
                error: { code: "BACKUP_TOO_LARGE", message: "backup source exceeds the size limit" },
            },
        }),
    );
    assert.deepEqual(await pending.then((result) => ({ ok: result.ok, error: result.error })), {
        ok: false,
        error: "BACKUP_TOO_LARGE",
    });
});

test("backup.save rejects paths, filenames, unknown keys, and oversized envelopes", async () => {
    const { window } = createBackupSaveRuntime();
    const parsed = JSON.parse(BACKUP_SAVE_PAYLOAD);
    // Renderer-supplied output path / filename / dialog option.
    for (const extra of [{ outputPath: "C:\\x.zip" }, { filename: "x.zip" }, { defaultPath: "C:\\x" }]) {
        await assert.rejects(
            window.remcord.backup.save(JSON.stringify({ ...parsed, ...extra })),
            /exactly includes and clientMods/,
        );
    }
    // Unknown include key.
    await assert.rejects(
        window.remcord.backup.save(JSON.stringify({ ...parsed, includes: { ...parsed.includes, everything: true } })),
        /seven known booleans/,
    );
    // Non-boolean include.
    await assert.rejects(
        window.remcord.backup.save(JSON.stringify({ ...parsed, includes: { ...parsed.includes, modBundles: "yes" } })),
        /seven known booleans/,
    );
    // Unknown clientMods field.
    await assert.rejects(
        window.remcord.backup.save(JSON.stringify({ ...parsed, clientMods: { discordToken: "x" } })),
        /unknown field/,
    );
    // Oversized client string.
    await assert.rejects(
        window.remcord.backup.save(
            JSON.stringify({
                ...parsed,
                clientMods: { vencordLocalStorage: "x".repeat(1048577) },
            }),
        ),
        /at most 1048576/,
    );
    // Non-string payload / invalid JSON.
    await assert.rejects(window.remcord.backup.save(42), /JSON string/);
    await assert.rejects(window.remcord.backup.save("{not json"), /valid JSON/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "backup_save"),
        false,
    );
});

test("backup.save throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createBackupSaveRuntime(["bridge.ping", "backup.list_bundles"]);
    assert.equal(window.remcord.native.supports("backup.save"), false);
    assert.throws(
        () => window.remcord.backup.save(BACKUP_SAVE_PAYLOAD),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "backup_save"),
        false,
    );
});

test("backup.save surfaces malformed save outcomes as the Electron error shape", async () => {
    // A path riding along with the outcome.
    const withPath = createBackupSaveRuntime();
    const pendingPath = withPath.window.remcord.backup.save(BACKUP_SAVE_PAYLOAD);
    const messagePath = withPath.window.messages.find((entry) => entry.command?.type === "backup_save");
    respondBackupSaved(withPath.window, messagePath, { canceled: false, bytes: 12, path: "C:\\x.zip" });
    const malformedPath = await pendingPath;
    assert.equal(malformedPath.ok, false);
    assert.equal(malformedPath.error, "UNKNOWN");

    // Negative bytes.
    const negative = createBackupSaveRuntime();
    const pendingNegative = negative.window.remcord.backup.save(BACKUP_SAVE_PAYLOAD);
    const messageNegative = negative.window.messages.find((entry) => entry.command?.type === "backup_save");
    respondBackupSaved(negative.window, messageNegative, { canceled: false, bytes: -1 });
    const malformedNegative = await pendingNegative;
    assert.equal(malformedNegative.ok, false);
    assert.equal(malformedNegative.error, "UNKNOWN");
});

test("backup.save rejects pending requests when the document is disposed", async () => {
    const { window } = createBackupSaveRuntime();
    const pending = window.remcord.backup.save(BACKUP_SAVE_PAYLOAD);
    // Cancellation-by-disposal maps through the Electron error shape, never
    // an unhandled rejection: the pending promise settles as ok:false.
    const rejection = pending.then((result) => {
        assert.equal(result.ok, false);
    });
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

function createBackupRestoreRuntime(
    capabilities = ["bridge.ping", "backup.inspect", "backup.commit_restore", "backup.discard_restore"],
) {
    return createRuntime("https://discord.com", capabilities);
}

const BACKUP_RESTORE_ENVELOPE = {
    vencordLocalStorage: '{"probe":1}',
    equicordLocalStorage: null,
    shelter: { plugins: {}, enabledPlugins: [] },
};

const BACKUP_PREVIEW_PAYLOAD = {
    canceled: false,
    includes: {
        remcordConfig: true,
        remcordThemesAndQuickCss: true,
        remcordExtensionPlugins: true,
        vencordModData: true,
        equicordModData: false,
        shelterModData: false,
        modBundles: false,
    },
    archive_bytes: 100,
    uncompressed_bytes: 200,
    entry_count: 2,
    client_mods_bytes: 10,
    has_vencord_mod_data: true,
    has_equicord_mod_data: false,
    has_shelter_mod_data: false,
    entries: ["data/quickCss.css", "data/themes/t/src.css"],
    entries_truncated: false,
};

function respondBackupInspected(window, request, payload = BACKUP_PREVIEW_PAYLOAD) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "backup_inspected", payload },
                error: null,
            },
        }),
    );
}

function respondBackupRestored(window, request, payload = { canceled: false, client_mods: BACKUP_RESTORE_ENVELOPE }) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "backup_restored", payload },
                error: null,
            },
        }),
    );
}

function respondBackupDiscarded(window, request, payload = { discarded: true }) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "backup_restore_discarded", payload },
                error: null,
            },
        }),
    );
}

test("backup.inspect sends a parameter-free typed request and resolves the preview", async () => {
    const { window } = createBackupRestoreRuntime();
    const pending = window.remcord.backup.inspect();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "backup_inspect");
    assert.ok(message);
    // No archive path, filename, URL, filter, or dialog option may ride along.
    assert.deepEqual(message.command, { type: "backup_inspect", payload: {} });

    respondBackupInspected(window, message);
    const inspected = await pending;
    assert.equal(inspected.ok, true);
    assert.equal(inspected.preview.entry_count, 2);
    assert.equal(inspected.preview.entries.length, 2);
    assert.equal(inspected.preview.entries[0], "data/quickCss.css");
    assert.equal(inspected.preview.entries[1], "data/themes/t/src.css");
    assert.equal(inspected.preview.includes.remcordConfig, true);
    assert.equal(inspected.preview.has_vencord_mod_data, true);
});

test("backup.inspect resolves cancellation as the silent Electron shape", async () => {
    const { window } = createBackupRestoreRuntime();
    const pending = window.remcord.backup.inspect();
    const message = window.messages.find((entry) => entry.command?.type === "backup_inspect");
    respondBackupInspected(window, message, {
        canceled: true,
        includes: BACKUP_PREVIEW_PAYLOAD.includes,
        archive_bytes: 0,
        uncompressed_bytes: 0,
        entry_count: 0,
        client_mods_bytes: 0,
        has_vencord_mod_data: false,
        has_equicord_mod_data: false,
        has_shelter_mod_data: false,
        entries: [],
        entries_truncated: false,
    });
    const canceled = await pending;
    assert.equal(canceled.ok, false);
    assert.equal(canceled.error, "CANCELLED");
});

test("backup.inspect maps typed host failures to the Electron error shape", async () => {
    const { window } = createBackupRestoreRuntime();
    const pending = window.remcord.backup.inspect();
    const message = window.messages.find((entry) => entry.command?.type === "backup_inspect");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: false,
                payload: null,
                error: { code: "INVALID_ARCHIVE", message: "backup archive is invalid" },
            },
        }),
    );
    assert.deepEqual(await pending.then((result) => ({ ok: result.ok, error: result.error })), {
        ok: false,
        error: "INVALID_ARCHIVE",
    });
});

test("backup.inspect throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createBackupRestoreRuntime(["bridge.ping", "backup.commit_restore"]);
    assert.equal(window.remcord.native.supports("backup.inspect"), false);
    assert.throws(
        () => window.remcord.backup.inspect(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "backup_inspect"),
        false,
    );
});

test("backup.inspect surfaces malformed previews without paths", async () => {
    // Like the rest of the backup facade, schema violations resolve the
    // shared { ok: false, error } shape (UNKNOWN for a failed schema —
    // never a path, never a rejection).
    // A path riding along with the preview.
    const withPath = createBackupRestoreRuntime();
    const pendingPath = withPath.window.remcord.backup.inspect();
    const messagePath = withPath.window.messages.find((entry) => entry.command?.type === "backup_inspect");
    respondBackupInspected(withPath.window, messagePath, { ...BACKUP_PREVIEW_PAYLOAD, path: "C:\\x.zip" });
    assert.deepEqual(await pendingPath.then((result) => ({ ok: result.ok, error: result.error })), {
        ok: false,
        error: "UNKNOWN",
    });

    // An over-long listing the host should have truncated itself.
    const longListing = createBackupRestoreRuntime();
    const pendingLong = longListing.window.remcord.backup.inspect();
    const messageLong = longListing.window.messages.find((entry) => entry.command?.type === "backup_inspect");
    respondBackupInspected(longListing.window, messageLong, {
        ...BACKUP_PREVIEW_PAYLOAD,
        entries: new Array(129).fill("data/x.css"),
    });
    assert.deepEqual(await pendingLong.then((result) => ({ ok: result.ok, error: result.error })), {
        ok: false,
        error: "UNKNOWN",
    });

    // An unknown include key.
    const unknownKey = createBackupRestoreRuntime();
    const pendingUnknown = unknownKey.window.remcord.backup.inspect();
    const messageUnknown = unknownKey.window.messages.find((entry) => entry.command?.type === "backup_inspect");
    respondBackupInspected(unknownKey.window, messageUnknown, {
        ...BACKUP_PREVIEW_PAYLOAD,
        includes: { ...BACKUP_PREVIEW_PAYLOAD.includes, evil: true },
    });
    assert.deepEqual(await pendingUnknown.then((result) => ({ ok: result.ok, error: result.error })), {
        ok: false,
        error: "UNKNOWN",
    });
});

test("backup.commit_restore sends a parameter-free typed request and resolves the envelope", async () => {
    const { window } = createBackupRestoreRuntime();
    const pending = window.remcord.backup.commitRestore();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "backup_commit_restore");
    assert.ok(message);
    // No archive path, bytes, manifest, or selector may ride along: the
    // commit applies exactly the held staged bytes.
    assert.deepEqual(message.command, { type: "backup_commit_restore", payload: {} });

    respondBackupRestored(window, message);
    const restored = await pending;
    assert.equal(restored.ok, true);
    assert.deepEqual(restored.clientMods, BACKUP_RESTORE_ENVELOPE);
});

test("backup.commit_restore maps a missing hold to the typed error shape", async () => {
    const { window } = createBackupRestoreRuntime();
    const pending = window.remcord.backup.commitRestore();
    const message = window.messages.find((entry) => entry.command?.type === "backup_commit_restore");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: false,
                payload: null,
                error: { code: "NO_STAGED_RESTORE", message: "no staged restore is held" },
            },
        }),
    );
    assert.deepEqual(await pending.then((result) => ({ ok: result.ok, error: result.error })), {
        ok: false,
        error: "NO_STAGED_RESTORE",
    });
});

test("backup.commit_restore throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createBackupRestoreRuntime(["bridge.ping", "backup.inspect"]);
    assert.equal(window.remcord.native.supports("backup.commit_restore"), false);
    assert.throws(
        () => window.remcord.backup.commitRestore(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "backup_commit_restore"),
        false,
    );
});

test("backup.commit_restore surfaces malformed commit outcomes without paths", async () => {
    // A path riding along with the outcome.
    const withPath = createBackupRestoreRuntime();
    const pendingPath = withPath.window.remcord.backup.commitRestore();
    const messagePath = withPath.window.messages.find((entry) => entry.command?.type === "backup_commit_restore");
    respondBackupRestored(withPath.window, messagePath, {
        canceled: false,
        client_mods: BACKUP_RESTORE_ENVELOPE,
        path: "C:\\x.zip",
    });
    const malformedPath = await pendingPath;
    assert.equal(malformedPath.ok, false);
    assert.equal(malformedPath.error, "UNKNOWN");

    // An unknown clientMods key: the dispatcher rejects the schema before
    // the facade ever sees it.
    const unknownKey = createBackupRestoreRuntime();
    const pendingUnknown = unknownKey.window.remcord.backup.commitRestore();
    const messageUnknown = unknownKey.window.messages.find((entry) => entry.command?.type === "backup_commit_restore");
    respondBackupRestored(unknownKey.window, messageUnknown, {
        canceled: false,
        client_mods: { discordToken: "x" },
    });
    const malformedUnknown = await pendingUnknown;
    assert.equal(malformedUnknown.ok, false);
    assert.equal(malformedUnknown.error, "UNKNOWN");
});

test("backup.discard_restore sends a parameter-free typed request and resolves the ack", async () => {
    const { window } = createBackupRestoreRuntime();
    const pending = window.remcord.backup.discardRestore();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "backup_discard_restore");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "backup_discard_restore", payload: {} });

    respondBackupDiscarded(window, message, { discarded: true });
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), { discarded: true });

    // Discarding with nothing held resolves false, never an error.
    const second = createBackupRestoreRuntime();
    const pendingSecond = second.window.remcord.backup.discardRestore();
    const messageSecond = second.window.messages.find((entry) => entry.command?.type === "backup_discard_restore");
    respondBackupDiscarded(second.window, messageSecond, { discarded: false });
    assert.deepEqual(JSON.parse(JSON.stringify(await pendingSecond)), { discarded: false });
});

test("backup.discard_restore throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createBackupRestoreRuntime(["bridge.ping", "backup.inspect"]);
    assert.equal(window.remcord.native.supports("backup.discard_restore"), false);
    assert.throws(
        () => window.remcord.backup.discardRestore(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "backup_discard_restore"),
        false,
    );
});

test("backup.discard_restore rejects malformed acks", async () => {
    const { window } = createBackupRestoreRuntime();
    const pending = window.remcord.backup.discardRestore();
    const message = window.messages.find((entry) => entry.command?.type === "backup_discard_restore");
    respondBackupDiscarded(window, message, { discarded: true, path: "C:\\x" });
    await assert.rejects(pending, /backup discard response schema is invalid/);
});

test("backup trio maps timeouts through the Electron error shape", async () => {
    // Like save, no trio member rejects on timeout: the shared pending map
    // rejects with REQUEST_TIMEOUT and each facade maps it to the resolved
    // { ok: false, error } shape (transport misuse still throws
    // synchronously before dispatch).
    const { window } = createBackupRestoreRuntime();
    const inspectTimeout = await window.remcord.backup.inspect(50);
    assert.equal(inspectTimeout.ok, false);
    assert.equal(inspectTimeout.error, "REQUEST_TIMEOUT");
    const commitTimeout = await window.remcord.backup.commitRestore(50);
    assert.equal(commitTimeout.ok, false);
    assert.equal(commitTimeout.error, "REQUEST_TIMEOUT");
});

test("backup trio rejects pending requests when the document is disposed", async () => {
    const { window } = createBackupRestoreRuntime();
    const pendingInspect = window.remcord.backup.inspect();
    const pendingCommit = window.remcord.backup.commitRestore();
    const pendingDiscard = window.remcord.backup.discardRestore();
    // Cancellation-by-disposal maps through the resolved error shape, never
    // an unhandled rejection: each pending promise settles as ok:false
    // (inspect/commit) or rejects honestly (discard, which has no Electron
    // error shape of its own).
    const settled = Promise.all([
        pendingInspect.then((result) => assert.equal(result.ok, false)),
        pendingCommit.then((result) => assert.equal(result.ok, false)),
        assert.rejects(pendingDiscard, /document was disposed/),
    ]);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await settled;
});

function createFoldersRuntime(capabilities = ["bridge.ping", "folders.open", "folders.open_theme"]) {
    return createRuntime("https://discord.com", capabilities);
}

function respondFoldersOpened(window, request, location = "themes") {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "folders_opened", payload: { location } },
                error: null,
            },
        }),
    );
}

function respondThemeFolderOpened(window, request, id = "my-theme") {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "theme_folder_opened", payload: { id } },
                error: null,
            },
        }),
    );
}

test("the facade exposes scoped folder reveal and theme open capabilities", () => {
    const { window } = createFoldersRuntime();
    assert.equal(typeof window.remcord.folders.open, "function");
    assert.equal(typeof window.remcord.folders.openTheme, "function");
    assert.equal(Object.isFrozen(window.remcord.folders), true);
    assert.equal(window.remcord.native.supports("folders.open"), true);
    assert.equal(window.remcord.native.supports("folders.open_theme"), true);
    // No generic path opener, dialog, picker, or shell surface may exist.
    assert.equal(window.remcord.folders.openPath, undefined);
    assert.equal(window.remcord.folders.openDialog, undefined);
    assert.equal(window.remcord.folders.shell, undefined);
});

test("folders.open sends a closed-enum reveal request and resolves the location", async () => {
    const { window } = createFoldersRuntime();
    for (const location of ["storage", "themes", "plugins", "gallery"]) {
        const pending = window.remcord.folders.open(location);
        assert.ok(pending instanceof Promise);
        const message = window.messages.find((entry) => entry.command?.type === "folders_open");
        assert.ok(message);
        assert.deepEqual(message.command, { type: "folders_open", payload: { location } });

        respondFoldersOpened(window, message, location);
        assert.equal(await pending, location);
        window.messages.length = 0;
    }
});

test("folders.openTheme sends an id-only open request and resolves the id", async () => {
    const { window } = createFoldersRuntime();
    const pending = window.remcord.folders.openTheme("my-theme");
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "folders_open_theme");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "folders_open_theme", payload: { id: "my-theme" } });

    respondThemeFolderOpened(window, message, "my-theme");
    assert.equal(await pending, "my-theme");
});

test("folder capabilities throw UNSUPPORTED_CAPABILITY when the snapshot denies them", () => {
    const { window } = createFoldersRuntime(["bridge.ping"]);
    assert.equal(window.remcord.native.supports("folders.open"), false);
    assert.equal(window.remcord.native.supports("folders.open_theme"), false);
    assert.throws(
        () => window.remcord.folders.open("themes"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.throws(
        () => window.remcord.folders.openTheme("my-theme"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "folders_open"),
        false,
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "folders_open_theme"),
        false,
    );
});

test("folder requests reject non-enum locations and hostile ids without sending", async () => {
    const { window } = createFoldersRuntime();
    for (const location of ["extensions", "Themes", "", "C:\\themes", "../themes", 42, null]) {
        await assert.rejects(window.remcord.folders.open(location), /location must be one of/);
    }
    for (const id of ["", ".", "..", "../escape", "..\\escape", "a/b", "a\\b", "C:\\x", "has space", 42, null]) {
        await assert.rejects(window.remcord.folders.openTheme(id), /id must match/);
    }
    const oversized = `t${"t".repeat(64)}`;
    await assert.rejects(window.remcord.folders.openTheme(oversized), /id must match/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "folders_open"),
        false,
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "folders_open_theme"),
        false,
    );
});

test("folder responses reject mismatched echoes and path-bearing members", async () => {
    // Host echoes a different location than requested.
    const mismatched = createFoldersRuntime();
    const pendingMismatch = mismatched.window.remcord.folders.open("themes");
    const messageMismatch = mismatched.window.messages.find((entry) => entry.command?.type === "folders_open");
    respondFoldersOpened(mismatched.window, messageMismatch, "storage");
    await assert.rejects(pendingMismatch, /folder response schema is invalid/);

    // Unknown location in a well-formed envelope.
    const unknown = createFoldersRuntime();
    const pendingUnknown = unknown.window.remcord.folders.open("themes");
    const messageUnknown = unknown.window.messages.find((entry) => entry.command?.type === "folders_open");
    respondFoldersOpened(unknown.window, messageUnknown, "extensions");
    await assert.rejects(pendingUnknown, /folder response schema is invalid/);

    // Path riding along with a theme open response.
    const withPath = createFoldersRuntime();
    const pendingPath = withPath.window.remcord.folders.openTheme("my-theme");
    const messagePath = withPath.window.messages.find((entry) => entry.command?.type === "folders_open_theme");
    withPath.window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: messagePath.request_id,
                ok: true,
                payload: { type: "theme_folder_opened", payload: { id: "my-theme", path: "C:\\secret" } },
                error: null,
            },
        }),
    );
    await assert.rejects(pendingPath, /theme folder response schema is invalid/);

    // Host echoes a different id than requested.
    const wrongId = createFoldersRuntime();
    const pendingId = wrongId.window.remcord.folders.openTheme("my-theme");
    const messageId = wrongId.window.messages.find((entry) => entry.command?.type === "folders_open_theme");
    respondThemeFolderOpened(wrongId.window, messageId, "other-theme");
    await assert.rejects(pendingId, /theme folder response schema is invalid/);
});

test("folder requests reject host failures with the typed error", async () => {
    const { window } = createFoldersRuntime();
    const pendingReveal = window.remcord.folders.open("themes");
    const messageReveal = window.messages.find((entry) => entry.command?.type === "folders_open");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: messageReveal.request_id,
                ok: false,
                payload: null,
                error: { code: "REVEAL_FAILED", message: "folder reveal failed" },
            },
        }),
    );
    await assert.rejects(pendingReveal, /folder reveal failed/);

    const pendingOpen = window.remcord.folders.openTheme("my-theme");
    const messageOpen = window.messages.find((entry) => entry.command?.type === "folders_open_theme");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: messageOpen.request_id,
                ok: false,
                payload: null,
                error: { code: "UNKNOWN_THEME", message: "theme folder is unknown" },
            },
        }),
    );
    await assert.rejects(pendingOpen, /theme folder is unknown/);
});

test("folder requests reject after their timeout", async () => {
    const { window } = createFoldersRuntime();
    await assert.rejects(window.remcord.folders.open("themes", 50), /timed out/);
    await assert.rejects(window.remcord.folders.openTheme("my-theme", 50), /timed out/);
});

test("folder requests reject pending requests when the document is disposed", async () => {
    const { window } = createFoldersRuntime();
    const pendingReveal = window.remcord.folders.open("themes");
    const pendingOpen = window.remcord.folders.openTheme("my-theme");
    const rejections = Promise.all([
        assert.rejects(pendingReveal, /document was disposed/),
        assert.rejects(pendingOpen, /document was disposed/),
    ]);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejections;
});

test("a reloaded document gets a fresh folders facade with no stale promises", async () => {
    const firstDocument = createFoldersRuntime();
    const stale = firstDocument.window.remcord.folders.open("themes");
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createFoldersRuntime();
    assert.notEqual(firstDocument.window.remcord, nextDocument.window.remcord);
    const pending = nextDocument.window.remcord.folders.openTheme("my-theme");
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "folders_open_theme");
    respondThemeFolderOpened(nextDocument.window, message, "my-theme");
    assert.equal(await pending, "my-theme");
});

function createThemesRuntime(capabilities = ["bridge.ping", "themes.install_code", "themes.write_css"]) {
    return createRuntime("https://discord.com", capabilities);
}

const INSTALL_FIXTURE_CSS = "/**\n * @name Midnight Harbor\n * @author Harbor Line\n */\n:root {\n  --bg: #000;\n}\n";

function respondThemeInstalled(window, request, payload = { id: "Midnight-Harbor-BD", enabled: false }) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: {
                    type: "theme_installed",
                    payload: { ...payload, css_bytes: INSTALL_FIXTURE_CSS.length },
                },
                error: null,
            },
        }),
    );
}

test("the facade exposes a CSS-text-only themes.install_code capability", () => {
    const { window } = createThemesRuntime();
    assert.equal(typeof window.remcord.themes.installFromCode, "function");
    assert.equal(typeof window.remcord.themes.writeCss, "function");
    assert.equal(Object.isFrozen(window.remcord.themes), true);
    assert.equal(window.remcord.native.supports("themes.install_code"), true);
    assert.equal(window.remcord.native.supports("themes.write_css"), true);
    // No URL install, Electron sync setters, dialog, or picker surface may
    // exist. getThemes is the one closed synchronous exception (settings
    // shim): it returns only the frozen snapshot projection, never a host
    // operation.
    assert.equal(window.remcord.themes.install, undefined);
    assert.equal(typeof window.remcord.themes.getThemes, "function");
    assert.equal(window.remcord.themes.set, undefined);
    assert.equal(window.remcord.themes.edit, undefined);
    assert.equal(window.remcord.themes.openImportPicker, undefined);
});

test("themes.install_code sends CSS text only and resolves bounded metadata", async () => {
    const { window } = createThemesRuntime();
    const pending = window.remcord.themes.installFromCode(INSTALL_FIXTURE_CSS);
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "theme_install_code");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "theme_install_code", payload: { css: INSTALL_FIXTURE_CSS } });

    respondThemeInstalled(window, message);
    assert.deepEqual(await pending, {
        id: "Midnight-Harbor-BD",
        enabled: false,
        css_bytes: INSTALL_FIXTURE_CSS.length,
    });
});

test("themes.install_code throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createThemesRuntime(["bridge.ping"]);
    assert.equal(window.remcord.native.supports("themes.install_code"), false);
    assert.throws(
        () => window.remcord.themes.installFromCode(INSTALL_FIXTURE_CSS),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "theme_install_code"),
        false,
    );
});

test("themes.install_code rejects non-string, empty, and oversized CSS without sending", async () => {
    const { window } = createThemesRuntime();
    await assert.rejects(window.remcord.themes.installFromCode(42), /css must be a non-empty string/);
    await assert.rejects(window.remcord.themes.installFromCode(""), /css must be a non-empty string/);
    await assert.rejects(window.remcord.themes.installFromCode("x".repeat(1048577)), /css must be a non-empty string/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "theme_install_code"),
        false,
    );
});

test("themes.install_code rejects malformed install responses", async () => {
    // Enabled must stay false by Electron parity.
    const enabled = createThemesRuntime();
    const pendingEnabled = enabled.window.remcord.themes.installFromCode(INSTALL_FIXTURE_CSS);
    const messageEnabled = enabled.window.messages.find((entry) => entry.command?.type === "theme_install_code");
    respondThemeInstalled(enabled.window, messageEnabled, { id: "Midnight-Harbor-BD", enabled: true });
    await assert.rejects(pendingEnabled, /theme install response schema is invalid/);

    // The id must be host-derived (<name>-BD): a foreign id is rejected.
    const foreign = createThemesRuntime();
    const pendingForeign = foreign.window.remcord.themes.installFromCode(INSTALL_FIXTURE_CSS);
    const messageForeign = foreign.window.messages.find((entry) => entry.command?.type === "theme_install_code");
    respondThemeInstalled(foreign.window, messageForeign, { id: "other", enabled: false });
    await assert.rejects(pendingForeign, /theme install response schema is invalid/);

    // No path, URL, source, or extra member may ride along.
    const withPath = createThemesRuntime();
    const pendingPath = withPath.window.remcord.themes.installFromCode(INSTALL_FIXTURE_CSS);
    const messagePath = withPath.window.messages.find((entry) => entry.command?.type === "theme_install_code");
    withPath.window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: messagePath.request_id,
                ok: true,
                payload: {
                    type: "theme_installed",
                    payload: {
                        id: "Midnight-Harbor-BD",
                        enabled: false,
                        css_bytes: INSTALL_FIXTURE_CSS.length,
                        path: "C:\\themes\\x",
                    },
                },
                error: null,
            },
        }),
    );
    await assert.rejects(pendingPath, /theme install response schema is invalid/);
});

test("themes.install_code rejects host failures with the typed error", async () => {
    const { window } = createThemesRuntime();
    const pending = window.remcord.themes.installFromCode(INSTALL_FIXTURE_CSS);
    const message = window.messages.find((entry) => entry.command?.type === "theme_install_code");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: false,
                payload: null,
                error: { code: "INVALID_NAME", message: "theme name is missing or invalid" },
            },
        }),
    );
    await assert.rejects(pending, /theme name is missing or invalid/);
});

test("themes.install_code rejects after its timeout", async () => {
    const { window } = createThemesRuntime();
    await assert.rejects(window.remcord.themes.installFromCode(INSTALL_FIXTURE_CSS, 50), /timed out/);
});

test("themes.install_code rejects pending requests when the document is disposed", async () => {
    const { window } = createThemesRuntime();
    const pending = window.remcord.themes.installFromCode(INSTALL_FIXTURE_CSS);
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh theme install facade with no stale promises", async () => {
    const firstDocument = createThemesRuntime();
    const stale = firstDocument.window.remcord.themes.installFromCode(INSTALL_FIXTURE_CSS);
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createThemesRuntime();
    assert.notEqual(firstDocument.window.remcord, nextDocument.window.remcord);
    const pending = nextDocument.window.remcord.themes.installFromCode(INSTALL_FIXTURE_CSS);
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "theme_install_code");
    respondThemeInstalled(nextDocument.window, message);
    assert.deepEqual(await pending, {
        id: "Midnight-Harbor-BD",
        enabled: false,
        css_bytes: INSTALL_FIXTURE_CSS.length,
    });
});

const WRITE_FIXTURE_CSS = ":root{\n  --remcord-native-write-probe: second;\n}\n";

function respondThemeWritten(window, request, payload = { id: "my-theme", enabled: true }) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: {
                    type: "theme_written",
                    payload: { ...payload, css_bytes: WRITE_FIXTURE_CSS.length },
                },
                error: null,
            },
        }),
    );
}

test("themes.write_css sends id plus CSS text and resolves bounded metadata", async () => {
    const { window } = createThemesRuntime();
    const pending = window.remcord.themes.writeCss("my-theme", WRITE_FIXTURE_CSS);
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "theme_write_css");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "theme_write_css", payload: { id: "my-theme", css: WRITE_FIXTURE_CSS } });

    respondThemeWritten(window, message);
    assert.deepEqual(await pending, {
        id: "my-theme",
        enabled: true,
        css_bytes: WRITE_FIXTURE_CSS.length,
    });
});

test("themes.write_css throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createThemesRuntime(["bridge.ping"]);
    assert.equal(window.remcord.native.supports("themes.write_css"), false);
    assert.throws(
        () => window.remcord.themes.writeCss("my-theme", WRITE_FIXTURE_CSS),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "theme_write_css"),
        false,
    );
});

test("themes.write_css rejects hostile ids and oversized CSS without sending", async () => {
    const { window } = createThemesRuntime();
    for (const id of ["", ".", "..", "../escape", "..\\escape", "a/b", "C:\\x", 42, null]) {
        await assert.rejects(window.remcord.themes.writeCss(id, WRITE_FIXTURE_CSS), /id must match/);
    }
    const oversized = `t${"t".repeat(64)}`;
    await assert.rejects(window.remcord.themes.writeCss(oversized, WRITE_FIXTURE_CSS), /id must match/);
    await assert.rejects(window.remcord.themes.writeCss("my-theme", 42), /css must be a string/);
    await assert.rejects(window.remcord.themes.writeCss("my-theme", "x".repeat(1048577)), /css must be a string/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "theme_write_css"),
        false,
    );
});

test("themes.write_css rejects mismatched echoes and foreign members", async () => {
    // Host echoes a different id than requested.
    const mismatched = createThemesRuntime();
    const pendingMismatch = mismatched.window.remcord.themes.writeCss("my-theme", WRITE_FIXTURE_CSS);
    const messageMismatch = mismatched.window.messages.find((entry) => entry.command?.type === "theme_write_css");
    respondThemeWritten(mismatched.window, messageMismatch, { id: "other-theme", enabled: true });
    await assert.rejects(pendingMismatch, /theme write response schema is invalid/);

    // A path riding along is rejected.
    const withPath = createThemesRuntime();
    const pendingPath = withPath.window.remcord.themes.writeCss("my-theme", WRITE_FIXTURE_CSS);
    const messagePath = withPath.window.messages.find((entry) => entry.command?.type === "theme_write_css");
    withPath.window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: messagePath.request_id,
                ok: true,
                payload: {
                    type: "theme_written",
                    payload: {
                        id: "my-theme",
                        enabled: true,
                        css_bytes: WRITE_FIXTURE_CSS.length,
                        path: "C:\\themes\\x",
                    },
                },
                error: null,
            },
        }),
    );
    await assert.rejects(pendingPath, /theme write response schema is invalid/);

    // Enabled must be a real boolean.
    const noBool = createThemesRuntime();
    const pendingBool = noBool.window.remcord.themes.writeCss("my-theme", WRITE_FIXTURE_CSS);
    const messageBool = noBool.window.messages.find((entry) => entry.command?.type === "theme_write_css");
    noBool.window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: messageBool.request_id,
                ok: true,
                payload: {
                    type: "theme_written",
                    payload: { id: "my-theme", enabled: "yes", css_bytes: WRITE_FIXTURE_CSS.length },
                },
                error: null,
            },
        }),
    );
    await assert.rejects(pendingBool, /theme write response schema is invalid/);
});

test("themes.write_css rejects host failures with the typed error", async () => {
    const { window } = createThemesRuntime();
    const pending = window.remcord.themes.writeCss("my-theme", WRITE_FIXTURE_CSS);
    const message = window.messages.find((entry) => entry.command?.type === "theme_write_css");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: false,
                payload: null,
                error: { code: "UNKNOWN_THEME", message: "theme folder is unknown" },
            },
        }),
    );
    await assert.rejects(pending, /theme folder is unknown/);
});

test("themes.write_css rejects after its timeout", async () => {
    const { window } = createThemesRuntime();
    await assert.rejects(window.remcord.themes.writeCss("my-theme", WRITE_FIXTURE_CSS, 50), /timed out/);
});

test("themes.write_css rejects pending requests when the document is disposed", async () => {
    const { window } = createThemesRuntime();
    const pending = window.remcord.themes.writeCss("my-theme", WRITE_FIXTURE_CSS);
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh theme write facade with no stale promises", async () => {
    const firstDocument = createThemesRuntime();
    const stale = firstDocument.window.remcord.themes.writeCss("my-theme", WRITE_FIXTURE_CSS);
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createThemesRuntime();
    assert.notEqual(firstDocument.window.remcord, nextDocument.window.remcord);
    const pending = nextDocument.window.remcord.themes.writeCss("my-theme", WRITE_FIXTURE_CSS);
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "theme_write_css");
    respondThemeWritten(nextDocument.window, message);
    assert.deepEqual(await pending, {
        id: "my-theme",
        enabled: true,
        css_bytes: WRITE_FIXTURE_CSS.length,
    });
});

test("theme requests stay inert off the trusted origin", async () => {
    const { window } = createRuntime("https://attacker.invalid", [
        "bridge.ping",
        "themes.install_code",
        "themes.write_css",
    ]);
    assert.equal(window.remcord, undefined);
});

function createImportRuntime(capabilities = ["bridge.ping", "themes.import_files"]) {
    return createRuntime("https://discord.com", capabilities);
}

function respondThemeImported(window, request, payload = { canceled: false, imported: 1, failed: 0 }) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "theme_imported", payload },
                error: null,
            },
        }),
    );
}

test("the facade exposes a parameter-free themes.import_files capability", () => {
    const { window } = createImportRuntime();
    assert.equal(typeof window.remcord.themes.importFiles, "function");
    assert.equal(Object.isFrozen(window.remcord.themes), true);
    assert.equal(window.remcord.native.supports("themes.import_files"), true);
    // No generic dialog, filesystem, path, or URL surface may exist.
    assert.equal(window.remcord.themes.openImportPicker, undefined);
    assert.equal(window.remcord.dialog, undefined);
    assert.equal(window.remcord.fs, undefined);
    assert.equal(window.remcord.themes.openFile, undefined);
    assert.equal(window.remcord.themes.readFile, undefined);
});

test("themes.import_files sends an exact empty payload and resolves the bounded summary", async () => {
    const { window } = createImportRuntime();
    const pending = window.remcord.themes.importFiles();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "theme_import_files");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "theme_import_files", payload: {} });

    respondThemeImported(window, message, { canceled: false, imported: 2, failed: 1 });
    assert.deepEqual(await pending, { canceled: false, imported: 2, failed: 1 });
});

test("themes.import_files resolves cancellation as control flow, never a rejection", async () => {
    const { window } = createImportRuntime();
    const pending = window.remcord.themes.importFiles();
    const message = window.messages.find((entry) => entry.command?.type === "theme_import_files");
    // A dismissed picker resolves { canceled: true }: no rejection, so no
    // unhandled rejection, error toast, or console error can follow.
    respondThemeImported(window, message, { canceled: true, imported: 0, failed: 0 });
    assert.deepEqual(await pending, { canceled: true, imported: 0, failed: 0 });
});

test("themes.import_files throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createImportRuntime(["bridge.ping"]);
    assert.equal(window.remcord.native.supports("themes.import_files"), false);
    assert.throws(
        () => window.remcord.themes.importFiles(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "theme_import_files"),
        false,
    );
});

test("themes.import_files rejects malformed summaries without path leakage", async () => {
    // Non-boolean cancellation flag.
    const noBool = createImportRuntime();
    const pendingBool = noBool.window.remcord.themes.importFiles();
    const messageBool = noBool.window.messages.find((entry) => entry.command?.type === "theme_import_files");
    respondThemeImported(noBool.window, messageBool, { canceled: 0, imported: 1, failed: 0 });
    await assert.rejects(pendingBool, /theme import response schema is invalid/);

    // Negative and non-integer counts.
    for (const payload of [
        { canceled: false, imported: -1, failed: 0 },
        { canceled: false, imported: 1.5, failed: 0 },
        { canceled: false, imported: 0, failed: -2 },
    ]) {
        const runtime = createImportRuntime();
        const pending = runtime.window.remcord.themes.importFiles();
        const message = runtime.window.messages.find((entry) => entry.command?.type === "theme_import_files");
        respondThemeImported(runtime.window, message, payload);
        await assert.rejects(pending, /theme import response schema is invalid/);
    }

    // No path, URL, id, or extra member may ride along with the summary.
    const withPath = createImportRuntime();
    const pendingPath = withPath.window.remcord.themes.importFiles();
    const messagePath = withPath.window.messages.find((entry) => entry.command?.type === "theme_import_files");
    withPath.window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: messagePath.request_id,
                ok: true,
                payload: {
                    type: "theme_imported",
                    payload: { canceled: false, imported: 1, failed: 0, path: "C:\\themes\\x" },
                },
                error: null,
            },
        }),
    );
    await assert.rejects(pendingPath, /theme import response schema is invalid/);
});

test("themes.import_files maps host failures to typed errors", async () => {
    const { window } = createImportRuntime();
    const pending = window.remcord.themes.importFiles();
    const message = window.messages.find((entry) => entry.command?.type === "theme_import_files");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: false,
                payload: null,
                error: { code: "IMPORT_FAILED", message: "theme import picker cannot be shown" },
            },
        }),
    );
    await assert.rejects(pending, /theme import picker cannot be shown/);
});

test("themes.import_files waits with the long-operation picker timeout", async () => {
    // A modal picker stays open as long as the user needs: the default is
    // the 120-second long-operation timeout (like the mod bundle retrieval),
    // never the ordinary 5-second command timeout.
    assert.match(bootstrapTemplate, /function importFiles\(timeoutMs = 120_000\)/);
    const { window } = createImportRuntime();
    await assert.rejects(window.remcord.themes.importFiles(50), /timed out/);
});

test("themes.import_files rejects pending requests when the document is disposed", async () => {
    const { window } = createImportRuntime();
    const pending = window.remcord.themes.importFiles();
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh theme import facade with no stale promises", async () => {
    const firstDocument = createImportRuntime();
    const stale = firstDocument.window.remcord.themes.importFiles();
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createImportRuntime();
    assert.notEqual(firstDocument.window.remcord, nextDocument.window.remcord);
    const pending = nextDocument.window.remcord.themes.importFiles();
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "theme_import_files");
    respondThemeImported(nextDocument.window, message, { canceled: true, imported: 0, failed: 0 });
    assert.deepEqual(await pending, { canceled: true, imported: 0, failed: 0 });
});

test("theme import requests stay inert off the trusted origin", async () => {
    const { window } = createRuntime("https://attacker.invalid", ["bridge.ping", "themes.import_files"]);
    assert.equal(window.remcord, undefined);
});

function createEnableRuntime(capabilities = ["bridge.ping", "themes.set_enabled"]) {
    return createRuntime("https://discord.com", capabilities);
}

function respondThemeEnabled(window, request, payload = { id: "my-theme", enabled: true }) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "theme_enabled", payload },
                error: null,
            },
        }),
    );
}

test("the facade exposes a typed themes.set_enabled flip and no generic set", () => {
    const { window } = createEnableRuntime();
    assert.equal(typeof window.remcord.themes.setEnabled, "function");
    assert.equal(window.remcord.native.supports("themes.set_enabled"), true);
    // No untyped set surface may exist. (The read-only `listThemes`
    // inventory is a separate slice with its own capability and tests.)
    assert.equal(window.remcord.themes.set, undefined);
    assert.equal(window.remcord.fs, undefined);
});

test("themes.set_enabled sends id plus boolean and resolves the echo", async () => {
    const { window } = createEnableRuntime();
    const pending = window.remcord.themes.setEnabled("my-theme", true);
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "theme_set_enabled");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "theme_set_enabled", payload: { id: "my-theme", enabled: true } });

    respondThemeEnabled(window, message, { id: "my-theme", enabled: true });
    assert.deepEqual(await pending, { id: "my-theme", enabled: true });
});

test("themes.set_enabled throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createEnableRuntime(["bridge.ping"]);
    assert.equal(window.remcord.native.supports("themes.set_enabled"), false);
    assert.throws(
        () => window.remcord.themes.setEnabled("my-theme", true),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "theme_set_enabled"),
        false,
    );
});

test("themes.set_enabled rejects hostile ids and non-boolean flags without sending", async () => {
    const { window } = createEnableRuntime();
    for (const id of ["", ".", "..", "../escape", "..\\escape", "a/b", "C:\\x", "has space"]) {
        await assert.rejects(window.remcord.themes.setEnabled(id, true), /id must match/);
    }
    const oversized = "t".repeat(65);
    await assert.rejects(window.remcord.themes.setEnabled(oversized, true), /id must match/);
    await assert.rejects(window.remcord.themes.setEnabled("my-theme", 42), /enabled must be a boolean/);
    await assert.rejects(window.remcord.themes.setEnabled("my-theme", "yes"), /enabled must be a boolean/);
    await assert.rejects(window.remcord.themes.setEnabled("my-theme", null), /enabled must be a boolean/);
    await assert.rejects(window.remcord.themes.setEnabled(42, true), /id must match/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "theme_set_enabled"),
        false,
    );
});

test("themes.set_enabled rejects echo mismatches and path ridealongs", async () => {
    // Id echo must equal the request: another theme's flip can never
    // resolve this call.
    const mismatched = createEnableRuntime();
    const pendingMismatch = mismatched.window.remcord.themes.setEnabled("my-theme", true);
    const messageMismatch = mismatched.window.messages.find((entry) => entry.command?.type === "theme_set_enabled");
    respondThemeEnabled(mismatched.window, messageMismatch, { id: "other-theme", enabled: true });
    await assert.rejects(pendingMismatch, /theme enable response schema is invalid/);

    // Enabled echo must equal the request.
    const flipped = createEnableRuntime();
    const pendingFlipped = flipped.window.remcord.themes.setEnabled("my-theme", true);
    const messageFlipped = flipped.window.messages.find((entry) => entry.command?.type === "theme_set_enabled");
    respondThemeEnabled(flipped.window, messageFlipped, { id: "my-theme", enabled: false });
    await assert.rejects(pendingFlipped, /theme enable response schema is invalid/);

    // No path, CSS, manifest, or extra member may ride along.
    const withPath = createEnableRuntime();
    const pendingPath = withPath.window.remcord.themes.setEnabled("my-theme", true);
    const messagePath = withPath.window.messages.find((entry) => entry.command?.type === "theme_set_enabled");
    withPath.window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: messagePath.request_id,
                ok: true,
                payload: {
                    type: "theme_enabled",
                    payload: { id: "my-theme", enabled: true, path: "C:\\themes\\x" },
                },
                error: null,
            },
        }),
    );
    await assert.rejects(pendingPath, /theme enable response schema is invalid/);
});

test("themes.set_enabled maps host failures to typed errors", async () => {
    const { window } = createEnableRuntime();
    const pending = window.remcord.themes.setEnabled("my-theme", true);
    const message = window.messages.find((entry) => entry.command?.type === "theme_set_enabled");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: false,
                payload: null,
                error: { code: "UNKNOWN_THEME", message: "theme folder is unknown" },
            },
        }),
    );
    await assert.rejects(pending, /theme folder is unknown/);
});

test("themes.set_enabled rejects after its timeout", async () => {
    const { window } = createEnableRuntime();
    await assert.rejects(window.remcord.themes.setEnabled("my-theme", true, 50), /timed out/);
});

test("themes.set_enabled rejects pending requests when the document is disposed", async () => {
    const { window } = createEnableRuntime();
    const pending = window.remcord.themes.setEnabled("my-theme", true);
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh theme enable facade with no stale promises", async () => {
    const firstDocument = createEnableRuntime();
    const stale = firstDocument.window.remcord.themes.setEnabled("my-theme", true);
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createEnableRuntime();
    assert.notEqual(firstDocument.window.remcord, nextDocument.window.remcord);
    const pending = nextDocument.window.remcord.themes.setEnabled("my-theme", false);
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "theme_set_enabled");
    respondThemeEnabled(nextDocument.window, message, { id: "my-theme", enabled: false });
    assert.deepEqual(await pending, { id: "my-theme", enabled: false });
});

test("theme enable requests stay inert off the trusted origin", async () => {
    const { window } = createRuntime("https://attacker.invalid", ["bridge.ping", "themes.set_enabled"]);
    assert.equal(window.remcord, undefined);
});

function createListRuntime(capabilities = ["bridge.ping", "themes.list"]) {
    return createRuntime("https://discord.com", capabilities);
}

const LIST_FIXTURE = [
    { id: "alpha-BD", name: "Alpha", author: "A", description: "First", version: "1", enabled: false },
    { id: "zulu-BD", name: "Zulu", author: "", description: "", version: "", enabled: true },
];

function respondThemesList(window, request, themes = LIST_FIXTURE) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "themes_list", payload: { themes } },
                error: null,
            },
        }),
    );
}

test("the facade exposes a parameter-free themes.list inventory", () => {
    const { window } = createListRuntime();
    assert.equal(typeof window.remcord.themes.listThemes, "function");
    assert.equal(window.remcord.native.supports("themes.list"), true);
    // No filesystem, path, or URL surface may exist. getThemes is
    // the one closed synchronous exception (settings shim snapshot
    // projection, not a host operation). The Quick CSS editor pair lives
    // under the separate `quickcss` namespace, never here.
    assert.equal(typeof window.remcord.themes.getThemes, "function");
    assert.equal(window.remcord.themes.getCSS, undefined);
    assert.equal(window.remcord.themes.read, undefined);
    assert.equal(window.remcord.themes.write, undefined);
    assert.equal(window.remcord.fs, undefined);
});

test("themes.list sends an exact empty payload and resolves the display array", async () => {
    const { window } = createListRuntime();
    const pending = window.remcord.themes.listThemes();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "themes_list");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "themes_list", payload: {} });

    respondThemesList(window, message);
    assert.deepEqual(await pending, LIST_FIXTURE);
});

test("themes.list throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createListRuntime(["bridge.ping"]);
    assert.equal(window.remcord.native.supports("themes.list"), false);
    assert.throws(
        () => window.remcord.themes.listThemes(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "themes_list"),
        false,
    );
});

test("themes.list rejects malformed inventories without path leakage", async () => {
    // Non-array inventory.
    const nonArray = createListRuntime();
    const pendingArray = nonArray.window.remcord.themes.listThemes();
    const messageArray = nonArray.window.messages.find((entry) => entry.command?.type === "themes_list");
    respondThemesList(nonArray.window, messageArray, { themes: "nope" });
    await assert.rejects(pendingArray, /theme list response schema is invalid/);

    // Entry with a path member, a wrong-typed field, and an oversized id.
    for (const themes of [
        [{ ...LIST_FIXTURE[0], path: "C:\\themes\\x" }],
        [{ ...LIST_FIXTURE[0], enabled: "yes" }],
        [{ ...LIST_FIXTURE[0], id: "t".repeat(65) }],
        [{ ...LIST_FIXTURE[0], extra: 1 }],
        "not-an-array",
    ]) {
        const runtime = createListRuntime();
        const pending = runtime.window.remcord.themes.listThemes();
        const message = runtime.window.messages.find((entry) => entry.command?.type === "themes_list");
        respondThemesList(runtime.window, message, themes);
        await assert.rejects(pending, /theme list response schema is invalid/);
    }

    // Over-long display string.
    const long = createListRuntime();
    const pendingLong = long.window.remcord.themes.listThemes();
    const messageLong = long.window.messages.find((entry) => entry.command?.type === "themes_list");
    respondThemesList(long.window, messageLong, [{ ...LIST_FIXTURE[0], description: "x".repeat(4097) }]);
    await assert.rejects(pendingLong, /theme list response schema is invalid/);

    // Excessive entry count.
    const many = createListRuntime();
    const pendingMany = many.window.remcord.themes.listThemes();
    const messageMany = many.window.messages.find((entry) => entry.command?.type === "themes_list");
    respondThemesList(
        many.window,
        messageMany,
        Array.from({ length: 257 }, (_, index) => ({ ...LIST_FIXTURE[0], id: `t-${index}-BD` })),
    );
    await assert.rejects(pendingMany, /theme list response schema is invalid/);
});

test("themes.list rejects after its timeout", async () => {
    const { window } = createListRuntime();
    await assert.rejects(window.remcord.themes.listThemes(50), /timed out/);
});

test("themes.list rejects pending requests when the document is disposed", async () => {
    const { window } = createListRuntime();
    const pending = window.remcord.themes.listThemes();
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh theme list facade with no stale promises", async () => {
    const firstDocument = createListRuntime();
    const stale = firstDocument.window.remcord.themes.listThemes();
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createListRuntime();
    assert.notEqual(firstDocument.window.remcord, nextDocument.window.remcord);
    const pending = nextDocument.window.remcord.themes.listThemes();
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "themes_list");
    respondThemesList(nextDocument.window, message, []);
    assert.deepEqual(await pending, []);
});

test("theme list requests stay inert off the trusted origin", async () => {
    const { window } = createRuntime("https://attacker.invalid", ["bridge.ping", "themes.list"]);
    assert.equal(window.remcord, undefined);
});

function createQuickCssRuntime(capabilities = ["bridge.ping", "quickcss.read", "quickcss.write"]) {
    return createRuntime("https://discord.com", capabilities);
}

function respondQuickCssRead(window, request, css = ":root{}") {
    const bytes = new TextEncoder().encode(css).length;
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "quick_css_read", payload: { css, bytes } },
                error: null,
            },
        }),
    );
}

function respondQuickCssWrite(window, request, bytes = 8) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "quick_css_written", payload: { bytes } },
                error: null,
            },
        }),
    );
}

test("the facade exposes a namespaced quickcss editor pair and no generic writer", () => {
    const { window } = createQuickCssRuntime();
    assert.equal(typeof window.remcord.quickcss.read, "function");
    assert.equal(typeof window.remcord.quickcss.write, "function");
    assert.equal(window.remcord.native.supports("quickcss.read"), true);
    assert.equal(window.remcord.native.supports("quickcss.write"), true);
    // Exactly the pair: no delete, list, open, or path-taking member.
    assert.deepEqual(Object.keys(window.remcord.quickcss).sort(), ["read", "write"]);
    assert.equal(window.remcord.fs, undefined);
});

test("quickcss.read sends an exact empty payload and resolves text plus bytes", async () => {
    const { window } = createQuickCssRuntime();
    const pending = window.remcord.quickcss.read();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "quick_css_read");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "quick_css_read", payload: {} });

    respondQuickCssRead(window, message, ":root{--q:1;}");
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), {
        css: ":root{--q:1;}",
        bytes: ":root{--q:1;}".length,
    });
});

test("quickcss.read throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createQuickCssRuntime(["bridge.ping", "quickcss.write"]);
    assert.equal(window.remcord.native.supports("quickcss.read"), false);
    assert.throws(
        () => window.remcord.quickcss.read(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "quick_css_read"),
        false,
    );
});

test("quickcss.read rejects malformed read responses", async () => {
    for (const payload of [
        { css: ":root{}", bytes: "8" },
        { css: ":root{}" },
        { bytes: 8 },
        { css: ":root{}", bytes: 8, path: "C:\\x" },
        { css: 42, bytes: 8 },
    ]) {
        const runtime = createQuickCssRuntime();
        const pending = runtime.window.remcord.quickcss.read();
        const message = runtime.window.messages.find((entry) => entry.command?.type === "quick_css_read");
        runtime.window.dispatchEvent(
            new FakeCustomEvent("remcord-native:response", {
                detail: {
                    version: 1,
                    request_id: message.request_id,
                    ok: true,
                    payload: { type: "quick_css_read", payload },
                    error: null,
                },
            }),
        );
        await assert.rejects(pending, /Quick CSS read response schema is invalid/);
    }
});

test("quickcss.write sends CSS text only and resolves the byte length", async () => {
    const { window } = createQuickCssRuntime();
    const pending = window.remcord.quickcss.write(":root{--w:2;}");
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "quick_css_write");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "quick_css_write", payload: { css: ":root{--w:2;}" } });

    respondQuickCssWrite(window, message, ":root{--w:2;}".length);
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), { bytes: ":root{--w:2;}".length });
});

test("quickcss.write throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createQuickCssRuntime(["bridge.ping", "quickcss.read"]);
    assert.equal(window.remcord.native.supports("quickcss.write"), false);
    assert.throws(
        () => window.remcord.quickcss.write(":root{}"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "quick_css_write"),
        false,
    );
});

test("quickcss.write rejects non-string and oversized CSS without sending", async () => {
    const { window } = createQuickCssRuntime();
    await assert.rejects(window.remcord.quickcss.write(42), /string/);
    await assert.rejects(window.remcord.quickcss.write(null), /string/);
    await assert.rejects(window.remcord.quickcss.write("x".repeat(1048576 + 1)), /at most/);
    // Empty string is a legal clear of the editor buffer.
    const pending = window.remcord.quickcss.write("");
    const message = window.messages.find((entry) => entry.command?.type === "quick_css_write");
    assert.ok(message);
    respondQuickCssWrite(window, message, 0);
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), { bytes: 0 });
});

test("quickcss.write rejects malformed write responses", async () => {
    const { window } = createQuickCssRuntime();
    const pending = window.remcord.quickcss.write(":root{}");
    const message = window.messages.find((entry) => entry.command?.type === "quick_css_write");
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: true,
                payload: { type: "quick_css_written", payload: { bytes: 8, path: "C:\\x" } },
                error: null,
            },
        }),
    );
    await assert.rejects(pending, /Quick CSS write response schema is invalid/);
});

test("quickcss editor requests stay inert off the trusted origin", async () => {
    const { window } = createRuntime("https://attacker.invalid", ["bridge.ping", "quickcss.read", "quickcss.write"]);
    assert.equal(window.remcord, undefined);
});

// ---------------------------------------------------------------------------
// Gallery disk cache contract (F5 core).
//
// The facade mirrors the Electron bridge method names and result shapes
// exactly, over host-owned storage. Import/export/root-pick need modal
// host dialogs (F5b slice): their methods exist for signature parity but
// throw UNSUPPORTED_CAPABILITY honestly until their capabilities exist.
// ---------------------------------------------------------------------------

const GALLERY_CAPS = [
    "bridge.ping",
    "gallery.list",
    "gallery.add_url",
    "gallery.add_link",
    "gallery.remove",
    "gallery.bulk_remove",
    "gallery.update",
    "gallery.categories",
    "gallery.category_add",
    "gallery.category_rename",
    "gallery.category_remove",
    "gallery.usage",
    "gallery.file",
    "gallery.root",
    "folders.open",
];

function createGalleryRuntime(capabilities = GALLERY_CAPS) {
    return createRuntime("https://discord.com", capabilities);
}

function respondGallery(window, request, type, payload) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type, payload },
                error: null,
            },
        }),
    );
}

test("the facade exposes the Electron-identical gallery namespace and nothing else", () => {
    const { window } = createGalleryRuntime();
    assert.deepEqual(Object.keys(window.remcord.gallery).sort(), [
        "addLink",
        "addUrl",
        "bulkRemove",
        "categories",
        "categoryAdd",
        "categoryRemove",
        "categoryRename",
        "exportItems",
        "getFile",
        "importDialog",
        "list",
        "openRoot",
        "pickRoot",
        "remove",
        "root",
        "update",
        "usage",
    ]);
    for (const capability of GALLERY_CAPS.slice(1)) {
        assert.equal(window.remcord.native.supports(capability), true);
    }
    // No path-taking member may exist alongside the closed namespace.
    assert.equal(window.remcord.gallery.openPath, undefined);
    assert.equal(window.remcord.gallery.saveFile, undefined);
});

test("gallery.list sends an exact empty payload and resolves the inventory", async () => {
    const { window } = createGalleryRuntime();
    const pending = window.remcord.gallery.list();
    const message = window.messages.find((entry) => entry.command?.type === "gallery_list");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "gallery_list", payload: {} });
    respondGallery(window, message, "gallery_list", {
        items: [{ id: "a-1", kind: "link", localFile: "" }],
        categories: [{ name: "clips" }],
    });
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), {
        items: [{ id: "a-1", kind: "link", localFile: "" }],
        categories: [{ name: "clips" }],
    });
});

test("gallery.list rejects hostile inventories", async () => {
    for (const payload of [
        { items: [{ id: "../x", kind: "link", localFile: "" }], categories: [] },
        { items: [{ id: "a-1", kind: "link", localFile: "/abs/path.png" }], categories: [] },
        { items: [{ id: "a-1", kind: "link", localFile: "files/x.png", path: "C:\\x" }], categories: [] },
        { items: [], categories: [{ name: "clips", path: "C:\\x" }] },
        { items: "nope", categories: [] },
    ]) {
        const runtime = createGalleryRuntime();
        const pending = runtime.window.remcord.gallery.list();
        const message = runtime.window.messages.find((entry) => entry.command?.type === "gallery_list");
        respondGallery(runtime.window, message, "gallery_list", payload);
        await assert.rejects(pending, /gallery list response schema is invalid/);
    }
});

test("gallery.addUrl sends URL plus metadata and resolves the Electron shape", async () => {
    const { window } = createGalleryRuntime();
    const pending = window.remcord.gallery.addUrl("https://x/y.png", { title: "clip" });
    const message = window.messages.find((entry) => entry.command?.type === "gallery_add_url");
    assert.ok(message);
    assert.deepEqual(message.command, {
        type: "gallery_add_url",
        payload: { url: "https://x/y.png", meta: { title: "clip" } },
    });
    respondGallery(window, message, "gallery_add", { ok: true, item: { id: "a-1" } });
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), { ok: true, item: { id: "a-1" } });
    // Meta defaults to {} when omitted.
    const second = createGalleryRuntime();
    const pending2 = second.window.remcord.gallery.addLink("https://x/y");
    const message2 = second.window.messages.find((entry) => entry.command?.type === "gallery_add_link");
    assert.deepEqual(message2.command, { type: "gallery_add_link", payload: { url: "https://x/y", meta: {} } });
    respondGallery(second.window, message2, "gallery_add", { ok: false, error: "INVALID_URL" });
    assert.deepEqual(JSON.parse(JSON.stringify(await pending2)), { ok: false, error: "INVALID_URL" });
});

test("gallery save/update inputs are validated before dispatch", async () => {
    const { window } = createGalleryRuntime();
    await assert.rejects(window.remcord.gallery.addUrl(""), /url must be/);
    await assert.rejects(window.remcord.gallery.addUrl("file:///C:/x"), /http\(s\)/);
    await assert.rejects(window.remcord.gallery.addUrl("https://x/y", "meta"), /meta must be/);
    await assert.rejects(window.remcord.gallery.remove("../x"), /id must match/);
    await assert.rejects(window.remcord.gallery.remove(""), /id must match/);
    await assert.rejects(window.remcord.gallery.bulkRemove([]), /ids must be/);
    await assert.rejects(window.remcord.gallery.bulkRemove(["ok", 42]), /ids must be/);
    await assert.rejects(window.remcord.gallery.update("a-1", null), /patch must be/);
    // Unknown patch members are dropped, not rejected (Electron's
    // updateGalleryItem reads only title/categories too): only the id
    // crosses.
    const dropMessage = (() => {
        const before = window.messages.length;
        // Short timeout: the request is intentionally never answered; the
        // rejection is swallowed — only the sent wire shape is asserted.
        const pending = window.remcord.gallery.update("a-1", { kind: "image", path: "C:\\x" }, 50);
        pending.catch(() => {});
        const sent = window.messages.slice(before).find((entry) => entry.command?.type === "gallery_update");
        assert.ok(sent);
        assert.deepEqual(sent.command, { type: "gallery_update", payload: { id: "a-1" } });
        return pending;
    })();
    await dropMessage.catch(() => {});
    await assert.rejects(window.remcord.gallery.categoryAdd(""), /name must be/);
    await assert.rejects(window.remcord.gallery.getFile("../x"), /id must match/);
    // (The dropped-patch update above intentionally sent: it proves only
    // the id crosses. Everything else in this test rejected pre-dispatch.)
});

test("gallery.remove/update/category mutations resolve Electron-parity outcomes", async () => {
    const { window } = createGalleryRuntime();
    const removed = window.remcord.gallery.remove("a-1");
    const removeMessage = window.messages.find((entry) => entry.command?.type === "gallery_remove");
    assert.deepEqual(removeMessage.command, { type: "gallery_remove", payload: { id: "a-1" } });
    respondGallery(window, removeMessage, "gallery_remove", { ok: true });
    assert.deepEqual(JSON.parse(JSON.stringify(await removed)), { ok: true });

    const bulk = window.remcord.gallery.bulkRemove(["a-1", "a-2"]);
    const bulkMessage = window.messages.find((entry) => entry.command?.type === "gallery_bulk_remove");
    respondGallery(window, bulkMessage, "gallery_bulk_remove", { removed: 2 });
    assert.deepEqual(JSON.parse(JSON.stringify(await bulk)), { removed: 2 });

    const updated = window.remcord.gallery.update("a-1", { title: "new", categories: ["clips"] });
    const updateMessage = window.messages.find((entry) => entry.command?.type === "gallery_update");
    assert.deepEqual(updateMessage.command, {
        type: "gallery_update",
        payload: { id: "a-1", title: "new", categories: ["clips"] },
    });
    respondGallery(window, updateMessage, "gallery_update", { ok: true, item: { id: "a-1" } });
    assert.deepEqual(JSON.parse(JSON.stringify(await updated)), { ok: true, item: { id: "a-1" } });

    const renamed = window.remcord.gallery.categoryRename("a", "b");
    const renameMessage = window.messages.find((entry) => entry.command?.type === "gallery_category_rename");
    assert.deepEqual(renameMessage.command, {
        type: "gallery_category_rename",
        payload: { old_name: "a", new_name: "b" },
    });
    respondGallery(window, renameMessage, "gallery_category_rename", { ok: true });
    assert.deepEqual(JSON.parse(JSON.stringify(await renamed)), { ok: true });
});

test("gallery.getFile bounds the preview and falls back honestly", async () => {
    const { window } = createGalleryRuntime();
    const pending = window.remcord.gallery.getFile("a-1");
    const message = window.messages.find((entry) => entry.command?.type === "gallery_file");
    assert.deepEqual(message.command, { type: "gallery_file", payload: { id: "a-1" } });
    respondGallery(window, message, "gallery_file", { ok: true, base64: "AAA=", mime: "image/png", fileName: "x.png" });
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), {
        ok: true,
        base64: "AAA=",
        mime: "image/png",
        fileName: "x.png",
    });
    // A path-bearing preview is rejected.
    const second = createGalleryRuntime();
    const pending2 = second.window.remcord.gallery.getFile("a-1");
    const message2 = second.window.messages.find((entry) => entry.command?.type === "gallery_file");
    respondGallery(second.window, message2, "gallery_file", {
        ok: true,
        base64: "AAA=",
        mime: "image/png",
        fileName: "x.png",
        path: "C:\\x",
    });
    await assert.rejects(pending2, /gallery file response schema is invalid/);
});

test("gallery dialog-backed methods throw UNSUPPORTED_CAPABILITY honestly", () => {
    // Import/export/root-pick need modal host dialogs (F5b slice): the
    // capabilities are absent, so the methods throw the closed
    // UNSUPPORTED_CAPABILITY error and the page gates those buttons.
    const { window } = createGalleryRuntime();
    assert.equal(window.remcord.native.supports("gallery.import_files"), false);
    assert.throws(
        () => window.remcord.gallery.importDialog({}),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.throws(
        () => window.remcord.gallery.exportItems(["a-1"]),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.throws(
        () => window.remcord.gallery.pickRoot(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type?.startsWith("gallery_")),
        false,
    );
});

test("gallery.openRoot reveals the fixed gallery root through folders.open", async () => {
    const { window } = createGalleryRuntime();
    const pending = window.remcord.gallery.openRoot();
    const message = window.messages.find((entry) => entry.command?.type === "folders_open");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "folders_open", payload: { location: "gallery" } });
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: message.request_id,
                ok: true,
                payload: { type: "folders_opened", payload: { location: "gallery" } },
                error: null,
            },
        }),
    );
    // Fire-and-forget reveal like Electron's send: resolves undefined.
    assert.equal(await pending, undefined);
});

test("gallery usage/root/categories resolve bounded host facts", async () => {
    const { window } = createGalleryRuntime();
    const usage = window.remcord.gallery.usage();
    respondGallery(
        window,
        window.messages.find((entry) => entry.command?.type === "gallery_usage"),
        "gallery_usage",
        {
            bytes: 10,
            files: 1,
            items: 1,
        },
    );
    assert.deepEqual(JSON.parse(JSON.stringify(await usage)), { bytes: 10, files: 1, items: 1 });

    const root = window.remcord.gallery.root();
    respondGallery(
        window,
        window.messages.find((entry) => entry.command?.type === "gallery_root"),
        "gallery_root",
        {
            root: "gallery",
        },
    );
    assert.deepEqual(JSON.parse(JSON.stringify(await root)), { root: "gallery" });

    const cats = window.remcord.gallery.categories();
    respondGallery(
        window,
        window.messages.find((entry) => entry.command?.type === "gallery_categories"),
        "gallery_categories",
        { categories: [{ name: "clips", color: "#5865f2", createdAt: 1 }] },
    );
    assert.deepEqual(JSON.parse(JSON.stringify(await cats)), [{ name: "clips", color: "#5865f2", createdAt: 1 }]);
});

function createDiagnosticsRuntime(capabilities = ["bridge.ping", "diagnostics.get_info", "mod.clear_cache"]) {
    return createRuntime("https://discord.com", capabilities);
}

const DIAGNOSTICS_FIXTURE = {
    app_version: "1.3.0",
    webview2_runtime: "154.0.4258.62",
    profile_kind: "release",
    channel: "stable",
    csp: "none",
    mods: ["vencord"],
};

function respondDiagnostics(window, request, payload = DIAGNOSTICS_FIXTURE) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "diagnostics_info", payload },
                error: null,
            },
        }),
    );
}

function respondModCacheCleared(window, request, payload = { cleared: ["vencord.js"], mod_cache_cleared: true }) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "mod_cache_cleared", payload },
                error: null,
            },
        }),
    );
}

test("the facade exposes namespaced diagnostics and mod cache capabilities and no generic surface", () => {
    const { window } = createDiagnosticsRuntime();
    assert.equal(typeof window.remcord.diagnostics.getInfo, "function");
    assert.equal(typeof window.remcord.mod.clearCache, "function");
    assert.equal(Object.isFrozen(window.remcord.diagnostics), true);
    assert.equal(Object.isFrozen(window.remcord.mod), true);
    assert.equal(window.remcord.native.supports("diagnostics.get_info"), true);
    assert.equal(window.remcord.native.supports("mod.clear_cache"), true);
    // Exactly one member each: no clipboard, delete, or selector surface.
    assert.deepEqual(Object.keys(window.remcord.diagnostics).sort(), ["getInfo"]);
    assert.deepEqual(Object.keys(window.remcord.mod).sort(), ["clearCache"]);
    assert.equal(window.remcord.clipboard, undefined);
    assert.equal(window.remcord.fs, undefined);
});

test("diagnostics.getInfo sends an exact empty payload and resolves the readout", async () => {
    const { window } = createDiagnosticsRuntime();
    const pending = window.remcord.diagnostics.getInfo();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "diagnostics_get_info");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "diagnostics_get_info", payload: {} });
    respondDiagnostics(window, message);
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), DIAGNOSTICS_FIXTURE);
});

test("diagnostics.getInfo throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createDiagnosticsRuntime(["bridge.ping", "mod.clear_cache"]);
    assert.equal(window.remcord.native.supports("diagnostics.get_info"), false);
    assert.throws(
        () => window.remcord.diagnostics.getInfo(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "diagnostics_get_info"),
        false,
    );
});

test("diagnostics.getInfo rejects malformed readouts", async () => {
    for (const payload of [
        { ...DIAGNOSTICS_FIXTURE, app_version: "" },
        { ...DIAGNOSTICS_FIXTURE, webview2_runtime: "x".repeat(65) },
        { ...DIAGNOSTICS_FIXTURE, profile_kind: "roaming" },
        { ...DIAGNOSTICS_FIXTURE, channel: "beta" },
        { ...DIAGNOSTICS_FIXTURE, csp: "permissive" },
        { ...DIAGNOSTICS_FIXTURE, mods: ["Vencord"] },
        { ...DIAGNOSTICS_FIXTURE, mods: ["vencord", "vencord", "shelter", "equicord", "custom", "x", "y", "z", "w"] },
        { ...DIAGNOSTICS_FIXTURE, profile_path: "C:\\secret" },
        { ...DIAGNOSTICS_FIXTURE, token: "abc" },
        { app_version: "1.3.0" },
    ]) {
        const runtime = createDiagnosticsRuntime();
        const pending = runtime.window.remcord.diagnostics.getInfo();
        const message = runtime.window.messages.find((entry) => entry.command?.type === "diagnostics_get_info");
        runtime.window.dispatchEvent(
            new FakeCustomEvent("remcord-native:response", {
                detail: {
                    version: 1,
                    request_id: message.request_id,
                    ok: true,
                    payload: { type: "diagnostics_info", payload },
                    error: null,
                },
            }),
        );
        await assert.rejects(pending, /diagnostics response schema is invalid/);
    }
});

function createGraphicsRuntime(capabilities = ["bridge.ping", "diagnostics.get_info", "graphics.get_info"]) {
    return createRuntime("https://discord.com", capabilities);
}

const GRAPHICS_FIXTURE = {
    webview2_runtime: "155.0.4283.45",
    additional_browser_arguments: "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection",
    environment: "remcord-pre-registered (wry parity + remcord scheme)",
    browser: {
        status: "measured",
        method: "Browser.getVersion",
        product: "Chrome/155.0.4283.45",
        revision: "@abcdef",
        user_agent: "Mozilla/5.0",
        js_version: "15.5.1",
        protocol_version: "1.3",
        error: null,
    },
    system: {
        status: "unavailable",
        method: "System.getInfo",
        model_name: null,
        devices: [],
        feature_status: [],
        command_line_flags: [],
        disable_features: null,
        enable_features: null,
        error: "the probe could not start",
    },
};

function respondGraphics(window, request, payload = GRAPHICS_FIXTURE) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "graphics_info", payload },
                error: null,
            },
        }),
    );
}

test("the facade exposes a namespaced graphics capability and no generic surface", () => {
    const { window } = createGraphicsRuntime();
    assert.equal(typeof window.remcord.graphics.getInfo, "function");
    assert.equal(Object.isFrozen(window.remcord.graphics), true);
    assert.equal(window.remcord.native.supports("graphics.get_info"), true);
    // Exactly one member: no method/target selector surface.
    assert.deepEqual(Object.keys(window.remcord.graphics).sort(), ["getInfo"]);
});

test("graphics.getInfo sends an exact empty payload and resolves the report", async () => {
    const { window } = createGraphicsRuntime();
    const pending = window.remcord.graphics.getInfo();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "graphics_get_info");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "graphics_get_info", payload: {} });
    respondGraphics(window, message);
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), GRAPHICS_FIXTURE);
});

test("graphics.getInfo throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createGraphicsRuntime(["bridge.ping", "diagnostics.get_info"]);
    assert.equal(window.remcord.native.supports("graphics.get_info"), false);
    assert.throws(
        () => window.remcord.graphics.getInfo(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "graphics_get_info"),
        false,
    );
});

test("graphics.getInfo rejects malformed reports", async () => {
    const measuredSystem = {
        status: "measured",
        method: "System.getInfo",
        model_name: "Laptop",
        devices: [
            {
                vendor_id: "4318",
                device_id: null,
                vendor_string: "Google Inc. (Intel)",
                device_string: "ANGLE (Intel)",
                driver_version: "31.0.101.4502",
            },
        ],
        feature_status: [{ name: "webgl", status: "hardware_accelerated" }],
        command_line_flags: ["--disable-features"],
        disable_features: "msWebOOUI",
        enable_features: null,
        error: null,
    };
    for (const payload of [
        { ...GRAPHICS_FIXTURE, webview2_runtime: "" },
        { ...GRAPHICS_FIXTURE, additional_browser_arguments: "x".repeat(513) },
        { ...GRAPHICS_FIXTURE, browser: { ...GRAPHICS_FIXTURE.browser, status: "turbo" } },
        { ...GRAPHICS_FIXTURE, browser: { ...GRAPHICS_FIXTURE.browser, method: "Target.attach" } },
        { ...GRAPHICS_FIXTURE, system: { ...measuredSystem, devices: new Array(17).fill(measuredSystem.devices[0]) } },
        { ...GRAPHICS_FIXTURE, system: { ...measuredSystem, commandLine: "C:\\secret" } },
        { ...GRAPHICS_FIXTURE, system: { ...measuredSystem, error: "x".repeat(513) } },
        { ...GRAPHICS_FIXTURE, token: "abc" },
        { webview2_runtime: "155.0.4283.45" },
    ]) {
        const runtime = createGraphicsRuntime();
        const pending = runtime.window.remcord.graphics.getInfo();
        const message = runtime.window.messages.find((entry) => entry.command?.type === "graphics_get_info");
        runtime.window.dispatchEvent(
            new FakeCustomEvent("remcord-native:response", {
                detail: {
                    version: 1,
                    request_id: message.request_id,
                    ok: true,
                    payload: { type: "graphics_info", payload },
                    error: null,
                },
            }),
        );
        await assert.rejects(pending, /graphics response schema is invalid/);
    }
});

function createPluginsRuntime(
    capabilities = [
        "bridge.ping",
        "plugins.list",
        "plugins.set_enabled",
        "plugins.reload",
        "plugins.get_runtime_entries",
        "plugins.get_runtime_script",
        "folders.open",
    ],
) {
    return createRuntime("https://discord.com", capabilities);
}

function respondPluginsList(window, request, plugins = []) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "plugins_list", payload: { plugins } },
                error: null,
            },
        }),
    );
}

function respondPluginFlip(window, request, payload) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "plugins_set_enabled", payload },
                error: null,
            },
        }),
    );
}

function respondPluginReloaded(window, request, payload) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "plugins_reloaded", payload },
                error: null,
            },
        }),
    );
}

function respondPluginEntries(window, request, target, entries = []) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "plugins_runtime_entries", payload: { target, entries } },
                error: null,
            },
        }),
    );
}

function respondPluginScript(window, request, pluginId, target, source) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: {
                    type: "plugins_runtime_script",
                    payload: { plugin_id: pluginId, target, source },
                },
                error: null,
            },
        }),
    );
}

const PLUGIN_A_INFO = {
    id: "alpha",
    name: "Alpha",
    version: "1.0.0",
    description: "First plugin",
    author: "tester",
    enabled: true,
    compatible: true,
    compatible_versions: ["*"],
    has_main: false,
    has_preload: true,
    has_renderer: true,
};

const PLUGIN_B_INFO = {
    id: "beta",
    name: "Beta",
    version: "2.0.0",
    enabled: false,
    compatible: false,
    compatibility_message: "Incompatible with Remcord 0.0.0 (supports: 9.9.x)",
    compatible_versions: ["9.9.x"],
    has_main: true,
    has_preload: false,
    has_renderer: false,
};

test("the facade exposes scoped runtime plugin capabilities and no generic surface", () => {
    const { window } = createPluginsRuntime();
    assert.equal(typeof window.remcord.plugins.list, "function");
    assert.equal(typeof window.remcord.plugins.setEnabled, "function");
    assert.equal(typeof window.remcord.plugins.reload, "function");
    assert.equal(typeof window.remcord.plugins.openFolder, "function");
    assert.equal(typeof window.remcord.native.getPluginEntries, "function");
    assert.equal(typeof window.remcord.native.getPluginScript, "function");
    assert.equal(Object.isFrozen(window.remcord.plugins), true);
    assert.equal(window.remcord.native.supports("plugins.list"), true);
    assert.equal(window.remcord.native.supports("plugins.set_enabled"), true);
    assert.equal(window.remcord.native.supports("plugins.reload"), true);
    assert.equal(window.remcord.native.supports("plugins.get_runtime_entries"), true);
    assert.equal(window.remcord.native.supports("plugins.get_runtime_script"), true);
    // No generic plugin source, eval, path, or manifest surface may exist.
    assert.equal(window.remcord.plugins.getRuntimeScript, undefined);
    assert.equal(window.remcord.plugins.execute, undefined);
    assert.equal(window.remcord.plugins.eval, undefined);
    assert.equal(window.remcord.plugins.readManifest, undefined);
});

test("plugins.list sends a parameter-free request and resolves bounded display metadata", async () => {
    const { window } = createPluginsRuntime();
    const pending = window.remcord.plugins.list();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "plugins_list");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "plugins_list", payload: {} });
    respondPluginsList(window, message, [PLUGIN_A_INFO, PLUGIN_B_INFO]);
    const plugins = await pending;
    assert.equal(plugins.length, 2);
    assert.equal(plugins[0].id, "alpha");
    assert.equal(plugins[0].compatible_versions.length, 1);
    assert.equal(plugins[1].compatible, false);
    // No path, URL, or source may ride along with any entry.
    const serialized = JSON.stringify(plugins);
    assert.doesNotMatch(serialized, /"path"|"url"|"source"/);
});

test("plugins.list rejects path-bearing and malformed entries", async () => {
    for (const plugins of [
        [{ ...PLUGIN_A_INFO, path: "C:\\plugins\\alpha" }],
        [{ ...PLUGIN_A_INFO, url: "https://attacker.invalid/x.js" }],
        [{ ...PLUGIN_A_INFO, id: "../escape" }],
        [{ ...PLUGIN_A_INFO, id: "" }],
        [{ ...PLUGIN_A_INFO, enabled: "yes" }],
        [{ ...PLUGIN_A_INFO, compatible_versions: "*" }],
        [{ ...PLUGIN_A_INFO, has_preload: 1 }],
        new Array(257).fill(PLUGIN_A_INFO),
        "not-an-array",
    ]) {
        const runtime = createPluginsRuntime();
        const pending = runtime.window.remcord.plugins.list();
        const message = runtime.window.messages.find((entry) => entry.command?.type === "plugins_list");
        respondPluginsList(runtime.window, message, plugins);
        await assert.rejects(pending, /plugin list response schema is invalid/, JSON.stringify(plugins).slice(0, 80));
    }
});

test("plugins.set_enabled sends id+boolean and resolves the echoed outcome", async () => {
    const { window } = createPluginsRuntime();
    const pending = window.remcord.plugins.setEnabled("alpha", true);
    const message = window.messages.find((entry) => entry.command?.type === "plugins_set_enabled");
    assert.ok(message);
    assert.deepEqual(message.command, {
        type: "plugins_set_enabled",
        payload: { plugin_id: "alpha", enabled: true },
    });
    respondPluginFlip(window, message, {
        ok: true,
        plugin_id: "alpha",
        enabled: true,
        source: "file",
        payload_bytes: 512,
    });
    assert.deepEqual(await pending, {
        ok: true,
        plugin_id: "alpha",
        enabled: true,
        source: "file",
        payload_bytes: 512,
    });
    // A known refusal resolves { ok: false } (Electron parity), not a rejection.
    const second = createPluginsRuntime();
    const denied = second.window.remcord.plugins.setEnabled("ghost", true);
    const deniedMessage = second.window.messages.find((entry) => entry.command?.type === "plugins_set_enabled");
    respondPluginFlip(second.window, deniedMessage, { ok: false });
    assert.deepEqual(await denied, { ok: false });
});

test("plugins.set_enabled rejects hostile ids and non-boolean states without sending", async () => {
    const { window } = createPluginsRuntime();
    for (const pluginId of ["", ".", "..", "../escape", "..\\escape", "a/b", "a\\b", "C:\\x", "has space", 42, null]) {
        await assert.rejects(window.remcord.plugins.setEnabled(pluginId, true), /pluginId must match/);
    }
    const oversized = `p${"p".repeat(64)}`;
    await assert.rejects(window.remcord.plugins.setEnabled(oversized, true), /pluginId must match/);
    // Dotted ids ARE valid runtime plugin ids (unlike plugin-storage ids).
    const dotted = createPluginsRuntime();
    const pending = dotted.window.remcord.plugins.setEnabled("demo.plugin", false);
    const message = dotted.window.messages.find((entry) => entry.command?.type === "plugins_set_enabled");
    assert.deepEqual(message.command.payload, { plugin_id: "demo.plugin", enabled: false });
    respondPluginFlip(dotted.window, message, {
        ok: true,
        plugin_id: "demo.plugin",
        enabled: false,
        source: "file",
        payload_bytes: 64,
    });
    assert.deepEqual(await pending, {
        ok: true,
        plugin_id: "demo.plugin",
        enabled: false,
        source: "file",
        payload_bytes: 64,
    });
    for (const enabled of ["yes", 1, null, undefined]) {
        await assert.rejects(window.remcord.plugins.setEnabled("alpha", enabled), /enabled must be a boolean/);
    }
    // Echo mismatches reject: one plugin's flip can never resolve another's call.
    const mismatch = createPluginsRuntime();
    const mismatchPending = mismatch.window.remcord.plugins.setEnabled("alpha", true);
    const mismatchMessage = mismatch.window.messages.find((entry) => entry.command?.type === "plugins_set_enabled");
    respondPluginFlip(mismatch.window, mismatchMessage, {
        ok: true,
        plugin_id: "beta",
        enabled: true,
        source: "file",
        payload_bytes: 1,
    });
    await assert.rejects(mismatchPending, /plugin flip response schema is invalid/);
});

test("plugins.reload sends id-only and resolves the re-validation outcome", async () => {
    const { window } = createPluginsRuntime();
    const pending = window.remcord.plugins.reload("alpha");
    const message = window.messages.find((entry) => entry.command?.type === "plugins_reload");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "plugins_reload", payload: { plugin_id: "alpha" } });
    respondPluginReloaded(window, message, { ok: true, plugin_id: "alpha" });
    assert.deepEqual(await pending, { ok: true, plugin_id: "alpha" });
    for (const pluginId of ["", "..", "../escape", 42, null]) {
        await assert.rejects(window.remcord.plugins.reload(pluginId), /pluginId must match/);
    }
    assert.equal(
        window.messages.filter((entry) => entry.command?.type === "plugins_reload").length,
        1,
        "hostile ids never reach the wire",
    );
});

test("plugins.openFolder reveals the fixed plugins root", async () => {
    const { window } = createPluginsRuntime();
    const pending = window.remcord.plugins.openFolder();
    const message = window.messages.find((entry) => entry.command?.type === "folders_open");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "folders_open", payload: { location: "plugins" } });
    respondFoldersOpened(window, message, "plugins");
    assert.equal(await pending, "plugins");
});

test("plugin capabilities throw UNSUPPORTED_CAPABILITY when the snapshot denies them", () => {
    const { window } = createPluginsRuntime(["bridge.ping"]);
    assert.equal(window.remcord.native.supports("plugins.list"), false);
    assert.equal(window.remcord.native.supports("plugins.get_runtime_script"), false);
    assert.throws(
        () => window.remcord.plugins.list(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.throws(
        () => window.remcord.native.getPluginEntries("preload"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "plugins_list"),
        false,
    );
});

test("plugins.get_runtime_entries validates the closed target and drops paths", async () => {
    for (const target of ["preload", "renderer"]) {
        const runtime = createPluginsRuntime();
        const pending = runtime.window.remcord.native.getPluginEntries(target);
        const message = runtime.window.messages.find((entry) => entry.command?.type === "plugins_get_runtime_entries");
        assert.ok(message);
        assert.deepEqual(message.command, { type: "plugins_get_runtime_entries", payload: { target } });
        respondPluginEntries(runtime.window, message, target, [{ id: "alpha", name: "Alpha" }]);
        assert.deepEqual(await pending, [{ id: "alpha", name: "Alpha" }]);
    }
    const { window } = createPluginsRuntime();
    // `main` has no native runtime: it fails closed before dispatch, exactly
    // like any other unknown target.
    for (const target of ["main", "PRELOAD", "", "preload ", 42, null, undefined]) {
        await assert.rejects(window.remcord.native.getPluginEntries(target), /target must be preload or renderer/);
    }
    assert.equal(
        window.messages.some((entry) => entry.command?.type === "plugins_get_runtime_entries"),
        false,
    );
    // A path-bearing entry is rejected even when the envelope is well-formed.
    const hostile = createPluginsRuntime();
    const pending = hostile.window.remcord.native.getPluginEntries("preload");
    const hostileMessage = hostile.window.messages.find(
        (entry) => entry.command?.type === "plugins_get_runtime_entries",
    );
    respondPluginEntries(hostile.window, hostileMessage, "preload", [{ id: "alpha", name: "Alpha", path: "C:\\x" }]);
    await assert.rejects(pending, /plugin entries response schema is invalid/);
    // A mismatched target echo is rejected.
    const mismatched = createPluginsRuntime();
    const mismatchPending = mismatched.window.remcord.native.getPluginEntries("preload");
    const mismatchMessage = mismatched.window.messages.find(
        (entry) => entry.command?.type === "plugins_get_runtime_entries",
    );
    respondPluginEntries(mismatched.window, mismatchMessage, "renderer", [{ id: "alpha", name: "Alpha" }]);
    await assert.rejects(mismatchPending, /plugin entries response schema is invalid/);
});

test("plugins.get_runtime_script validates id+target and bounds the source", async () => {
    const { window } = createPluginsRuntime();
    const pending = window.remcord.native.getPluginScript("alpha", "preload");
    const message = window.messages.find((entry) => entry.command?.type === "plugins_get_runtime_script");
    assert.ok(message);
    assert.deepEqual(message.command, {
        type: "plugins_get_runtime_script",
        payload: { plugin_id: "alpha", target: "preload" },
    });
    respondPluginScript(window, message, "alpha", "preload", "module.exports.activate = () => {};");
    assert.deepEqual(await pending, {
        plugin_id: "alpha",
        target: "preload",
        source: "module.exports.activate = () => {};",
    });
    // A null source (undeclared/missing/empty entry) resolves; the loader
    // skips falsy sources like Electron.
    const second = createPluginsRuntime();
    const nullPending = second.window.remcord.native.getPluginScript("alpha", "renderer");
    const nullMessage = second.window.messages.find((entry) => entry.command?.type === "plugins_get_runtime_script");
    respondPluginScript(second.window, nullMessage, "alpha", "renderer", null);
    assert.deepEqual(await nullPending, { plugin_id: "alpha", target: "renderer", source: null });
    for (const pluginId of ["", "..", "../escape"]) {
        await assert.rejects(window.remcord.native.getPluginScript(pluginId, "preload"), /pluginId must match/);
    }
    for (const target of ["main", "RENDERER", ""]) {
        await assert.rejects(
            window.remcord.native.getPluginScript("alpha", target),
            /target must be preload or renderer/,
        );
    }
    // An over-limit source is rejected before it can reach the loader.
    const big = createPluginsRuntime();
    const bigPending = big.window.remcord.native.getPluginScript("alpha", "preload");
    const bigMessage = big.window.messages.find((entry) => entry.command?.type === "plugins_get_runtime_script");
    respondPluginScript(big.window, bigMessage, "alpha", "preload", `x="${"x".repeat(1048576)}"`);
    await assert.rejects(bigPending, /plugin source response schema is invalid/);
});

const PLUGIN_LOADER_CAPS = ["bridge.ping", "plugins.get_runtime_entries", "plugins.get_runtime_script"];

function createPluginLoaderRuntime(origin = "https://discord.com", capabilities = PLUGIN_LOADER_CAPS) {
    const runtime = createRuntime(origin, capabilities);
    // The shared harness stubs only error/info; fixtures exercise the
    // plugin logger fully.
    runtime.context.console = { error() {}, info() {}, log() {}, warn() {} };
    return runtime;
}

/** Flushes pending microtasks so the loader's sequential chain can advance. */
async function flushLoader(rounds = 30) {
    for (let i = 0; i < rounds; i++) {
        await Promise.resolve();
    }
}

function pluginCommands(window) {
    return window.messages.filter((message) => message.command?.type?.startsWith("plugins_"));
}

function pluginResults(window) {
    return window.messages.filter((message) => message.event?.type === "plugin_result");
}

const ALPHA_PRELOAD_SOURCE = [
    "globalThis.__f1PreloadRan = true;",
    "module.exports.activate = (api) => {",
    "    globalThis.__f1ApiId = api.id;",
    '    api.logger.log("preload hello");',
    '    api.patcher.before(globalThis.__f1Target, "greet", function (name) { globalThis.__f1BeforeSeen = name; });',
    '    const unpatch = api.patcher.instead(globalThis.__f1Target, "calc", function (args, original) { return original(args[0], args[1]) * 10; });',
    "    globalThis.__f1Unpatch = unpatch;",
    "    api.onCleanup(() => { globalThis.__f1CleanupRan = true; });",
    "};",
].join("\n");

const ALPHA_RENDERER_SOURCE = [
    "const tick = `tick-42`;",
    "module.exports.activate = (api) => {",
    "    globalThis.__f1RendererRan = api.name;",
    "    globalThis.__f1Ticks = tick;",
    "};",
].join("\n");

const BOOM_SOURCE = 'module.exports.activate = () => { throw new Error("boom"); };';

/**
 * Drives one loader target to completion: answers the entries request,
 * then each script request from `sources` ({ id: source|null }), flushing
 * between steps so the sequential chain advances.
 */
async function drivePluginTarget(window, target, sources) {
    await flushLoader(10);
    const entriesMessage = window.messages.find(
        (message) =>
            message.command?.type === "plugins_get_runtime_entries" &&
            message.command.payload.target === target &&
            !message.answered,
    );
    assert.ok(entriesMessage, `entries request for ${target}`);
    entriesMessage.answered = true;
    respondPluginEntries(
        window,
        entriesMessage,
        target,
        Object.keys(sources).map((id) => ({ id, name: id })),
    );
    for (const [id, source] of Object.entries(sources)) {
        await flushLoader(10);
        const scriptMessage = window.messages.find(
            (message) =>
                message.command?.type === "plugins_get_runtime_script" &&
                message.command.payload.plugin_id === id &&
                message.command.payload.target === target &&
                !message.answered,
        );
        assert.ok(scriptMessage, `script request for ${id}/${target}`);
        scriptMessage.answered = true;
        respondPluginScript(window, scriptMessage, id, target, source);
    }
    await flushLoader(20);
}

test("the plugin loader executes preload then renderer entries with a uniform api", async () => {
    const { window, context } = createPluginLoaderRuntime();
    context.__f1Target = {
        greet(name) {
            return `hi ${name}`;
        },
        calc(a, b) {
            return a + b;
        },
    };
    vm.runInContext(pluginLoader, context);
    assert.equal(window.__remcordNativePluginLoader, true);
    await drivePluginTarget(window, "preload", { alpha: ALPHA_PRELOAD_SOURCE });
    await drivePluginTarget(window, "renderer", { alpha: ALPHA_RENDERER_SOURCE });
    // Execution order: preload entries fully precede renderer entries.
    const commands = pluginCommands(window).map((message) => message.command);
    assert.deepEqual(commands[0], { type: "plugins_get_runtime_entries", payload: { target: "preload" } });
    assert.ok(
        commands.some(
            (command) => command.type === "plugins_get_runtime_entries" && command.payload.target === "renderer",
        ),
    );
    // The fixture ran in the page world with the loader-built api.
    assert.equal(context.__f1PreloadRan, true);
    assert.equal(context.__f1ApiId, "alpha");
    assert.equal(context.__f1RendererRan, "alpha");
    // Backtick template sources survive (the wrapper concatenates; an
    // interpolated template literal would corrupt on these bytes).
    assert.equal(context.__f1Ticks, "tick-42");
    // The before/instead patches wrapped the target methods.
    assert.equal(context.__f1Target.greet("bob"), "hi bob");
    assert.equal(context.__f1BeforeSeen, "bob");
    assert.equal(context.__f1Target.calc(2, 3), 50);
    // The returned unpatch restores the original method.
    context.__f1Unpatch();
    assert.equal(context.__f1Target.calc(2, 3), 5);
    // Both targets reported success with the closed wire shape.
    const results = pluginResults(window);
    assert.equal(results.length, 2);
    for (const result of results) {
        assert.equal(result.event.payload.plugin_id, "alpha");
        assert.equal(result.event.payload.ok, true);
        assert.equal(result.event.payload.error, null);
    }
    assert.deepEqual(results.map((result) => result.event.payload.target).sort(), ["preload", "renderer"]);
    // The readable status object mirrors the reports exactly once each.
    const statuses = window.__remcordNativePlugins;
    assert.equal(statuses.alpha.status, "executed");
    assert.equal(statuses.alpha.error, null);
    // No source, path, or URL ever crosses back to the host.
    const serialized = JSON.stringify(results);
    assert.doesNotMatch(serialized, /"source"|"path"|"url"/);
});

test("the loader skips falsy sources and isolates per-plugin failures", async () => {
    const { window, context } = createPluginLoaderRuntime();
    vm.runInContext(pluginLoader, context);
    await drivePluginTarget(window, "preload", {
        good: "module.exports.activate = () => { globalThis.__f1Good = 1; };",
        bad: BOOM_SOURCE,
        missing: null,
    });
    await drivePluginTarget(window, "renderer", {});
    assert.equal(context.__f1Good, 1);
    const results = pluginResults(window);
    assert.equal(results.length, 2);
    const byId = Object.fromEntries(results.map((result) => [result.event.payload.plugin_id, result.event.payload]));
    assert.equal(byId.good.ok, true);
    assert.equal(byId.bad.ok, false);
    assert.match(byId.bad.error, /boom/);
    // A null source is skipped silently: no report, no failure.
    assert.equal("missing" in byId, false);
    assert.equal(window.__remcordNativePlugins.missing, undefined);
    assert.equal(window.__remcordNativePlugins.bad.status, "failed");
});

test("the loader ignores non-entry origins and double injection", async () => {
    const evil = createPluginLoaderRuntime("https://example.com");
    vm.runInContext(pluginLoader, evil.context);
    await flushLoader();
    assert.deepEqual(evil.window.messages, []);
    assert.equal(evil.window.__remcordNativePluginLoader, undefined);
    const { window, context } = createPluginLoaderRuntime();
    vm.runInContext(pluginLoader, context);
    vm.runInContext(pluginLoader, context);
    await drivePluginTarget(window, "preload", {});
    await drivePluginTarget(window, "renderer", {});
    assert.equal(
        window.messages.filter((message) => message.command?.type === "plugins_get_runtime_entries").length,
        2,
        "one entries request per target despite double injection",
    );
});

test("the loader stops without capabilities and carries no artifact surface", () => {
    const { window, context } = createPluginLoaderRuntime("https://discord.com", ["bridge.ping"]);
    vm.runInContext(pluginLoader, context);
    // The bootstrap metrics event is unrelated facade traffic; no plugin
    // command may be sent without its capability.
    assert.equal(
        window.messages.some((message) => message.command?.type?.startsWith("plugins_")),
        false,
    );
    const code = stripComments(pluginLoader);
    const urls = code.match(/https?:\/\/[^\s"'`)]+/g) ?? [];
    assert.deepEqual(urls, ["https://discord.com", "https://canary.discord.com", "https://ptb.discord.com"]);
    // The documented execution primitive is present; ambient Electron/Node
    // globals and file/path vocabulary are absent.
    assert.match(code, /new Function/);
    assert.match(code, /getPluginEntries/);
    assert.match(code, /getPluginScript/);
    assert.doesNotMatch(code, /require\(|webFrame|ipcRenderer|process\.|readFile|writeFile/);
    assert.doesNotMatch(code, /shelter-builds|githubusercontent|example\.invalid/);
});

test("mod.clearCache sends an exact empty payload and resolves the outcome", async () => {
    const { window } = createDiagnosticsRuntime();
    const pending = window.remcord.mod.clearCache();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "mod_clear_cache");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "mod_clear_cache", payload: {} });
    respondModCacheCleared(window, message);
    assert.deepEqual(JSON.parse(JSON.stringify(await pending)), {
        cleared: ["vencord.js"],
        mod_cache_cleared: true,
    });
});

test("mod.clearCache throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createDiagnosticsRuntime(["bridge.ping", "diagnostics.get_info"]);
    assert.equal(window.remcord.native.supports("mod.clear_cache"), false);
    assert.throws(
        () => window.remcord.mod.clearCache(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "mod_clear_cache"),
        false,
    );
});

test("mod.clearCache rejects malformed outcomes", async () => {
    for (const payload of [
        { cleared: ["custom.js"], mod_cache_cleared: true },
        { cleared: ["../escape"], mod_cache_cleared: false },
        { cleared: "vencord.js", mod_cache_cleared: true },
        {
            cleared: ["vencord.js", "vencord.css", "equicord.js", "equicord.css", "shelter.js", "extra.js"],
            mod_cache_cleared: true,
        },
        { cleared: [], mod_cache_cleared: "yes" },
        { cleared: [] },
        { cleared: [], mod_cache_cleared: false, path: "C:\\x" },
    ]) {
        const runtime = createDiagnosticsRuntime();
        const pending = runtime.window.remcord.mod.clearCache();
        const message = runtime.window.messages.find((entry) => entry.command?.type === "mod_clear_cache");
        runtime.window.dispatchEvent(
            new FakeCustomEvent("remcord-native:response", {
                detail: {
                    version: 1,
                    request_id: message.request_id,
                    ok: true,
                    payload: { type: "mod_cache_cleared", payload },
                    error: null,
                },
            }),
        );
        await assert.rejects(pending, /mod cache response schema is invalid/);
    }
});

test("diagnostics and mod cache requests stay inert off the trusted origin", async () => {
    const { window } = createRuntime("https://attacker.invalid", [
        "bridge.ping",
        "diagnostics.get_info",
        "mod.clear_cache",
    ]);
    assert.equal(window.remcord, undefined);
});

function createPluginStorageRuntime(capabilities = ["bridge.ping", "plugin_storage.read", "plugin_storage.write"]) {
    return createRuntime("https://discord.com", capabilities);
}

function respondPluginRead(window, request, data = "hello remcord") {
    const bytes = new TextEncoder().encode(data).length;
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "plugin_storage_read", payload: { data, payload_bytes: bytes } },
                error: null,
            },
        }),
    );
}

function respondPluginWrite(window, request, payloadBytes = 5) {
    window.dispatchEvent(
        new FakeCustomEvent("remcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload: { type: "plugin_storage_write", payload: { payload_bytes: payloadBytes } },
                error: null,
            },
        }),
    );
}

test("the facade exposes a namespaced pluginStorage capability and no generic fs", () => {
    const { window } = createPluginStorageRuntime();
    assert.equal(typeof window.remcord.pluginStorage.readFile, "function");
    assert.equal(typeof window.remcord.pluginStorage.writeFile, "function");
    assert.equal(Object.isFrozen(window.remcord.pluginStorage), true);
    assert.equal(window.remcord.native.supports("plugin_storage.read"), true);
    assert.equal(window.remcord.native.supports("plugin_storage.write"), true);
    // A generic filesystem abstraction must never exist.
    assert.equal(window.remcord.fs, undefined);
});

test("pluginStorage.writeFile sends a scoped typed request and resolves bytes", async () => {
    const { window } = createPluginStorageRuntime();
    const pending = window.remcord.pluginStorage.writeFile("my-plugin", "cache/note.txt", "hello");
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "plugin_storage_write");
    assert.ok(message);
    assert.deepEqual(message.command, {
        type: "plugin_storage_write",
        payload: { plugin_id: "my-plugin", relative_path: "cache/note.txt", data: "hello" },
    });
    respondPluginWrite(window, message, 5);
    assert.equal(await pending, 5);
});

test("pluginStorage.readFile sends a scoped typed request and resolves text", async () => {
    const { window } = createPluginStorageRuntime();
    const pending = window.remcord.pluginStorage.readFile("my-plugin", "cache/note.txt");
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "plugin_storage_read");
    assert.ok(message);
    assert.deepEqual(message.command, {
        type: "plugin_storage_read",
        payload: { plugin_id: "my-plugin", relative_path: "cache/note.txt" },
    });
    respondPluginRead(window, message, "hello remcord");
    assert.equal(await pending, "hello remcord");
});

test("pluginStorage throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const denied = createPluginStorageRuntime(["bridge.ping"]);
    assert.equal(denied.window.remcord.native.supports("plugin_storage.read"), false);
    assert.equal(denied.window.remcord.native.supports("plugin_storage.write"), false);
    assert.throws(
        () => denied.window.remcord.pluginStorage.readFile("p", "a.txt"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.throws(
        () => denied.window.remcord.pluginStorage.writeFile("p", "a.txt", "x"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        denied.window.messages.some((message) => message.command?.type === "plugin_storage_read"),
        false,
    );
    assert.equal(
        denied.window.messages.some((message) => message.command?.type === "plugin_storage_write"),
        false,
    );
});

test("pluginStorage rejects malformed requests without sending", async () => {
    const { window } = createPluginStorageRuntime();
    await assert.rejects(window.remcord.pluginStorage.readFile("", "a.txt"), /pluginId/);
    await assert.rejects(window.remcord.pluginStorage.readFile("a/b", "a.txt"), /pluginId/);
    await assert.rejects(window.remcord.pluginStorage.readFile("p", ""), /relativePath/);
    await assert.rejects(window.remcord.pluginStorage.writeFile("p", "a.txt", 42), /data must be a string/);
    await assert.rejects(window.remcord.pluginStorage.writeFile(42, "a.txt", "x"), /pluginId/);
    assert.equal(
        window.messages.some((message) => message.command?.type?.startsWith("plugin_storage")),
        false,
    );
});

test("pluginStorage rejects malformed responses", async () => {
    const { window } = createPluginStorageRuntime();
    const pending = window.remcord.pluginStorage.readFile("p", "a.txt");
    const message = window.messages.find((entry) => entry.command?.type === "plugin_storage_read");
    respondPluginRead(window, message, 42);
    await assert.rejects(pending, /plugin storage response schema is invalid/);

    const second = createPluginStorageRuntime();
    const pending2 = second.window.remcord.pluginStorage.writeFile("p", "a.txt", "x");
    const message2 = second.window.messages.find((entry) => entry.command?.type === "plugin_storage_write");
    respondPluginWrite(second.window, message2, -1);
    await assert.rejects(pending2, /plugin storage response schema is invalid/);
});

test("pluginStorage propagates typed host failures", async () => {
    for (const code of ["NOT_FOUND", "INVALID_PATH", "EXTENSION_DISABLED", "TOO_LARGE"]) {
        const { window } = createPluginStorageRuntime();
        const pending = window.remcord.pluginStorage.readFile("p", "a.txt");
        const message = window.messages.find((entry) => entry.command?.type === "plugin_storage_read");
        window.dispatchEvent(
            new FakeCustomEvent("remcord-native:response", {
                detail: {
                    version: 1,
                    request_id: message.request_id,
                    ok: false,
                    payload: null,
                    error: { code, message: code },
                },
            }),
        );
        await assert.rejects(pending, (error) => error.code === code);
    }
});

test("pluginStorage rejects after its timeout", async () => {
    const { window } = createPluginStorageRuntime();
    await assert.rejects(window.remcord.pluginStorage.readFile("p", "a.txt", 50), /timed out/);
    await assert.rejects(window.remcord.pluginStorage.writeFile("p", "a.txt", "x", 50), /timed out/);
});

test("pluginStorage rejects pending requests when the document is disposed", async () => {
    const { window } = createPluginStorageRuntime();
    const read = window.remcord.pluginStorage.readFile("p", "a.txt");
    const write = window.remcord.pluginStorage.writeFile("p", "a.txt", "x");
    const readRejection = assert.rejects(read, /document was disposed/);
    const writeRejection = assert.rejects(write, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await readRejection;
    await writeRejection;
});

test("a reloaded document gets a fresh pluginStorage facade with no stale promises", async () => {
    const firstDocument = createPluginStorageRuntime();
    const stale = firstDocument.window.remcord.pluginStorage.readFile("p", "a.txt");
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createPluginStorageRuntime();
    assert.notEqual(firstDocument.window.remcord, nextDocument.window.remcord);
    const pending = nextDocument.window.remcord.pluginStorage.readFile("p", "a.txt");
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "plugin_storage_read");
    respondPluginRead(nextDocument.window, message, "persisted");
    assert.equal(await pending, "persisted");
});

// Managed theme-CSS lifecycle contract (Phase D theme injection).
//
// The host injection template itself is owned by Rust
// (`managed_css_script`/`managed_css_remove_script` in
// `native/src/mod_substrate.rs`, structurally asserted there). These tests
// pin the DOM semantics that template relies on: one stable `<style>` per
// host style id, reuse instead of duplication, append-order cascade,
// idempotent removal, and clean reset on document reload. The helpers below
// implement exactly the documented contract step-for-step.
function createFakeThemeDocument() {
    const byId = new Map();
    const headChildren = [];
    const listeners = new Map();
    const document = {
        head: {
            children: headChildren,
            appendChild(child) {
                // Real-DOM semantics: appending an attached node MOVES it to
                // the end of head. This is exactly why the host template
                // appends only newly created styles and updates existing
                // ones in place.
                const index = headChildren.indexOf(child);
                if (index >= 0) headChildren.splice(index, 1);
                headChildren.push(child);
                if (child.id) byId.set(child.id, child);
            },
            querySelectorAll(selector) {
                if (selector !== "style") return [];
                return headChildren.filter((node) => node.tagName === "style" && !node.removed);
            },
        },
        getElementById(id) {
            const node = byId.get(id);
            return node && !node.removed ? node : null;
        },
        createElement(tag) {
            const node = {
                tagName: tag,
                id: "",
                textContent: "",
                removed: false,
                remove() {
                    this.removed = true;
                    byId.delete(this.id);
                    const index = headChildren.indexOf(this);
                    if (index >= 0) headChildren.splice(index, 1);
                },
            };
            return node;
        },
        addEventListener(type, listener) {
            const entries = listeners.get(type) ?? [];
            entries.push(listener);
            listeners.set(type, entries);
        },
        querySelectorAll(selector) {
            const match = /^style\[id\^="([^"]+)"\]$/.exec(selector);
            if (!match) return [];
            return headChildren.filter(
                (node) => node.tagName === "style" && !node.removed && node.id.startsWith(match[1]),
            );
        },
    };
    return { document, headChildren, listeners };
}

function managedThemeInject(document, styleId, css) {
    // Mirrors the fixed host template step-for-step: a newly created style
    // is appended once; an existing style updates in place and is NEVER
    // re-appended (re-appending would move it to the end of head and
    // corrupt the Vencord -> Quick CSS -> sorted-themes cascade).
    const inject = () => {
        const root = document.head;
        if (!root) return false;
        let style = document.getElementById(styleId);
        if (!style) {
            style = document.createElement("style");
            style.id = styleId;
            style.textContent = css;
            root.appendChild(style);
        } else {
            style.textContent = css;
        }
        return true;
    };
    if (!inject()) document.addEventListener("DOMContentLoaded", inject, { once: true });
}

function managedCssOrder(document, orderedIds) {
    // Mirrors the host order-enforcement script: re-append the listed
    // EXISTING nodes in the given order (missing ids skipped), moving them
    // into the correct relative sequence. Content-only updates never call
    // this; structural changes (add/remove/recreate) do.
    const root = document.head;
    if (!root) return;
    for (const id of orderedIds) {
        const style = document.getElementById(id);
        if (style) root.appendChild(style);
    }
}

function managedThemeRemove(document, styleId) {
    const style = document.getElementById(styleId);
    if (style) style.remove();
}

test("theme injection creates one managed style with the host identity", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:1;}");
    assert.equal(headChildren.length, 1);
    assert.equal(headChildren[0].id, "remcord-native-theme-alpha");
    assert.equal(headChildren[0].textContent, ":root{--a:1;}");
});

test("theme injection reuses its style instead of duplicating", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:2;}");
    assert.equal(headChildren.length, 1);
    assert.equal(document.getElementById("remcord-native-theme-alpha").textContent, ":root{--a:2;}");
});

test("themes keep host emission order in the document head", () => {
    const { document, headChildren } = createFakeThemeDocument();
    // The host emits sorted by theme id; append order is the cascade order.
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "remcord-native-theme-zulu", ":root{--z:1;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-native-theme-alpha", "remcord-native-theme-zulu"],
    );
});

test("theme removal is idempotent and removes only its own style", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "remcord-native-theme-zulu", ":root{--z:1;}");
    managedThemeRemove(document, "remcord-native-theme-alpha");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-native-theme-zulu"],
    );
    assert.equal(document.getElementById("remcord-native-theme-alpha"), null);
    // Removing a missing style (disabled theme, fresh document) is a no-op.
    managedThemeRemove(document, "remcord-native-theme-alpha");
    managedThemeRemove(document, "remcord-native-theme-never-enabled");
    assert.equal(headChildren.length, 1);
});

test("a reloaded document starts with no theme styles and re-injects cleanly", () => {
    const first = createFakeThemeDocument();
    managedThemeInject(first.document, "remcord-native-theme-alpha", ":root{--a:1;}");
    assert.equal(first.headChildren.length, 1);

    // Reload: a fresh document has no carried-over nodes.
    const next = createFakeThemeDocument();
    assert.equal(next.headChildren.length, 0);
    managedThemeRemove(next.document, "remcord-native-theme-alpha");
    managedThemeInject(next.document, "remcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(next.document, "remcord-native-theme-zulu", ":root{--z:1;}");
    assert.deepEqual(
        next.headChildren.map((node) => node.id),
        ["remcord-native-theme-alpha", "remcord-native-theme-zulu"],
    );
});

test("theme styles coexist with the Vencord managed style", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-native-vencord-css", "/*vencord*/");
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "remcord-native-vencord-css", "/*vencord*/");
    const ids = headChildren.map((node) => node.id);
    assert.ok(ids.includes("remcord-native-vencord-css"));
    assert.ok(ids.includes("remcord-native-theme-alpha"));
    assert.equal(ids.filter((id) => id === "remcord-native-vencord-css").length, 1);
    assert.equal(ids.filter((id) => id === "remcord-native-theme-alpha").length, 1);
});

test("the theme smoke observation sees exactly the managed theme styles", () => {
    const { document } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-native-vencord-css", "/*vencord*/");
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:1;}");
    const observed = document
        .querySelectorAll('style[id^="remcord-native-theme-"]')
        .map((node) => ({ id: node.id, bytes: (node.textContent || "").length }));
    assert.deepEqual(observed, [{ id: "remcord-native-theme-alpha", bytes: ":root{--a:1;}".length }]);
    // The smoke observation also records head order and Vencord coexistence.
    const headOrder = document.head.querySelectorAll("style").map((node) => node.id);
    assert.deepEqual(headOrder, ["remcord-native-vencord-css", "remcord-native-theme-alpha"]);
    assert.ok(headOrder.includes("remcord-native-vencord-css"));
});

// Managed Quick CSS lifecycle contract (Phase D Quick CSS host-push).
//
// The host push reuses the same managed-CSS builders as themes
// (`managed_css_script`/`managed_css_remove_script`, structurally asserted in
// Rust), so these tests pin the Quick CSS specifics on the shared contract:
// Electron's verbatim `remcord-quick-css` identity, injection BEFORE themes
// (Electron's `initQuickCss`-before-theme-loop order), empty-CSS push parity,
// and clean disabled/reload/reset behavior. Cascade note: for equal
// specificity, later stylesheets win, so append order IS the cascade order.
test("Quick CSS injects under Electron's verbatim style identity", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-quick-css", ":root{--q:1;}");
    assert.equal(headChildren.length, 1);
    assert.equal(headChildren[0].id, "remcord-quick-css");
    assert.equal(headChildren[0].textContent, ":root{--q:1;}");
});

test("Quick CSS injection reuses its style instead of duplicating", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-quick-css", ":root{--q:1;}");
    managedThemeInject(document, "remcord-quick-css", ":root{--q:2;}");
    assert.equal(headChildren.length, 1);
    assert.equal(document.getElementById("remcord-quick-css").textContent, ":root{--q:2;}");
});

test("empty Quick CSS is pushed as an empty managed style like Electron", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-quick-css", "");
    assert.equal(headChildren.length, 1);
    assert.equal(document.getElementById("remcord-quick-css").textContent, "");
});

test("Quick CSS injects before themes in the document head", () => {
    const { document, headChildren } = createFakeThemeDocument();
    // The host emits Quick CSS before the sorted theme set, matching
    // Electron's initQuickCss-before-theme-loop order.
    managedThemeInject(document, "remcord-quick-css", ":root{--probe:quickcss;}");
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--probe:theme;}");
    managedThemeInject(document, "remcord-native-theme-zulu", ":root{--probe:theme;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-quick-css", "remcord-native-theme-alpha", "remcord-native-theme-zulu"],
    );
});

test("disabled Quick CSS leaves no style while themes still inject", () => {
    const { document, headChildren } = createFakeThemeDocument();
    // Disabled means the host never pushes: no node is created at all.
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:1;}");
    assert.equal(document.getElementById("remcord-quick-css"), null);
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-native-theme-alpha"],
    );
    // Disabling after an injection removes exactly the Quick CSS style.
    managedThemeInject(document, "remcord-quick-css", ":root{--q:1;}");
    managedThemeRemove(document, "remcord-quick-css");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-native-theme-alpha"],
    );
});

test("a reloaded document resets Quick CSS alongside themes", () => {
    const first = createFakeThemeDocument();
    managedThemeInject(first.document, "remcord-quick-css", ":root{--q:1;}");
    managedThemeInject(first.document, "remcord-native-theme-alpha", ":root{--a:1;}");
    assert.equal(first.headChildren.length, 2);

    const next = createFakeThemeDocument();
    assert.equal(next.headChildren.length, 0);
    managedThemeRemove(next.document, "remcord-quick-css");
    managedThemeInject(next.document, "remcord-quick-css", ":root{--q:1;}");
    managedThemeInject(next.document, "remcord-native-theme-alpha", ":root{--a:1;}");
    assert.deepEqual(
        next.headChildren.map((node) => node.id),
        ["remcord-quick-css", "remcord-native-theme-alpha"],
    );
});

test("Quick CSS coexists with Vencord and themes in cascade order", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-native-vencord-css", "/*vencord*/");
    managedThemeInject(document, "remcord-quick-css", ":root{--probe:quickcss;}");
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--probe:theme;}");
    // Re-injecting everything (as a reload would) duplicates nothing.
    managedThemeInject(document, "remcord-quick-css", ":root{--probe:quickcss;}");
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--probe:theme;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-native-vencord-css", "remcord-quick-css", "remcord-native-theme-alpha"],
    );
});

test("the smoke observation reports Quick CSS presence and bytes", () => {
    const { document } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-quick-css", ":root{--q:1;}");
    const quickCssNode = document.getElementById("remcord-quick-css");
    const observed = {
        quickcss_present: Boolean(quickCssNode),
        quickcss_bytes: quickCssNode ? (quickCssNode.textContent || "").length : 0,
    };
    assert.deepEqual(observed, { quickcss_present: true, quickcss_bytes: ":root{--q:1;}".length });

    const empty = createFakeThemeDocument();
    const missing = empty.document.getElementById("remcord-quick-css");
    assert.deepEqual(
        { quickcss_present: Boolean(missing), quickcss_bytes: missing ? missing.textContent.length : 0 },
        { quickcss_present: false, quickcss_bytes: 0 },
    );
});

// Live CSS lifecycle contract (Phase D live watching).
//
// The host owns exactly two native watchers (profile dir filtered to the
// fixed quickCss.css name; themes root recursive), debounces bursts (~300
// ms), re-reads authoritative state, and applies minimal managed-CSS work:
// content updates in place, structural changes finish with an order repair.
// These tests pin the renderer-side DOM semantics the live path relies on:
// updates never move nodes, order repair restores cascade position, bursts
// collapse to one final state, disabled/deleted resources remove exactly
// their own style, and reloads reset cleanly without watcher or style
// accumulation (watcher ownership itself is asserted in Rust).
test("a live content update never moves the style in the cascade", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-quick-css", ":root{--probe:quickcss;}");
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "remcord-native-theme-zulu", ":root{--z:1;}");
    // Live: alpha CSS changes on disk -> in-place update, no order repair.
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:2;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-quick-css", "remcord-native-theme-alpha", "remcord-native-theme-zulu"],
    );
    assert.equal(document.getElementById("remcord-native-theme-alpha").textContent, ":root{--a:2;}");
    // Live: Quick CSS changes -> still first, themes untouched.
    managedThemeInject(document, "remcord-quick-css", ":root{--probe:quickcss2;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-quick-css", "remcord-native-theme-alpha", "remcord-native-theme-zulu"],
    );
});

test("a recreated Quick CSS style is repaired back before themes", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-quick-css", ":root{--q:1;}");
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:1;}");
    // Live: file deleted then recreated -> remove + fresh append lands last.
    managedThemeRemove(document, "remcord-quick-css");
    assert.equal(document.getElementById("remcord-quick-css"), null);
    managedThemeInject(document, "remcord-quick-css", ":root{--q:2;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-native-theme-alpha", "remcord-quick-css"],
    );
    // Host structural repair: Quick CSS first, then sorted themes.
    managedCssOrder(document, ["remcord-quick-css", "remcord-native-theme-alpha"]);
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-quick-css", "remcord-native-theme-alpha"],
    );
    assert.equal(document.getElementById("remcord-quick-css").textContent, ":root{--q:2;}");
});

test("a newly added theme is repaired into sorted cascade position", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-quick-css", ":root{--q:1;}");
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "remcord-native-theme-zulu", ":root{--z:1;}");
    // Live: theme "mike" created -> appended at the end, out of position.
    managedThemeInject(document, "remcord-native-theme-mike", ":root{--m:1;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-quick-css", "remcord-native-theme-alpha", "remcord-native-theme-zulu", "remcord-native-theme-mike"],
    );
    // Host structural repair restores sorted order without touching bytes.
    managedCssOrder(document, [
        "remcord-quick-css",
        "remcord-native-theme-alpha",
        "remcord-native-theme-mike",
        "remcord-native-theme-zulu",
    ]);
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-quick-css", "remcord-native-theme-alpha", "remcord-native-theme-mike", "remcord-native-theme-zulu"],
    );
});

test("rapid live edits collapse to one node with the final bytes", () => {
    const { document, headChildren } = createFakeThemeDocument();
    // Burst A -> B -> C within one debounce window: the host re-reads once
    // and pushes only the latest bytes.
    managedThemeInject(document, "remcord-quick-css", ":root{--v:a;}");
    managedThemeInject(document, "remcord-quick-css", ":root{--v:b;}");
    managedThemeInject(document, "remcord-quick-css", ":root{--v:c;}");
    assert.equal(headChildren.length, 1);
    assert.equal(document.getElementById("remcord-quick-css").textContent, ":root{--v:c;}");
});

test("disable removes the style and re-enable injects current bytes once", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-quick-css", ":root{--q:1;}");
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:1;}");
    // Live: disabled while the file keeps changing -> style removed, file
    // writes issue no renderer work (nothing injected here).
    managedThemeRemove(document, "remcord-quick-css");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-native-theme-alpha"],
    );
    // Live: re-enabled -> current bytes appear exactly once, before themes.
    managedThemeInject(document, "remcord-quick-css", ":root{--q:99;}");
    managedCssOrder(document, ["remcord-quick-css", "remcord-native-theme-alpha"]);
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-quick-css", "remcord-native-theme-alpha"],
    );
    assert.equal(headChildren.filter((node) => node.id === "remcord-quick-css").length, 1);
});

test("deleted themes remove exactly their stale style", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "remcord-quick-css", ":root{--q:1;}");
    managedThemeInject(document, "remcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "remcord-native-theme-zulu", ":root{--z:1;}");
    // Live: zulu directory deleted -> stale style removed, rest untouched.
    managedThemeRemove(document, "remcord-native-theme-zulu");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-quick-css", "remcord-native-theme-alpha"],
    );
    // Live: zulu recreated -> injected, then repaired into sorted position.
    managedThemeInject(document, "remcord-native-theme-zulu", ":root{--z:2;}");
    managedCssOrder(document, ["remcord-quick-css", "remcord-native-theme-alpha", "remcord-native-theme-zulu"]);
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["remcord-quick-css", "remcord-native-theme-alpha", "remcord-native-theme-zulu"],
    );
});

test("live updates keep Vencord first across reloads without duplication", () => {
    const first = createFakeThemeDocument();
    managedThemeInject(first.document, "remcord-native-vencord-css", "/*vencord*/");
    managedThemeInject(first.document, "remcord-quick-css", ":root{--q:1;}");
    managedThemeInject(first.document, "remcord-native-theme-alpha", ":root{--a:1;}");
    // Live content update: Vencord stays first, no duplicates.
    managedThemeInject(first.document, "remcord-native-theme-alpha", ":root{--a:2;}");
    managedThemeInject(first.document, "remcord-quick-css", ":root{--q:2;}");
    assert.deepEqual(
        first.headChildren.map((node) => node.id),
        ["remcord-native-vencord-css", "remcord-quick-css", "remcord-native-theme-alpha"],
    );
    // Reload: fresh document re-pushes the same order, still no duplicates.
    const next = createFakeThemeDocument();
    managedThemeInject(next.document, "remcord-native-vencord-css", "/*vencord*/");
    managedThemeInject(next.document, "remcord-quick-css", ":root{--q:2;}");
    managedThemeInject(next.document, "remcord-native-theme-alpha", ":root{--a:2;}");
    assert.deepEqual(
        next.headChildren.map((node) => node.id),
        ["remcord-native-vencord-css", "remcord-quick-css", "remcord-native-theme-alpha"],
    );
    assert.equal(first.headChildren.length, 3);
});

test("the facade exposes a frozen settingsBoot surface with exactly the approved members", () => {
    const { window } = createRuntime();
    const boot = window.remcord.settingsBoot;
    assert.ok(boot);
    assert.deepEqual(Object.keys(boot).sort(), [
        "i18n",
        "lang",
        "restart",
        "setLang",
        "settings",
        "settingsSource",
        "themes",
    ]);
    assert.equal(Object.isFrozen(window.remcord), true);
    assert.equal(Object.isFrozen(boot), true);
    assert.equal(Object.isFrozen(boot.settings), true);
    assert.equal(Object.isFrozen(boot.themes), true);
    assert.equal(Object.isFrozen(boot.i18n), true);
    assert.equal(boot.settingsSource, "defaults");
    assert.equal(boot.lang, "en-US");
    assert.equal(boot.i18n["invite-goBackToApp"], "Go back to the Discord app");
    assert.equal(typeof boot.setLang, "function");
    assert.equal(typeof boot.restart, "function");
});

test("settingsBoot carries no path, URL, source, or renderer-supplied members", () => {
    const { window } = createRuntime();
    const boot = window.remcord.settingsBoot;
    for (const key of ["path", "url", "source", "file", "pluginId", "plugin_id", "eval"]) {
        assert.equal(key in boot, false, `settingsBoot must not carry ${key}`);
        assert.equal(key in (boot.settings ?? {}), false);
    }
    const serialized = JSON.stringify({ settings: boot.settings, themes: boot.themes });
    for (const forbidden of ["remcord://", "ts-out", "userData", "AppData"]) {
        assert.equal(serialized.includes(forbidden), false);
    }
});

test("settingsBoot.setLang sends one narrow fire-and-forget locale event", () => {
    const { window } = createRuntime();
    const result = window.remcord.settingsBoot.setLang("en-US");
    assert.equal(result, undefined);
    const events = window.messages.filter((message) => message.event?.type === "settings_boot_set_lang");
    assert.equal(events.length, 1);
    assert.deepEqual(events[0].event.payload, { locale: "en-US" });
});

test("settingsBoot.setLang rejects empty, oversized, and hostile locales without sending", () => {
    const { window } = createRuntime();
    for (const locale of ["", "en US", "en;US", "../escape", "a".repeat(33), 42, null, undefined, {}]) {
        assert.throws(() => window.remcord.settingsBoot.setLang(locale), TypeError);
    }
    assert.equal(
        window.messages.some((message) => message.event?.type === "settings_boot_set_lang"),
        false,
    );
});

test("settingsBoot.restart sends an empty fire-and-forget event", () => {
    const { window } = createRuntime();
    const result = window.remcord.settingsBoot.restart();
    assert.equal(result, undefined);
    const events = window.messages.filter((message) => message.event?.type === "settings_boot_restart");
    assert.equal(events.length, 1);
    assert.deepEqual(events[0].event.payload, {});
});

test("an unavailable settings source carries a null config with working narrow functions", () => {
    const { window } = createRuntime("https://discord.com", ["bridge.ping", "mod.get_bundle"], {
        settingsSource: "unavailable",
        settings: null,
        themes: [],
        lang: "en-US",
        i18n: {},
    });
    const boot = window.remcord.settingsBoot;
    assert.equal(boot.settingsSource, "unavailable");
    assert.equal(boot.settings, null);
    assert.equal(Object.isFrozen(boot), true);
    window.remcord.settingsBoot.setLang("de");
    window.remcord.settingsBoot.restart();
    assert.equal(window.messages.filter((message) => message.event?.type === "settings_boot_set_lang").length, 1);
    assert.equal(window.messages.filter((message) => message.event?.type === "settings_boot_restart").length, 1);
});

test("a reloaded document gets a fresh frozen settingsBoot with no shared identity", () => {
    const first = createRuntime();
    const next = createRuntime();
    assert.ok(first.window.remcord.settingsBoot);
    assert.ok(next.window.remcord.settingsBoot);
    assert.equal(first.window.remcord.settingsBoot === next.window.remcord.settingsBoot, false);
    assert.equal(first.window.remcord.settingsBoot.settings === next.window.remcord.settingsBoot.settings, false);
    assert.equal(Object.isFrozen(next.window.remcord.settingsBoot), true);
});

test("the settingsBoot surface adds no generic sync bridge, eval, or invocation", () => {
    const code = stripComments(bootstrapTemplate);
    assert.equal(/sendSync/.test(code), false);
    assert.equal(/[^a-zA-Z]eval\(/.test(code), false);
    assert.equal(/new Function/.test(code), false);
    assert.equal(/require\(["\x27]electron["\x27]\)/.test(code), false);
    assert.equal(code.includes("settings_boot_set_lang"), true);
    assert.equal(code.includes("settings_boot_restart"), true);
});

/** Runtime with the settings capability granted, as the production boot snapshot grants it. */
function createShimRuntime(origin = "https://discord.com", settingsBoot) {
    return createRuntime(origin, ["bridge.ping", "settings.get_config"], settingsBoot);
}

test("the closed shim exposes exactly the seven sync members plus two platform strings", () => {
    const { window } = createShimRuntime();
    const remcord = window.remcord;
    assert.equal(typeof remcord.settings.getConfig, "function");
    assert.equal(typeof remcord.settings.setLang, "function");
    assert.equal(typeof remcord.themes.getThemes, "function");
    assert.equal(typeof remcord.translations, "object");
    assert.equal(typeof remcord.restart, "function");
    assert.equal(remcord.platform, "win32");
    assert.equal(remcord.osRelease, "10.0.22631");
    assert.equal(Object.isFrozen(remcord), true);
    assert.equal(Object.isFrozen(remcord.settings), true);
    assert.equal(Object.isFrozen(remcord.themes), true);
});

test("the shim exposes no generic sync API beyond the closed members", () => {
    const { window } = createRuntime();
    const remcord = window.remcord;
    assert.equal(remcord.settings.setConfig, undefined);
    assert.equal(remcord.themes.install, undefined);
    assert.equal(remcord.themes.set, undefined);
    assert.equal(remcord.themes.edit, undefined);
    assert.equal(remcord.themes.refresh, undefined);
    assert.equal(remcord.themes.folder, undefined);
    assert.equal(remcord.version, undefined);
    assert.equal(remcord.fs, undefined);
    // The plugins namespace is closed to its four members (list, enable
    // flip, reload, folder reveal): no source, path, manifest, or eval
    // surface may exist alongside them.
    assert.deepEqual(Object.keys(remcord.plugins).sort(), ["list", "openFolder", "reload", "setEnabled"]);
    assert.equal(remcord.plugins.execute, undefined);
    // backup.save plus the inspect/commit/discard trio are closed members:
    // the typed JSON-string save and the parameter-free preview/commit/
    // discard, each with the long-operation timeout. None takes a path.
    assert.equal(typeof remcord.backup.save, "function");
    assert.equal(typeof remcord.backup.inspect, "function");
    assert.equal(typeof remcord.backup.commitRestore, "function");
    assert.equal(typeof remcord.backup.discardRestore, "function");
    assert.equal(remcord.backup.restore, undefined);
    assert.equal(typeof remcord.translations, "object");
});

test("shim getConfig returns the mutable snapshot synchronously with no throw", () => {
    const { window } = createShimRuntime();
    const first = window.remcord.settings.getConfig();
    assert.equal(first.blockPowerSavingInVoiceChat, false);
    assert.equal(JSON.stringify(first.mods), JSON.stringify([]));
    // The plugin mutates the working copy (setConfig writes, keybind push):
    // frozen-source throws would break it, so writes must succeed.
    first.blockPowerSavingInVoiceChat = true;
    assert.equal(first.blockPowerSavingInVoiceChat, true);
    first.mods.push("shelter");
    assert.equal(JSON.stringify(first.mods), JSON.stringify(["shelter"]));
    // ...while the host snapshot stays frozen and unchanged.
    assert.equal(window.remcord.settingsBoot.settings.blockPowerSavingInVoiceChat, false);
    assert.equal(Object.isFrozen(window.remcord.settingsBoot.settings), true);
});

test("shim getConfig preserves identity within a document and isolates across documents", () => {
    const first = createShimRuntime();
    const again = first.window.remcord.settings.getConfig();
    assert.equal(first.window.remcord.settings.getConfig() === again, true);
    again.blockPowerSavingInVoiceChat = true;
    assert.equal(first.window.remcord.settings.getConfig().blockPowerSavingInVoiceChat, true);

    const next = createShimRuntime();
    assert.equal(next.window.remcord.settings.getConfig() === again, false);
    assert.equal(next.window.remcord.settings.getConfig().blockPowerSavingInVoiceChat, false);
});

test("shim getThemes returns the closed six-field projection", () => {
    const snapshot = {
        settingsSource: "file",
        settings: { mods: [] },
        themes: [
            {
                name: "Full",
                author: "a",
                description: "d",
                version: "1",
                enabled: true,
                theme: "src.css",
                updateSrc: "https://x",
                id: "full",
            },
            { name: "Sparse", enabled: false, id: "sparse" },
            { name: 42, enabled: true, id: "bad" },
        ],
        lang: "en-US",
        i18n: {},
    };
    const { window } = createRuntime("https://discord.com", ["bridge.ping"], snapshot);
    const themes = window.remcord.themes.getThemes();
    assert.equal(themes.length, 2);
    // JSON-compared: the projection lives in the document realm, so its
    // prototype differs from test-realm literals.
    assert.equal(
        JSON.stringify(themes[0]),
        JSON.stringify({
            id: "full",
            name: "Full",
            author: "a",
            description: "d",
            version: "1",
            enabled: true,
        }),
    );
    assert.equal(
        JSON.stringify(themes[1]),
        JSON.stringify({
            id: "sparse",
            name: "Sparse",
            author: "",
            description: "",
            version: "",
            enabled: false,
        }),
    );
    assert.equal(window.remcord.themes.getThemes() === themes, true);
});

test("shim translations exposes the frozen snapshot object as-is", () => {
    const { window } = createRuntime();
    assert.equal(window.remcord.translations === window.remcord.settingsBoot.i18n, true);
    assert.equal(Object.isFrozen(window.remcord.translations), true);
    assert.equal(window.remcord.translations["invite-goBackToApp"], "Go back to the Discord app");
});

test("shim setLang posts the narrow locale event and restart posts the empty event", () => {
    const { window } = createRuntime();
    assert.equal(window.remcord.settings.setLang("de"), undefined);
    assert.equal(window.remcord.restart(), undefined);
    assert.deepEqual(
        window.messages
            .filter((message) => message.event?.type === "settings_boot_set_lang")
            .map((message) => message.event.payload),
        [{ locale: "de" }],
    );
    assert.deepEqual(
        window.messages
            .filter((message) => message.event?.type === "settings_boot_restart")
            .map((message) => message.event.payload),
        [{}],
    );
    // No synchronous forbidden work: the only outbound traffic from both
    // calls is the two closed one-way events (no commands, no responses).
    assert.equal(
        window.messages.some((message) => message.command),
        false,
    );
    assert.throws(() => window.remcord.settings.setLang("../escape"), TypeError);
    assert.throws(() => window.remcord.settings.setLang(""), TypeError);
});

test("the shim replays the real plugin onLoad reads without sync-member failures", () => {
    // Mirrors src/shelter/settings/index.ts:90-99 + settings.ts:14-20:
    // refreshSettings, refreshThemes, translations, setLang,
    // registerSections flag - twice-called getConfig included.
    const { window } = createShimRuntime();
    const store = {};
    store.settings = window.remcord.settings.getConfig();
    store.themes = window.remcord.themes.getThemes();
    store.i18n = window.remcord.translations;
    window.remcord.settings.setLang("en-US");
    const sections = ["remcord-settings", "remcord-themes"];
    if (window.remcord.settings.getConfig().showExperimentalPluginMenu) sections.push("remcord-plugins");
    assert.deepEqual(sections, ["remcord-settings", "remcord-themes"]);
    assert.equal(store.settings.blockPowerSavingInVoiceChat, false);
    assert.equal(store.themes.length, 0);
    assert.equal(store.i18n["invite-goBackToApp"], "Go back to the Discord app");
});

test("the renderer cannot replace the host snapshot or shim before plugin execution", () => {
    const { window } = createRuntime();
    assert.throws(() => {
        window.remcord.settingsBoot = {};
    }, TypeError);
    assert.throws(() => {
        window.remcord.translations = {};
    }, TypeError);
    assert.throws(() => {
        window.remcord.platform = "darwin";
    }, TypeError);
    assert.equal(window.remcord.platform, "win32");
});

test("the embedded stream picker script parses and carries no unspliced placeholder", () => {
    // Regression guard: a syntax error in the Rust-embedded picker script
    // kills the ENTIRE document-start bundle (settings boot, mod adapters,
    // quality wrapper, titlebar plugin). Parsing here without executing
    // catches that class before it ships.
    const rust = readFileSync(new URL("../src/screenshare_picker.rs", import.meta.url), "utf8");
    const marker = 'const SCREENSHARE_PICKER_SCRIPT: &str = r#"';
    const start = rust.indexOf(marker);
    assert.notEqual(start, -1, "picker script constant must exist");
    let end = -1;
    for (let i = start + marker.length; i < rust.length - 1; i++) {
        if (rust[i] === '"' && rust[i + 1] === "#") {
            end = i;
            break;
        }
    }
    assert.notEqual(end, -1, "raw string terminator must exist");
    const script = rust.slice(start + marker.length, end);
    assert.doesNotThrow(() => new vm.Script(script), "picker script must parse");
    assert.ok(!script.includes("__REMCORD_"), "no unspliced placeholder may ship");
    for (const token of ["__remcordStreamPicker", "NotAllowedError", "displaySurface", "data-rmc-sp"]) {
        assert.ok(script.includes(token), `${token} must be present`);
    }
});

// ---------------------------------------------------------------------------
// F4: in-mod plugin settings pages (Vencord/Equicord/Shelter) need no host
// capability. Each mod renders its own settings UI page-side once the shared
// substrate executes its bundle; the host contributes storage reachability
// plus the Vencord-family QuickCSS editor handoff only. These tests pin that
// "no new surface" contract on the shipped renderer sources.
// ---------------------------------------------------------------------------

test("F4 adds no settings-page capability or wire command to the facade", () => {
    const { window } = createRuntime();
    // No settings-page namespace may exist alongside the closed namespaces.
    assert.equal(window.remcord.modSettings, undefined);
    assert.equal(window.remcord.pluginSettings, undefined);
    assert.equal(window.remcord.settingsPage, undefined);
    for (const capability of [
        "mod.settings",
        "mod.settings_page",
        "mod_settings",
        "plugin.settings",
        "plugin.settings_page",
        "plugin_settings",
        "settings.page",
        "settings_page",
        "open_settings",
    ]) {
        assert.equal(window.remcord.native.supports(capability), false);
    }
    // No settings-page wire traffic may exist either.
    assert.equal(
        window.messages.some(
            (message) =>
                (message.command?.type ?? "").includes("settings_page") ||
                (message.command?.type ?? "").includes("mod_settings") ||
                (message.command?.type ?? "").includes("plugin_settings"),
        ),
        false,
    );
    const code = stripComments(bootstrapTemplate);
    for (const fragment of ["mod_settings", "plugin_settings", "settings_page", "open_settings"]) {
        assert.equal(code.includes(fragment), false, `${fragment} must not ship in the facade`);
    }
});

test("F4 keeps the QuickCSS handoff as the only editor bridge for Vencord-family settings", () => {
    // The handoff event name and owning modes are the contract asserted in
    // Rust (`native/src/mod_settings.rs`): the settings plugin titles the
    // editor and resolves storage from the mode.
    const substrateCode = stripComments(modSubstrate);
    assert.ok(substrateCode.includes("remcord-native:open-quickcss-editor"));
    assert.ok(substrateCode.includes("overrideQuickCssEditor"));
    const vencordCode = stripComments(vencordAdapter);
    const equicordCode = stripComments(equicordAdapter);
    assert.ok(vencordCode.includes('overrideQuickCssEditor("vencord")'));
    assert.ok(equicordCode.includes('overrideQuickCssEditor("equicord")'));
    // Shelter and Custom install no editor override: their settings UI has
    // no popup-editor path to hand off.
    assert.equal(shelterAdapterCode.includes("overrideQuickCssEditor"), false);
    assert.equal(customAdapterCode.includes("overrideQuickCssEditor"), false);
});

test("F4 keeps mod settings storage page-side with host preservation only", () => {
    // Vencord/Equicord persist in localStorage (owned by their bundles; the
    // host only keeps storage reachable and snapshots its state), Shelter
    // keeps its state in IndexedDB (no host key at all). The preservation
    // script is the host mechanism the settings UI depends on.
    assert.ok(storagePreservation.includes("localStorage"));
    assert.ok(storagePreservation.includes("__remcordNativeStorageAtDocumentStart"));
    // The adapters report the storage snapshot back to the host with the
    // snake_case wire names; the factory carries the camelCase state.
    const vencordCode = stripComments(vencordAdapter);
    assert.ok(vencordCode.includes("local_storage_api"));
    assert.ok(vencordCode.includes("settings_state"));
    const substrateCode = stripComments(modSubstrate);
    assert.ok(substrateCode.includes("createModAdapter"));
});

test("F19 exposes no deeplink capability: discord:// navigation stays host-only", () => {
    // F19 is host-side OS integration (scheme registration, argv scan,
    // pushState navigation, second-instance handoff). The renderer must
    // never be able to trigger navigation, so the facade carries no
    // deeplink namespace, capability, or wire command.
    const { window } = createRuntime();
    for (const capability of [
        "discord.deeplink",
        "deeplink.navigate",
        "protocol.open",
        "protocol.handle",
        "protocol.register",
        "open_url",
        "second_instance",
    ]) {
        assert.equal(window.remcord.native.supports(capability), false);
    }
    // No deeplink wire traffic may exist either.
    assert.equal(
        window.messages.some(
            (message) =>
                (message.command?.type ?? "").includes("deeplink") ||
                (message.command?.type ?? "").includes("discord_link") ||
                (message.command?.type ?? "").includes("protocol_link"),
        ),
        false,
    );
    const code = stripComments(bootstrapTemplate);
    for (const fragment of [
        "deeplink",
        "discord://",
        "second-instance",
        "open-url",
        "pushState",
        "setAsDefaultProtocolClient",
    ]) {
        assert.equal(code.includes(fragment), false, `${fragment} must not ship in the facade`);
    }
});

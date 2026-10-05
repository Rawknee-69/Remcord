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

function createBootstrap(capabilities) {
    return bootstrapTemplate.replace(
        "__LEGCORD_BOOT_SNAPSHOT__",
        JSON.stringify({
            runtime: "webview2",
            platform: "win32",
            protocol_version: 1,
            capabilities,
        }),
    );
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

function createRuntime(origin = "https://discord.com", capabilities = ["bridge.ping", "mod.get_bundle"]) {
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
    vm.runInContext(createBootstrap(capabilities), context);
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
    window.legcord = {
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
        new FakeCustomEvent("legcord-native:response", {
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
        new FakeCustomEvent("legcord-native:response", {
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
        stateName: "__legcordNativeVencord",
        adapterFlag: "__legcordNativeVencordAdapter",
        domEvent: "legcord-native:vencord-bootstrap",
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
        stateName: "__legcordNativeShelter",
        adapterFlag: "__legcordNativeShelterAdapter",
        domEvent: "legcord-native:shelter-bootstrap",
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
        stateName: "__legcordNativeEquicord",
        adapterFlag: "__legcordNativeEquicordAdapter",
        domEvent: "legcord-native:equicord-bootstrap",
        marker: "equicord-bootstrap-executed",
        version: "1489c0e2a4435d854d7343a5cfb0abb894772318",
        resultEvent: "mod_bootstrap_result",
        ackEvent: "mod_response_ack",
        errorEvent: "mod_adapter_error",
        echoesModId: true,
        cssBytes: 217092,
        cssInjectionMs: 5,
    },
};

function respondMod(window, request, modId, payload = {}) {
    const wire = MOD_WIRE[modId];
    window.dispatchEvent(
        new FakeCustomEvent("legcord-native:response", {
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
        new FakeCustomEvent("legcord-native:response", {
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
    assert.equal(typeof window.legcord.settings.getConfig, "function");
    assert.equal(Object.isFrozen(window.legcord.settings), true);
    assert.equal(window.legcord.native.supports("settings.get_config"), true);
});

test("settings.getConfig sends a parameter-free typed request and resolves the snapshot", async () => {
    const { window } = createSettingsRuntime();
    const pending = window.legcord.settings.getConfig();
    assert.ok(pending instanceof Promise);
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
    assert.equal(window.legcord.native.supports("settings.get_config"), false);
    assert.throws(
        () => window.legcord.settings.getConfig(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "settings_get_config"),
        false,
    );
});

test("settings.getConfig rejects malformed snapshots", async () => {
    const { window } = createSettingsRuntime();
    const pending = window.legcord.settings.getConfig();
    const message = window.messages.find((entry) => entry.command?.type === "settings_get_config");
    respondSettings(window, message, { blockPowerSavingInVoiceChat: "yes" });
    await assert.rejects(pending, /settings response schema is invalid/);
});

test("settings.getConfig rejects host failures without a snapshot", async () => {
    const { window } = createSettingsRuntime();
    const pending = window.legcord.settings.getConfig();
    const message = window.messages.find((entry) => entry.command?.type === "settings_get_config");
    window.dispatchEvent(
        new FakeCustomEvent("legcord-native:response", {
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

test("settings.getConfig rejects after its timeout", async () => {
    const { window } = createSettingsRuntime();
    await assert.rejects(window.legcord.settings.getConfig(50), /timed out/);
});

test("settings.getConfig rejects pending requests when the document is disposed", async () => {
    const { window } = createSettingsRuntime();
    const pending = window.legcord.settings.getConfig();
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh settings facade with no stale promises", async () => {
    const firstDocument = createSettingsRuntime();
    const stale = firstDocument.window.legcord.settings.getConfig();
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createSettingsRuntime();
    assert.notEqual(firstDocument.window.legcord, nextDocument.window.legcord);
    const pending = nextDocument.window.legcord.settings.getConfig();
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
        new FakeCustomEvent("legcord-native:response", {
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
    assert.equal(typeof window.legcord.settings.setBlockPowerSavingInVoiceChat, "function");
    assert.equal(Object.isFrozen(window.legcord.settings), true);
    assert.equal(window.legcord.native.supports("settings.set_block_power_saving_in_voice_chat"), true);
    // No generic key/value RPC may exist.
    assert.equal(window.legcord.settings.setConfig, undefined);
});

test("the mutation setter sends a bool-only typed request and resolves the applied value", async () => {
    const { window } = createMutationRuntime();
    const pending = window.legcord.settings.setBlockPowerSavingInVoiceChat(true);
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "settings_set_block_power_saving");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "settings_set_block_power_saving", payload: { value: true } });

    respondMutation(window, message, true);
    assert.equal(await pending, true);
});

test("the mutation setter throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createMutationRuntime(["bridge.ping", "settings.get_config"]);
    assert.equal(window.legcord.native.supports("settings.set_block_power_saving_in_voice_chat"), false);
    assert.throws(
        () => window.legcord.settings.setBlockPowerSavingInVoiceChat(true),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "settings_set_block_power_saving"),
        false,
    );
});

test("the mutation setter rejects non-boolean values without sending", async () => {
    const { window } = createMutationRuntime();
    await assert.rejects(window.legcord.settings.setBlockPowerSavingInVoiceChat("yes"), /must be a boolean/);
    await assert.rejects(window.legcord.settings.setBlockPowerSavingInVoiceChat(1), /must be a boolean/);
    await assert.rejects(window.legcord.settings.setBlockPowerSavingInVoiceChat(undefined), /must be a boolean/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "settings_set_block_power_saving"),
        false,
    );
});

test("the mutation setter rejects malformed mutation responses", async () => {
    const { window } = createMutationRuntime();
    const pending = window.legcord.settings.setBlockPowerSavingInVoiceChat(true);
    const message = window.messages.find((entry) => entry.command?.type === "settings_set_block_power_saving");
    respondMutation(window, message, "yes");
    await assert.rejects(pending, /mutation response schema is invalid/);

    const second = createMutationRuntime();
    const pending2 = second.window.legcord.settings.setBlockPowerSavingInVoiceChat(false);
    const message2 = second.window.messages.find((entry) => entry.command?.type === "settings_set_block_power_saving");
    respondMutation(second.window, message2, false, { source: "defaults" });
    await assert.rejects(pending2, /mutation response schema is invalid/);
});

test("the mutation setter rejects host failures without applying state", async () => {
    const { window } = createMutationRuntime();
    const pending = window.legcord.settings.setBlockPowerSavingInVoiceChat(true);
    const message = window.messages.find((entry) => entry.command?.type === "settings_set_block_power_saving");
    window.dispatchEvent(
        new FakeCustomEvent("legcord-native:response", {
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
    await assert.rejects(window.legcord.settings.setBlockPowerSavingInVoiceChat(true, 50), /timed out/);
});

test("the mutation setter rejects pending requests when the document is disposed", async () => {
    const { window } = createMutationRuntime();
    const pending = window.legcord.settings.setBlockPowerSavingInVoiceChat(true);
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh mutation facade with no stale promises", async () => {
    const firstDocument = createMutationRuntime();
    const stale = firstDocument.window.legcord.settings.setBlockPowerSavingInVoiceChat(true);
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createMutationRuntime();
    const pending = nextDocument.window.legcord.settings.setBlockPowerSavingInVoiceChat(false);
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
    const pending = window.legcord.settings.setBlockPowerSavingInVoiceChat(true);
    const message = window.messages.find((entry) => entry.command?.type === "settings_set_block_power_saving");
    assert.equal(store.blockPowerSavingInVoiceChat, false);
    respondMutation(window, message, true);
    store.blockPowerSavingInVoiceChat = await pending;
    assert.equal(store.blockPowerSavingInVoiceChat, true);

    const failing = createMutationRuntime();
    const rejected = failing.window.legcord.settings.setBlockPowerSavingInVoiceChat(true);
    const failedMessage = failing.window.messages.find(
        (entry) => entry.command?.type === "settings_set_block_power_saving",
    );
    failing.window.dispatchEvent(
        new FakeCustomEvent("legcord-native:response", {
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

test("initializes a frozen compatibility facade with the boot snapshot", () => {
    const { window } = createRuntime();
    assert.ok(window.legcord);
    assert.equal(window.legcord.boot.runtime, "webview2");
    assert.equal(window.legcord.boot.platform, "win32");
    assert.equal(Object.isFrozen(window.legcord), true);
    assert.equal(Object.isFrozen(window.legcord.boot), true);
    assert.equal(window.legcord.native.supports("mod.get_bundle"), true);
    assert.equal(
        window.messages.some((message) => message.event?.type === "bootstrap_metrics"),
        true,
    );
});

test("rejects capabilities that are not in the immutable snapshot", () => {
    const { window } = createRuntime();
    assert.equal(window.legcord.native.supports("bridge.ping"), true);
    assert.equal(window.legcord.native.supports("filesystem"), false);
    assert.throws(
        () => window.legcord.native.requireCapability("filesystem"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
});

test("correlates asynchronous responses independently of their arrival order", async () => {
    const { window } = createRuntime();
    const first = window.legcord.native.ping("first");
    const second = window.legcord.native.ping("second");
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
    await assert.rejects(window.legcord.native.ping("é".repeat(65)), /128 UTF-8 bytes/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "ping"),
        false,
    );
});

test("disposes pending requests on navigation and initializes fresh document state", async () => {
    const firstDocument = createRuntime();
    const pending = firstDocument.window.legcord.native.ping("leaving");
    const rejection = assert.rejects(pending, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;

    const nextDocument = createRuntime();
    assert.ok(nextDocument.window.legcord);
    const request = nextDocument.window.legcord.native.ping("reloaded");
    const nextPing = nextDocument.window.messages.find((message) => message.command?.type === "ping");
    respondPing(nextDocument.window, nextPing, { nonce: "reloaded", message: "pong" });
    assert.deepEqual(await request, { nonce: "reloaded", message: "pong" });
    assert.equal(firstDocument.window.legcord === nextDocument.window.legcord, false);
});

test("does not expose the facade outside the exact trusted origin", () => {
    const { window } = createRuntime("https://canary.discord.com");
    assert.equal(window.legcord, undefined);
    assert.equal(window.messages.length, 0);
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

test("requests only the shared bundle capability with the registered mod id", async () => {
    const { window } = createRuntime();
    const request = window.legcord.native.getVencordBundle();
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
    const request = window.legcord.native.getModBundle("vencord");
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
            () => window.legcord.native.getModBundle(hostile),
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
    const { window } = createRuntime();
    for (const unregistered of ["vencord.js", "shelter", "equicord", "custom", "Vencord"]) {
        const request = window.legcord.native.getModBundle(unregistered);
        const message = window.messages.filter((entry) => entry.command?.type === "mod_get_bundle").pop();
        assert.deepEqual(message.command.payload, { mod_id: unregistered });
        window.dispatchEvent(
            new FakeCustomEvent("legcord-native:response", {
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
        () => window.legcord.native.getModBundle("vencord"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.throws(
        () => window.legcord.native.getVencordBundle(),
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
        const request = window.legcord.native.getModBundle("vencord");
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
        () => window.legcord.native.getModBundle("vencord", 5),
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
        new FakeCustomEvent("legcord-native:response", {
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
    assert.equal(window.__legcordNativeVencord.status, "failed");
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
    assert.equal(window.__legcordNativeVencord.status, "failed");
});

test("Vencord adapter executes once and marks only the bootstrap result", async () => {
    const { window, context } = createRuntime();
    vm.runInContext(vencordAdapter, context);
    vm.runInContext(vencordAdapter, context);
    const request = window.messages.find((message) => message.command?.type === "mod_get_bundle");
    assert.ok(request);
    assert.equal(window.messages.filter((message) => message.command?.type === "mod_get_bundle").length, 1);
    assert.equal(window.__legcordNativeVencord.status, "pending");

    window.dispatchEvent(
        new FakeCustomEvent("legcord-native:vencord-bootstrap", {
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
    assert.equal(window.__legcordNativeVencord.status, "executed");
    assert.equal(window.__legcordNativeVencord.marker, "vencord-bootstrap-executed");
    assert.equal(window.__legcordNativeVencord.executionCount, 1);
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
    assert.notEqual(firstDocument.window.__legcordNativeVencord, secondDocument.window.__legcordNativeVencord);
    assert.equal(secondDocument.window.__legcordNativeVencord.status, "pending");
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
    assert.notEqual(firstDocument.window.legcord, secondDocument.window.legcord);

    // Disposing the previous document clears its pending bundle request, so
    // a late host answer can no longer resolve anything.
    const lateRequest = firstDocument.window.legcord.native.getModBundle("vencord");
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await assert.rejects(lateRequest, /document was disposed/);
    assert.equal(secondDocument.window.__legcordNativeVencord.status, "pending");
    respondVencord(secondDocument.window, secondRequest, { mod_id: "equicord" });
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    // The adapter's own identity check rejects a response whose echoed
    // registry id is not Vencord: a cross-mod bundle can never be
    // acknowledged as a Vencord bootstrap even with a perfect marker.
    secondDocument.window.dispatchEvent(
        new FakeCustomEvent("legcord-native:vencord-bootstrap", {
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
    assert.equal(secondDocument.window.__legcordNativeVencord.status, "executed");
    respondVencord(secondDocument.window, secondRequest, { mod_id: "equicord" });
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    assert.match(secondDocument.window.__legcordNativeVencord.error, /not for Vencord/);
    assert.equal(
        secondDocument.window.messages.some((message) => message.event?.type === "vencord_response_ack"),
        false,
    );
    secondDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
});

test("the substrate factory is idempotent and shared by every adapter", () => {
    const { window, context } = createRuntime();
    const factory = window.__legcordNativeModSubstrate;
    assert.ok(factory);
    assert.equal(typeof factory.createModAdapter, "function");
    assert.equal(Object.isFrozen(factory), true);
    vm.runInContext(modSubstrate, context);
    assert.equal(window.__legcordNativeModSubstrate, factory);
});

// ---------------------------------------------------------------------------
// Registered mod consumers (Shelter, Equicord) share this lifecycle verbatim.
// Every assertion below runs for BOTH mods, which is what makes the shared
// substrate's generality a tested property rather than a claim.
// ---------------------------------------------------------------------------

const REGISTERED_ADAPTERS = { shelter: shelterAdapter, equicord: equicordAdapter };

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
        assert.notEqual(first.window.legcord, second.window.legcord);
        assert.equal(second.window[wire.stateName].status, "pending");
        assert.equal(second.window[wire.stateName].executionCount, 0);

        // The previous document's pending request is disposed, so a late host
        // answer cannot resolve anything in the new document.
        const late = first.window.legcord.native.getModBundle(modId);
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
            new FakeCustomEvent("legcord-native:response", {
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

    test(`${modId} does not expose the facade or request anything off the trusted origin`, () => {
        for (const origin of [
            "https://canary.discord.com",
            "https://example.com",
            "https://discord.com.attacker.invalid",
        ]) {
            const { window, context } = createRuntime(origin);
            vm.runInContext(adapter, context);
            assert.equal(window.legcord, undefined, origin);
            assert.deepEqual(window.messages, [], origin);
        }
    });

    test(`${modId} adapter carries no artifact URL, path, source, or eval surface`, () => {
        // Assertions run on executable/configuration content only: comments
        // legitimately document what the adapter deliberately omits.
        const code = stripComments(adapter);
        // The ONLY URL an adapter may name is the exact trusted Discord origin
        // it gates on. No bundle source, no upstream host, no cache file.
        const urls = code.match(/https?:\/\/[^\s"'`)]+/g) ?? [];
        assert.deepEqual(urls, ["https://discord.com"], `${modId} names only the origin gate`);
        assert.doesNotMatch(code, /shelter-builds|githubusercontent|github\.com|releases\/download/);
        assert.doesNotMatch(code, /\.css["'`]/, "an adapter names no cache file");
        assert.doesNotMatch(code, /readFile|writeFile|loadScript|executeSource|executeScript/);
        assert.doesNotMatch(code, /eval\(|new Function/);
        assert.doesNotMatch(code, /process\.|require\(|webFrame|ipcRenderer|window\.legcord\.fs/);
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
    assert.equal(window.__legcordNativeShelter.status, "executed");
    // Shelter's own adapter declares no CSS vocabulary at all: there is no
    // stylesheet to inject and no empty style element to manufacture.
    assert.doesNotMatch(shelterAdapterCode, /cssResultEvent/);
    assert.doesNotMatch(shelterAdapterCode, /insertCSS|css_result/);
    // But it still READS the host's CSS metadata, which is null for this mod.
    assert.match(shelterAdapterCode, /cssBytes: result\.css_bytes/);
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
    const state = window.__legcordNativeEquicord;
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
    const noCssRequest = noCss.window.legcord.native.getModBundle("shelter");
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
        const pending = runtime.window.legcord.native.getModBundle("shelter");
        respondMod(runtime.window, { request_id: pending.requestId }, "shelter", payload);
        await assert.rejects(pending, /schema is invalid/, JSON.stringify(payload));
    }
});

test("the shared factory serves Shelter and Equicord without Vencord identifiers", () => {
    for (const code of [shelterAdapterCode, equicordAdapterCode]) {
        assert.match(code, /__legcordNativeModSubstrate/);
        assert.match(code, /substrate\.createModAdapter\(/);
        // Neither adapter reaches into Vencord's identity or state: they use
        // only the shared factory and their own names.
        assert.doesNotMatch(code, /__legcordNativeVencord/);
        assert.doesNotMatch(code, /vencord-bootstrap-executed/);
        assert.doesNotMatch(code, /getModBundle\("vencord"\)/);
        assert.doesNotMatch(code, /legcord-native-vencord/);
        assert.doesNotMatch(code, /vencord_css_result/);
    }
});

test("a synthetic second mod runs the full substrate cycle without Vencord", async () => {
    const { window, host } = createSyntheticModRuntime();
    assert.equal(window.__legcordNativeTestMod.status, "pending");
    assert.equal(window.__legcordNativeVencord, undefined);

    window.dispatchEvent(
        new FakeCustomEvent("legcord-native:testmod-bootstrap", {
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
    assert.equal(window.__legcordNativeTestMod.status, "executed");
    assert.equal(window.__legcordNativeTestMod.marker, "testmod-bootstrap-executed");
    assert.equal(window.__legcordNativeTestMod.executionCount, 1);
    assert.equal(window.__legcordNativeTestMod.bundleVersion, "9.9.9");
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
    assert.equal(window.__legcordNativeTestMod.status, "pending");

    window.dispatchEvent(
        new FakeCustomEvent("legcord-native:testmod-bootstrap", {
            detail: { request_id: "testmod-1", ok: true, marker: "testmod-bootstrap-executed", execution_ms: 1 },
        }),
    );
    host().resolve({ marker: "wrong-marker", bundle_version: "9.9.9" });
    await Promise.resolve();
    await Promise.resolve();
    assert.equal(window.__legcordNativeTestMod.status, "failed");
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
        new FakeCustomEvent("legcord-native:response", {
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

test("the facade exposes getLang at the declared window.legcord location", () => {
    const { window } = createLangRuntime();
    assert.equal(typeof window.legcord.getLang, "function");
    assert.equal(window.legcord.native.supports("localization.get_lang"), true);
});

test("getLang sends one typed key-only request and resolves the string", async () => {
    const { window } = createLangRuntime();
    const pending = window.legcord.getLang("invite-goBackToApp");
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
    const pending = window.legcord.getLang("legcord-native-no-such-key");
    const message = window.messages.find((entry) => entry.command?.type === "localization_get_lang");
    respondLang(window, message, "");
    assert.equal(await pending, "");
});

test("getLang throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createLangRuntime(["bridge.ping"]);
    assert.throws(
        () => window.legcord.getLang("invite-goBackToApp"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "localization_get_lang"),
        false,
    );
});

test("getLang rejects empty, non-string, and oversized keys without sending", async () => {
    const { window } = createLangRuntime();
    await assert.rejects(window.legcord.getLang(""), /non-empty string/);
    await assert.rejects(window.legcord.getLang(42), /non-empty string/);
    await assert.rejects(window.legcord.getLang("k".repeat(129)), /128 UTF-8 bytes/);
    assert.equal(
        window.messages.some((message) => message.command?.type === "localization_get_lang"),
        false,
    );
});

test("getLang rejects malformed localization responses", async () => {
    const { window } = createLangRuntime();
    const pending = window.legcord.getLang("invite-goBackToApp");
    const message = window.messages.find((entry) => entry.command?.type === "localization_get_lang");
    respondLang(window, message, 42);
    await assert.rejects(pending, /localization response schema is invalid/);
});

test("getLang rejects host failures with the typed error", async () => {
    const { window } = createLangRuntime();
    const pending = window.legcord.getLang("invite-goBackToApp");
    const message = window.messages.find((entry) => entry.command?.type === "localization_get_lang");
    window.dispatchEvent(
        new FakeCustomEvent("legcord-native:response", {
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
    await assert.rejects(window.legcord.getLang("invite-goBackToApp", 50), /timed out/);
});

test("getLang rejects pending requests when the document is disposed", async () => {
    const { window } = createLangRuntime();
    const pending = window.legcord.getLang("invite-goBackToApp");
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh getLang facade with no stale promises", async () => {
    const firstDocument = createLangRuntime();
    const stale = firstDocument.window.legcord.getLang("invite-goBackToApp");
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createLangRuntime();
    assert.notEqual(firstDocument.window.legcord, nextDocument.window.legcord);
    const pending = nextDocument.window.legcord.getLang("invite-goBackToApp");
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "localization_get_lang");
    respondLang(nextDocument.window, message);
    assert.equal(await pending, "Go back to the Discord app");
});

function createAppRuntime(capabilities = ["bridge.ping", "app.get_version"]) {
    return createRuntime("https://discord.com", capabilities);
}

function respondAppVersion(window, request, version = "1.3.0") {
    window.dispatchEvent(
        new FakeCustomEvent("legcord-native:response", {
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
    assert.equal(typeof window.legcord.native.getVersion, "function");
    assert.equal(Object.isFrozen(window.legcord.native), true);
    assert.equal(window.legcord.native.supports("app.get_version"), true);
    // No synchronous version field or generic RPC may exist.
    assert.equal(window.legcord.version, undefined);
});

test("app.get_version sends a parameter-free typed request and resolves the version", async () => {
    const { window } = createAppRuntime();
    const pending = window.legcord.native.getVersion();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "app_get_version");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "app_get_version", payload: {} });

    respondAppVersion(window, message);
    assert.equal(await pending, "1.3.0");
});

test("app.get_version resolves the dev version exactly like Electron isDev", async () => {
    const { window } = createAppRuntime();
    const pending = window.legcord.native.getVersion();
    const message = window.messages.find((entry) => entry.command?.type === "app_get_version");
    respondAppVersion(window, message, "0.0.0");
    assert.equal(await pending, "0.0.0");
});

test("app.get_version throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createAppRuntime(["bridge.ping"]);
    assert.equal(window.legcord.native.supports("app.get_version"), false);
    assert.throws(
        () => window.legcord.native.getVersion(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "app_get_version"),
        false,
    );
});

test("app.get_version rejects malformed version responses", async () => {
    const { window } = createAppRuntime();
    const pending = window.legcord.native.getVersion();
    const message = window.messages.find((entry) => entry.command?.type === "app_get_version");
    respondAppVersion(window, message, 42);
    await assert.rejects(pending, /app version response schema is invalid/);

    const second = createAppRuntime();
    const pendingEmpty = second.window.legcord.native.getVersion();
    const messageEmpty = second.window.messages.find((entry) => entry.command?.type === "app_get_version");
    respondAppVersion(second.window, messageEmpty, "");
    await assert.rejects(pendingEmpty, /app version response schema is invalid/);
});

test("app.get_version rejects host failures with the typed error", async () => {
    const { window } = createAppRuntime();
    const pending = window.legcord.native.getVersion();
    const message = window.messages.find((entry) => entry.command?.type === "app_get_version");
    window.dispatchEvent(
        new FakeCustomEvent("legcord-native:response", {
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
    await assert.rejects(window.legcord.native.getVersion(50), /timed out/);
});

test("app.get_version rejects pending requests when the document is disposed", async () => {
    const { window } = createAppRuntime();
    const pending = window.legcord.native.getVersion();
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh app version facade with no stale promises", async () => {
    const firstDocument = createAppRuntime();
    const stale = firstDocument.window.legcord.native.getVersion();
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createAppRuntime();
    assert.notEqual(firstDocument.window.legcord, nextDocument.window.legcord);
    const pending = nextDocument.window.legcord.native.getVersion();
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "app_get_version");
    respondAppVersion(nextDocument.window, message);
    assert.equal(await pending, "1.3.0");
});

test("the migrated update checker compares the host-confirmed version", async () => {
    const { window } = createAppRuntime();
    // Mirrors src/shelter/settings/components/HeroUpdater.tsx: the remote
    // dotted version is compared against the capability-resolved local
    // version with dots stripped, never against a sync field.
    const pending = window.legcord.native.getVersion();
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
        new FakeCustomEvent("legcord-native:testmod-bootstrap", {
            detail: { request_id: "testmod-1", ok: false, marker: "", execution_ms: 1, error: "global missing" },
        }),
    );
    host().resolve({ marker: "testmod-bootstrap-executed", bundle_version: "9.9.9" });
    await Promise.resolve();
    await Promise.resolve();
    assert.equal(window.__legcordNativeTestMod.status, "failed");
    const result = window.messages.find((message) => message.event?.type === "testmod_bootstrap_result");
    assert.equal(result.event.payload.ok, false);
});

test("the synthetic adapter is denied when its capability is not granted", async () => {
    const { window } = createSyntheticModRuntime(["bridge.ping"]);
    await Promise.resolve();
    assert.equal(window.__legcordNativeTestMod.status, "failed");
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
    assert.equal(window.__legcordNativeStorageAtDocumentStart.localStorageApi, "function");
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
        new FakeCustomEvent("legcord-native:response", {
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

test("the facade exposes a read-only backup.list_bundles capability and no save/restore surface", () => {
    const { window } = createBundlesRuntime();
    assert.equal(typeof window.legcord.backup.listBundles, "function");
    assert.equal(Object.isFrozen(window.legcord.backup), true);
    assert.equal(window.legcord.native.supports("backup.list_bundles"), true);
    // Save/restore (dialogs + zip) are deferred: only discovery exists.
    assert.equal(window.legcord.backup.save, undefined);
    assert.equal(window.legcord.backup.restore, undefined);
    assert.equal(window.legcord.native.supports("backup.save"), false);
    assert.equal(window.legcord.native.supports("backup.restore"), false);
});

test("backup.list_bundles sends a parameter-free typed request and resolves the inventory", async () => {
    const { window } = createBundlesRuntime();
    const pending = window.legcord.backup.listBundles();
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "backup_list_bundles");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "backup_list_bundles", payload: {} });

    respondBundlesList(window, message);
    assert.deepEqual(await pending, BUNDLE_FIXTURE);
});

test("backup.list_bundles throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const { window } = createBundlesRuntime(["bridge.ping"]);
    assert.equal(window.legcord.native.supports("backup.list_bundles"), false);
    assert.throws(
        () => window.legcord.backup.listBundles(),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.equal(
        window.messages.some((message) => message.command?.type === "backup_list_bundles"),
        false,
    );
});

test("backup.list_bundles rejects malformed inventories", async () => {
    // Wrong entry count.
    const short = createBundlesRuntime();
    const pendingShort = short.window.legcord.backup.listBundles();
    const messageShort = short.window.messages.find((entry) => entry.command?.type === "backup_list_bundles");
    respondBundlesList(short.window, messageShort, BUNDLE_FIXTURE.slice(0, 6));
    await assert.rejects(pendingShort, /bundle inventory response schema is invalid/);

    // Wrong name or order.
    const renamed = createBundlesRuntime();
    const pendingRenamed = renamed.window.legcord.backup.listBundles();
    const messageRenamed = renamed.window.messages.find(
        (entry) => entry.command?.type === "backup_list_bundles",
    );
    respondBundlesList(renamed.window, messageRenamed, [
        ...BUNDLE_FIXTURE.slice(0, 5),
        { name: "custom.css", present: false, bytes: 0 },
        { name: "custom.js", present: false, bytes: 0 },
    ]);
    await assert.rejects(pendingRenamed, /bundle inventory response schema is invalid/);

    // Absent entry carrying bytes.
    const absentBytes = createBundlesRuntime();
    const pendingAbsent = absentBytes.window.legcord.backup.listBundles();
    const messageAbsent = absentBytes.window.messages.find(
        (entry) => entry.command?.type === "backup_list_bundles",
    );
    respondBundlesList(absentBytes.window, messageAbsent, [
        ...BUNDLE_FIXTURE.slice(0, 6),
        { name: "custom.css", present: false, bytes: 12 },
    ]);
    await assert.rejects(pendingAbsent, /bundle inventory response schema is invalid/);

    // Metadata-only: a path riding along is rejected.
    const withPath = createBundlesRuntime();
    const pendingPath = withPath.window.legcord.backup.listBundles();
    const messagePath = withPath.window.messages.find((entry) => entry.command?.type === "backup_list_bundles");
    respondBundlesList(withPath.window, messagePath, [
        ...BUNDLE_FIXTURE.slice(0, 6),
        { name: "custom.css", present: false, bytes: 0, path: "C:\\secret" },
    ]);
    await assert.rejects(pendingPath, /bundle inventory response schema is invalid/);
});

test("backup.list_bundles rejects host failures with the typed error", async () => {
    const { window } = createBundlesRuntime();
    const pending = window.legcord.backup.listBundles();
    const message = window.messages.find((entry) => entry.command?.type === "backup_list_bundles");
    window.dispatchEvent(
        new FakeCustomEvent("legcord-native:response", {
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
    await assert.rejects(window.legcord.backup.listBundles(50), /timed out/);
});

test("backup.list_bundles rejects pending requests when the document is disposed", async () => {
    const { window } = createBundlesRuntime();
    const pending = window.legcord.backup.listBundles();
    const rejection = assert.rejects(pending, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await rejection;
});

test("a reloaded document gets a fresh bundle inventory facade with no stale promises", async () => {
    const firstDocument = createBundlesRuntime();
    const stale = firstDocument.window.legcord.backup.listBundles();
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createBundlesRuntime();
    assert.notEqual(firstDocument.window.legcord, nextDocument.window.legcord);
    const pending = nextDocument.window.legcord.backup.listBundles();
    const message = nextDocument.window.messages.find((entry) => entry.command?.type === "backup_list_bundles");
    respondBundlesList(nextDocument.window, message);
    assert.deepEqual(await pending, BUNDLE_FIXTURE);
});

function createPluginStorageRuntime(capabilities = ["bridge.ping", "plugin_storage.read", "plugin_storage.write"]) {
    return createRuntime("https://discord.com", capabilities);
}

function respondPluginRead(window, request, data = "hello legcord") {
    const bytes = new TextEncoder().encode(data).length;
    window.dispatchEvent(
        new FakeCustomEvent("legcord-native:response", {
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
        new FakeCustomEvent("legcord-native:response", {
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
    assert.equal(typeof window.legcord.pluginStorage.readFile, "function");
    assert.equal(typeof window.legcord.pluginStorage.writeFile, "function");
    assert.equal(Object.isFrozen(window.legcord.pluginStorage), true);
    assert.equal(window.legcord.native.supports("plugin_storage.read"), true);
    assert.equal(window.legcord.native.supports("plugin_storage.write"), true);
    // A generic filesystem abstraction must never exist.
    assert.equal(window.legcord.fs, undefined);
});

test("pluginStorage.writeFile sends a scoped typed request and resolves bytes", async () => {
    const { window } = createPluginStorageRuntime();
    const pending = window.legcord.pluginStorage.writeFile("my-plugin", "cache/note.txt", "hello");
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
    const pending = window.legcord.pluginStorage.readFile("my-plugin", "cache/note.txt");
    assert.ok(pending instanceof Promise);
    const message = window.messages.find((entry) => entry.command?.type === "plugin_storage_read");
    assert.ok(message);
    assert.deepEqual(message.command, {
        type: "plugin_storage_read",
        payload: { plugin_id: "my-plugin", relative_path: "cache/note.txt" },
    });
    respondPluginRead(window, message, "hello legcord");
    assert.equal(await pending, "hello legcord");
});

test("pluginStorage throws UNSUPPORTED_CAPABILITY when the snapshot denies it", () => {
    const denied = createPluginStorageRuntime(["bridge.ping"]);
    assert.equal(denied.window.legcord.native.supports("plugin_storage.read"), false);
    assert.equal(denied.window.legcord.native.supports("plugin_storage.write"), false);
    assert.throws(
        () => denied.window.legcord.pluginStorage.readFile("p", "a.txt"),
        (error) => error.code === "UNSUPPORTED_CAPABILITY",
    );
    assert.throws(
        () => denied.window.legcord.pluginStorage.writeFile("p", "a.txt", "x"),
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
    await assert.rejects(window.legcord.pluginStorage.readFile("", "a.txt"), /pluginId/);
    await assert.rejects(window.legcord.pluginStorage.readFile("a/b", "a.txt"), /pluginId/);
    await assert.rejects(window.legcord.pluginStorage.readFile("p", ""), /relativePath/);
    await assert.rejects(window.legcord.pluginStorage.writeFile("p", "a.txt", 42), /data must be a string/);
    await assert.rejects(window.legcord.pluginStorage.writeFile(42, "a.txt", "x"), /pluginId/);
    assert.equal(
        window.messages.some((message) => message.command?.type?.startsWith("plugin_storage")),
        false,
    );
});

test("pluginStorage rejects malformed responses", async () => {
    const { window } = createPluginStorageRuntime();
    const pending = window.legcord.pluginStorage.readFile("p", "a.txt");
    const message = window.messages.find((entry) => entry.command?.type === "plugin_storage_read");
    respondPluginRead(window, message, 42);
    await assert.rejects(pending, /plugin storage response schema is invalid/);

    const second = createPluginStorageRuntime();
    const pending2 = second.window.legcord.pluginStorage.writeFile("p", "a.txt", "x");
    const message2 = second.window.messages.find((entry) => entry.command?.type === "plugin_storage_write");
    respondPluginWrite(second.window, message2, -1);
    await assert.rejects(pending2, /plugin storage response schema is invalid/);
});

test("pluginStorage propagates typed host failures", async () => {
    for (const code of ["NOT_FOUND", "INVALID_PATH", "EXTENSION_DISABLED", "TOO_LARGE"]) {
        const { window } = createPluginStorageRuntime();
        const pending = window.legcord.pluginStorage.readFile("p", "a.txt");
        const message = window.messages.find((entry) => entry.command?.type === "plugin_storage_read");
        window.dispatchEvent(
            new FakeCustomEvent("legcord-native:response", {
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
    await assert.rejects(window.legcord.pluginStorage.readFile("p", "a.txt", 50), /timed out/);
    await assert.rejects(window.legcord.pluginStorage.writeFile("p", "a.txt", "x", 50), /timed out/);
});

test("pluginStorage rejects pending requests when the document is disposed", async () => {
    const { window } = createPluginStorageRuntime();
    const read = window.legcord.pluginStorage.readFile("p", "a.txt");
    const write = window.legcord.pluginStorage.writeFile("p", "a.txt", "x");
    const readRejection = assert.rejects(read, /document was disposed/);
    const writeRejection = assert.rejects(write, /document was disposed/);
    window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await readRejection;
    await writeRejection;
});

test("a reloaded document gets a fresh pluginStorage facade with no stale promises", async () => {
    const firstDocument = createPluginStorageRuntime();
    const stale = firstDocument.window.legcord.pluginStorage.readFile("p", "a.txt");
    const staleRejection = assert.rejects(stale, /document was disposed/);
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
    await staleRejection;

    const nextDocument = createPluginStorageRuntime();
    assert.notEqual(firstDocument.window.legcord, nextDocument.window.legcord);
    const pending = nextDocument.window.legcord.pluginStorage.readFile("p", "a.txt");
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
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:1;}");
    assert.equal(headChildren.length, 1);
    assert.equal(headChildren[0].id, "legcord-native-theme-alpha");
    assert.equal(headChildren[0].textContent, ":root{--a:1;}");
});

test("theme injection reuses its style instead of duplicating", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:2;}");
    assert.equal(headChildren.length, 1);
    assert.equal(document.getElementById("legcord-native-theme-alpha").textContent, ":root{--a:2;}");
});

test("themes keep host emission order in the document head", () => {
    const { document, headChildren } = createFakeThemeDocument();
    // The host emits sorted by theme id; append order is the cascade order.
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "legcord-native-theme-zulu", ":root{--z:1;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-native-theme-alpha", "legcord-native-theme-zulu"],
    );
});

test("theme removal is idempotent and removes only its own style", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "legcord-native-theme-zulu", ":root{--z:1;}");
    managedThemeRemove(document, "legcord-native-theme-alpha");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-native-theme-zulu"],
    );
    assert.equal(document.getElementById("legcord-native-theme-alpha"), null);
    // Removing a missing style (disabled theme, fresh document) is a no-op.
    managedThemeRemove(document, "legcord-native-theme-alpha");
    managedThemeRemove(document, "legcord-native-theme-never-enabled");
    assert.equal(headChildren.length, 1);
});

test("a reloaded document starts with no theme styles and re-injects cleanly", () => {
    const first = createFakeThemeDocument();
    managedThemeInject(first.document, "legcord-native-theme-alpha", ":root{--a:1;}");
    assert.equal(first.headChildren.length, 1);

    // Reload: a fresh document has no carried-over nodes.
    const next = createFakeThemeDocument();
    assert.equal(next.headChildren.length, 0);
    managedThemeRemove(next.document, "legcord-native-theme-alpha");
    managedThemeInject(next.document, "legcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(next.document, "legcord-native-theme-zulu", ":root{--z:1;}");
    assert.deepEqual(
        next.headChildren.map((node) => node.id),
        ["legcord-native-theme-alpha", "legcord-native-theme-zulu"],
    );
});

test("theme styles coexist with the Vencord managed style", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-native-vencord-css", "/*vencord*/");
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "legcord-native-vencord-css", "/*vencord*/");
    const ids = headChildren.map((node) => node.id);
    assert.ok(ids.includes("legcord-native-vencord-css"));
    assert.ok(ids.includes("legcord-native-theme-alpha"));
    assert.equal(ids.filter((id) => id === "legcord-native-vencord-css").length, 1);
    assert.equal(ids.filter((id) => id === "legcord-native-theme-alpha").length, 1);
});

test("the theme smoke observation sees exactly the managed theme styles", () => {
    const { document } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-native-vencord-css", "/*vencord*/");
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:1;}");
    const observed = document
        .querySelectorAll('style[id^="legcord-native-theme-"]')
        .map((node) => ({ id: node.id, bytes: (node.textContent || "").length }));
    assert.deepEqual(observed, [{ id: "legcord-native-theme-alpha", bytes: ":root{--a:1;}".length }]);
    // The smoke observation also records head order and Vencord coexistence.
    const headOrder = document.head.querySelectorAll("style").map((node) => node.id);
    assert.deepEqual(headOrder, ["legcord-native-vencord-css", "legcord-native-theme-alpha"]);
    assert.ok(headOrder.includes("legcord-native-vencord-css"));
});

// Managed Quick CSS lifecycle contract (Phase D Quick CSS host-push).
//
// The host push reuses the same managed-CSS builders as themes
// (`managed_css_script`/`managed_css_remove_script`, structurally asserted in
// Rust), so these tests pin the Quick CSS specifics on the shared contract:
// Electron's verbatim `legcord-quick-css` identity, injection BEFORE themes
// (Electron's `initQuickCss`-before-theme-loop order), empty-CSS push parity,
// and clean disabled/reload/reset behavior. Cascade note: for equal
// specificity, later stylesheets win, so append order IS the cascade order.
test("Quick CSS injects under Electron's verbatim style identity", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-quick-css", ":root{--q:1;}");
    assert.equal(headChildren.length, 1);
    assert.equal(headChildren[0].id, "legcord-quick-css");
    assert.equal(headChildren[0].textContent, ":root{--q:1;}");
});

test("Quick CSS injection reuses its style instead of duplicating", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-quick-css", ":root{--q:1;}");
    managedThemeInject(document, "legcord-quick-css", ":root{--q:2;}");
    assert.equal(headChildren.length, 1);
    assert.equal(document.getElementById("legcord-quick-css").textContent, ":root{--q:2;}");
});

test("empty Quick CSS is pushed as an empty managed style like Electron", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-quick-css", "");
    assert.equal(headChildren.length, 1);
    assert.equal(document.getElementById("legcord-quick-css").textContent, "");
});

test("Quick CSS injects before themes in the document head", () => {
    const { document, headChildren } = createFakeThemeDocument();
    // The host emits Quick CSS before the sorted theme set, matching
    // Electron's initQuickCss-before-theme-loop order.
    managedThemeInject(document, "legcord-quick-css", ":root{--probe:quickcss;}");
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--probe:theme;}");
    managedThemeInject(document, "legcord-native-theme-zulu", ":root{--probe:theme;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-quick-css", "legcord-native-theme-alpha", "legcord-native-theme-zulu"],
    );
});

test("disabled Quick CSS leaves no style while themes still inject", () => {
    const { document, headChildren } = createFakeThemeDocument();
    // Disabled means the host never pushes: no node is created at all.
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:1;}");
    assert.equal(document.getElementById("legcord-quick-css"), null);
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-native-theme-alpha"],
    );
    // Disabling after an injection removes exactly the Quick CSS style.
    managedThemeInject(document, "legcord-quick-css", ":root{--q:1;}");
    managedThemeRemove(document, "legcord-quick-css");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-native-theme-alpha"],
    );
});

test("a reloaded document resets Quick CSS alongside themes", () => {
    const first = createFakeThemeDocument();
    managedThemeInject(first.document, "legcord-quick-css", ":root{--q:1;}");
    managedThemeInject(first.document, "legcord-native-theme-alpha", ":root{--a:1;}");
    assert.equal(first.headChildren.length, 2);

    const next = createFakeThemeDocument();
    assert.equal(next.headChildren.length, 0);
    managedThemeRemove(next.document, "legcord-quick-css");
    managedThemeInject(next.document, "legcord-quick-css", ":root{--q:1;}");
    managedThemeInject(next.document, "legcord-native-theme-alpha", ":root{--a:1;}");
    assert.deepEqual(
        next.headChildren.map((node) => node.id),
        ["legcord-quick-css", "legcord-native-theme-alpha"],
    );
});

test("Quick CSS coexists with Vencord and themes in cascade order", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-native-vencord-css", "/*vencord*/");
    managedThemeInject(document, "legcord-quick-css", ":root{--probe:quickcss;}");
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--probe:theme;}");
    // Re-injecting everything (as a reload would) duplicates nothing.
    managedThemeInject(document, "legcord-quick-css", ":root{--probe:quickcss;}");
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--probe:theme;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-native-vencord-css", "legcord-quick-css", "legcord-native-theme-alpha"],
    );
});

test("the smoke observation reports Quick CSS presence and bytes", () => {
    const { document } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-quick-css", ":root{--q:1;}");
    const quickCssNode = document.getElementById("legcord-quick-css");
    const observed = {
        quickcss_present: Boolean(quickCssNode),
        quickcss_bytes: quickCssNode ? (quickCssNode.textContent || "").length : 0,
    };
    assert.deepEqual(observed, { quickcss_present: true, quickcss_bytes: ":root{--q:1;}".length });

    const empty = createFakeThemeDocument();
    const missing = empty.document.getElementById("legcord-quick-css");
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
    managedThemeInject(document, "legcord-quick-css", ":root{--probe:quickcss;}");
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "legcord-native-theme-zulu", ":root{--z:1;}");
    // Live: alpha CSS changes on disk -> in-place update, no order repair.
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:2;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-quick-css", "legcord-native-theme-alpha", "legcord-native-theme-zulu"],
    );
    assert.equal(document.getElementById("legcord-native-theme-alpha").textContent, ":root{--a:2;}");
    // Live: Quick CSS changes -> still first, themes untouched.
    managedThemeInject(document, "legcord-quick-css", ":root{--probe:quickcss2;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-quick-css", "legcord-native-theme-alpha", "legcord-native-theme-zulu"],
    );
});

test("a recreated Quick CSS style is repaired back before themes", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-quick-css", ":root{--q:1;}");
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:1;}");
    // Live: file deleted then recreated -> remove + fresh append lands last.
    managedThemeRemove(document, "legcord-quick-css");
    assert.equal(document.getElementById("legcord-quick-css"), null);
    managedThemeInject(document, "legcord-quick-css", ":root{--q:2;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-native-theme-alpha", "legcord-quick-css"],
    );
    // Host structural repair: Quick CSS first, then sorted themes.
    managedCssOrder(document, ["legcord-quick-css", "legcord-native-theme-alpha"]);
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-quick-css", "legcord-native-theme-alpha"],
    );
    assert.equal(document.getElementById("legcord-quick-css").textContent, ":root{--q:2;}");
});

test("a newly added theme is repaired into sorted cascade position", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-quick-css", ":root{--q:1;}");
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "legcord-native-theme-zulu", ":root{--z:1;}");
    // Live: theme "mike" created -> appended at the end, out of position.
    managedThemeInject(document, "legcord-native-theme-mike", ":root{--m:1;}");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-quick-css", "legcord-native-theme-alpha", "legcord-native-theme-zulu", "legcord-native-theme-mike"],
    );
    // Host structural repair restores sorted order without touching bytes.
    managedCssOrder(document, [
        "legcord-quick-css",
        "legcord-native-theme-alpha",
        "legcord-native-theme-mike",
        "legcord-native-theme-zulu",
    ]);
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-quick-css", "legcord-native-theme-alpha", "legcord-native-theme-mike", "legcord-native-theme-zulu"],
    );
});

test("rapid live edits collapse to one node with the final bytes", () => {
    const { document, headChildren } = createFakeThemeDocument();
    // Burst A -> B -> C within one debounce window: the host re-reads once
    // and pushes only the latest bytes.
    managedThemeInject(document, "legcord-quick-css", ":root{--v:a;}");
    managedThemeInject(document, "legcord-quick-css", ":root{--v:b;}");
    managedThemeInject(document, "legcord-quick-css", ":root{--v:c;}");
    assert.equal(headChildren.length, 1);
    assert.equal(document.getElementById("legcord-quick-css").textContent, ":root{--v:c;}");
});

test("disable removes the style and re-enable injects current bytes once", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-quick-css", ":root{--q:1;}");
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:1;}");
    // Live: disabled while the file keeps changing -> style removed, file
    // writes issue no renderer work (nothing injected here).
    managedThemeRemove(document, "legcord-quick-css");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-native-theme-alpha"],
    );
    // Live: re-enabled -> current bytes appear exactly once, before themes.
    managedThemeInject(document, "legcord-quick-css", ":root{--q:99;}");
    managedCssOrder(document, ["legcord-quick-css", "legcord-native-theme-alpha"]);
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-quick-css", "legcord-native-theme-alpha"],
    );
    assert.equal(headChildren.filter((node) => node.id === "legcord-quick-css").length, 1);
});

test("deleted themes remove exactly their stale style", () => {
    const { document, headChildren } = createFakeThemeDocument();
    managedThemeInject(document, "legcord-quick-css", ":root{--q:1;}");
    managedThemeInject(document, "legcord-native-theme-alpha", ":root{--a:1;}");
    managedThemeInject(document, "legcord-native-theme-zulu", ":root{--z:1;}");
    // Live: zulu directory deleted -> stale style removed, rest untouched.
    managedThemeRemove(document, "legcord-native-theme-zulu");
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-quick-css", "legcord-native-theme-alpha"],
    );
    // Live: zulu recreated -> injected, then repaired into sorted position.
    managedThemeInject(document, "legcord-native-theme-zulu", ":root{--z:2;}");
    managedCssOrder(document, ["legcord-quick-css", "legcord-native-theme-alpha", "legcord-native-theme-zulu"]);
    assert.deepEqual(
        headChildren.map((node) => node.id),
        ["legcord-quick-css", "legcord-native-theme-alpha", "legcord-native-theme-zulu"],
    );
});

test("live updates keep Vencord first across reloads without duplication", () => {
    const first = createFakeThemeDocument();
    managedThemeInject(first.document, "legcord-native-vencord-css", "/*vencord*/");
    managedThemeInject(first.document, "legcord-quick-css", ":root{--q:1;}");
    managedThemeInject(first.document, "legcord-native-theme-alpha", ":root{--a:1;}");
    // Live content update: Vencord stays first, no duplicates.
    managedThemeInject(first.document, "legcord-native-theme-alpha", ":root{--a:2;}");
    managedThemeInject(first.document, "legcord-quick-css", ":root{--q:2;}");
    assert.deepEqual(
        first.headChildren.map((node) => node.id),
        ["legcord-native-vencord-css", "legcord-quick-css", "legcord-native-theme-alpha"],
    );
    // Reload: fresh document re-pushes the same order, still no duplicates.
    const next = createFakeThemeDocument();
    managedThemeInject(next.document, "legcord-native-vencord-css", "/*vencord*/");
    managedThemeInject(next.document, "legcord-quick-css", ":root{--q:2;}");
    managedThemeInject(next.document, "legcord-native-theme-alpha", ":root{--a:2;}");
    assert.deepEqual(
        next.headChildren.map((node) => node.id),
        ["legcord-native-vencord-css", "legcord-quick-css", "legcord-native-theme-alpha"],
    );
    assert.equal(first.headChildren.length, 3);
});

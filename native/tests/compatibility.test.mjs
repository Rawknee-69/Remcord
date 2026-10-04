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
const testModAdapter = readFileSync(new URL("../web/mods/testmod/adapter.js", import.meta.url), "utf8");

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

function createRuntime(origin = "https://discord.com", capabilities = ["bridge.ping", "mod.vencord.get_bundle"]) {
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
                    type: "vencord_bundle",
                    payload: {
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
    assert.equal(window.legcord.native.supports("mod.vencord.get_bundle"), true);
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

test("requests only the typed Vencord capability and validates its response schema", async () => {
    const { window } = createRuntime();
    const request = window.legcord.native.getVencordBundle();
    const message = window.messages.find((entry) => entry.command?.type === "mod_vencord_get_bundle");
    assert.ok(message);
    assert.deepEqual(message.command, { type: "mod_vencord_get_bundle", payload: {} });
    assert.equal(request.requestId, message.request_id);

    respondVencord(window, message);
    assert.equal((await request).marker, "vencord-bootstrap-executed");
});

test("rejects malformed Vencord responses", async () => {
    const { window } = createRuntime();
    const request = window.legcord.native.getVencordBundle();
    const message = window.messages.find((entry) => entry.command?.type === "mod_vencord_get_bundle");
    respondVencord(window, message, { document_execution_count: 2 });
    await assert.rejects(request, /response schema is invalid/);
});

test("propagates a typed bundle failure without reclassifying it as adapter initialization", async () => {
    const { window, context } = createRuntime();
    vm.runInContext(vencordAdapter, context);
    const request = window.messages.find((message) => message.command?.type === "mod_vencord_get_bundle");
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
        window.messages.some((message) => message.command?.type === "mod_vencord_get_bundle"),
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
    const request = window.messages.find((message) => message.command?.type === "mod_vencord_get_bundle");
    assert.ok(request);
    assert.equal(window.messages.filter((message) => message.command?.type === "mod_vencord_get_bundle").length, 1);
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
        secondDocument.window.messages.filter((message) => message.command?.type === "mod_vencord_get_bundle").length,
        1,
    );
    firstDocument.window.dispatchEvent(new FakeCustomEvent("pagehide"));
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

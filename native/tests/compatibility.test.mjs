import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const bootstrap = readFileSync(new URL("../web/compatibility.js", import.meta.url), "utf8").replace(
    "__LEGCORD_BOOT_SNAPSHOT__",
    JSON.stringify({
        runtime: "webview2",
        platform: "win32",
        protocol_version: 1,
        capabilities: ["bridge.ping"],
    }),
);

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

function createRuntime(origin) {
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
    vm.runInContext(bootstrap, context);
    return { window, context };
}

function respond(window, request, payload) {
    window.dispatchEvent(
        new FakeCustomEvent("legcord-native:response", {
            detail: {
                version: 1,
                request_id: request.request_id,
                ok: true,
                payload,
                error: null,
            },
        }),
    );
}

test("initializes a frozen compatibility facade with the boot snapshot", () => {
    const { window } = createRuntime();
    assert.ok(window.legcord);
    assert.equal(window.legcord.boot.runtime, "webview2");
    assert.equal(window.legcord.boot.platform, "win32");
    assert.equal(Object.isFrozen(window.legcord), true);
    assert.equal(Object.isFrozen(window.legcord.boot), true);
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

    respond(window, requests[1], { nonce: "second", message: "pong" });
    respond(window, requests[0], { nonce: "first", message: "pong" });
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
    respond(nextDocument.window, nextPing, { nonce: "reloaded", message: "pong" });
    assert.deepEqual(await request, { nonce: "reloaded", message: "pong" });
    assert.equal(firstDocument.window.legcord === nextDocument.window.legcord, false);
});

test("does not expose the facade outside the exact trusted origin", () => {
    const { window } = createRuntime("https://canary.discord.com");
    assert.equal(window.legcord, undefined);
    assert.equal(window.messages.length, 0);
});

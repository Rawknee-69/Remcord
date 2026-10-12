import assert from "node:assert/strict";
import test from "node:test";

// Covers the native-shell document-title pin in
// `src/shelter/titlebar/index.ts`: Task Manager names the expanded WebView
// child row after the page's document title, so the native shell pins it to
// "Remcord" on load and re-pins it when Discord's SPA rewrites it on
// navigation. Electron keeps following Discord (no pin, no observer).

type StubDocument = {
    _title: string;
    title: string;
    head: object;
    documentElement: object;
};

type ObserverStub = { cb: () => void };

let observers: ObserverStub[] = [];

function installStubs(native: boolean): StubDocument {
    observers = [];
    for (const key of ["shelter", "window", "document", "MutationObserver"]) {
        Reflect.deleteProperty(globalThis, key);
    }
    Reflect.set(globalThis, "shelter", {
        util: { log() {} },
        flux: { dispatcher: { subscribe() {}, unsubscribe() {} } },
    });
    const remcord: Record<string, unknown> = {
        settings: { getConfig: () => ({ windowStyle: "overlay", minimizeToTray: true }) },
        window: {
            minimize() {},
            maximized: () => false,
            isNormal: () => true,
            maximize() {},
            unmaximize() {},
            hide() {},
            quit() {},
        },
        platform: "win32",
    };
    if (native) remcord.boot = { runtime: "webview2" };
    Reflect.set(globalThis, "window", { remcord });
    const buttonStub = () => ({ addEventListener() {} });
    const doc = {
        _title: "Discord",
        get title() {
            return this._title;
        },
        set title(value: string) {
            this._title = value;
        },
        head: {},
        documentElement: {},
        body: { append() {}, prepend() {}, setAttribute() {}, removeAttribute() {} },
        createElement: () => ({ set innerHTML(_value: string) {}, addEventListener() {}, append() {} }),
        getElementById: (id: string) => {
            if (id === "minimize" || id === "maximize" || id === "quit" || id === "window-controls-container") {
                return buttonStub();
            }
            return null;
        },
        addEventListener() {},
    };
    Reflect.set(globalThis, "document", doc);
    Reflect.set(
        globalThis,
        "MutationObserver",
        class {
            cb: () => void;
            constructor(cb: () => void) {
                this.cb = cb;
                observers.push(this);
            }
            observe() {}
            disconnect() {}
        },
    );
    return doc;
}

function uninstallStubs() {
    for (const key of ["shelter", "window", "document", "MutationObserver"]) {
        Reflect.deleteProperty(globalThis, key);
    }
}

async function loadPlugin() {
    return await import("../src/shelter/titlebar/index.js");
}

test("native shell pins the document title and re-applies after rewrites", async () => {
    // One module load: the pin binds once per document, so load and the
    // simulated SPA rewrite share it (matching one real document).
    const doc = installStubs(true);
    try {
        const plugin = await loadPlugin();
        plugin.onLoad();
        assert.equal(doc.title, "Remcord");
        assert.equal(observers.length, 1);
        // Simulate a Discord SPA navigation rewriting the title.
        doc._title = "Friends - Discord";
        for (const observer of observers) observer.cb();
        assert.equal(doc.title, "Remcord");
        plugin.onUnload();
    } finally {
        uninstallStubs();
    }
});

test("Electron shell keeps following the Discord title", async () => {
    const doc = installStubs(false);
    try {
        const plugin = await loadPlugin();
        plugin.onLoad();
        assert.equal(doc.title, "Discord");
        assert.equal(observers.length, 0);
        plugin.onUnload();
    } finally {
        uninstallStubs();
    }
});

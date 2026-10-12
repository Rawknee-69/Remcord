import assert from "node:assert/strict";
import test from "node:test";

import {
    shouldSkipTopBandDrag,
    TOP_BAND_CLICKABLE_SELECTOR,
    TOP_BAND_HEIGHT_PX,
} from "../src/shelter/titlebar/dragGuard.js";

// Covers the native-shell top-band drag guard in
// `src/shelter/titlebar/`: a mousedown there starts a native caption drag
// whose OS move-loop swallows the mouse-up, so any Discord control the guard
// misses LOSES its click. Reported symptom: the Discord home/DM button
// (div-based React control, not a link/button) stopped navigating.

type StubTarget = {
    nodeType: number;
    tokens: string[];
    cursor: string;
    closest(selectors: string): object | null;
    ownerDocument: {
        defaultView: { getComputedStyle: () => { cursor: string } };
    };
};

function stubTarget(tokens: string[], cursor: string): StubTarget {
    const target: StubTarget = {
        nodeType: 1,
        tokens,
        cursor,
        closest(selectors: string) {
            const parts = selectors.split(",");
            for (const part of parts) {
                for (const token of tokens) {
                    if (part.includes(token)) return {};
                }
            }
            return null;
        },
        ownerDocument: {
            defaultView: { getComputedStyle: () => ({ cursor }) },
        },
    };
    return target;
}

test("drag band keeps its documented height", () => {
    assert.equal(TOP_BAND_HEIGHT_PX, 64);
});

test("exclusion selector names Discord's div-based navigation", () => {
    for (const token of [
        "#remcordNavControls",
        "a",
        "button",
        "[role='treeitem']",
        "[data-list-item-id]",
        "[role='tab']",
        "svg",
    ]) {
        assert.ok(TOP_BAND_CLICKABLE_SELECTOR.includes(token), `selector must cover ${token}`);
    }
});

test("Discord home/DM button keeps its click", () => {
    // Div-based guild-rail control: role + data attribute, pointer cursor.
    const home = stubTarget(["div", "[role='treeitem']", "[data-list-item-id]"], "pointer");
    assert.equal(shouldSkipTopBandDrag(home, home.cursor), true);
});

test("unrecognized pointer-cursor markup keeps its click", () => {
    // Future Discord markup the selector does not name: the cursor backstop
    // still protects it.
    const unknown = stubTarget(["div"], "pointer");
    assert.equal(shouldSkipTopBandDrag(unknown, unknown.cursor), true);
});

test("truly empty pixels still drag", () => {
    const empty = stubTarget(["div"], "default");
    assert.equal(shouldSkipTopBandDrag(empty, empty.cursor), false);
});

test("unknown targets fail closed to no-drag", () => {
    assert.equal(shouldSkipTopBandDrag(null, ""), true);
    assert.equal(shouldSkipTopBandDrag({} as unknown as StubTarget, ""), true);
    const throwing = {
        closest() {
            throw new Error("nope");
        },
    };
    assert.equal(shouldSkipTopBandDrag(throwing, ""), true);
});

// --- Behavioral pass through the real document-level handler ---

type CapturedListener = {
    type: string;
    listener: (event: Record<string, unknown>) => void;
    capture: unknown;
};

let dragCalls = 0;
let listeners: CapturedListener[] = [];

function installPluginStubs() {
    dragCalls = 0;
    listeners = [];
    for (const key of ["shelter", "window", "document", "MutationObserver", "getComputedStyle"]) {
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
        native: {
            window: {
                drag() {
                    dragCalls += 1;
                },
            },
        },
        platform: "win32",
        boot: { runtime: "webview2" },
    };
    Reflect.set(globalThis, "window", { remcord });
    const buttonStub = () => ({ addEventListener() {} });
    Reflect.set(globalThis, "document", {
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
        addEventListener(type: string, listener: CapturedListener["listener"], capture: unknown) {
            listeners.push({ type, listener, capture });
        },
    });
    Reflect.set(
        globalThis,
        "MutationObserver",
        class {
            observe() {}
            disconnect() {}
        },
    );
}

function uninstallPluginStubs() {
    for (const key of ["shelter", "window", "document", "MutationObserver", "getComputedStyle"]) {
        Reflect.deleteProperty(globalThis, key);
    }
}

function mousedownListener(): CapturedListener["listener"] {
    const found = listeners.find((entry) => entry.type === "mousedown" && entry.capture === true);
    assert.ok(found, "top-band mousedown listener must be bound");
    return found.listener;
}

function fire(listener: CapturedListener["listener"], target: StubTarget | null, clientY: number) {
    let prevented = false;
    listener({
        button: 0,
        clientY,
        target,
        preventDefault() {
            prevented = true;
        },
    });
    return prevented;
}

test("handler drags empty pixels but never the DM button", async () => {
    installPluginStubs();
    try {
        const plugin = await import("../src/shelter/titlebar/index.js");
        plugin.onLoad();
        const onMousedown = mousedownListener();

        // Empty top-band pixel: drag starts.
        dragCalls = 0;
        const empty = stubTarget(["div"], "default");
        assert.equal(fire(onMousedown, empty, 20), true);
        assert.equal(dragCalls, 1);

        // Discord home/DM button: no drag, no preventDefault — the click
        // (and its mouse-up) reaches Discord.
        dragCalls = 0;
        const home = stubTarget(["div", "[role='treeitem']", "[data-list-item-id]"], "pointer");
        assert.equal(fire(onMousedown, home, 20), false);
        assert.equal(dragCalls, 0);

        // Below the band: untouched regardless of target.
        dragCalls = 0;
        assert.equal(fire(onMousedown, empty, 200), false);
        assert.equal(dragCalls, 0);

        plugin.onUnload();
    } finally {
        uninstallPluginStubs();
    }
});

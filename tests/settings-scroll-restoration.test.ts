import assert from "node:assert/strict";
import { test } from "node:test";
import { createSettingsScrollRestoration } from "../src/shelter/settings/settingsScrollRestoration.js";

type FakeElement = {
    name: string;
    scrollTop: number;
    connected: boolean;
    focused: boolean;
};

function makeHarness() {
    let discovered: FakeElement | null = null;
    let resolved: FakeElement | null = null;
    let discoveries = 0;
    let activeElement: FakeElement | null = null;
    let nextFrameId = 0;
    let frameRequests = 0;
    let frameCancels = 0;
    let writes = 0;
    const frames = new Map<number, () => void>();
    const restorer = createSettingsScrollRestoration<FakeElement, FakeElement>({
        findScroller: () => {
            discoveries++;
            return discovered;
        },
        resolveScroller: (cached) => {
            const candidate = resolved ?? cached;
            return candidate?.connected ? candidate : discovered;
        },
        getScrollTop: (element) => element.scrollTop,
        setScrollTop: (element, top) => {
            writes++;
            element.scrollTop = top;
        },
        getActiveElement: () => activeElement,
        contains: (element) => element.connected,
        isActive: (element) => element.focused,
        focus: (element) => {
            if (activeElement) activeElement.focused = false;
            activeElement = element;
            element.focused = true;
        },
        requestFrame: (callback) => {
            const id = ++nextFrameId;
            frameRequests++;
            frames.set(id, callback);
            return id;
        },
        cancelFrame: (id) => {
            frameCancels++;
            frames.delete(id);
        },
    });

    return {
        restorer,
        frames,
        setScroller: (element: FakeElement | null) => {
            discovered = element;
        },
        setResolvedScroller: (element: FakeElement | null) => {
            resolved = element;
        },
        setActiveElement: (element: FakeElement | null) => {
            activeElement = element;
        },
        get metrics() {
            return { frameRequests, frameCancels, writes, pendingFrames: frames.size };
        },
        get discoveryCount() {
            return discoveries;
        },
        runFrame(id: number) {
            const callback = frames.get(id);
            frames.delete(id);
            callback?.();
        },
    };
}

function element(name: string, scrollTop = 0): FakeElement {
    return { name, scrollTop, connected: true, focused: false };
}

test("one restoration request schedules one effective restoration", () => {
    const harness = makeHarness();
    const scroller = element("settings", 240);
    harness.setScroller(scroller);
    const unmount = harness.restorer.mount();

    assert.equal(
        harness.restorer.preserve(() => "done"),
        "done",
    );
    assert.deepEqual(harness.metrics, { frameRequests: 1, frameCancels: 0, writes: 0, pendingFrames: 1 });
    assert.equal(harness.discoveryCount, 1);
    scroller.scrollTop = 0;
    harness.runFrame(1);
    assert.equal(scroller.scrollTop, 240);
    assert.equal(harness.metrics.writes, 1);

    unmount();
});

test("repeated requests in one turn coalesce and keep the original snapshot", () => {
    const harness = makeHarness();
    const scroller = element("settings", 125);
    harness.setScroller(scroller);
    const unmount = harness.restorer.mount();

    harness.restorer.preserve(() => {
        scroller.scrollTop = 0;
    });
    harness.restorer.preserve(() => {
        scroller.scrollTop = 10;
    });
    harness.restorer.preserve(() => {
        scroller.scrollTop = 20;
    });

    assert.deepEqual(harness.metrics, { frameRequests: 1, frameCancels: 0, writes: 0, pendingFrames: 1 });
    assert.equal(harness.discoveryCount, 1);
    scroller.scrollTop = 0;
    harness.runFrame(1);
    assert.equal(scroller.scrollTop, 125);
    assert.equal(harness.metrics.writes, 1);

    unmount();
});

test("a newer request supersedes stale scheduled state", () => {
    const harness = makeHarness();
    const scroller = element("settings", 310);
    harness.setScroller(scroller);
    const unmount = harness.restorer.mount();

    harness.restorer.preserve(() => {});
    const callback = harness.frames.get(1);
    const replacement = element("replacement");
    scroller.connected = false;
    harness.setScroller(replacement);
    harness.restorer.invalidate();
    harness.restorer.preserve(() => {});
    replacement.scrollTop = 0;

    callback?.();
    assert.equal(scroller.scrollTop, 310);
    assert.equal(replacement.scrollTop, 310);
    assert.equal(harness.metrics.writes, 1);

    unmount();
});

test("cleanup cancels pending callbacks", () => {
    const harness = makeHarness();
    const scroller = element("settings", 80);
    harness.setScroller(scroller);
    const unmount = harness.restorer.mount();
    harness.restorer.preserve(() => {});

    unmount();
    assert.deepEqual(harness.metrics, { frameRequests: 1, frameCancels: 1, writes: 0, pendingFrames: 0 });
});

test("remount discovers and restores the fresh settings scroller", () => {
    const harness = makeHarness();
    const first = element("first", 20);
    harness.setScroller(first);
    const unmountFirst = harness.restorer.mount();
    harness.restorer.preserve(() => {});
    unmountFirst();

    const replacement = element("replacement", 75);
    first.connected = false;
    harness.setScroller(replacement);
    const unmountReplacement = harness.restorer.mount();
    harness.restorer.preserve(() => {
        replacement.scrollTop = 0;
    });
    harness.runFrame(2);

    assert.equal(replacement.scrollTop, 75);
    assert.equal(first.scrollTop, 20);
    unmountReplacement();
});

test("resize invalidation rediscovers the current scroller for restoration", () => {
    const harness = makeHarness();
    const original = element("original", 30);
    harness.setScroller(original);
    const unmount = harness.restorer.mount();

    const resized = element("resized", 145);
    original.connected = false;
    harness.setScroller(resized);
    harness.restorer.invalidate();
    harness.restorer.preserve(() => {
        resized.scrollTop = 0;
    });
    harness.runFrame(1);

    assert.equal(resized.scrollTop, 145);
    assert.equal(original.scrollTop, 30);
    unmount();
});

test("restoration preserves scroll position and focus without scrolling on focus", () => {
    const harness = makeHarness();
    const scroller = element("settings", 420);
    const control = element("control");
    harness.setScroller(scroller);
    harness.setActiveElement(control);
    const unmount = harness.restorer.mount();

    harness.restorer.preserve(() => {
        scroller.scrollTop = 0;
        control.focused = false;
    });
    harness.runFrame(1);

    assert.equal(scroller.scrollTop, 420);
    assert.equal(control.focused, true);
    assert.equal(harness.metrics.writes, 1);
    unmount();
});

test("a focused nested scroller is restored instead of the outer settings scroller", () => {
    const harness = makeHarness();
    const outer = element("outer", 500);
    const nested = element("nested", 65);
    harness.setScroller(outer);
    harness.setResolvedScroller(nested);
    const unmount = harness.restorer.mount();

    harness.restorer.preserve(() => {
        nested.scrollTop = 0;
    });
    harness.runFrame(1);

    assert.equal(outer.scrollTop, 500);
    assert.equal(nested.scrollTop, 65);
    unmount();
});

test("pending callback count never exceeds one and no timers are scheduled", () => {
    const harness = makeHarness();
    const scroller = element("settings", 55);
    harness.setScroller(scroller);
    const unmount = harness.restorer.mount();

    for (let i = 0; i < 10; i++) {
        harness.restorer.preserve(() => {});
        assert.equal(harness.metrics.pendingFrames, 1);
    }
    assert.equal(harness.metrics.frameRequests, 1);
    assert.equal(harness.metrics.frameCancels, 0);
    assert.equal(harness.discoveryCount, 1);
    harness.runFrame(1);
    assert.equal(harness.metrics.pendingFrames, 0);

    unmount();
});

import assert from "node:assert/strict";
import test from "node:test";

// Covers the `listen` blacklist lookup in `src/shelter/rpc/index.ts`: the
// native shell exposes no `window.remcord.rpc` facade, so an RPC activity
// arriving without one must still dispatch (treated as "not blacklisted")
// instead of throwing `Cannot read properties of undefined (reading
// 'getBlacklist')` and breaking the dispatch chain.
//
// One shared install: the module binds the `shelter` global once at import,
// so every test mutates the same `window` object the module already reads.

type Dispatched = { type: string; [key: string]: unknown };

const dispatched: Dispatched[] = [];
const mutableWindow: {
    remcordRPC: { listen?: (msg: never) => Promise<void>; lastDetectedGames?: unknown[]; onLastDetectedUpdate?: null };
    remcord: { rpc?: { getBlacklist?: () => Array<{ id: number; name?: string }> } };
} = {
    remcordRPC: {},
    remcord: {},
};

function activity(application_id: number, name: string) {
    return { activity: { assets: null, application_id, name } } as never;
}

test("listen without an rpc facade dispatches the activity instead of throwing", async () => {
    Reflect.set(globalThis, "shelter", {
        flux: {
            dispatcher: {
                dispatch: (payload: Dispatched) => {
                    dispatched.push(payload);
                },
            },
        },
        http: { ready: Promise.resolve() },
    });
    Reflect.set(globalThis, "window", mutableWindow);
    const { onLoad } = await import("../src/shelter/rpc/index.js");
    onLoad();
    assert.equal(typeof mutableWindow.remcordRPC.listen, "function");
    // No rpc facade at all: must not throw; the activity dispatches as-is.
    await mutableWindow.remcordRPC.listen!(activity(1234, "Some Game"));
    assert.equal(dispatched.length, 1);
    assert.equal(dispatched[0]!.type, "LOCAL_ACTIVITY_UPDATE");
});

test("listen dispatches a non-blacklisted activity through", async () => {
    mutableWindow.remcord.rpc = { getBlacklist: () => [] };
    dispatched.length = 0;
    await mutableWindow.remcordRPC.listen!(activity(1234, "Some Game"));
    assert.equal(dispatched.length, 1);
    assert.equal(dispatched[0]!.type, "LOCAL_ACTIVITY_UPDATE");
});

test("listen clears a blacklisted activity", async () => {
    let lookups = 0;
    mutableWindow.remcord.rpc = {
        getBlacklist: () => {
            lookups += 1;
            return [{ name: "Some Game", id: 1234 }];
        },
    };
    dispatched.length = 0;
    await mutableWindow.remcordRPC.listen!(activity(1234, "Some Game"));
    assert.equal(lookups, 1);
    assert.equal(dispatched.length, 1);
    assert.equal((dispatched[0] as { activity: unknown }).activity, null);
});

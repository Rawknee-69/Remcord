import assert from "node:assert/strict";
import { test } from "node:test";
import { createRpcBlacklistCache, type RpcBlacklistEntry } from "../src/discord/preload/rpcBlacklistCache.js";

test("RPC blacklist cache snapshots once, returns isolated values, and accepts updates", () => {
    const initial: RpcBlacklistEntry[] = [{ name: "Game A", id: 1 }];
    let initialReads = 0;
    let listener: ((entries: RpcBlacklistEntry[]) => void) | undefined;
    let removals = 0;
    const order: string[] = [];

    const cache = createRpcBlacklistCache(
        () => {
            order.push("initial");
            initialReads++;
            return initial;
        },
        (subscribe) => {
            order.push("subscribe");
            listener = subscribe;
            return () => {
                removals++;
                listener = undefined;
            };
        },
    );

    assert.deepEqual(order, ["subscribe", "initial"]);
    assert.deepEqual(cache.get(), initial);
    assert.deepEqual(cache.get(), initial);
    assert.equal(initialReads, 1);

    const returnedEntries = cache.get();
    returnedEntries[0].name = "Mutated locally";
    assert.deepEqual(cache.get(), initial);

    const updated: RpcBlacklistEntry[] = [
        { name: "Game A", id: 1 },
        { name: "Game B", id: 2 },
    ];
    cache.update(updated);
    assert.deepEqual(cache.get(), updated);

    const eventUpdated: RpcBlacklistEntry[] = [{ name: "Game C", id: 3 }];
    listener?.(eventUpdated);
    assert.deepEqual(cache.get(), eventUpdated);

    cache.dispose();
    cache.dispose();
    assert.equal(removals, 1);
    listener?.([]);
    cache.update(updated);
    assert.deepEqual(cache.get(), eventUpdated);
});

test("a new preload realm starts from a fresh snapshot and owns one subscription", () => {
    const listeners = new Set<(entries: RpcBlacklistEntry[]) => void>();
    const subscribe = (listener: (entries: RpcBlacklistEntry[]) => void) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
    };
    let snapshot: RpcBlacklistEntry[] = [{ name: "First realm", id: 1 }];

    const firstRealm = createRpcBlacklistCache(() => snapshot, subscribe);
    assert.equal(listeners.size, 1);
    firstRealm.dispose();
    assert.equal(listeners.size, 0);

    snapshot = [{ name: "Second realm", id: 2 }];
    const secondRealm = createRpcBlacklistCache(() => snapshot, subscribe);
    assert.deepEqual(secondRealm.get(), snapshot);
    assert.equal(listeners.size, 1);

    secondRealm.dispose();
    assert.equal(listeners.size, 0);
});

import assert from "node:assert/strict";
import { test } from "node:test";
import { replaceGuildItems } from "../src/discord/touchbarGuildItems.js";

type Callback = (payload: { event: string; properties: { enabled: string } }) => void;

function makeDispatcher() {
    const callbacks = new Map<string, Set<Callback>>();
    let registrations = 0;
    let removals = 0;

    return {
        subscribe(event: string, callback: Callback) {
            registrations++;
            let listeners = callbacks.get(event);
            if (!listeners) {
                listeners = new Set();
                callbacks.set(event, listeners);
            }
            listeners.add(callback);
        },
        unsubscribe(event: string, callback: Callback) {
            removals++;
            callbacks.get(event)?.delete(callback);
        },
        emit(event: string, payload: { event: string; properties: { enabled: string } }) {
            for (const callback of callbacks.get(event) ?? []) callback(payload);
        },
        count(event: string) {
            return callbacks.get(event)?.size ?? 0;
        },
        get registrations() {
            return registrations;
        },
        get removals() {
            return removals;
        },
    };
}

test("power plugin keeps one TRACK callback through three enable/reload cycles", async () => {
    const dispatcher = makeDispatcher();
    let powerChanges = 0;
    Reflect.set(globalThis, "shelter", {
        util: { log() {} },
        flux: { dispatcher },
    });
    Reflect.set(globalThis, "window", {
        legcord: {
            settings: { getConfig: () => ({ blockPowerSavingInVoiceChat: true }) },
            power: {
                setPowerSaving() {
                    powerChanges++;
                },
            },
        },
    });

    const plugin = await import(`../src/shelter/power/index.ts?lifecycle=${Date.now()}`);
    for (let cycle = 1; cycle <= 3; cycle++) {
        plugin.onLoad();
        assert.equal(dispatcher.count("TRACK"), 1);
        dispatcher.emit("TRACK", { event: "join_voice_channel", properties: { enabled: "true" } });
        assert.equal(powerChanges, cycle);
        plugin.onUnload();
        assert.equal(dispatcher.count("TRACK"), 0);
    }

    assert.equal(dispatcher.registrations, 3);
    assert.equal(dispatcher.removals, 3);
});

test("Touch Bar plugin removes subscriptions and pending guild imports across three reloads", async () => {
    const dispatcher = makeDispatcher();
    const timeouts = new Map<number, () => void>();
    let nextTimeout = 0;
    let voiceStateUpdates = 0;
    let voiceBarUpdates = 0;
    let imports = 0;
    Reflect.set(globalThis, "shelter", {
        util: { log() {} },
        flux: {
            dispatcher,
            storesFlat: {
                MediaEngineStore: { getSettings: () => ({ mute: false, deaf: false }) },
                GuildStore: {
                    getGuildIds: () => ["guild-1"],
                    getGuild: () => ({ icon: "icon-hash" }),
                },
            },
        },
    });
    Reflect.set(globalThis, "window", {
        legcord: {
            platform: "darwin",
            touchbar: {
                setVoiceState() {
                    voiceStateUpdates++;
                },
                setVoiceTouchbar() {
                    voiceBarUpdates++;
                },
                importGuilds() {
                    imports++;
                },
            },
        },
    });
    const originalSetTimeout = globalThis.setTimeout;
    const originalClearTimeout = globalThis.clearTimeout;
    Reflect.set(globalThis, "setTimeout", (callback: () => void) => {
        const id = ++nextTimeout;
        timeouts.set(id, callback);
        return id;
    });
    Reflect.set(globalThis, "clearTimeout", (id: number) => {
        timeouts.delete(id);
    });

    try {
        const plugin = await import(`../src/shelter/touchbar/index.ts?lifecycle=${Date.now()}`);
        for (let cycle = 1; cycle <= 3; cycle++) {
            plugin.onLoad();
            assert.equal(dispatcher.count("TRACK"), 1);
            assert.equal(dispatcher.count("AUDIO_TOGGLE_SELF_MUTE"), 1);
            assert.equal(dispatcher.count("AUDIO_TOGGLE_SELF_DEAF"), 1);
            assert.equal(timeouts.size, 1);
            dispatcher.emit("TRACK", {
                event: "join_voice_channel",
                properties: { enabled: "true" },
            });
            assert.equal(voiceBarUpdates, cycle);
            const previousVoiceStateUpdates = voiceStateUpdates;
            dispatcher.emit("AUDIO_TOGGLE_SELF_MUTE", { event: "", properties: { enabled: "true" } });
            dispatcher.emit("AUDIO_TOGGLE_SELF_DEAF", { event: "", properties: { enabled: "true" } });
            assert.equal(voiceStateUpdates, previousVoiceStateUpdates + 2);

            plugin.onUnload();
            assert.equal(dispatcher.count("TRACK"), 0);
            assert.equal(dispatcher.count("AUDIO_TOGGLE_SELF_MUTE"), 0);
            assert.equal(dispatcher.count("AUDIO_TOGGLE_SELF_DEAF"), 0);
            assert.equal(timeouts.size, 0);
            dispatcher.emit("TRACK", {
                event: "join_voice_channel",
                properties: { enabled: "true" },
            });
            assert.equal(voiceBarUpdates, cycle);
        }
        plugin.onLoad();
        assert.equal(dispatcher.count("TRACK"), 1);
        assert.equal(dispatcher.count("AUDIO_TOGGLE_SELF_MUTE"), 1);
        assert.equal(dispatcher.count("AUDIO_TOGGLE_SELF_DEAF"), 1);
        assert.equal(timeouts.size, 1);
        const [timeoutId, importCallback] = timeouts.entries().next().value!;
        timeouts.delete(timeoutId);
        importCallback();
        assert.equal(imports, 1);
        plugin.onUnload();
        assert.equal(dispatcher.registrations, 12);
        assert.equal(dispatcher.removals, 12);
        assert.equal(voiceStateUpdates, 10);
    } finally {
        Reflect.set(globalThis, "setTimeout", originalSetTimeout);
        Reflect.set(globalThis, "clearTimeout", originalClearTimeout);
    }
});

test("Touch Bar guild item refresh replaces the old collection instead of appending", () => {
    const guildItems = ["old-guild-1", "old-guild-2"];
    const initialCollection = guildItems;

    replaceGuildItems(guildItems, ["guild-a", "guild-b"]);
    assert.equal(guildItems, initialCollection);
    assert.deepEqual(guildItems, ["guild-a", "guild-b"]);

    replaceGuildItems(guildItems, ["guild-b", "guild-c"]);
    assert.equal(guildItems, initialCollection);
    assert.deepEqual(guildItems, ["guild-b", "guild-c"]);
});

import assert from "node:assert/strict";
import test from "node:test";

// Covers the renderer-side Go Live stream-parameter control in
// `src/shelter/settings/screenshareStream.ts`: the Electron-mirrored
// sender-level caps that make Balanced/Low actually constrain the sent
// stream now that the capture-track cap is proven rejected by WebView2.
// Default installs nothing; everything fail-open (missing flux, logged
// out, idle, or misshapen connections never throw and never touch Discord).

type ShelterFlux = {
    dispatcher: {
        subscriptions: Map<string, Array<(dispatch: unknown) => void>>;
        subscribe: (event: string, handler: (dispatch: unknown) => void) => void;
        unsubscribe: (event: string, handler: (dispatch: unknown) => void) => void;
    };
    stores: {
        UserStore: { getCurrentUser: () => { id: string } };
        MediaEngineStore: { getMediaEngine: () => { connections: unknown[] } };
    };
};

function makeSender(setParametersCalls: unknown[][]) {
    return {
        track: { kind: "video" },
        getParameters: () => ({ encodings: [{ maxBitrate: 1_000_000 }] }),
        setParameters: (params: unknown) => {
            setParametersCalls.push([params]);
            return Promise.resolve();
        },
    };
}

function makeConnection(sender: unknown) {
    return {
        streamUserId: "user-1",
        pc: { getSenders: () => [sender] },
        videoStreamParameters: [{}],
        videoQualityManager: { goliveMaxQuality: {} },
    };
}

function installShelter(options: { mode?: string; connections?: unknown[]; flux?: boolean }): {
    flux: ShelterFlux;
    dispatch: (event: string, payload: unknown) => void;
} {
    const connections = options.connections ?? [];
    const subscribed = new Map<string, Array<(dispatch: unknown) => void>>();
    const flux = {
        dispatcher: {
            subscriptions: subscribed,
            subscribe: (event: string, handler: (dispatch: unknown) => void) => {
                const list = subscribed.get(event) ?? [];
                list.push(handler);
                subscribed.set(event, list);
            },
            unsubscribe: (event: string, handler: (dispatch: unknown) => void) => {
                subscribed.set(
                    event,
                    (subscribed.get(event) ?? []).filter((entry) => entry !== handler),
                );
            },
        },
        stores: {
            UserStore: { getCurrentUser: () => ({ id: "user-1" }) },
            MediaEngineStore: { getMediaEngine: () => ({ connections }) },
        },
    };
    const shelter: Record<string, unknown> = {
        plugin: { store: { settings: { screenshareQuality: options.mode ?? "default" }, themes: [] } },
    };
    if (options.flux !== false) shelter.flux = flux;
    Reflect.set(globalThis, "shelter", shelter);
    Reflect.set(globalThis, "window", { remcord: { settings: {} } });
    Reflect.set(globalThis, "document", {
        activeElement: null,
        querySelectorAll: () => [],
        contains: () => false,
    });
    Reflect.set(globalThis, "HTMLElement", class {});
    Reflect.set(globalThis, "Element", class {});
    Reflect.set(globalThis, "getComputedStyle", () => ({ overflowY: "visible" }));
    Reflect.set(globalThis, "requestAnimationFrame", () => 0);
    Reflect.set(globalThis, "cancelAnimationFrame", () => undefined);
    return {
        flux: flux as ShelterFlux,
        dispatch: (event, payload) => {
            for (const handler of subscribed.get(event) ?? []) handler(payload);
        },
    };
}

async function loadStream() {
    return await import("../src/shelter/settings/screenshareStream.js");
}

test("default mode installs nothing and patches nothing", async () => {
    const { flux } = installShelter({ mode: "default", connections: [makeConnection(makeSender([]))] });
    try {
        const stream = await loadStream();
        stream.resetScreenshareStreamControlForTests();
        assert.equal(stream.installScreenshareStreamControl(), false);
        assert.equal(flux.dispatcher.subscriptions.size, 0);
        assert.equal(stream.patchScreenshareStream("test"), false);
    } finally {
        (await loadStream()).resetScreenshareStreamControlForTests();
    }
});

test("active mode without flux installs nothing and never throws", async () => {
    installShelter({ mode: "balanced", flux: false });
    try {
        const stream = await loadStream();
        stream.resetScreenshareStreamControlForTests();
        assert.equal(stream.installScreenshareStreamControl(), false);
        assert.equal(stream.patchScreenshareStream("test"), false);
    } finally {
        (await loadStream()).resetScreenshareStreamControlForTests();
    }
});

test("balanced caps the sent stream at 720p30 with Electron-parity bitrates", async () => {
    const setParametersCalls: unknown[][] = [];
    const connection = makeConnection(makeSender(setParametersCalls));
    const { flux, dispatch } = installShelter({ mode: "balanced", connections: [connection] });
    try {
        const stream = await loadStream();
        stream.resetScreenshareStreamControlForTests();
        assert.equal(stream.installScreenshareStreamControl(), true);
        assert.deepEqual([...flux.dispatcher.subscriptions.keys()].sort(), [
            "MEDIA_ENGINE_VIDEO_SOURCE_QUALITY_CHANGED",
            "STREAM_DELETE",
        ]);
        dispatch("MEDIA_ENGINE_VIDEO_SOURCE_QUALITY_CHANGED", {});
        await new Promise((resolve) => setImmediate(resolve));
        const params = connection.videoStreamParameters[0] as Record<string, unknown>;
        assert.equal(params.maxFrameRate, 30);
        assert.deepEqual(params.maxResolution, { type: "fixed", width: 1280, height: 720 });
        assert.equal(params.maxPixelCount, 1280 * 720);
        // 720p30: target 4.0 Mbps, min 3.2, max 4.8 (Electron table math).
        assert.equal(params.maxBitrate, 4_800_000);
        const golive = connection.videoQualityManager.goliveMaxQuality as Record<string, unknown>;
        assert.equal(golive.bitrateTarget, 4_000_000);
        assert.equal(golive.bitrateMin, 3_200_000);
        assert.equal(golive.bitrateMax, 4_800_000);
        // Sender ceiling raised (was 1.0 Mbps), framerate pinned.
        assert.equal(setParametersCalls.length, 1);
        const encodings = (setParametersCalls[0][0] as { encodings: Array<Record<string, unknown>> }).encodings;
        assert.equal(encodings[0].maxBitrate, 4_800_000);
        assert.equal(encodings[0].maxFramerate, 30);
    } finally {
        const stream = await loadStream();
        stream.uninstallScreenshareStreamControl();
        assert.equal(flux.dispatcher.subscriptions.get("MEDIA_ENGINE_VIDEO_SOURCE_QUALITY_CHANGED")?.length ?? 0, 0);
        assert.equal(flux.dispatcher.subscriptions.get("STREAM_DELETE")?.length ?? 0, 0);
    }
});

test("low caps at 540p15 and never lowers an existing higher sender ceiling", async () => {
    const setParametersCalls: unknown[][] = [];
    const sender = {
        track: { kind: "video" },
        getParameters: () => ({ encodings: [{ maxBitrate: 10_000_000 }] }),
        setParameters: (params: unknown) => {
            setParametersCalls.push([params]);
            return Promise.resolve();
        },
    };
    const connection = makeConnection(sender);
    installShelter({ mode: "low", connections: [connection] });
    try {
        const stream = await loadStream();
        stream.resetScreenshareStreamControlForTests();
        assert.equal(stream.patchScreenshareStream("test"), true);
        const params = connection.videoStreamParameters[0] as Record<string, unknown>;
        assert.equal(params.maxFrameRate, 15);
        assert.deepEqual(params.maxResolution, { type: "fixed", width: 960, height: 540 });
        // 540p15: nearest table key 480 → target 2.5 Mbps, min 2.0, max 3.0.
        const golive = connection.videoQualityManager.goliveMaxQuality as Record<string, unknown>;
        assert.equal(golive.bitrateTarget, 2_500_000);
        assert.equal(golive.bitrateMax, 3_000_000);
        // Existing 10 Mbps ceiling is kept (raise-only), framerate pinned.
        const encodings = (setParametersCalls[0][0] as { encodings: Array<Record<string, unknown>> }).encodings;
        assert.equal(encodings[0].maxBitrate, 10_000_000);
        assert.equal(encodings[0].maxFramerate, 15);
    } finally {
        (await loadStream()).resetScreenshareStreamControlForTests();
    }
});

test("idle or foreign connections patch nothing and never throw", async () => {
    installShelter({ mode: "balanced", connections: [{ streamUserId: "someone-else" }] });
    try {
        const stream = await loadStream();
        stream.resetScreenshareStreamControlForTests();
        assert.equal(stream.patchScreenshareStream("test"), false);
    } finally {
        (await loadStream()).resetScreenshareStreamControlForTests();
    }
});

test("misshapen connections fail open without throwing", async () => {
    installShelter({
        mode: "low",
        connections: [{ streamUserId: "user-1", videoStreamParameters: "nope", pc: {} }],
    });
    try {
        const stream = await loadStream();
        stream.resetScreenshareStreamControlForTests();
        // Present connection: attempted, never throws (boolean honest).
        assert.equal(typeof stream.patchScreenshareStream("test"), "boolean");
    } finally {
        (await loadStream()).resetScreenshareStreamControlForTests();
    }
});

test("new presets apply Electron-parity bitrate math per height", async () => {
    // [mode, width, height, fps, target, min, max]
    const cases: Array<[string, number, number, number, number, number, number]> = [
        ["ultra-low", 256, 144, 15, 2_500_000, 2_000_000, 3_000_000],
        ["full-hd", 1920, 1080, 30, 8_000_000, 6_400_000, 9_600_000],
        ["quad-hd", 2560, 1440, 30, 12_000_000, 9_600_000, 14_400_000],
        ["ultra-hd", 3840, 2160, 30, 18_000_000, 14_400_000, 21_600_000],
    ];
    for (const [mode, width, height, fps, target, min, max] of cases) {
        const setParametersCalls: unknown[][] = [];
        const connection = makeConnection(makeSender(setParametersCalls));
        installShelter({ mode, connections: [connection] });
        try {
            const stream = await loadStream();
            stream.resetScreenshareStreamControlForTests();
            assert.equal(stream.patchScreenshareStream("test"), true, `${mode} must patch`);
            const params = connection.videoStreamParameters[0] as Record<string, unknown>;
            assert.equal(params.maxFrameRate, fps, `${mode} fps`);
            assert.deepEqual(params.maxResolution, { type: "fixed", width, height }, `${mode} resolution`);
            assert.equal(params.maxPixelCount, width * height, `${mode} pixel count`);
            assert.equal(params.maxBitrate, max, `${mode} stream max`);
            const golive = connection.videoQualityManager.goliveMaxQuality as Record<string, unknown>;
            assert.equal(golive.bitrateTarget, target, `${mode} target`);
            assert.equal(golive.bitrateMin, min, `${mode} min`);
            assert.equal(golive.bitrateMax, max, `${mode} max`);
            const encodings = (setParametersCalls[0][0] as { encodings: Array<Record<string, unknown>> }).encodings;
            assert.equal(encodings[0].maxFramerate, fps, `${mode} sender fps`);
        } finally {
            (await loadStream()).resetScreenshareStreamControlForTests();
        }
    }
});

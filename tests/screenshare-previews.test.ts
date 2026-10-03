import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { serializeCapturerSources } from "../src/discord/screenshareSources.ts";
import {
    installDisplayMediaPatch,
    restoreDisplayMediaPatch,
} from "../src/shelter/screenshare/components/displayMediaPatch.ts";

function makeMediaDevices(original: MediaDevices["getDisplayMedia"]): MediaDevices {
    return { getDisplayMedia: original } as unknown as MediaDevices;
}

function passthrough(
    original: MediaDevices["getDisplayMedia"],
    thisArg: MediaDevices,
    options: DisplayMediaStreamOptions,
) {
    return original.call(thisArg, options);
}

function fakeImage(dataUrl: string) {
    return {
        toDataURL: () => dataUrl,
        isEmpty: () => dataUrl.length === 0,
    };
}

describe("serializeCapturerSources", () => {
    it("gives each source a distinct usable image and keeps names", () => {
        const sources = [
            { id: "screen:0:0", name: "Entire screen", thumbnail: fakeImage("data:image/png;base64,SCREEN0") },
            { id: "window:11:0", name: "Jokel Comms DOD", thumbnail: fakeImage("data:image/png;base64,JOKEL") },
            { id: "window:22:0", name: "MSI Afterburner", thumbnail: fakeImage("data:image/png;base64,MSIAB") },
        ];

        const result = serializeCapturerSources(sources);
        assert.equal(result.length, 3);
        const images = result.map((source) => source.thumbnail);
        assert.equal(new Set(images).size, 3);
        for (const [index, source] of result.entries()) {
            assert.equal(source.id, sources[index].id);
            assert.equal(source.name, sources[index].name);
            assert.ok(source.thumbnail.startsWith("data:image/"), `expected data URL for ${source.name}`);
            assert.ok(source.thumbnail.length > "data:image/".length);
        }
    });

    it("uses a source-specific app icon when the thumbnail is missing", () => {
        const sources = [
            {
                id: "window:1:0",
                name: "Friends List",
                thumbnail: fakeImage(""),
                appIcon: fakeImage("data:image/png;base64,FRIENDICON"),
            },
            {
                id: "window:2:0",
                name: "Pull requests",
                thumbnail: fakeImage(""),
                appIcon: fakeImage("data:image/png;base64,PRICON"),
            },
        ];

        const result = serializeCapturerSources(sources);
        assert.equal(result[0].name, "Friends List");
        assert.equal(result[0].thumbnail, "data:image/png;base64,FRIENDICON");
        assert.equal(result[1].name, "Pull requests");
        assert.equal(result[1].thumbnail, "data:image/png;base64,PRICON");
        assert.notEqual(result[0].thumbnail, result[1].thumbnail);
    });

    it("does not reuse a duplicated compositor thumbnail across unrelated sources", () => {
        const sameFrame = "data:image/png;base64,SAMEFRAME";
        const sources = [
            { id: "screen:0:0", name: "Entire screen", thumbnail: fakeImage(sameFrame) },
            {
                id: "window:4432:0",
                name: "Jokel Comms DOD",
                thumbnail: fakeImage(sameFrame),
                appIcon: fakeImage("data:image/png;base64,JOKELICON"),
            },
            { id: "window:9:0", name: "Legcord Screen Share Low FP", thumbnail: fakeImage(sameFrame) },
        ];

        const result = serializeCapturerSources(sources);
        const images = result.map((source) => source.thumbnail);
        assert.equal(new Set(images).size, 3);
        assert.equal(result[1].thumbnail, "data:image/png;base64,JOKELICON");
        for (const source of result) {
            assert.ok(source.thumbnail.startsWith("data:image/"));
        }
    });
});

describe("getDisplayMedia patch lifecycle", () => {
    it("installs one interception layer across repeated installs and restores the original", async () => {
        const stream = {} as MediaStream;
        let originalCalls = 0;
        let originalThis: unknown;
        let originalOptions: DisplayMediaStreamOptions | undefined;
        const original: MediaDevices["getDisplayMedia"] = function (options = {}) {
            originalCalls++;
            originalThis = this;
            originalOptions = options;
            return Promise.resolve(stream);
        };
        const mediaDevices = makeMediaDevices(original);
        const options: DisplayMediaStreamOptions = { video: true };
        let interceptionCalls = 0;

        for (let i = 0; i < 5; i++) {
            installDisplayMediaPatch(mediaDevices, (method, thisArg, constraints) => {
                interceptionCalls++;
                return method.call(thisArg, constraints);
            });
        }

        const wrapper = mediaDevices.getDisplayMedia;
        assert.notEqual(wrapper, original);
        assert.equal(await mediaDevices.getDisplayMedia(options), stream);
        assert.equal(mediaDevices.getDisplayMedia, wrapper);
        assert.equal(interceptionCalls, 1);
        assert.equal(originalCalls, 1);
        assert.equal(originalThis, mediaDevices);
        assert.equal(originalOptions, options);

        restoreDisplayMediaPatch(mediaDevices);
        assert.equal(mediaDevices.getDisplayMedia, original);
    });

    it("supports reinstall after cleanup and a fresh mediaDevices object after navigation", async () => {
        const original: MediaDevices["getDisplayMedia"] = () => Promise.resolve({} as MediaStream);
        const firstDocument = makeMediaDevices(original);
        let interceptionCalls = 0;
        const interceptor = (
            method: MediaDevices["getDisplayMedia"],
            thisArg: MediaDevices,
            options: DisplayMediaStreamOptions,
        ) => {
            interceptionCalls++;
            return method.call(thisArg, options);
        };

        installDisplayMediaPatch(firstDocument, interceptor);
        const firstWrapper = firstDocument.getDisplayMedia;
        restoreDisplayMediaPatch(firstDocument);
        installDisplayMediaPatch(firstDocument, interceptor);
        assert.notEqual(firstDocument.getDisplayMedia, original);
        assert.notEqual(firstDocument.getDisplayMedia, firstWrapper);
        await firstDocument.getDisplayMedia();
        assert.equal(interceptionCalls, 1);
        restoreDisplayMediaPatch(firstDocument);

        const reloadedDocument = makeMediaDevices(original);
        interceptionCalls = 0;
        installDisplayMediaPatch(reloadedDocument, interceptor);
        assert.notEqual(reloadedDocument.getDisplayMedia, original);
        await reloadedDocument.getDisplayMedia();
        assert.equal(interceptionCalls, 1);
        restoreDisplayMediaPatch(reloadedDocument);
        assert.equal(reloadedDocument.getDisplayMedia, original);
    });

    it("does not overwrite a newer implementation during cleanup", async () => {
        const original: MediaDevices["getDisplayMedia"] = () => Promise.resolve({} as MediaStream);
        const replacement: MediaDevices["getDisplayMedia"] = () => Promise.resolve({} as MediaStream);
        const mediaDevices = makeMediaDevices(original);
        let interceptions = 0;

        installDisplayMediaPatch(mediaDevices, (method, thisArg, options) => {
            interceptions++;
            return method.call(thisArg, options);
        });
        const staleWrapper = mediaDevices.getDisplayMedia;
        mediaDevices.getDisplayMedia = replacement;
        restoreDisplayMediaPatch(mediaDevices);

        assert.equal(mediaDevices.getDisplayMedia, replacement);
        await staleWrapper();
        assert.equal(interceptions, 0);
    });

    it("restores the exact original on cleanup and forwards rejection unchanged", async () => {
        const failure = new Error("capture failed");
        const original: MediaDevices["getDisplayMedia"] = () => Promise.reject(failure);
        const mediaDevices = makeMediaDevices(original);
        installDisplayMediaPatch(mediaDevices, passthrough);

        await assert.rejects(mediaDevices.getDisplayMedia(), failure);
        restoreDisplayMediaPatch(mediaDevices);
        assert.equal(mediaDevices.getDisplayMedia, original);
    });
});

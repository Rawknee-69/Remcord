import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import {
    checkForUpdates,
    compareVersions,
    DOWNLOAD_PAGE_URL,
    isUpdateFeedConfigured,
    UPDATE_FEED_URL,
    UPDATE_UNCONFIGURED_MESSAGE,
} from "../src/common/updateFeed.js";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

test("update feed is explicitly unconfigured (no endpoint invented)", () => {
    assert.equal(UPDATE_FEED_URL, null);
    assert.equal(DOWNLOAD_PAGE_URL, null);
    assert.equal(isUpdateFeedConfigured(), false);
    assert.match(UPDATE_UNCONFIGURED_MESSAGE, /not configured/);
});

test("unconfigured check performs no network request and cannot fall back", async () => {
    let fetchCalls = 0;
    const outcome = await checkForUpdates("1.3.0", {
        fetchJson: async (_url: string) => {
            fetchCalls++;
            throw new Error("network must not be touched when unconfigured");
        },
    });
    assert.deepEqual(outcome, { status: "unconfigured" });
    assert.equal(fetchCalls, 0);
});

test("version comparison keeps the dot-stripped semantics without I/O", () => {
    assert.deepEqual(compareVersions("1.3.0", "1.4.0"), { status: "update-available" });
    assert.deepEqual(compareVersions("1.3.0", "1.3.0"), { status: "up-to-date" });
    assert.deepEqual(compareVersions("1.4.0", "1.3.0"), { status: "up-to-date" });
});

test("HeroUpdater performs no request when unconfigured and shows neutral text", () => {
    const source = readFileSync(join(repoRoot, "src/shelter/settings/components/HeroUpdater.tsx"), "utf-8");
    assert.doesNotMatch(source, /legcord/i);
    assert.doesNotMatch(source, /https?:\/\//);
    assert.match(source, /Updates are not configured for this build/);
    assert.match(source, /fetch\(UPDATE_FEED_URL\)/);
});

test("splash screen makes no update-endpoint request", () => {
    const source = readFileSync(join(repoRoot, "src/splash/splash.html"), "utf-8");
    assert.doesNotMatch(source, /legcord/i);
    assert.doesNotMatch(source, /fetch\(/);
    assert.match(source, /Updates are not configured for this build/);
});

test("store updater has no obsolete owner/repo and stays unavailable", () => {
    const source = readFileSync(join(repoRoot, "src/updater.ts"), "utf-8");
    assert.doesNotMatch(source, /legcord/i);
    assert.match(source, /getUpdateProvider/);
    assert.match(source, /Automatic updates are not configured/);
});

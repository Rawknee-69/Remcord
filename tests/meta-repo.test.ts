import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { metaReleasesUrl, resolveMetaRepo } from "../scripts/utils/metaRepo.js";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

test("metadata script requires explicit configuration (unconfigured case)", () => {
    assert.throws(() => resolveMetaRepo({}), /REMCORD_META_REPO is not configured/);
});

test("metadata script rejects malformed configuration", () => {
    assert.throws(() => resolveMetaRepo({ REMCORD_META_REPO: "not-a-repo" }), /must be "owner\/repo"/);
    assert.throws(() => resolveMetaRepo({ REMCORD_META_REPO: "a/b/c" }), /must be "owner\/repo"/);
});

test("metadata script accepts an explicit valid source (configured case)", () => {
    const source = resolveMetaRepo({ REMCORD_META_REPO: "example-owner/remcord" });
    assert.deepEqual(source, { owner: "example-owner", repo: "remcord" });
    assert.equal(metaReleasesUrl(source), "https://api.github.com/repos/example-owner/remcord/releases/latest");
});

test("updateMeta carries no obsolete default owner/repository", () => {
    const source = readFileSync(join(repoRoot, "scripts/utils/updateMeta.ts"), "utf-8");
    assert.doesNotMatch(source, /legcord/i);
    assert.match(source, /resolveMetaRepo/);
});

test("updateMeta fails closed without configuration (no network request)", () => {
    const result = spawnSync(
        process.execPath,
        ["--experimental-strip-types", join(repoRoot, "scripts/utils/updateMeta.ts")],
        {
            env: { ...process.env, REMCORD_META_REPO: "" },
            encoding: "utf-8",
            timeout: 60_000,
        },
    );
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /REMCORD_META_REPO is not configured/);
});

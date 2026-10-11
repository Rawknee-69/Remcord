import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import {
    CUSTOM_BUNDLE_PLACEHOLDER,
    DEFAULT_CUSTOM_CSS_BUNDLE,
    DEFAULT_CUSTOM_JS_BUNDLE,
    isCustomBundleUnset,
    validateCustomBundleUrl,
} from "../src/common/customBundles.js";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

test("custom bundle defaults are empty (no foreign endpoint)", () => {
    assert.equal(DEFAULT_CUSTOM_JS_BUNDLE, "");
    assert.equal(DEFAULT_CUSTOM_CSS_BUNDLE, "");
    assert.equal(isCustomBundleUnset(DEFAULT_CUSTOM_JS_BUNDLE), true);
    assert.equal(isCustomBundleUnset(DEFAULT_CUSTOM_CSS_BUNDLE), true);
    assert.equal(isCustomBundleUnset(undefined), true);
    // The reserved example value is clearly non-resolving.
    assert.match(CUSTOM_BUNDLE_PLACEHOLDER, /example\.invalid/);
});

test("user-supplied bundle URLs keep the http(s)-only validation contract", () => {
    assert.deepEqual(validateCustomBundleUrl("https://example.com/bundle.js"), { ok: true });
    assert.deepEqual(validateCustomBundleUrl("http://127.0.0.1:8080/custom.css"), { ok: true });
    assert.deepEqual(validateCustomBundleUrl(""), { ok: false, reason: "empty" });
    assert.deepEqual(validateCustomBundleUrl("   "), { ok: false, reason: "empty" });
    assert.deepEqual(validateCustomBundleUrl("not a url"), { ok: false, reason: "malformed" });
    assert.deepEqual(validateCustomBundleUrl("file:///etc/passwd"), { ok: false, reason: "unsupported-scheme" });
    assert.deepEqual(validateCustomBundleUrl("ftp://example.com/bundle.js"), {
        ok: false,
        reason: "unsupported-scheme",
    });
    assert.deepEqual(validateCustomBundleUrl(undefined), { ok: false, reason: "empty" });
});

test("Electron defaults use the empty bundle values", () => {
    const source = readFileSync(join(repoRoot, "src/common/config.ts"), "utf-8");
    assert.doesNotMatch(source, /legcord/i);
    assert.match(source, /DEFAULT_CUSTOM_JS_BUNDLE/);
    assert.match(source, /DEFAULT_CUSTOM_CSS_BUNDLE/);
});

test("native defaults use the empty bundle values", () => {
    const source = readFileSync(join(repoRoot, "native/src/settings.rs"), "utf-8");
    assert.doesNotMatch(source, /legcord/i);
    assert.match(source, /String::new\(\)/);
});

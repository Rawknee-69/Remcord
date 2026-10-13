import assert from "node:assert/strict";
import test from "node:test";

type Calls = {
    nativeSave: string[];
    nativeInspect: number;
    nativeCommit: number;
    nativeDiscard: number;
    electronSave: string[];
    electronRestore: number;
    remotePlugins: Array<{ id: string; src: string; update: boolean }>;
    storageWrites: Array<[string, string]>;
};

function installGlobals(options: {
    runtime?: string;
    saveImpl?: (payload: string) => Promise<{ ok: boolean; error?: string }>;
    inspectImpl?: () => Promise<unknown>;
    commitImpl?: () => Promise<unknown>;
    discardImpl?: () => Promise<unknown>;
    electronSaveImpl?: (payload: string) => Promise<{ ok: boolean; error?: string }>;
    electronRestoreImpl?: () => Promise<string>;
}): Calls {
    const calls: Calls = {
        nativeSave: [],
        nativeInspect: 0,
        nativeCommit: 0,
        nativeDiscard: 0,
        electronSave: [],
        electronRestore: 0,
        remotePlugins: [],
        storageWrites: [],
    };
    Reflect.set(globalThis, "shelter", {
        plugin: { store: { settings: {} } },
        plugins: {
            addRemotePlugin: (id: string, src: string, update: boolean) => {
                calls.remotePlugins.push({ id, src, update });
            },
        },
    });
    Reflect.set(globalThis, "localStorage", {
        setItem: (key: string, value: string) => {
            calls.storageWrites.push([key, value]);
        },
    });
    const remcord: Record<string, unknown> = {};
    if (options.runtime !== undefined) {
        remcord.boot = { runtime: options.runtime, platform: "win32", protocol_version: 1, capabilities: [] };
    }
    // One `backup` namespace per shell: the native shell carries the
    // inspect/commit/discard trio (OBJECT shapes), Electron the
    // JSON-STRING save/restore. Same save member name.
    if (options.saveImpl || options.inspectImpl || options.commitImpl || options.discardImpl) {
        const namespace: Record<string, unknown> = {};
        if (options.saveImpl) {
            const impl = options.saveImpl;
            namespace.save = (payload: string) => {
                calls.nativeSave.push(payload);
                return impl(payload);
            };
        }
        if (options.inspectImpl) {
            const impl = options.inspectImpl;
            namespace.inspect = () => {
                calls.nativeInspect += 1;
                return impl();
            };
        }
        if (options.commitImpl) {
            const impl = options.commitImpl;
            namespace.commitRestore = () => {
                calls.nativeCommit += 1;
                return impl();
            };
        }
        if (options.discardImpl) {
            const impl = options.discardImpl;
            namespace.discardRestore = () => {
                calls.nativeDiscard += 1;
                return impl();
            };
        }
        remcord.backup = namespace;
    }
    if (options.electronSaveImpl || options.electronRestoreImpl) {
        const namespace: Record<string, unknown> = {};
        if (options.electronSaveImpl) {
            const impl = options.electronSaveImpl;
            namespace.save = (payload: string) => {
                calls.electronSave.push(payload);
                return impl(payload);
            };
        }
        if (options.electronRestoreImpl) {
            const impl = options.electronRestoreImpl;
            namespace.restore = () => {
                calls.electronRestore += 1;
                return impl();
            };
        }
        // Electron harness: no boot marker, so the gate must stay Electron.
        if (options.runtime === undefined) remcord.backup = { ...((remcord.backup ?? {}) as object), ...namespace };
        else remcord.electronBackup = namespace;
    }
    Reflect.set(globalThis, "window", { remcord });
    return calls;
}

async function loadHelpers() {
    return await import("../src/shelter/settings/settings.js");
}

const PAYLOAD = JSON.stringify({
    includes: {
        remcordConfig: true,
        remcordThemesAndQuickCss: false,
        remcordExtensionPlugins: true,
        vencordModData: true,
        equicordModData: false,
        shelterModData: true,
        modBundles: false,
    },
    clientMods: {
        vencordLocalStorage: '{"a":1}',
        shelter: { plugins: {}, enabledPlugins: [] },
    },
});

const PREVIEW_PAYLOAD = {
    canceled: false,
    includes: {
        remcordConfig: true,
        remcordThemesAndQuickCss: true,
        remcordExtensionPlugins: true,
        vencordModData: true,
        equicordModData: true,
        shelterModData: true,
        modBundles: true,
    },
    archive_bytes: 100,
    uncompressed_bytes: 200,
    entry_count: 2,
    client_mods_bytes: 10,
    has_vencord_mod_data: true,
    has_equicord_mod_data: false,
    has_shelter_mod_data: false,
    entries: ["data/quickCss.css", "data/themes/t/src.css"],
    entries_truncated: false,
};

function trioImpls(
    overrides: {
        inspectImpl?: () => Promise<unknown>;
        commitImpl?: () => Promise<unknown>;
        discardImpl?: () => Promise<unknown>;
    } = {},
) {
    return {
        inspectImpl: () => Promise.resolve({ ok: true, preview: PREVIEW_PAYLOAD }),
        commitImpl: () => Promise.resolve({ ok: true, clientMods: {} }),
        discardImpl: () => Promise.resolve({ discarded: true }),
        ...overrides,
    };
}

test("native shell with save only selects no native restore path", async () => {
    installGlobals({ runtime: "webview2", saveImpl: () => Promise.resolve({ ok: true }) });
    const { isNativeBackupAvailable } = await loadHelpers();
    // Save-only namespace is NOT enough: the inspect/commit/discard trio
    // must all be present.
    assert.equal(isNativeBackupAvailable(), false);
});

test("native shell with the full trio selects the native path", async () => {
    installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        inspectImpl: () => Promise.resolve({ ok: true, preview: PREVIEW_PAYLOAD }),
        commitImpl: () => Promise.resolve({ ok: true, clientMods: {} }),
        discardImpl: () => Promise.resolve({ discarded: true }),
    });
    const { isNativeBackupAvailable } = await loadHelpers();
    assert.equal(isNativeBackupAvailable(), true);
});

test("native shell with a partial trio falls back safely", async () => {
    installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        inspectImpl: () => Promise.resolve({ ok: true, preview: PREVIEW_PAYLOAD }),
    });
    const { isNativeBackupAvailable } = await loadHelpers();
    assert.equal(isNativeBackupAvailable(), false);
});

test("Electron shell never selects the native path", async () => {
    installGlobals({
        electronSaveImpl: () => Promise.resolve({ ok: true }),
        electronRestoreImpl: () => Promise.resolve('{"ok":true}'),
    });
    const { isNativeBackupAvailable } = await loadHelpers();
    assert.equal(isNativeBackupAvailable(), false);
});

test("non-WebView2 runtime never selects the native path", async () => {
    installGlobals({
        runtime: "electron",
        saveImpl: () => Promise.resolve({ ok: true }),
        inspectImpl: () => Promise.resolve({ ok: true, preview: PREVIEW_PAYLOAD }),
        commitImpl: () => Promise.resolve({ ok: true, clientMods: {} }),
        discardImpl: () => Promise.resolve({ discarded: true }),
    });
    const { isNativeBackupAvailable } = await loadHelpers();
    assert.equal(isNativeBackupAvailable(), false);
});

test("native shell without the backup namespace falls back safely", async () => {
    installGlobals({ runtime: "webview2" });
    const { isNativeBackupAvailable, inspectBackupNative, commitRestoreNative } = await loadHelpers();
    assert.equal(isNativeBackupAvailable(), false);
    await assert.rejects(inspectBackupNative(), /unavailable/);
    await assert.rejects(commitRestoreNative(), /unavailable/);
});

test("save calls the shell backup.save exactly once with the exact payload", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        ...trioImpls(),
    });
    const { saveBackup } = await loadHelpers();
    const result = await saveBackup(PAYLOAD);
    assert.deepEqual(calls.nativeSave, [PAYLOAD]);
    assert.equal(result.ok, true);
    // The include flags and clientMods survive byte-identical: no renderer
    // duplication of flag logic, no Rust-side payload construction.
    assert.equal(JSON.parse(calls.nativeSave[0]).includes.remcordThemesAndQuickCss, false);
    assert.equal(JSON.parse(calls.nativeSave[0]).clientMods.vencordLocalStorage, '{"a":1}');
});

test("Electron save uses the existing bridge exactly once", async () => {
    const calls = installGlobals({
        electronSaveImpl: () => Promise.resolve({ ok: true }),
        electronRestoreImpl: () => Promise.resolve('{"ok":true}'),
    });
    const { saveBackup } = await loadHelpers();
    const result = await saveBackup(PAYLOAD);
    assert.deepEqual(calls.electronSave, [PAYLOAD]);
    assert.equal(result.ok, true);
});

test("save cancellation and failure pass through for the existing toasts", async () => {
    installGlobals({
        runtime: "webview2",
        saveImpl: (payload: string) =>
            Promise.resolve(
                payload === "cancel" ? { ok: false, error: "CANCELLED" } : { ok: false, error: "BACKUP_TOO_LARGE" },
            ),
        ...trioImpls(),
    });
    const { saveBackup } = await loadHelpers();
    assert.deepEqual(await saveBackup("cancel"), { ok: false, error: "CANCELLED" });
    assert.deepEqual(await saveBackup("other"), { ok: false, error: "BACKUP_TOO_LARGE" });
});

test("save rejections propagate so the caller shows the error state", async () => {
    installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.reject(new Error("transport down")),
        ...trioImpls(),
    });
    const { saveBackup } = await loadHelpers();
    await assert.rejects(saveBackup(PAYLOAD), /transport down/);
});

test("native inspect stages exactly once and normalizes the preview", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        ...trioImpls(),
    });
    const { isNativeBackupAvailable, inspectBackupNative, normalizeNativeInspectResult } = await loadHelpers();
    assert.equal(isNativeBackupAvailable(), true);
    const raw = await inspectBackupNative();
    assert.equal(calls.nativeInspect, 1);
    assert.equal(calls.nativeCommit, 0);
    assert.equal(calls.nativeDiscard, 0);
    assert.equal(calls.electronRestore, 0);
    const outcome = normalizeNativeInspectResult(raw);
    assert.ok("preview" in outcome);
    if ("preview" in outcome) {
        assert.equal(outcome.preview.entryCount, 2);
        assert.deepEqual(outcome.preview.entries, ["data/quickCss.css", "data/themes/t/src.css"]);
        assert.equal(outcome.preview.entriesTruncated, false);
        assert.equal(outcome.preview.includes.remcordConfig, true);
        assert.equal(outcome.preview.hasVencordModData, true);
        assert.equal(outcome.preview.hasShelterModData, false);
    }
});

test("native inspect maps cancellation, typed failures, and garbage", async () => {
    installGlobals({ runtime: "webview2", saveImpl: () => Promise.resolve({ ok: true }), ...trioImpls() });
    const { normalizeNativeInspectResult } = await loadHelpers();
    assert.deepEqual(normalizeNativeInspectResult({ ok: false, error: "CANCELLED" }), { canceled: true });
    assert.deepEqual(normalizeNativeInspectResult({ ok: false, error: "INVALID_ARCHIVE" }), {
        error: "INVALID_ARCHIVE",
    });
    for (const garbage of [
        null,
        undefined,
        "string",
        42,
        [],
        {},
        { ok: "yes" },
        { ok: false },
        { ok: true },
        { ok: true, preview: null },
        { ok: true, preview: { ...PREVIEW_PAYLOAD, entries: ["x".repeat(513)] } },
        { ok: true, preview: { ...PREVIEW_PAYLOAD, extra: 1 } },
    ]) {
        assert.deepEqual(normalizeNativeInspectResult(garbage), { invalid: true });
    }
    // A preview missing the canceled flag is misshapen, never a silent pass.
    const { canceled: _dropped, ...noFlag } = PREVIEW_PAYLOAD;
    assert.deepEqual(normalizeNativeInspectResult({ ok: true, preview: noFlag }), { invalid: true });
});

test("native commit applies the held envelope and nothing else", async () => {
    const envelope = { vencordLocalStorage: '{"a":1}', shelter: { plugins: {} } };
    const calls = installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        ...trioImpls({ commitImpl: () => Promise.resolve({ ok: true, clientMods: envelope }) }),
    });
    const { commitRestoreNative, normalizeNativeRestoreResult } = await loadHelpers();
    const raw = await commitRestoreNative();
    assert.equal(calls.nativeCommit, 1);
    const outcome = normalizeNativeRestoreResult(raw);
    assert.equal(outcome.ok, true);
    // The envelope passes through for the existing renderer application
    // path — and nothing here applies it: no localStorage write, no remote
    // plugin installation at this boundary.
    assert.deepEqual(calls.storageWrites, []);
    assert.deepEqual(calls.remotePlugins, []);
});

test("native commit maps cancellation and typed failures without paths", async () => {
    installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        ...trioImpls({ commitImpl: () => Promise.resolve({ ok: false, error: "NO_STAGED_RESTORE" }) }),
    });
    const { commitRestoreNative, normalizeNativeRestoreResult } = await loadHelpers();
    assert.deepEqual(normalizeNativeRestoreResult(await commitRestoreNative()), {
        ok: false,
        error: "NO_STAGED_RESTORE",
    });
});

test("native discard is best-effort and never throws", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        ...trioImpls({ discardImpl: () => Promise.reject(new Error("transport down")) }),
    });
    const { discardRestoreNative } = await loadHelpers();
    assert.equal(await discardRestoreNative(), false);
    assert.equal(calls.nativeDiscard, 1);
    installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        ...trioImpls(),
    });
    const helpers = await loadHelpers();
    assert.equal(await helpers.discardRestoreNative(), true);
});

test("Electron restore parses the JSON string into the shared outcome", async () => {
    installGlobals({
        electronSaveImpl: () => Promise.resolve({ ok: true }),
        electronRestoreImpl: () => Promise.resolve('{"ok":true,"clientMods":{"shelter":{"plugins":{}}}}'),
    });
    const { parseElectronRestoreResult } = await loadHelpers();
    const outcome = parseElectronRestoreResult('{"ok":true,"clientMods":{"shelter":{"plugins":{}}}}');
    assert.equal(outcome.ok, true);
    assert.deepEqual(parseElectronRestoreResult('{"ok":false,"error":"CANCELLED"}'), {
        ok: false,
        error: "CANCELLED",
    });
    assert.deepEqual(parseElectronRestoreResult('{"ok":false,"error":"boom"}'), {
        ok: false,
        error: "boom",
    });
    // Explicit failure without a message keeps the generic error state
    // (existing unknownError UX), not the invalid-file state.
    assert.deepEqual(parseElectronRestoreResult('{"ok":false}'), { ok: false, error: "UNKNOWN" });
    // Unparseable and non-string input keeps the invalid-file state.
    assert.deepEqual(parseElectronRestoreResult("{not json"), { ok: false, error: "INVALID" });
    assert.deepEqual(parseElectronRestoreResult(42), { ok: false, error: "INVALID" });
    assert.deepEqual(parseElectronRestoreResult(null), { ok: false, error: "INVALID" });
    assert.deepEqual(parseElectronRestoreResult("[1,2]"), { ok: false, error: "INVALID" });
});

test("parsing never executes remote plugins or touches storage", async () => {
    const calls = installGlobals({
        electronSaveImpl: () => Promise.resolve({ ok: true }),
        electronRestoreImpl: () =>
            Promise.resolve(
                '{"ok":true,"clientMods":{"shelter":{"plugins":{"evil":{"src":"https://attacker.invalid/x.js","update":true}}}}}',
            ),
    });
    const { parseElectronRestoreResult, normalizeNativeRestoreResult } = await loadHelpers();
    parseElectronRestoreResult(
        '{"ok":true,"clientMods":{"shelter":{"plugins":{"evil":{"src":"https://attacker.invalid/x.js","update":true}}}}}',
    );
    normalizeNativeRestoreResult({
        ok: true,
        clientMods: { shelter: { plugins: { evil: { src: "https://attacker.invalid/x.js", update: true } } } },
    });
    // Applying that envelope stays the pre-existing renderer's job
    // (applyClientModsFromRestore in BackupSection.tsx) — this layer only
    // normalizes shapes and must not fetch, execute, or persist anything.
    assert.deepEqual(calls.storageWrites, []);
    assert.deepEqual(calls.remotePlugins, []);
});

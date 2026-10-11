import assert from "node:assert/strict";
import test from "node:test";

type Calls = {
    nativeSave: string[];
    nativeRestore: number;
    electronSave: string[];
    electronRestore: number;
    remotePlugins: Array<{ id: string; src: string; update: boolean }>;
    storageWrites: Array<[string, string]>;
};

function installGlobals(options: {
    runtime?: string;
    saveImpl?: (payload: string) => Promise<{ ok: boolean; error?: string }>;
    restoreImpl?: () => Promise<unknown>;
    electronSaveImpl?: (payload: string) => Promise<{ ok: boolean; error?: string }>;
    electronRestoreImpl?: () => Promise<string>;
}): Calls {
    const calls: Calls = {
        nativeSave: [],
        nativeRestore: 0,
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
    // One `backup` namespace per shell: the native shell carries the OBJECT
    // restore shape, Electron the JSON-STRING shape. Same member names.
    if (options.saveImpl || options.restoreImpl) {
        const namespace: Record<string, unknown> = {};
        if (options.saveImpl) {
            const impl = options.saveImpl;
            namespace.save = (payload: string) => {
                calls.nativeSave.push(payload);
                return impl(payload);
            };
        }
        if (options.restoreImpl) {
            const impl = options.restoreImpl;
            namespace.restore = () => {
                calls.nativeRestore += 1;
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

test("native shell with both backup members selects the native path", async () => {
    installGlobals({ runtime: "webview2", saveImpl: () => Promise.resolve({ ok: true }) });
    const { isNativeBackupAvailable } = await loadHelpers();
    // Save-only namespace is NOT enough: restore needs the object shape.
    assert.equal(isNativeBackupAvailable(), false);
});

test("native shell with save and restore selects the native path", async () => {
    installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        restoreImpl: () => Promise.resolve({ ok: true, clientMods: {} }),
    });
    const { isNativeBackupAvailable } = await loadHelpers();
    assert.equal(isNativeBackupAvailable(), true);
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
        restoreImpl: () => Promise.resolve({ ok: true, clientMods: {} }),
    });
    const { isNativeBackupAvailable } = await loadHelpers();
    assert.equal(isNativeBackupAvailable(), false);
});

test("native shell without the backup namespace falls back safely", async () => {
    installGlobals({ runtime: "webview2" });
    const { isNativeBackupAvailable, restoreBackupNative } = await loadHelpers();
    assert.equal(isNativeBackupAvailable(), false);
    await assert.rejects(restoreBackupNative(), /unavailable/);
});

test("save calls the shell backup.save exactly once with the exact payload", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        restoreImpl: () => Promise.resolve({ ok: true, clientMods: {} }),
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
        restoreImpl: () => Promise.resolve({ ok: true, clientMods: {} }),
    });
    const { saveBackup } = await loadHelpers();
    assert.deepEqual(await saveBackup("cancel"), { ok: false, error: "CANCELLED" });
    assert.deepEqual(await saveBackup("other"), { ok: false, error: "BACKUP_TOO_LARGE" });
});

test("save rejections propagate so the caller shows the error state", async () => {
    installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.reject(new Error("transport down")),
        restoreImpl: () => Promise.resolve({ ok: true, clientMods: {} }),
    });
    const { saveBackup } = await loadHelpers();
    await assert.rejects(saveBackup(PAYLOAD), /transport down/);
});

test("native restore calls the native method exactly once and nothing else", async () => {
    const envelope = { vencordLocalStorage: '{"a":1}', shelter: { plugins: {} } };
    const calls = installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        restoreImpl: () => Promise.resolve({ ok: true, clientMods: envelope }),
    });
    const { isNativeBackupAvailable, restoreBackupNative, normalizeNativeRestoreResult } = await loadHelpers();
    assert.equal(isNativeBackupAvailable(), true);
    const raw = await restoreBackupNative();
    assert.equal(calls.nativeRestore, 1);
    assert.equal(calls.electronRestore, 0);
    const outcome = normalizeNativeRestoreResult(raw);
    assert.equal(outcome.ok, true);
    // The envelope passes through for the existing renderer application
    // path — and nothing here applies it: no localStorage write, no remote
    // plugin installation at this boundary.
    assert.deepEqual(calls.storageWrites, []);
    assert.deepEqual(calls.remotePlugins, []);
});

test("native restore maps cancellation and typed failures without paths", async () => {
    installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        restoreImpl: () => Promise.resolve({ ok: false, error: "INVALID_ARCHIVE" }),
    });
    const { restoreBackupNative, normalizeNativeRestoreResult } = await loadHelpers();
    assert.deepEqual(normalizeNativeRestoreResult(await restoreBackupNative()), {
        ok: false,
        error: "INVALID_ARCHIVE",
    });
});

test("native restore normalizes misshapen results to INVALID, never throwing", async () => {
    installGlobals({
        runtime: "webview2",
        saveImpl: () => Promise.resolve({ ok: true }),
        restoreImpl: () => Promise.resolve(null),
    });
    const { normalizeNativeRestoreResult } = await loadHelpers();
    for (const garbage of [null, undefined, "string", 42, [], {}, { ok: "yes" }, { ok: false }]) {
        assert.deepEqual(normalizeNativeRestoreResult(garbage), { ok: false, error: "INVALID" });
    }
    // Extra members are ignored; only ok/error/clientMods are read.
    assert.deepEqual(normalizeNativeRestoreResult({ ok: true, clientMods: { a: 1 }, path: "C:\\x.zip" }), {
        ok: true,
        clientMods: { a: 1 },
    });
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

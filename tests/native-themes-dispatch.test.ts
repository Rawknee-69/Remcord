import assert from "node:assert/strict";
import test from "node:test";

type ThemeEntry = {
    id: string;
    name: string;
    author: string;
    description: string;
    version: string;
    enabled: boolean;
};

type Calls = {
    electron: string[];
    list: number;
    setEnabled: Array<[string, boolean]>;
    importFiles: number;
    folder: string[];
    warnings: string[];
};

const VALID_A: ThemeEntry = {
    id: "ClearVision-BD",
    name: "ClearVision",
    author: "me",
    description: "desc",
    version: "1.0",
    enabled: false,
};
const VALID_B: ThemeEntry = {
    id: "Midnight-Harbor-BD",
    name: "Midnight",
    author: "you",
    description: "",
    version: "2.0",
    enabled: true,
};

function installGlobals(options: {
    runtime?: string;
    themes?: Record<string, unknown>;
    listImpl?: () => Promise<unknown>;
    setEnabledImpl?: (id: string, enabled: boolean) => Promise<unknown>;
    importImpl?: () => Promise<unknown>;
}): Calls {
    const calls: Calls = { electron: [], list: 0, setEnabled: [], importFiles: 0, folder: [], warnings: [] };
    const themes: Record<string, unknown> = {
        set: (id: string, state: boolean) => {
            calls.electron.push(`themes.set:${id}:${state}`);
        },
        getThemes: () => {
            calls.electron.push("themes.getThemes");
            return [];
        },
        refresh: () => {
            calls.electron.push("themes.refresh");
            return [];
        },
        openImportPicker: () => {
            calls.electron.push("themes.openImportPicker");
        },
        folder: (id: string) => {
            calls.folder.push(`electron:${id}`);
            calls.electron.push(`themes.folder:${id}`);
        },
        install: (url: string) => {
            calls.electron.push(`themes.install:${url}`);
        },
        edit: (id: string) => {
            calls.electron.push(`themes.edit:${id}`);
        },
        uninstall: (id: string) => {
            calls.electron.push(`themes.uninstall:${id}`);
        },
        ...(options.themes ?? {}),
    };
    if (options.listImpl) {
        themes.listThemes = () => {
            calls.list += 1;
            return options.listImpl!();
        };
    }
    if (options.setEnabledImpl) {
        themes.setEnabled = (id: string, enabled: boolean) => {
            calls.setEnabled.push([id, enabled]);
            return options.setEnabledImpl!(id, enabled);
        };
    }
    if (options.importImpl) {
        themes.importFiles = () => {
            calls.importFiles += 1;
            return options.importImpl!();
        };
    }
    const remcord: Record<string, unknown> = { themes };
    if (options.runtime !== undefined) {
        remcord.boot = { runtime: options.runtime, platform: "win32", protocol_version: 1, capabilities: [] };
    }
    Reflect.set(globalThis, "shelter", { plugin: { store: { settings: {}, themes: [] } } });
    Reflect.set(globalThis, "window", { remcord });
    const originalWarn = console.warn;
    console.warn = ((message?: unknown) => {
        calls.warnings.push(String(message));
    }) as typeof console.warn;
    Reflect.set(globalThis, "__restoreWarn", originalWarn);
    return calls;
}

function restoreConsole() {
    const originalWarn = Reflect.get(globalThis, "__restoreWarn") as typeof console.warn | undefined;
    if (originalWarn) console.warn = originalWarn;
}

async function loadHelpers() {
    return await import("../src/shelter/settings/settings.js");
}

test("native list capability loads inventory exactly once", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        listImpl: () => Promise.resolve([VALID_A, VALID_B]),
    });
    try {
        const helpers = await loadHelpers();
        assert.equal(helpers.isNativeThemeListAvailable(), true);
        const themes = await helpers.loadNativeThemes();
        assert.equal(calls.list, 1);
        assert.equal(themes.length, 2);
        assert.equal(themes[0]?.id, "ClearVision-BD");
        assert.equal(themes[0]?.enabled, false);
        assert.equal(themes[1]?.enabled, true);
        assert.deepEqual(calls.electron, []);
    } finally {
        restoreConsole();
    }
});

test("native list absent keeps the Electron path", async () => {
    const calls = installGlobals({});
    try {
        const helpers = await loadHelpers();
        assert.equal(helpers.isNativeThemeListAvailable(), false);
        assert.equal(helpers.isNativeThemeToggleAvailable(), false);
        assert.equal(helpers.isNativeThemeImportAvailable(), false);
        assert.equal(helpers.isNativeThemesMode(), false);
        assert.deepEqual(calls.electron, []);
    } finally {
        restoreConsole();
    }
});

test("malformed native entries are skipped safely", async () => {
    installGlobals({
        runtime: "webview2",
        listImpl: () =>
            Promise.resolve([
                VALID_A,
                { id: "", name: "bad", enabled: true },
                { name: "missing-id", enabled: false },
                "not-an-object",
                { ...VALID_B, enabled: "yes" },
                VALID_B,
            ]),
    });
    try {
        const helpers = await loadHelpers();
        const themes = await helpers.loadNativeThemes();
        assert.equal(themes.length, 2);
        assert.equal(themes[0]?.id, "ClearVision-BD");
        assert.equal(themes[1]?.id, "Midnight-Harbor-BD");
        assert.equal(helpers.toThemeManifest(null), null);
    } finally {
        restoreConsole();
    }
});

test("native toggle calls setEnabled once with the verbatim id", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        setEnabledImpl: (id, enabled) => Promise.resolve({ id, enabled }),
    });
    try {
        const helpers = await loadHelpers();
        assert.equal(helpers.isNativeThemeToggleAvailable(), true);
        await helpers.setNativeThemeEnabled("ClearVision-BD", true);
        assert.deepEqual(calls.setEnabled, [["ClearVision-BD", true]]);
        assert.deepEqual(calls.electron, []);
    } finally {
        restoreConsole();
    }
});

test("two themes stay enabled independently", async () => {
    const state = new Map<string, boolean>([
        ["a-BD", false],
        ["b-BD", false],
    ]);
    const calls = installGlobals({
        runtime: "webview2",
        listImpl: () =>
            Promise.resolve([
                { ...VALID_A, id: "a-BD", enabled: state.get("a-BD") },
                { ...VALID_B, id: "b-BD", enabled: state.get("b-BD") },
            ]),
        setEnabledImpl: (id, enabled) => {
            state.set(id, enabled);
            return Promise.resolve({ id, enabled });
        },
    });
    try {
        const helpers = await loadHelpers();
        await helpers.setNativeThemeEnabled("a-BD", true);
        await helpers.setNativeThemeEnabled("b-BD", true);
        const themes = await helpers.loadNativeThemes();
        assert.equal(themes.find((t) => t.id === "a-BD")?.enabled, true);
        assert.equal(themes.find((t) => t.id === "b-BD")?.enabled, true);
        assert.equal(calls.setEnabled.length, 2);
        assert.deepEqual(calls.electron, []);
    } finally {
        restoreConsole();
    }
});

test("native toggle failure rejects path-free for the card to revert", async () => {
    installGlobals({
        runtime: "webview2",
        setEnabledImpl: () => Promise.reject(new Error("SET_ENABLED_FAILED: C:\\secret")),
    });
    try {
        const helpers = await loadHelpers();
        await assert.rejects(() => helpers.setNativeThemeEnabled("a-BD", true), /Could not update the theme/);
    } finally {
        restoreConsole();
    }
});

test("native import success returns counts; cancellation stays silent", async () => {
    const ok = installGlobals({
        runtime: "webview2",
        importImpl: () => Promise.resolve({ canceled: false, imported: 1, failed: 0 }),
    });
    try {
        const helpers = await loadHelpers();
        assert.equal(helpers.isNativeThemeImportAvailable(), true);
        const result = await helpers.importNativeThemes();
        assert.deepEqual(result, { canceled: false, imported: 1, failed: 0 });
        assert.equal(ok.importFiles, 1);
        assert.deepEqual(ok.warnings, []);
    } finally {
        restoreConsole();
    }
    const cancelled = installGlobals({
        runtime: "webview2",
        importImpl: () => Promise.resolve({ canceled: true, imported: 0, failed: 0 }),
    });
    try {
        const helpers = await loadHelpers();
        const result = await helpers.importNativeThemes();
        assert.deepEqual(result, { canceled: true, imported: 0, failed: 0 });
        assert.equal(cancelled.importFiles, 1);
        assert.deepEqual(cancelled.warnings, []);
    } finally {
        restoreConsole();
    }
});

test("native import failure never rejects and warns path-free", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        importImpl: () => Promise.reject(new Error("IMPORT_FAILED: C:\\secret\\theme.css")),
    });
    try {
        const helpers = await loadHelpers();
        const result = await helpers.importNativeThemes();
        assert.deepEqual(result, { canceled: true, imported: 0, failed: 0 });
        assert.equal(calls.warnings.length, 1);
        assert.doesNotMatch(calls.warnings[0]!, /C:\\|secret/);
    } finally {
        restoreConsole();
    }
});

test("import capability absent returns null for the Electron picker path", async () => {
    const calls = installGlobals({});
    try {
        const helpers = await loadHelpers();
        assert.equal(await helpers.importNativeThemes(), null);
        assert.deepEqual(calls.electron, []);
    } finally {
        restoreConsole();
    }
});

test("deferred operations expose no native RPC", async () => {
    installGlobals({ runtime: "webview2" });
    try {
        const helpers = await loadHelpers();
        assert.equal(typeof (helpers as Record<string, unknown>).installNativeThemeFromUrl, "undefined");
        assert.equal(typeof (helpers as Record<string, unknown>).uninstallNativeTheme, "undefined");
        assert.equal(typeof (helpers as Record<string, unknown>).editNativeTheme, "undefined");
    } finally {
        restoreConsole();
    }
});

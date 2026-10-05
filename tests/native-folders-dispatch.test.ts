import assert from "node:assert/strict";
import test from "node:test";

type Calls = {
    electron: string[];
    open: string[];
    openTheme: string[];
    warnings: string[];
};

function installGlobals(options: {
    runtime?: string;
    folders?: boolean;
    openImpl?: (location: string) => Promise<string>;
    openThemeImpl?: (id: string) => Promise<string>;
}): Calls {
    const calls: Calls = { electron: [], open: [], openTheme: [], warnings: [] };
    Reflect.set(globalThis, "shelter", { plugin: { store: { settings: {} } } });
    const legcord: Record<string, unknown> = {
        settings: {
            openStorageFolder: () => {
                calls.electron.push("settings.openStorageFolder");
            },
            openThemesFolder: () => {
                calls.electron.push("settings.openThemesFolder");
            },
        },
        plugins: {
            openFolder: () => {
                calls.electron.push("plugins.openFolder");
            },
        },
        themes: {
            folder: (id: string) => {
                calls.electron.push(`themes.folder:${id}`);
            },
        },
    };
    if (options.runtime !== undefined) {
        legcord.boot = { runtime: options.runtime, platform: "win32", protocol_version: 1, capabilities: [] };
    }
    if (options.folders !== false) {
        legcord.folders = {
            open: (location: string) => {
                calls.open.push(location);
                return options.openImpl ? options.openImpl(location) : Promise.resolve(location);
            },
            openTheme: (id: string) => {
                calls.openTheme.push(id);
                return options.openThemeImpl ? options.openThemeImpl(id) : Promise.resolve(id);
            },
        };
    }
    Reflect.set(globalThis, "window", { legcord });
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

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

test("native environment reveals each fixed root through folders.open exactly once", async () => {
    for (const location of ["storage", "themes", "plugins"] as const) {
        const calls = installGlobals({ runtime: "webview2" });
        try {
            const { revealFolderNative } = await loadHelpers();
            assert.equal(revealFolderNative(location), true);
            assert.deepEqual(calls.open, [location]);
            assert.deepEqual(calls.electron, []);
        } finally {
            restoreConsole();
        }
    }
});

test("native environment opens a theme folder with the verbatim id", async () => {
    const calls = installGlobals({ runtime: "webview2" });
    try {
        const { openThemeFolderNative } = await loadHelpers();
        assert.equal(openThemeFolderNative("my-theme_1.2"), true);
        assert.deepEqual(calls.openTheme, ["my-theme_1.2"]);
        assert.deepEqual(calls.electron, []);
    } finally {
        restoreConsole();
    }
});

test("the renderer does not filter ids; the host owns validation", async () => {
    const calls = installGlobals({ runtime: "webview2" });
    try {
        const { openThemeFolderNative } = await loadHelpers();
        assert.equal(openThemeFolderNative("../escape"), true);
        assert.deepEqual(calls.openTheme, ["../escape"]);
        assert.deepEqual(calls.electron, []);
    } finally {
        restoreConsole();
    }
});

test("Electron environment keeps the existing IPC path", async () => {
    const calls = installGlobals({});
    try {
        const { revealFolderNative, openThemeFolderNative } = await loadHelpers();
        assert.equal(revealFolderNative("storage"), false);
        assert.equal(openThemeFolderNative("my-theme"), false);
        assert.deepEqual(calls.open, []);
        assert.deepEqual(calls.openTheme, []);
    } finally {
        restoreConsole();
    }
});

test("native environment without the folders namespace falls back to Electron", async () => {
    const calls = installGlobals({ runtime: "webview2", folders: false });
    try {
        const { revealFolderNative, openThemeFolderNative } = await loadHelpers();
        assert.equal(revealFolderNative("themes"), false);
        assert.equal(openThemeFolderNative("my-theme"), false);
        assert.deepEqual(calls.open, []);
        assert.deepEqual(calls.openTheme, []);
    } finally {
        restoreConsole();
    }
});

test("native rejections are swallowed with a path-free diagnostic", async () => {
    const calls = installGlobals({
        runtime: "webview2",
        openImpl: () => Promise.reject(new Error("REVEAL_FAILED: C:\\secret\\themes")),
        openThemeImpl: () => Promise.reject(new Error("OPEN_FAILED")),
    });
    try {
        const { revealFolderNative, openThemeFolderNative } = await loadHelpers();
        assert.equal(revealFolderNative("storage"), true);
        assert.equal(openThemeFolderNative("my-theme"), true);
        await tick();
        await tick();
        assert.equal(calls.warnings.length, 2);
        for (const message of calls.warnings) {
            assert.match(message, /^\[Legcord\] Could not (reveal|open)/);
            assert.doesNotMatch(message, /C:\\|secret|my-theme/);
        }
        assert.deepEqual(calls.electron, []);
    } finally {
        restoreConsole();
    }
});

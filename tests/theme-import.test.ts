import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { addTheme } from "../src/common/dom.js";
import { isSafeThemeCssFile, isSafeThemeId } from "../src/common/sanitization.js";

// Covers the BetterDiscord `.theme.css` import and application lifecycle in
// the Electron shell: safe theme-id / CSS-file validation, DOM injection
// without duplicates, honest import-outcome reporting, and the wired
// main/preload/renderer contracts (picker completion event, uninstall style
// removal, guarded style removal, rejected nameless installs).

function repoSource(path: string): string {
    return readFileSync(new URL(path, import.meta.url), "utf8");
}

// --- Pure validation: theme ids -------------------------------------------

test("theme ids accept what Electron itself creates", () => {
    for (const id of ["ClearVision-BD", "Midnight-Harbor-BD", "null-BD", "a.b_c-d", "My-Cool Theme-BD", "x"]) {
        assert.equal(isSafeThemeId(id), true, `${id} must stay usable`);
    }
});

test("theme ids reject filesystem escape shapes", () => {
    for (const id of [
        "",
        ".",
        "..",
        "../escape",
        "..\\escape",
        "a/b",
        "a\\b",
        "/abs",
        "C:\\secret",
        "C:evil",
        "a:b",
        "nul\0byte",
        "x".repeat(129),
    ]) {
        assert.equal(isSafeThemeId(id), false, `${JSON.stringify(id)} must be rejected`);
    }
});

test("theme CSS filenames stay bare single-component names", () => {
    for (const name of ["src.css", "theme.css", "my-theme.CSS"]) {
        assert.equal(isSafeThemeCssFile(name), true, `${name} must stay usable`);
    }
    for (const name of [
        "",
        ".",
        "..",
        "a/b.css",
        "a\\b.css",
        "../x.css",
        "/abs.css",
        "C:\\x.css",
        "C:x.css",
        "style.js",
        "noextension",
        ".css",
        "x".repeat(129),
    ]) {
        assert.equal(isSafeThemeCssFile(name), false, `${JSON.stringify(name)} must be rejected`);
    }
});

// --- Pure reporting: import outcome ----------------------------------------

function installSettingsGlobals() {
    Reflect.set(globalThis, "shelter", { plugin: { store: { settings: {}, themes: [] } } });
    Reflect.set(globalThis, "window", { remcord: {} });
}

async function loadSettingsHelpers() {
    return await import("../src/shelter/settings/settings.js");
}

test("import counts map to exactly one honest outcome", async () => {
    installSettingsGlobals();
    try {
        const helpers = await loadSettingsHelpers();
        assert.equal(helpers.themeImportOutcome(1, 0), "success");
        assert.equal(helpers.themeImportOutcome(3, 0), "success");
        assert.equal(helpers.themeImportOutcome(1, 1), "partial");
        assert.equal(helpers.themeImportOutcome(0, 2), "failed");
    } finally {
        Reflect.deleteProperty(globalThis, "shelter");
        Reflect.deleteProperty(globalThis, "window");
    }
});

// --- Executed DOM behavior: injection without duplication -------------------

type StubElement = { tagName: string; id: string; textContent: string | null };

function installDomStub() {
    const byId = new Map<string, StubElement>();
    const headChildren: StubElement[] = [];
    Reflect.set(globalThis, "document", {
        createElement: (tag: string): StubElement => ({ tagName: tag, id: "", textContent: null }),
        getElementById: (id: string): StubElement | null => byId.get(id) ?? null,
        head: {
            append: (el: StubElement) => {
                headChildren.push(el);
                byId.set(el.id, el);
            },
        },
    });
    return { byId, headChildren };
}

test("theme CSS injects once per id and coexists with Quick CSS", () => {
    const { byId, headChildren } = installDomStub();
    try {
        addTheme("remcord-quick-css", ":root{}");
        addTheme("Midnight-Harbor-BD", ":root{--x:1}");
        addTheme("Midnight-Harbor-BD", ":root{--x:2}");
        assert.equal(headChildren.length, 2);
        assert.equal(byId.get("Midnight-Harbor-BD")?.textContent, ":root{--x:1}");
        assert.equal(byId.get("remcord-quick-css")?.textContent, ":root{}");
    } finally {
        Reflect.deleteProperty(globalThis, "document");
    }
});

// --- Wired contracts (repo pattern for Electron-main code) ------------------

test("picker completes sequentially with counts and a completion event", () => {
    const ipc = repoSource("../src/discord/ipc.ts");
    assert.match(ipc, /for \(const file of result\.filePaths\) \{[\s\S]{0,400}?themesChanged/);
    assert.match(ipc, /webContents\.send\("themesChanged", \{ imported, failed \}\)/);
    assert.match(ipc, /if \(result\.canceled\) return;/);
});

test("uninstall removes the directory and the live stylesheet", () => {
    const themesMain = repoSource("../src/common/themes.ts");
    assert.match(themesMain, /webContents\.send\("removeTheme", /);
    assert.doesNotMatch(themesMain, /rmdirSync\(.*recursive/);
});

test("nameless theme sources are rejected instead of colliding", () => {
    const themesMain = repoSource("../src/common/themes.ts");
    assert.match(themesMain, /manifest\.name === "null"/);
    assert.match(themesMain, /Not a manifest/);
});

test("enable and uninstall validate ids before touching the filesystem", () => {
    const themesMain = repoSource("../src/common/themes.ts");
    assert.match(themesMain, /isSafeThemeId\(id\)/);
    assert.match(themesMain, /isSafeThemeCssFile\(manifest\.theme\)/);
});

test("preload style removal tolerates a missing element", () => {
    const themesPreload = repoSource("../src/discord/preload/themes.ts");
    assert.match(themesPreload, /ipcRenderer\.on\("removeTheme"/);
    assert.doesNotMatch(themesPreload, /getElementById\(name\)!\.remove/);
});

test("bridge exposes the picker completion event beside the picker send", () => {
    const bridge = repoSource("../src/discord/preload/bridge.ts");
    assert.match(bridge, /openImportPicker: \(\) => ipcRenderer\.send\("openImportPicker"\)/);
    assert.match(bridge, /onThemesChanged/);
    assert.match(bridge, /ipcRenderer\.on\("themesChanged"/);
});

test("URL install reports failure honestly instead of always succeeding", () => {
    const themesPage = repoSource("../src/shelter/settings/pages/ThemesPage.tsx");
    assert.match(themesPage, /window\.remcord\.themes\.install\(downloadUrl\(\)\)/);
    assert.match(themesPage, /themes-importFailed/);
    assert.doesNotMatch(themesPage, /window\.remcord\.themes\.install\(downloadUrl\(\)\);\s*setDownloadUrl\(""\)/);
});

test("file-picker import refreshes the list from the completion event", () => {
    const themesPage = repoSource("../src/shelter/settings/pages/ThemesPage.tsx");
    assert.match(themesPage, /window\.remcord\.themes\.openImportPicker/);
    assert.match(themesPage, /onThemesChanged/);
    assert.match(themesPage, /typeof window\.remcord\.themes\.onThemesChanged === "function"/);
    assert.match(themesPage, /onCleanup/);
});

test("theme update reinstall reports failure honestly", () => {
    const themesCard = repoSource("../src/shelter/settings/components/ThemesCard.tsx");
    assert.match(themesCard, /window\.remcord\.themes\.install\(props\.theme\.updateSrc\)/);
    assert.match(themesCard, /themes-importFailed/);
});

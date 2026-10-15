import assert from "node:assert/strict";
import test from "node:test";

// Covers the QuickCSS editor source resolution in
// `src/shelter/settings/quickCssEditorSource.ts`: the native mod adapters
// replace the dead about:blank popup with a handoff to the in-Discord
// editor, which edits wherever the CSS actually lives. Read failures must
// throw the real error (callers toast it and never open an empty editor
// over existing content); a missing storage API throws an explicit
// unavailability error instead of a TypeError off a missing member.

function setWindow(remcord: Record<string, unknown>, vencordNative?: Record<string, unknown>) {
    const target: Record<string, unknown> = { remcord };
    if (vencordNative !== undefined) target.VencordNative = vencordNative;
    Reflect.set(globalThis, "window", target);
}

test("editor titles name each CSS owner", async () => {
    const { editorTitle } = await import("../src/shelter/settings/quickCssEditorSource.js");
    assert.equal(editorTitle("vencord"), "Vencord QuickCSS");
    assert.equal(editorTitle("equicord"), "Equicord QuickCSS");
    assert.equal(editorTitle("remcord"), "Remcord QuickCSS");
});

test("remcord mode without the native capability throws unavailability", async () => {
    setWindow({});
    const { loadQuickCssEditorSource } = await import("../src/shelter/settings/quickCssEditorSource.js");
    await assert.rejects(loadQuickCssEditorSource("remcord"), /unavailable in this shell/);
});

test("remcord mode resolves host-read content and a writing save", async () => {
    const writes: string[] = [];
    setWindow({
        quickcss: {
            read: () => Promise.resolve({ css: ":root{--q:1;}", bytes: 15 }),
            write: (css: string) => {
                writes.push(css);
                return Promise.resolve({ bytes: css.length });
            },
        },
    });
    const { loadQuickCssEditorSource } = await import("../src/shelter/settings/quickCssEditorSource.js");
    const source = await loadQuickCssEditorSource("remcord");
    assert.equal(source.initialCss, ":root{--q:1;}");
    await source.saveCss(":root{--w:2;}");
    assert.deepEqual(writes, [":root{--w:2;}"]);
});

test("remcord mode read failure throws the real error", async () => {
    setWindow({
        quickcss: {
            read: () => Promise.reject(new Error("QUICKCSS_UNAVAILABLE")),
            write: () => Promise.resolve({ bytes: 0 }),
        },
    });
    const { loadQuickCssEditorSource } = await import("../src/shelter/settings/quickCssEditorSource.js");
    await assert.rejects(loadQuickCssEditorSource("remcord"), /QUICKCSS_UNAVAILABLE/);
});

test("mod mode without a running mod throws unavailability", async () => {
    setWindow({ quickcss: {} });
    const { loadQuickCssEditorSource } = await import("../src/shelter/settings/quickCssEditorSource.js");
    await assert.rejects(loadQuickCssEditorSource("vencord"), /not running/);
    await assert.rejects(loadQuickCssEditorSource("equicord"), /not running/);
});

test("mod mode resolves mod-stored content and a persisting save", async () => {
    const saved: string[] = [];
    setWindow(
        {},
        {
            quickCss: {
                get: () => Promise.resolve(".x{color:red}"),
                set: (css: string) => {
                    saved.push(css);
                    return Promise.resolve();
                },
            },
        },
    );
    const { loadQuickCssEditorSource } = await import("../src/shelter/settings/quickCssEditorSource.js");
    const source = await loadQuickCssEditorSource("equicord");
    assert.equal(source.initialCss, ".x{color:red}");
    await source.saveCss(".x{color:blue}");
    assert.deepEqual(saved, [".x{color:blue}"]);
});

test("mod mode read failure throws the real error", async () => {
    setWindow(
        {},
        {
            quickCss: {
                get: () => Promise.reject(new Error("boom")),
                set: () => Promise.resolve(),
            },
        },
    );
    const { loadQuickCssEditorSource } = await import("../src/shelter/settings/quickCssEditorSource.js");
    await assert.rejects(loadQuickCssEditorSource("vencord"), /boom/);
});

function setNativeWindow(remcord: Record<string, unknown>) {
    Reflect.set(globalThis, "window", { remcord });
    if (Reflect.get(globalThis, "shelter") === undefined) {
        Reflect.set(globalThis, "shelter", { plugin: { store: { settings: {}, themes: [] } } });
    }
}

test("system CSS editor handoff is unavailable without the native shell", async () => {
    setNativeWindow({});
    const { isNativeCssEditorAvailable, openSystemCssEditor } = await import("../src/shelter/settings/settings.js");
    assert.equal(isNativeCssEditorAvailable(), false);
    await assert.rejects(openSystemCssEditor(), /unavailable/);
});

test("system CSS editor handoff opens through the host namespace", async () => {
    let opened = 0;
    setNativeWindow({
        boot: { runtime: "webview2" },
        cssEditor: {
            openSystem: () => {
                opened += 1;
                return Promise.resolve({ opened: true });
            },
        },
    });
    const { isNativeCssEditorAvailable, openSystemCssEditor } = await import("../src/shelter/settings/settings.js");
    assert.equal(isNativeCssEditorAvailable(), true);
    assert.equal(await openSystemCssEditor(), true);
    assert.equal(opened, 1);
});

test("system CSS editor handoff rejects misshapen host outcomes", async () => {
    setNativeWindow({
        boot: { runtime: "webview2" },
        cssEditor: { openSystem: () => Promise.resolve({ opened: false }) },
    });
    const { openSystemCssEditor } = await import("../src/shelter/settings/settings.js");
    await assert.rejects(openSystemCssEditor(), /Could not open/);
});

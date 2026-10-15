/**
 * Headless behavioral test for the built gallery context-menu plugin
 * (`ts-out/plugins/gallery/plugin.js` — the exact bytes the app embeds).
 *
 * It drives the real bundle through the full user flow with a minimal fake
 * DOM: right-click media → menu renders → click Copy Link / Save Image /
 * Save to Gallery → the save popup appears with working actions.
 *
 * Run: `node --test native/tests/gallery-plugin.test.mjs`
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const pluginSource = readFileSync(new URL("../../ts-out/plugins/gallery/plugin.js", import.meta.url), "utf8");

// ---------------------------------------------------------------------------
// Minimal fake DOM with real capture/bubble propagation semantics.
// ---------------------------------------------------------------------------

function matchesSelector(el, selector) {
    const parts = selector
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
    return parts.some((part) => matchesSingle(el, part));
}

function matchesSingle(el, selector) {
    if (!(el instanceof FakeElement)) return false;
    const trimmed = selector.trim();
    const classMatch = trimmed.match(/^\.([\w-]+)$/);
    if (classMatch) {
        return String(el.className ?? "")
            .split(" ")
            .includes(classMatch[1]);
    }
    const m = trimmed.match(/^([a-zA-Z0-9*]+)?(\[.*\])?$/);
    if (!m) return false;
    const [, tag, attrPart] = m;
    if (tag && tag !== "*" && el.tagName.toLowerCase() !== tag.toLowerCase()) return false;
    if (!attrPart) return true;
    const attr = attrPart.match(/^\[([\w-]+)(="([^"]*)")?\]$/);
    if (!attr) return false;
    const [, name, , value] = attr;
    if (value === undefined) return el.hasAttribute(name);
    return el.getAttribute(name) === value;
}

class FakeText {
    constructor(data) {
        this.data = String(data);
        this.parentNode = null;
    }
    get textContent() {
        return this.data;
    }
}

class FakeEvent {
    constructor(type, init = {}) {
        this.type = type;
        this.target = null;
        this.bubbles = init.bubbles !== false;
        this._stopped = false;
        this._prevented = false;
        Object.assign(this, init);
    }
    stopPropagation() {
        this._stopped = true;
    }
    preventDefault() {
        this._prevented = true;
    }
}

class FakeElement {
    constructor(tag, ownerDocument = null) {
        this.tagName = String(tag).toUpperCase();
        this.ownerDocument = ownerDocument;
        this.children = [];
        this.parentNode = null;
        this.attributes = {};
        this.listeners = new Map();
        this.style = {};
        this.style.setProperty = (name, value) => {
            this.style[name] = String(value);
        };
        this.style.getPropertyValue = (name) => this.style[name] ?? "";
        this.style.removeProperty = (name) => {
            delete this.style[name];
        };
        this.value = "";
        this._clicked = false;
    }
    get className() {
        return this.attributes.class ?? "";
    }
    set className(value) {
        this.attributes.class = String(value);
    }
    get id() {
        return this.attributes.id ?? "";
    }
    set id(value) {
        this.setAttribute("id", value);
    }
    setAttribute(name, value) {
        this.attributes[name] = String(value);
        if (name === "id" && this.ownerDocument) this.ownerDocument._ids[value] = this;
    }
    getAttribute(name) {
        return name in this.attributes ? this.attributes[name] : null;
    }
    hasAttribute(name) {
        return name in this.attributes;
    }
    removeAttribute(name) {
        delete this.attributes[name];
    }
    appendChild(child) {
        child.parentNode = this;
        if (!child.ownerDocument && this.ownerDocument) child.ownerDocument = this.ownerDocument;
        if (child instanceof FakeElement && child.getAttribute("id") && this.ownerDocument) {
            this.ownerDocument._ids[child.getAttribute("id")] = child;
        }
        this.children.push(child);
        return child;
    }
    append(...nodes) {
        for (const node of nodes) this.appendChild(node);
    }
    insertBefore(node, ref) {
        const index = this.children.indexOf(ref);
        node.parentNode = this;
        if (index === -1) this.children.push(node);
        else this.children.splice(index, 0, node);
        return node;
    }
    remove() {
        if (!this.parentNode) return;
        const siblings = this.parentNode.children;
        const index = siblings.indexOf(this);
        if (index !== -1) siblings.splice(index, 1);
        this.parentNode = null;
    }
    get parentElement() {
        return this.parentNode instanceof FakeElement ? this.parentNode : null;
    }
    get textContent() {
        return this.children.map((c) => (typeof c.textContent === "string" ? c.textContent : "")).join("");
    }
    set textContent(value) {
        this.children = [new FakeText(value)];
        this.children[0].parentNode = this;
    }
    cloneNode(deep) {
        const clone = new FakeElement(this.tagName, this.ownerDocument);
        for (const [key, value] of Object.entries(this.attributes)) clone.setAttribute(key, value);
        clone.value = this.value;
        if (deep) {
            for (const child of this.children) {
                if (child instanceof FakeElement) clone.appendChild(child.cloneNode(true));
                else if (child instanceof FakeText) clone.appendChild(new FakeText(child.data));
            }
        }
        return clone;
    }
    querySelectorAll(selector) {
        const found = [];
        const walk = (node) => {
            for (const child of node.children) {
                if (child instanceof FakeElement) {
                    if (matchesSelector(child, selector)) found.push(child);
                    walk(child);
                }
            }
        };
        walk(this);
        return found;
    }
    querySelector(selector) {
        return this.querySelectorAll(selector)[0] ?? null;
    }
    closest(selector) {
        let node = this;
        while (node) {
            if (node instanceof FakeElement && matchesSelector(node, selector)) return node;
            node = node.parentNode;
        }
        return null;
    }
    addEventListener(type, listener, options) {
        const capture = typeof options === "boolean" ? options : options?.capture === true;
        const list = this.listeners.get(type) ?? [];
        list.push({ listener, capture });
        this.listeners.set(type, list);
    }
    removeEventListener(type, listener) {
        const list = this.listeners.get(type) ?? [];
        this.listeners.set(
            type,
            list.filter((entry) => entry.listener !== listener),
        );
    }
    dispatchEvent(event) {
        event.target = this;
        const path = [];
        let node = this;
        while (node) {
            path.push(node);
            node = node.parentNode;
        }
        // Capture: document → target's parent (capture listeners only).
        for (let index = path.length - 1; index >= 1; index -= 1) {
            if (event._stopped) return !event._prevented;
            for (const { listener, capture } of path[index].listeners.get(event.type) ?? []) {
                if (capture) listener(event);
                if (event._stopped) return !event._prevented;
            }
        }
        // Target: capture listeners, then bubble listeners.
        const targetListeners = path[0].listeners.get(event.type) ?? [];
        for (const { listener } of targetListeners.filter((entry) => entry.capture)) {
            listener(event);
            if (event._stopped) return !event._prevented;
        }
        for (const { listener } of targetListeners.filter((entry) => !entry.capture)) {
            listener(event);
            if (event._stopped) return !event._prevented;
        }
        // Bubble: target's parent → document (bubble listeners only).
        if (event.bubbles) {
            for (let index = 1; index < path.length; index += 1) {
                if (event._stopped) return !event._prevented;
                for (const { listener, capture } of path[index].listeners.get(event.type) ?? []) {
                    if (!capture) listener(event);
                    if (event._stopped) return !event._prevented;
                }
            }
        }
        return !event._prevented;
    }
    focus() {}
    click() {
        this._clicked = true;
    }
    getContext(type) {
        if (this.tagName !== "CANVAS" || type !== "2d") return null;
        let fillStyle = "";
        return {
            get fillStyle() {
                return fillStyle;
            },
            set fillStyle(value) {
                fillStyle = String(value);
            },
            clearRect() {},
            fillRect() {},
            getImageData() {
                // WebView2's browser-native color parser converts modern
                // CSS Color values to sRGB pixels. These values mirror the
                // production probe's exact Discord Oklab surface/text pair.
                const colors = {
                    "oklab(0.174112 -0.00520672 -0.0129313)": [11, 17, 22, 255],
                    "oklab(0.752989 -0.03581 -0.107396)": [120, 179, 245, 255],
                    "oklab(1 0 0)": [255, 255, 255, 255],
                    "oklab(0 0 0)": [0, 0, 0, 255],
                };
                return { data: new Uint8ClampedArray(colors[fillStyle] ?? [0, 0, 0, 0]) };
            },
        };
    }
}

class FakeDocument extends FakeElement {
    constructor() {
        super("#document");
        this._ids = {};
        this.head = this.createElement("head");
        this.body = this.createElement("body");
        this.documentElement = this.createElement("html");
        this.appendChild(this.documentElement);
        this.documentElement.appendChild(this.head);
        this.documentElement.appendChild(this.body);
    }
    createElement(tag) {
        return new FakeElement(tag, this);
    }
    createTextNode(data) {
        return new FakeText(data);
    }
    createTreeWalker(root, _what) {
        const texts = [];
        const walk = (node) => {
            for (const child of node.children ?? []) {
                if (child instanceof FakeText) texts.push(child);
                else if (child instanceof FakeElement) walk(child);
            }
        };
        walk(root);
        let index = 0;
        return {
            nextNode() {
                return index < texts.length ? texts[index++] : null;
            },
        };
    }
    getElementById(id) {
        if (this._ids[id]) return this._ids[id];
        const found = this.documentElement.querySelector(`[id="${id}"]`);
        return found ?? null;
    }
    querySelector(selector) {
        if (matchesSelector(this.documentElement, selector)) return this.documentElement;
        return this.documentElement.querySelector(selector);
    }
    dispatchEvent(event) {
        event.target = this;
        for (const { listener } of [...(this.listeners.get(event.type) ?? [])]) listener(event);
        return true;
    }
}

// ---------------------------------------------------------------------------
// Harness: one fresh runtime per test.
// ---------------------------------------------------------------------------

function createHarness({ modernDiscordTheme = false, modernLightTheme = false, rawCustomCss = false } = {}) {
    const modernTheme = modernDiscordTheme || modernLightTheme || rawCustomCss;
    const modernSurface = modernLightTheme ? "oklab(1 0 0)" : "oklab(0.174112 -0.00520672 -0.0129313)";
    const modernForeground = modernLightTheme ? "oklab(0 0 0)" : "oklab(0.752989 -0.03581 -0.107396)";
    const toasts = [];
    const clipboard = [];
    const opened = [];
    const fetched = [];
    const addedUrls = [];
    const document = new FakeDocument();
    const menuCallbacks = [];
    const modernVars = Object.assign(
        Object.fromEntries(Array.from({ length: 511 }, (_, index) => [`--discord-custom-${index}`, `custom-${index}`])),
        {
            // These aliases exist in the live computed style, but the
            // legacy primary token does not. Their fixture values prove the
            // derived surface palette replaces an incoherent old palette.
            "--background-secondary-alt": "legacy-secondary",
            "--text-muted": "legacy-muted",
            "--input-background-default": "legacy-input-background",
            "--input-text-default": "legacy-input-text",
            "--input-border-default": "legacy-input-border",
            "--border-subtle": "legacy-border",
            "--brand-500": "#5865f2",
            "--background-secondary": "legacy-background-secondary",
            "--shadow-high": "legacy-shadow",
            "--font-primary": "legacy-font",
        },
    );
    const legacyVars = {
        "--background-primary": "#313338",
        "--text-normal": "#dbdee1",
        "--brand-500": "#5865f2",
        "--custom-accent": "#ff73fa",
    };
    const window = {
        remcord: {
            gallery: {
                addUrl: (url, meta) => {
                    addedUrls.push({ url, meta });
                    return Promise.resolve({ ok: true });
                },
                categories: () => Promise.resolve([{ name: "clips", color: "#5865f2", createdAt: 1 }]),
            },
        },
        open: (...args) => {
            opened.push(args);
            return null;
        },
        matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
    };
    const shelter = {
        util: { log() {} },
        flux: {
            storesFlat: {
                SelectedGuildStore: { getGuildId: () => "" },
                SelectedChannelStore: { getChannelId: () => "" },
                GuildStore: { getGuild: () => null },
                ChannelStore: { getChannel: () => null },
            },
        },
        ui: { showToast: (toast) => toasts.push(toast) },
        observeDom: (_selector, callback) => {
            menuCallbacks.push(callback);
            return () => {};
        },
    };
    const context = vm.createContext({
        shelter,
        document,
        window,
        navigator: {
            clipboard: {
                writeText: async (text) => {
                    clipboard.push(text);
                },
            },
        },
        fetch: async (url) => {
            fetched.push(url);
            return { ok: true, blob: async () => ({ type: "image/png" }) };
        },
        NodeFilter: { SHOW_TEXT: 4 },
        KeyboardEvent: class extends FakeEvent {},
        getComputedStyle: (el) => {
            // Production-faithful: theme variables resolve only inside the
            // themed subtree, while every rendered surface still paints its
            // own background/foreground colors (the menu layer included).
            const vars = modernTheme ? modernVars : legacyVars;
            const classesOf = (node) => String(node.className ?? "").split(" ");
            let themed = false;
            let inMenu = false;
            let node = el;
            while (node) {
                const classes = classesOf(node);
                if (classes.includes("theme-dark") || classes.includes("theme-light")) themed = true;
                try {
                    const role = typeof node.getAttribute === "function" ? node.getAttribute("role") : null;
                    if (role === "menu" || role === "menuitem") inMenu = true;
                } catch {}
                node = node.parentNode;
            }
            const names = themed ? Object.keys(vars) : [];
            const isAppMount = el.getAttribute?.("id") === "app-mount";
            const background = modernTheme
                ? isAppMount
                    ? modernSurface
                    : "rgba(0, 0, 0, 0)"
                : themed
                  ? "rgb(49, 51, 56)"
                  : inMenu
                    ? "rgb(24, 25, 28)"
                    : "rgba(0, 0, 0, 0)";
            const foreground = modernTheme
                ? inMenu
                    ? modernForeground
                    : isAppMount
                      ? modernSurface
                      : ""
                : themed || inMenu
                  ? "rgb(219, 222, 225)"
                  : "";
            const style = {
                length: names.length,
                getPropertyValue: (name) => {
                    if (name === "background-color") return background;
                    if (name === "color") return foreground;
                    return themed ? (vars[name] ?? "") : "";
                },
            };
            names.forEach((name, index) => {
                style[index] = name;
            });
            return style;
        },
        URL: (() => {
            const RealURL = URL;
            RealURL.createObjectURL = () => "blob:mock";
            RealURL.revokeObjectURL = () => {};
            return RealURL;
        })(),
        setTimeout,
        clearTimeout,
    });
    const wrapped =
        "const __exp = {};\n" +
        pluginSource.replace(/\}\)\(\{\}\);?\s*$/, "})(__exp);") +
        "\nglobalThis.__hooks = __exp;";
    vm.runInContext(wrapped, context);
    const hooks = vm.runInContext("globalThis.__hooks", context);
    // Production-faithful tree: Discord's themed container lives under body
    // while menu layers portal outside it. The modern Discord test instead
    // places its theme classes on <html> and renders the app surface in the
    // #app-mount portal host.
    if (modernTheme) {
        document.documentElement.className = rawCustomCss
            ? "platform-web custom-theme-background"
            : modernLightTheme
              ? "platform-web theme-light"
              : "platform-web theme-dark theme-darker custom-theme-background";
        const appMount = document.createElement("div");
        appMount.id = "app-mount";
        document.body.appendChild(appMount);
    } else {
        const themed = document.createElement("div");
        themed.className = "theme-dark";
        document.body.appendChild(themed);
    }
    return { document, window, shelter, hooks, toasts, clipboard, opened, fetched, addedUrls, menuCallbacks, context };
}

function rightClickImage(harness, src) {
    const img = harness.document.createElement("img");
    img.setAttribute("src", src);
    harness.document.body.appendChild(img);
    img.dispatchEvent(new FakeEvent("contextmenu", { bubbles: true }));
    return img;
}

function renderMenu(harness) {
    // A Discord-like image menu layer: portal container holding the menu.
    const layer = harness.document.createElement("div");
    harness.document.body.appendChild(layer);
    const menu = harness.document.createElement("div");
    menu.setAttribute("role", "menu");
    const labels = ["Open Link", "Copy Link", "Copy Image", "Save Image"];
    for (const label of labels) {
        const item = harness.document.createElement("div");
        item.setAttribute("role", "menuitem");
        const span = harness.document.createElement("span");
        span.textContent = label;
        item.appendChild(span);
        menu.appendChild(item);
    }
    const separator = harness.document.createElement("div");
    separator.setAttribute("role", "separator");
    menu.appendChild(separator);
    layer.appendChild(menu);
    for (const callback of harness.menuCallbacks) callback(menu);
    return { layer, menu };
}

function findItem(menu, text) {
    return menu.querySelectorAll('[role="menuitem"]').find((item) => item.textContent.includes(text)) ?? null;
}

function overlayOf(harness) {
    const overlays = [];
    const walk = (node) => {
        for (const child of node.children ?? []) {
            if (child instanceof FakeElement) {
                if ((child.getAttribute("class") ?? "").split(" ").includes("remcord-gal-overlay")) {
                    overlays.push(child);
                }
                walk(child);
            }
        }
    };
    walk(harness.document);
    return overlays[0] ?? null;
}

function popupButtons(overlay) {
    return overlay.querySelectorAll("button").map((button) => button.textContent);
}

const tick = () => new Promise((resolve) => setImmediate(resolve));

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

test("right-click image renders the Save to Gallery menu entry", () => {
    const harness = createHarness();
    harness.hooks.onLoad();
    rightClickImage(harness, "https://cdn.discordapp.com/attachments/1/2/photo.png");
    const { menu } = renderMenu(harness);
    const item = findItem(menu, "Save to Gallery");
    assert.ok(item, "the Save to Gallery entry must be injected");
});

test("clicking the Save entry opens the themed popup", () => {
    const harness = createHarness();
    harness.hooks.onLoad();
    rightClickImage(harness, "https://cdn.discordapp.com/attachments/1/2/photo.png");
    const { menu } = renderMenu(harness);
    const item = findItem(menu, "Save to Gallery");
    assert.ok(item);
    item.dispatchEvent(new FakeEvent("pointerdown", { bubbles: true }));
    const overlay = overlayOf(harness);
    assert.ok(overlay, "the popup overlay must attach");
    assert.equal(overlay.parentNode, harness.document.body, "the overlay must live on body");
    assert.equal(
        overlay.style.getPropertyValue("--background-primary"),
        "#313338",
        "the overlay must carry Discord's resolved theme variables",
    );
    assert.equal(
        overlay.style.getPropertyValue("--custom-accent"),
        "#ff73fa",
        "the overlay must carry custom-CSS variables too",
    );
    const buttons = popupButtons(overlay);
    for (const expected of ["Save to Gallery", "Copy Link", "Download", "Cancel"]) {
        assert.ok(buttons.includes(expected), `popup must offer ${expected}`);
    }
    assert.ok(overlay.querySelector("input"), "popup must have a name field");
    assert.ok(overlay.querySelector("select"), "popup must have a collection picker");
    assert.ok(overlay.querySelector("textarea"), "popup must have a note field");
});

test("popup follows the themed container when the menu is outside it", () => {
    // Production shape: menus portal outside `.theme-dark`, where variable
    // reads come back empty. The popup must still resolve the theme.
    const harness = createHarness();
    harness.hooks.onLoad();
    assert.ok(harness.document.querySelector(".theme-dark"), "the harness must provide a themed container");
    rightClickImage(harness, "https://cdn.discordapp.com/attachments/1/2/photo.png");
    const { menu } = renderMenu(harness);
    findItem(menu, "Save to Gallery").dispatchEvent(new FakeEvent("click", { bubbles: true }));
    const overlay = overlayOf(harness);
    assert.ok(overlay, "the popup must open");
    assert.equal(overlay.style.getPropertyValue("--background-primary"), "#313338");
    assert.equal(overlay.style.colorScheme, "dark");
});

test("popup derives its palette when no variables resolve anywhere", () => {
    // Hardest production shape: a custom theme styling surfaces directly
    // with no theme variables at all. The popup must still match the
    // rendered menu surface instead of falling back to white.
    const harness = createHarness();
    harness.hooks.onLoad();
    harness.document.querySelector(".theme-dark").remove();
    assert.equal(harness.document.querySelector(".theme-dark"), null);
    rightClickImage(harness, "https://cdn.discordapp.com/attachments/1/2/photo.png");
    const { menu } = renderMenu(harness);
    findItem(menu, "Save to Gallery").dispatchEvent(new FakeEvent("click", { bubbles: true }));
    const overlay = overlayOf(harness);
    assert.ok(overlay, "the popup must open");
    assert.equal(
        overlay.style.getPropertyValue("--background-primary"),
        "rgb(24, 25, 28)",
        "the overlay must carry the sampled menu surface, not a fallback",
    );
    assert.equal(overlay.style.colorScheme, "dark");
});

test("popup samples Discord's modern Oklab surface when legacy theme tokens are absent", () => {
    // Exact production shape observed in WebView2: Discord puts theme-dark
    // on <html>, exposes thousands of enumerable custom properties, but no
    // --background-primary; app-mount paints an opaque Oklab surface and the
    // clicked menu item paints its text in Oklab too. Before browser-native
    // CSS color parsing, sampleSurface returned null and the dialog was
    // white with low-contrast inputs.
    const harness = createHarness({ modernDiscordTheme: true });
    harness.hooks.onLoad();
    assert.ok(harness.document.querySelector(".theme-dark"));
    assert.ok(harness.document.getElementById("app-mount"));
    rightClickImage(harness, "https://cdn.discordapp.com/attachments/1/2/photo.png");
    const { menu } = renderMenu(harness);
    findItem(menu, "Save to Gallery").dispatchEvent(new FakeEvent("click", { bubbles: true }));
    const overlay = overlayOf(harness);
    assert.ok(overlay, "the popup must open for the live Discord theme shape");
    assert.equal(overlay.style.getPropertyValue("--background-primary"), "rgb(11, 17, 22)");
    assert.equal(overlay.style.getPropertyValue("--text-normal"), "rgb(120, 179, 245)");
    assert.equal(overlay.style.getPropertyValue("--background-secondary-alt"), "rgb(24, 36, 49)");
    assert.equal(overlay.style.getPropertyValue("--input-background-default"), "rgb(19, 28, 38)");
    assert.equal(overlay.style.getPropertyValue("--brand-500"), "#5865f2");
    assert.equal(
        Object.keys(overlay.style).filter((name) => name.startsWith("--")).length,
        524,
        "the 521 enumerated/backfilled variables must remain copied alongside three derived values",
    );
    assert.equal(overlay.style.colorScheme, "dark");
});

test("popup derives a light color scheme from the rendered Oklab surface", () => {
    const harness = createHarness({ modernLightTheme: true });
    harness.hooks.onLoad();
    rightClickImage(harness, "https://cdn.discordapp.com/attachments/1/2/photo.png");
    const { menu } = renderMenu(harness);
    findItem(menu, "Save to Gallery").dispatchEvent(new FakeEvent("click", { bubbles: true }));
    const overlay = overlayOf(harness);
    assert.ok(overlay);
    assert.equal(overlay.style.getPropertyValue("--background-primary"), "rgb(255, 255, 255)");
    assert.equal(overlay.style.getPropertyValue("--text-normal"), "rgb(0, 0, 0)");
    assert.equal(overlay.style.getPropertyValue("--input-background-default"), "rgb(237, 237, 237)");
    assert.equal(overlay.style.colorScheme, "light");
});

test("popup derives colors from raw custom CSS with no theme variables", () => {
    const harness = createHarness({ rawCustomCss: true });
    harness.hooks.onLoad();
    assert.equal(harness.document.querySelector(".theme-dark, .theme-light"), null);
    rightClickImage(harness, "https://cdn.discordapp.com/attachments/1/2/photo.png");
    const { menu } = renderMenu(harness);
    findItem(menu, "Save to Gallery").dispatchEvent(new FakeEvent("click", { bubbles: true }));
    const overlay = overlayOf(harness);
    assert.ok(overlay);
    assert.equal(overlay.style.getPropertyValue("--background-primary"), "rgb(11, 17, 22)");
    assert.equal(overlay.style.getPropertyValue("--text-normal"), "rgb(120, 179, 245)");
    assert.equal(overlay.style.colorScheme, "dark");
});

test("native Copy Link opens the popup instead of copying silently", () => {
    const harness = createHarness();
    harness.hooks.onLoad();
    rightClickImage(harness, "https://cdn.discordapp.com/attachments/1/2/photo.png");
    const { menu } = renderMenu(harness);
    const copy = findItem(menu, "Copy Link");
    assert.ok(copy);
    copy.dispatchEvent(new FakeEvent("pointerdown", { bubbles: true }));
    assert.ok(overlayOf(harness), "Copy Link must open the popup");
    assert.equal(harness.clipboard.length, 0, "nothing may copy before the user chooses");
});

test("native Copy Image opens the popup", () => {
    const harness = createHarness();
    harness.hooks.onLoad();
    rightClickImage(harness, "https://cdn.discordapp.com/attachments/1/2/photo.png");
    const { menu } = renderMenu(harness);
    const copyImage = findItem(menu, "Copy Image");
    assert.ok(copyImage);
    copyImage.dispatchEvent(new FakeEvent("click", { bubbles: true }));
    assert.ok(overlayOf(harness), "Copy Image must open the popup");
});

test("native Save Image opens the popup with Download", () => {
    const harness = createHarness();
    harness.hooks.onLoad();
    rightClickImage(harness, "https://cdn.discordapp.com/attachments/1/2/photo.png");
    const { menu } = renderMenu(harness);
    const saveImage = findItem(menu, "Save Image");
    assert.ok(saveImage);
    saveImage.dispatchEvent(new FakeEvent("pointerdown", { bubbles: true }));
    const overlay = overlayOf(harness);
    assert.ok(overlay, "Save Image must open the popup");
    assert.ok(popupButtons(overlay).includes("Download"), "media popup must offer Download");
});

test("popup DOM failure falls back to save, clipboard, and download actions", async () => {
    const saveHarness = createHarness();
    saveHarness.hooks.onLoad();
    rightClickImage(saveHarness, "https://cdn.discordapp.com/attachments/1/2/save.png");
    const saveMenu = renderMenu(saveHarness).menu;
    saveHarness.document.createElement = () => {
        throw new Error("simulated popup DOM failure");
    };
    findItem(saveMenu, "Save to Gallery").dispatchEvent(new FakeEvent("click", { bubbles: true }));
    await tick();
    assert.equal(saveHarness.addedUrls.length, 1, "Save must use the direct bridge fallback");

    const copyHarness = createHarness();
    copyHarness.hooks.onLoad();
    const copyUrl = "https://cdn.discordapp.com/attachments/1/2/copy.png";
    rightClickImage(copyHarness, copyUrl);
    const copyMenu = renderMenu(copyHarness).menu;
    copyHarness.document.createElement = () => {
        throw new Error("simulated popup DOM failure");
    };
    findItem(copyMenu, "Copy Link").dispatchEvent(new FakeEvent("click", { bubbles: true }));
    await tick();
    assert.deepEqual(copyHarness.clipboard, [copyUrl], "Copy must use the clipboard fallback");

    const downloadHarness = createHarness();
    downloadHarness.hooks.onLoad();
    const downloadUrl = "https://cdn.discordapp.com/attachments/1/2/download.png";
    rightClickImage(downloadHarness, downloadUrl);
    const downloadMenu = renderMenu(downloadHarness).menu;
    downloadHarness.document.createElement = () => {
        throw new Error("simulated popup DOM failure");
    };
    findItem(downloadMenu, "Save Image").dispatchEvent(new FakeEvent("click", { bubbles: true }));
    await tick();
    await tick();
    assert.deepEqual(downloadHarness.opened[0], [downloadUrl, "_blank", "noopener"]);
});

test("unrelated entries keep native behavior", () => {
    const harness = createHarness();
    harness.hooks.onLoad();
    rightClickImage(harness, "https://cdn.discordapp.com/attachments/1/2/photo.png");
    const { menu } = renderMenu(harness);
    const open = findItem(menu, "Open Link");
    assert.ok(open);
    open.dispatchEvent(new FakeEvent("pointerdown", { bubbles: true }));
    open.dispatchEvent(new FakeEvent("click", { bubbles: true }));
    assert.equal(overlayOf(harness), null, "Open Link must not open the popup");
});

test("Save in the popup stores name, collection, and note", async () => {
    const harness = createHarness();
    harness.hooks.onLoad();
    const url = "https://cdn.discordapp.com/attachments/1/2/photo.png";
    rightClickImage(harness, url);
    const { menu } = renderMenu(harness);
    findItem(menu, "Save to Gallery").dispatchEvent(new FakeEvent("click", { bubbles: true }));
    const overlay = overlayOf(harness);
    assert.ok(overlay);
    overlay.querySelector("input").value = "My photo";
    overlay.querySelector("textarea").value = "why I saved it";
    await tick();
    overlay.querySelector("select").value = "clips";
    popupButtons(overlay);
    const saveButton = overlay.querySelectorAll("button").find((b) => b.textContent === "Save to Gallery");
    saveButton.dispatchEvent(new FakeEvent("click", { bubbles: true }));
    await tick();
    await tick();
    assert.equal(harness.addedUrls.length, 1);
    assert.equal(harness.addedUrls[0].url, url);
    assert.equal(harness.addedUrls[0].meta.title, "My photo");
    assert.equal(harness.addedUrls[0].meta.note, "why I saved it");
    assert.deepEqual([...harness.addedUrls[0].meta.categories], ["clips"]);
    assert.ok(
        harness.toasts.some((toast) => toast.title === "Saved to Gallery"),
        "a confirmation toast must show",
    );
});

test("Copy in the popup copies the link", async () => {
    const harness = createHarness();
    harness.hooks.onLoad();
    const url = "https://cdn.discordapp.com/attachments/1/2/photo.png";
    rightClickImage(harness, url);
    const { menu } = renderMenu(harness);
    findItem(menu, "Copy Link").dispatchEvent(new FakeEvent("pointerdown", { bubbles: true }));
    const overlay = overlayOf(harness);
    assert.ok(overlay);
    overlay
        .querySelectorAll("button")
        .find((b) => b.textContent === "Copy Link")
        .dispatchEvent(new FakeEvent("click", { bubbles: true }));
    await tick();
    assert.deepEqual(harness.clipboard, [url]);
});

test("menu re-render keeps working (pending is not consumed by injection)", () => {
    const harness = createHarness();
    harness.hooks.onLoad();
    rightClickImage(harness, "https://cdn.discordapp.com/attachments/1/2/photo.png");
    renderMenu(harness);
    // Discord re-renders the menu: a brand-new node flows through the observer.
    const second = renderMenu(harness);
    const item = findItem(second.menu, "Save to Gallery");
    assert.ok(item, "the re-rendered menu must get the entry too");
    item.dispatchEvent(new FakeEvent("click", { bubbles: true }));
    assert.ok(overlayOf(harness), "the re-rendered entry must open the popup");
});

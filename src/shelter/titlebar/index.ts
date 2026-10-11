/// <reference path="../../../node_modules/@uwu/shelter-defs/dist/shelter-defs/rootdefs.d.ts" />
const {
    util: { log },
    flux: { dispatcher },
} = shelter;
const titlebarOverlayHTML = `<nav class="titlebar">
          <div class="window-title" id="window-title"></div>
        </nav>`;

const titlebarNavControls = `
          <div id="window-controls-container">
              <div id="spacer"></div>
              <div id="minimize"><div id="minimize-icon"></div></div>
              <div id="maximize"><div id="maximize-icon"></div></div>
              <div id="quit"><div id="quit-icon"></div></div>
          </div>
`;

const settings = window.remcord.settings.getConfig();

/** True under the native WebView2 shell, which draws its own frameless
 * chrome for every window style (the host window is always
 * `decorations(false)`). Electron keeps the style-branched behavior below. */
function isNativeShell(): boolean {
    return (window.remcord as unknown as { boot?: { runtime?: unknown } })?.boot?.runtime === "webview2";
}

/** Narrow native window operations available in both shells where present.
 * Every member is probed before use so the Electron path never touches the
 * native-only `drag` capability. */
function nativeWindowOps(): {
    minimize?: () => void;
    maximized?: () => boolean;
    isNormal?: () => boolean;
    maximize?: () => void;
    unmaximize?: () => void;
    hide?: () => void;
    quit?: () => void;
    drag?: () => void;
} {
    const w = (window.remcord as unknown as { window?: Record<string, unknown> }).window ?? {};
    const fn = (name: string) => (typeof w[name] === "function" ? (w[name] as () => void) : undefined);
    const boolFn = (name: string) => (typeof w[name] === "function" ? (w[name] as () => boolean) : undefined);
    return {
        minimize: fn("minimize"),
        maximized: boolFn("maximized"),
        isNormal: boolFn("isNormal"),
        maximize: fn("maximize"),
        unmaximize: fn("unmaximize"),
        hide: fn("hide"),
        quit: fn("quit"),
        drag: (window.remcord as unknown as { native?: { window?: { drag?: () => void } } }).native?.window?.drag,
    };
}

function injectButtonControls() {
    if (document.getElementById("remcordNavControls")) return;
    const elem = document.createElement("div");
    elem.innerHTML = titlebarNavControls;
    elem.id = "remcordNavControls";
    document.body.append(elem);
    const ops = nativeWindowOps();
    const container = document.getElementById("window-controls-container");
    const minimize = document.getElementById("minimize");
    const maximize = document.getElementById("maximize");
    const quit = document.getElementById("quit");

    minimize!.addEventListener("click", (event) => {
        // Buttons must never start a titlebar drag: stop the event at the
        // control so drag handles below ignore it.
        event.stopPropagation();
        event.preventDefault();
        (ops.minimize ?? window.remcord.window.minimize)();
    });

    maximize!.addEventListener("click", (event) => {
        event.stopPropagation();
        event.preventDefault();
        const maximized = ops.maximized ?? window.remcord.window.maximized;
        const isNormal = ops.isNormal ?? window.remcord.window.isNormal;
        if (maximized() === true) {
            (ops.unmaximize ?? window.remcord.window.unmaximize)();
            document.body.removeAttribute("isMaximized");
        } else if (isNormal() === true) {
            (ops.maximize ?? window.remcord.window.maximize)();
        }
    });
    const minimizeToTray = settings.minimizeToTray;
    quit!.addEventListener("click", (event) => {
        event.stopPropagation();
        event.preventDefault();
        if (minimizeToTray === true) {
            (ops.hide ?? window.remcord.window.hide)();
        } else if (minimizeToTray === false) {
            (ops.quit ?? window.remcord.window.quit)();
        }
    });

    // Frameless native chrome has no OS drag region (wry honors no
    // `-webkit-app-region`). Mousedown on the controls-container background
    // (spacer/empty area — never on the buttons above) starts a host drag
    // through the narrowly typed `window.drag` operation.
    if (typeof ops.drag === "function" && container) {
        container.addEventListener("mousedown", (event) => {
            const target = event.target as HTMLElement | null;
            if (target?.closest("#minimize,#maximize,#quit")) return;
            if (event.button !== 0) return;
            event.preventDefault();
            ops.drag!();
        });
    }
}

/**
 * Native-shell header label: the frameless window draws no caption of its
 * own, so Remcord signs Discord's existing top-bar title instead of taking
 * vertical space with a separate strip. The label is appended inside the
 * header title element (e.g. beside "Friends"), inheriting its alignment so
 * both read as one centered title. Returns the title element, or null when
 * no suitable header exists (Discord DOM drift): callers degrade to
 * controls-only rather than guessing.
 */
const REMCORD_HEADER_LABEL_ID = "remcordHeaderLabel";
let headerObserver: MutationObserver | null = null;
let labelRetryTimer: ReturnType<typeof setInterval> | null = null;

function stopLabelRetry() {
    if (labelRetryTimer !== null) {
        clearInterval(labelRetryTimer);
        labelRetryTimer = null;
    }
    (window as unknown as { __remcordHeaderLabelDone?: boolean }).__remcordHeaderLabelDone = true;
}

/**
 * Content-based fallback: walk the top band for the header title itself
 * (e.g. "Friends") instead of trusting obfuscated class names. Cheap
 * filters first (no layout), rect checks last; collects candidates and
 * keeps the most specific (shortest text) so a wrapping container never
 * beats the title it holds. Bounded walk, layout-free until the final
 * handful of survivors.
 */
function scanTopBarTitle(): HTMLElement | null {
    const skipTags = new Set(["SCRIPT", "STYLE", "SVG", "VIDEO", "AUDIO", "NOSCRIPT", "IFRAME", "CANVAS", "IMG"]);
    let walker: TreeWalker | null = null;
    try {
        walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
    } catch {
        return null;
    }
    const candidates: HTMLElement[] = [];
    let scanned = 0;
    let node: Node | null;
    while ((node = walker.nextNode())) {
        if (++scanned > 20000) break;
        const el = node as HTMLElement;
        if (skipTags.has(el.tagName)) continue;
        if (el.id === "remcordNavControls" || el.id === REMCORD_HEADER_LABEL_ID) continue;
        if (el.children.length > 20) continue;
        const text = (el.textContent ?? "").trim();
        if (text.length < 3 || text.length > 80) continue;
        if (el.querySelector("input,textarea,[contenteditable='true']")) continue;
        candidates.push(el);
        if (candidates.length >= 50) break;
    }
    let best: HTMLElement | null = null;
    let bestLen = 81;
    for (const el of candidates) {
        let rect: DOMRect;
        try {
            rect = el.getBoundingClientRect();
        } catch {
            continue;
        }
        if (rect.top < 0 || rect.top > 150 || rect.height <= 0 || rect.height > 120 || rect.width < 80) continue;
        const len = (el.textContent ?? "").trim().length;
        if (len < bestLen) {
            bestLen = len;
            best = el;
        }
    }
    return best;
}

function placeHeaderLabel(header: HTMLElement): HTMLElement {
    const label = document.createElement("span");
    label.id = REMCORD_HEADER_LABEL_ID;
    label.textContent = " \u2502 Remcord";
    label.style.cssText = "opacity:0.75;user-select:none;cursor:default;white-space:nowrap;";
    header.append(label);
    bindHeaderDrag(header);
    watchHeaderLabel(header);
    return header;
}
/** Attach the drag handle to a header element (once per node). */
function bindHeaderDrag(header: HTMLElement) {
    const ops = nativeWindowOps();
    if (typeof ops.drag !== "function" || header.dataset.remcordDragBound) return;
    header.dataset.remcordDragBound = "1";
    header.addEventListener("mousedown", (event) => {
        const target = event.target as HTMLElement | null;
        if (
            target?.closest(
                "#minimize,#maximize,#quit,a,button,[role='button'],input,textarea,[role='textbox'],[contenteditable='true']",
            )
        )
            return;
        if (event.button !== 0) return;
        event.preventDefault();
        ops.drag!();
    });
}

/** Watch the label's parent for childList swaps (React title updates wipe
 * unknown children): re-insert on the next mutation. Scoped to the header
 * node only — chat churn elsewhere never reaches this callback. */
function watchHeaderLabel(header: HTMLElement) {
    if (headerObserver) headerObserver.disconnect();
    headerObserver = new MutationObserver(() => {
        if (!document.getElementById(REMCORD_HEADER_LABEL_ID)) ensureHeaderLabel();
    });
    headerObserver.observe(header, { childList: true, subtree: false });
}

/**
 * Ensure the label exists, is bound, and is watched. Returns true when the
 * label is present afterwards. Re-queries Discord's header from scratch
 * whenever the span is missing, so a replaced header node is adopted
 * instead of mourned.
 */
function ensureHeaderLabel(): boolean {
    const live = document.getElementById(REMCORD_HEADER_LABEL_ID);
    if (live) {
        const parent = live.parentElement as HTMLElement | null;
        if (parent) {
            bindHeaderDrag(parent);
            watchHeaderLabel(parent);
        }
        return true;
    }
    // Discord obfuscates its classes: try the title slot first (exact
    // inline alignment beside e.g. "Friends"), then wider top-bar
    // containers as fallback (label still lands in the top bar).
    const selectors = ['div[class*="title"]', 'div[class*="toolbar"]', 'div[class*="header"]', "header"];
    for (const selector of selectors) {
        const titles = document.querySelectorAll(selector);
        for (const el of Array.from(titles)) {
            const header = el as HTMLElement;
            // The label is a sibling span, never an input child: containers
            // that happen to hold a search box elsewhere are still fine.
            if ((header.textContent ?? "").trim().length === 0) continue;
            // Top bar only: never sign a title halfway down the page.
            try {
                if (header.getBoundingClientRect().top > 150) continue;
            } catch {
                continue;
            }
            placeHeaderLabel(header);
            return true;
        }
    }
    // Class hooks missed (obfuscated drift): fall back to finding the title
    // by its content and position, then place there.
    const scanned = scanTopBarTitle();
    if (scanned) {
        placeHeaderLabel(scanned);
        return true;
    }
    return false;
}

/**
 * Native-shell titlebar assembly: working controls plus the inline header
 * label. No separate strip, no layout push, no extra vertical space: the
 * window chrome lives inside Discord's own top bar. Discord's header may
 * mount after the plugin starts, so a bounded retry (3s) covers late
 * mounts; React title swaps are healed by the header observer.
 */
function injectNativeTitlebar() {
    injectButtonControls();
    if (ensureHeaderLabel()) {
        stopLabelRetry();
        return;
    }
    // Each host (re)start gets its own retry budget: a previous pass may
    // have given up before Discord mounted its header.
    (window as unknown as { __remcordHeaderLabelDone?: boolean }).__remcordHeaderLabelDone = false;
    if (labelRetryTimer === null) {
        let attempts = 0;
        labelRetryTimer = setInterval(() => {
            attempts += 1;
            if (ensureHeaderLabel() || attempts >= 6) stopLabelRetry();
        }, 1000);
    }
}

function layerPush(payload: { type: string; component: string }) {
    console.log(payload.component);
    if (payload.component === "USER_SETTINGS") {
        const elem = document.createElement("div");
        elem.innerHTML = titlebarOverlayHTML;
        elem.id = "remcordTitlebar";
        document.body.prepend(elem);
    }
}

function layerPop() {
    console.log("pop!");
    document.getElementById("remcordTitlebar")?.remove();
}

export function onLoad() {
    log("Remcord Titlebar Controller");
    // The native shell is always frameless, independent of the
    // Electron-consumed `windowStyle` value: sign Discord's own top-bar
    // title and inject working controls, without taking any extra vertical
    // space. The overlay subscriptions below still apply so
    // settings-layer chrome keeps working.
    const native = isNativeShell();
    if (native) {
        document.body.setAttribute("customTitlebar", "");
        injectNativeTitlebar();
    }
    if (!native && settings.windowStyle === "default") {
        document.body.setAttribute("customTitlebar", "");
        injectButtonControls();
        return;
    }

    // Native + transparency on macOS uses the same overlay chrome as "overlay" (Remcord#1095).
    const overlayLike =
        settings.windowStyle === "overlay" ||
        (settings.windowStyle === "native" && window.remcord.platform === "darwin" && settings.transparency !== "none");

    if (overlayLike) {
        document.body.setAttribute("customTitlebar", "");
        dispatcher.subscribe("LAYER_PUSH", layerPush);
        dispatcher.subscribe("LAYER_POP", layerPop);
        return;
    }

    if (!native) {
        log("Unsupported window style");
    }
}

export function onUnload() {
    dispatcher.unsubscribe("LAYER_PUSH", layerPush);
    dispatcher.unsubscribe("LAYER_POP", layerPop);
    // The host stale-sweep also covers these, but an unload must not leave
    // dead controls (or their listeners) behind in either shell. Timers and
    // observers die here too — never a retry loop or a watcher left
    // running.
    if (labelRetryTimer !== null) {
        clearInterval(labelRetryTimer);
        labelRetryTimer = null;
    }
    if (headerObserver) {
        headerObserver.disconnect();
        headerObserver = null;
    }
    document.getElementById("remcordNavControls")?.remove();
    document.getElementById("remcordTitlebar")?.remove();
    document.getElementById(REMCORD_HEADER_LABEL_ID)?.remove();
}

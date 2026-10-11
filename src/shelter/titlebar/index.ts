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
 * Native-shell full titlebar: the frameless window has no OS caption, so
 * Remcord draws its own slim strip across the whole top edge (wordmark on
 * the left, window controls top-right). The strip is a drag handle
 * everywhere except on the buttons; Discord's own content is pushed below
 * it so nothing is covered and no Discord click is stolen.
 */
function injectNativeTitlebar() {
    if (!document.getElementById("remcordTitlebar")) {
        const nav = document.createElement("div");
        nav.innerHTML = titlebarOverlayHTML;
        nav.id = "remcordTitlebar";
        document.body.prepend(nav);
    }
    injectButtonControls();
    const strip = document.querySelector("#remcordTitlebar .titlebar") as HTMLElement | null;
    const appMount = document.getElementById("app-mount");
    if (strip && appMount) {
        // The stock strip is roomier than a native caption needs to be:
        // trim ~5px so it reads as a slim caption bar, then push Discord's
        // content below the trimmed height.
        const natural = strip.offsetHeight > 0 ? strip.offsetHeight : 30;
        const height = Math.max(20, natural - 5);
        strip.style.height = `${height}px`;
        const applied = strip.offsetHeight > 0 ? strip.offsetHeight : height;
        appMount.style.marginTop = `${applied}px`;
        // Never clobber an inline height Discord itself set.
        if (!appMount.style.height) {
            appMount.style.height = `calc(100vh - ${applied}px)`;
        }
    }
    const ops = nativeWindowOps();
    if (typeof ops.drag === "function" && strip && !strip.dataset.remcordDragBound) {
        strip.dataset.remcordDragBound = "1";
        strip.addEventListener("mousedown", (event) => {
            const target = event.target as HTMLElement | null;
            if (target?.closest("#minimize,#maximize,#quit")) return;
            if (event.button !== 0) return;
            event.preventDefault();
            ops.drag!();
        });
    }
}

function resetNativeTitlebarPush() {
    const appMount = document.getElementById("app-mount");
    appMount?.style.removeProperty("margin-top");
    // Only remove the compensation we added; leave Discord's own inline
    // height (if any) untouched.
    if (appMount?.style.height.startsWith("calc(100vh -")) {
        appMount.style.removeProperty("height");
    }
}

function layerPush(payload: { type: string; component: string }) {
    console.log(payload.component);
    if (payload.component === "USER_SETTINGS") {
        // The native shell already draws its own persistent strip: never
        // stack a second one when the settings layer opens.
        if (document.getElementById("remcordTitlebar")) return;
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
    // Electron-consumed `windowStyle` value: draw the full titlebar strip
    // (drag handle + working controls, Discord content pushed below) for
    // every style. The overlay subscriptions below still apply so
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
    // dead controls (or their listeners) behind in either shell. The native
    // content push is removed too so no gap is left behind.
    document.getElementById("remcordNavControls")?.remove();
    document.getElementById("remcordTitlebar")?.remove();
    resetNativeTitlebarPush();
}

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
        // control so the container drag handle below ignores it.
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
    // Electron-consumed `windowStyle` value: inject working controls (and the
    // drag handle) for every style. The overlay subscriptions below still
    // apply so settings-layer chrome keeps working.
    const native = isNativeShell();
    if (native) {
        document.body.setAttribute("customTitlebar", "");
        injectButtonControls();
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
    // dead controls (or their listeners) behind in either shell.
    document.getElementById("remcordNavControls")?.remove();
    document.getElementById("remcordTitlebar")?.remove();
}

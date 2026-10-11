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

function injectButtonControls() {
    const elem = document.createElement("div");
    elem.innerHTML = titlebarNavControls;
    elem.id = "remcordNavControls";
    document.body.append(elem);
    const minimize = document.getElementById("minimize");
    const maximize = document.getElementById("maximize");
    const quit = document.getElementById("quit");

    minimize!.addEventListener("click", () => {
        window.remcord.window.minimize();
    });

    maximize!.addEventListener("click", () => {
        if (window.remcord.window.maximized() === true) {
            window.remcord.window.unmaximize();
            document.body.removeAttribute("isMaximized");
        } else if (window.remcord.window.isNormal() === true) {
            window.remcord.window.maximize();
        }
    });
    const minimizeToTray = settings.minimizeToTray;
    quit!.addEventListener("click", () => {
        if (minimizeToTray === true) {
            window.remcord.window.hide();
        } else if (minimizeToTray === false) {
            window.remcord.window.quit();
        }
    });
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
    if (settings.windowStyle === "default") {
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

    log("Unsupported window style");
}

export function onUnload() {
    dispatcher.unsubscribe("LAYER_PUSH", layerPush);
    dispatcher.unsubscribe("LAYER_POP", layerPop);
}

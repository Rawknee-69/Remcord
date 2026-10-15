import { join } from "node:path";
import { app, Menu, nativeImage, Tray } from "electron";
import { getConfig } from "../common/config.js";
import { navigateTo } from "../common/dom.js";
import { setForceQuit } from "../common/forceQuit.js";
import { getLang } from "../common/lang.js";
import { getDisplayVersion } from "../common/version.js";
import { revealWindow } from "../common/windowVisibility.js";
import { handleRestart } from "../main.js";
import { mainWindows } from "./window.js";
export let tray: Tray;

export function createTray() {
    let trayIcon = getConfig("tray");
    // Normalize missing/unknown values to the default instead of loading a
    // missing file (which yields an empty tray image).
    if (typeof trayIcon !== "string" || trayIcon.length === 0) {
        trayIcon = "dynamic";
    }
    if (trayIcon === "disabled") {
        return;
    }
    if (trayIcon === "dynamic") {
        trayIcon = "rem-smile";
    }
    // Backward compat: pre-rename configs may still name a removed
    // ac_plug/dsc-tray asset — fall back to the default Rem icon instead
    // of loading a missing file (which yields an empty tray image).
    const knownIcons = new Set([
        "rem-blush",
        "rem-glasses",
        "rem-happy",
        "rem-smile",
        "rem-sparkler",
        "rem-sky",
        "rem-school",
        "rem-upside",
    ]);
    if (!knownIcons.has(trayIcon)) {
        trayIcon = "rem-smile";
    }
    let trayImg = nativeImage.createFromPath(join(import.meta.dirname, "../", `/assets/${trayIcon}.png`));
    switch (process.platform) {
        case "win32":
            trayImg = trayImg.resize({ height: 16 });
            break;
        case "darwin":
            trayImg = trayImg.resize({ height: 18 });
            break;
        case "linux":
            trayImg = trayImg.resize({ height: 24 });
            break;
    }

    tray = new Tray(trayImg);
    const contextMenu = Menu.buildFromTemplate([
        {
            label: `${getLang("menu-remcord")} ${getDisplayVersion()}`,
            icon: trayImg,
            enabled: false,
        },
        {
            type: "separator",
        },
        {
            label: getLang("tray-openRemcord"),
            click() {
                mainWindows.forEach((mainWindow) => {
                    revealWindow(mainWindow);
                });
            },
        },
        {
            label: getLang("tray-openSettings"),
            click() {
                mainWindows.forEach((mainWindow) => {
                    revealWindow(mainWindow);
                });
            },
        },
        {
            label: getLang("tray-supportServer"),
            click() {
                mainWindows.forEach((mainWindow) => {
                    navigateTo(mainWindow, "/invite/TnhxcqynZ2");
                });
            },
        },
        {
            type: "separator",
        },
        {
            label: getLang("tray-restartRemcord"),
            click() {
                handleRestart();
            },
        },
        {
            label: getLang("tray-quitRemcord"),
            click() {
                setForceQuit(true);
                app.quit();
            },
        },
    ]);
    tray.setContextMenu(contextMenu);

    tray.setToolTip(getLang("tray-tooltip"));
    tray.on("click", () => {
        mainWindows.forEach((mainWindow) => {
            revealWindow(mainWindow);
        });
    });
}

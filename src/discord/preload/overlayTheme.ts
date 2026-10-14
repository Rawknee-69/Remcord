import { cssColorToHex, pickOverlayColor, pickOverlaySymbolColor } from "../../common/overlayColors.js";

const { ipcRenderer } = require("electron");

/**
 * Keeps the window chrome (minimize / maximize / close) painted with the
 * live Discord theme instead of stale construction-time values.
 *
 * Two chrome paths exist, and both are covered:
 * - `overlay` style: Chromium draws native caption buttons from the colors
 *   passed to `setTitleBarOverlay`. Those are re-pushed over IPC whenever
 *   the sampled theme colors change.
 * - `default` / `legacy` styles: Remcord's own CSS controls resolve their
 *   icon color from `--remcord-chrome-icon`, which is set here on
 *   `<html>` (falling back to `--interactive-icon-default` when unset).
 *
 * Sampling beats naming a variable because themes are free to restyle the
 * top toolbar itself: the color is read off one of Discord's own toolbar
 * icons, so custom controls always match what Discord renders next to them.
 * Discord themes — light mode, custom themes, Quick CSS — are picked up
 * through the same observers.
 */

const CHROME_ICON_VAR = "--remcord-chrome-icon";
const FALLBACK_BG = "#121214";
const FALLBACK_ICON = "#99aab5";

let lastSent = "";
let lastVar = "";

function rootVar(name: string): string {
    try {
        return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    } catch {
        return "";
    }
}

/** Background behind the native overlay buttons: walk up from the top-right corner until opaque. */
function sampleTopBarBackground(): string | null {
    try {
        const el = document.elementFromPoint(window.innerWidth - 100, 10) as Element | null;
        let node: Element | null = el;
        let depth = 0;
        while (node && depth < 12) {
            const hex = cssColorToHex(getComputedStyle(node).backgroundColor);
            if (hex) return hex;
            node = node.parentElement;
            depth++;
        }
    } catch {
        /* fall through to variables */
    }
    return null;
}

/** Color of Discord's own top-toolbar icons (the icons our buttons sit next to). */
function sampleToolbarIconColor(): string | null {
    try {
        const probe = document.elementFromPoint(Math.max(0, window.innerWidth - 260), 14) as Element | null;
        const scope = probe?.closest?.('[class*="toolbar"], [class*="trailing"], header, nav') ?? undefined;
        const svg = scope?.querySelector?.("svg") ?? document.querySelector("header svg, nav svg");
        if (!svg) return null;
        return cssColorToHex(getComputedStyle(svg).color);
    } catch {
        return null;
    }
}

function currentColors(): { color: string; symbolColor: string } {
    const sampledIcon = sampleToolbarIconColor();
    const color = pickOverlayColor([
        sampleTopBarBackground(),
        rootVar("--background-base-lowest"),
        rootVar("--bg-base-tertiary"),
        rootVar("--background-tertiary"),
        FALLBACK_BG,
    ]);
    const symbolColor = pickOverlaySymbolColor([
        sampledIcon,
        rootVar("--interactive-icon-default"),
        rootVar("--text-muted"),
        FALLBACK_ICON,
    ]);
    return { color, symbolColor };
}

function windowStyle(): string {
    try {
        return ipcRenderer.sendSync("getConfig", "windowStyle") as string;
    } catch {
        return "";
    }
}

function pushOverlayColors(color: string, symbolColor: string): void {
    try {
        const key = `${color}|${symbolColor}`;
        if (key === lastSent) return;
        lastSent = key;
        ipcRenderer.send("win-setOverlayColors", color, symbolColor);
    } catch {
        /* never break page input */
    }
}

function setChromeIconVar(symbolColor: string): void {
    try {
        if (symbolColor === lastVar) return;
        lastVar = symbolColor;
        document.documentElement.style.setProperty(CHROME_ICON_VAR, symbolColor);
    } catch {
        /* never break page input */
    }
}

function sync(): void {
    const { color, symbolColor } = currentColors();
    if (windowStyle() === "overlay") {
        pushOverlayColors(color, symbolColor);
    } else {
        // Custom CSS controls read this var (with theme-var fallback).
        setChromeIconVar(symbolColor);
    }
}

function start(): void {
    sync();
    // Discord applies its theme (and Remcord injects themes/QuickCSS) after
    // load; re-sample late so the first paint already matches.
    let timer: ReturnType<typeof setTimeout> | null = null;
    const schedule = (delay = 250) => {
        if (timer) clearTimeout(timer);
        timer = setTimeout(() => {
            timer = null;
            sync();
        }, delay);
    };
    schedule(1500);
    schedule(4000);
    try {
        const observer = new MutationObserver(() => schedule());
        // Theme switches flip classes/variables on <html>/<body>; theme
        // installs add/remove <style> nodes in <head>.
        observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "style"] });
        if (document.head) observer.observe(document.head, { childList: true });
        if (document.body) observer.observe(document.body, { attributes: true, attributeFilter: ["class", "style"] });
    } catch {
        /* one-shot sync above still holds */
    }
}

if (windowStyle() === "overlay" || windowStyle() === "default" || windowStyle() === "legacy") {
    if (document.readyState === "complete" || document.readyState === "interactive") {
        setTimeout(start, 300);
    } else {
        window.addEventListener("DOMContentLoaded", () => setTimeout(start, 300), { once: true });
    }
}

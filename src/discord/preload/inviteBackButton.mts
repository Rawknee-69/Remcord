import { addScript, addStyle } from "../../common/dom.js";

const { ipcRenderer } = require("electron");

/** True when running under the native WebView2 shell (Phase C-2+). */
function isNativeWebView(): boolean {
    return (window as unknown as { remcord?: { boot?: { runtime?: unknown } } })?.remcord?.boot?.runtime === "webview2";
}

/**
 * Invite button label lookup. Electron uses the raw async `getLang` invoke
 * (unchanged); native WebView uses the typed async `localization.get_lang`
 * capability at the same public location. Either way a missing key resolves
 * to `""` and the caller falls back to English below.
 */
async function getBackLabel(): Promise<string> {
    if (isNativeWebView()) {
        return await (window as unknown as { remcord: { getLang: (key: string) => Promise<string> } }).remcord.getLang(
            "invite-goBackToApp",
        );
    }
    return (await ipcRenderer.invoke("getLang", "invite-goBackToApp")) as string;
}

document.addEventListener("DOMContentLoaded", () => {
    void (async () => {
        const label = (await getBackLabel()) || "Go back to the Discord app";

        addStyle("remcord://assets/css/inviteBack.css");

        // Injected into the page world so Discord SPA history.pushState/replaceState is visible.
        addScript(`
(() => {
    const BTN_ID = "remcord-invite-back";
    const LABEL = ${JSON.stringify(label)};

    function isInvitePath(pathname) {
        const path = (pathname || "").toLowerCase();
        return path.startsWith("/invite/") || path.startsWith("/guest-invite/");
    }

    function ensureButton() {
        let btn = document.getElementById(BTN_ID);
        if (btn) return btn;
        btn = document.createElement("button");
        btn.id = BTN_ID;
        btn.type = "button";
        btn.textContent = LABEL;
        btn.setAttribute("aria-label", LABEL);
        btn.addEventListener("click", (event) => {
            event.preventDefault();
            event.stopPropagation();
            try {
                history.pushState({}, "", "/app");
                window.dispatchEvent(new PopStateEvent("popstate", { state: {} }));
            } catch (_) {
                location.assign("/app");
            }
        });
        (document.body || document.documentElement).appendChild(btn);
        return btn;
    }

    function sync() {
        const btn = ensureButton();
        const onInvite = isInvitePath(location.pathname);
        btn.setAttribute("data-visible", onInvite ? "true" : "false");
        btn.hidden = !onInvite;
        btn.setAttribute("aria-hidden", onInvite ? "false" : "true");
    }

    function patchHistory(method) {
        const original = history[method];
        if (typeof original !== "function") return;
        history[method] = function () {
            const result = original.apply(this, arguments);
            queueMicrotask(sync);
            return result;
        };
    }

    patchHistory("pushState");
    patchHistory("replaceState");
    window.addEventListener("popstate", sync);
    window.addEventListener("hashchange", sync);

    sync();
})();
`);
    })();
});

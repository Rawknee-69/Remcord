(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic document-start script.
    "use strict";

    // Windows overlay/badge icons (F20 core): Electron derives the taskbar
    // overlay from the `page-title-updated` event (`•` unread dot, `(N)`
    // mentions, else clear — `src/discord/window.ts`). WebView2 exposes no
    // title-changed event to the host, so this document-start script
    // observes `document.title` and reports it as one-way `badge_title`
    // events carrying only the bounded title text. The host parses
    // (`native/src/badge.rs`), applies the overlay through
    // `ITaskbarList3::SetOverlayIcon`, and tracks state for
    // `badge.get_state`. The observer never touches the title, the DOM, or
    // navigation — it only reads and reports.
    //
    // Entry origins for the three release channels (stable/canary/ptb).
    // Exact match only — no subdomains, no other hosts.
    if (
        window.location.origin !== "https://discord.com" &&
        window.location.origin !== "https://canary.discord.com" &&
        window.location.origin !== "https://ptb.discord.com"
    )
        return;
    if (Object.hasOwn(window, "__remcordNativeBadgeObserver")) return;
    Object.defineProperty(window, "__remcordNativeBadgeObserver", {
        configurable: false,
        enumerable: false,
        value: true,
    });

    // Top document only: initialization scripts also run in subframes
    // (embeds, popouts), whose titles must never drive the taskbar badge.
    // The comparison itself never touches cross-origin properties.
    if (window.self !== window.top) return;

    // The host rejects titles past its own bound; slice shorter here so a
    // legitimate long channel name still reports instead of rejecting.
    const MAX_TITLE_CHARS = 200;

    function currentTitle() {
        try {
            const title = document.title;
            if (typeof title !== "string") return null;
            return title.slice(0, MAX_TITLE_CHARS);
        } catch {
            return null;
        }
    }

    let lastReported = null;

    function report(title) {
        if (title === null || title === lastReported) return;
        lastReported = title;
        try {
            window.ipc.postMessage(JSON.stringify({ version: 1, event: { type: "badge_title", payload: { title } } }));
        } catch (error) {
            // A failed post must not wedge reporting: allow a retry on the
            // next title change.
            lastReported = null;
            console.error("[Badge native][adapter_initialization] could not report title", error);
        }
    }

    function poll() {
        report(currentTitle());
    }

    try {
        // The host advertises badge support through the get_state
        // capability; without it (an older host) stay silent instead of
        // posting events the bridge would refuse.
        const native = window.remcord?.native;
        if (!native?.supports?.("badge.get_state")) return;
        poll();
        // `document` (not `documentElement`) is observed: document-start
        // runs before `<html>` exists, and title sets mutate the `<title>`
        // text node later — both surface as mutations under `document`.
        if (typeof MutationObserver !== "undefined") {
            const observer = new MutationObserver(() => {
                try {
                    poll();
                } catch {
                    // Never break page scripting.
                }
            });
            observer.observe(document, { childList: true, characterData: true, subtree: true });
        }
    } catch {
        // Never break page load.
    }
})();

(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic script, not a module.
    "use strict";

    // F13 page-side badge helper (Electron `page-title-updated` parity,
    // `src/discord/window.ts:260-287`). Pure title -> badge mapping for the
    // future F20 overlay slice: NO bridge, NO fetch, NO storage, NO eval.
    // Not wired into the document-start pipeline yet (wiring ships with the
    // OS tray + overlay icons); shipped now so the mapping is pinned and
    // tested on both sides of the boundary (see `native/src/tray.rs`).
    if (
        window.location.origin !== "https://discord.com" &&
        window.location.origin !== "https://canary.discord.com" &&
        window.location.origin !== "https://ptb.discord.com"
    ) {
        return;
    }

    const MAX_TITLE_BYTES = 512;
    const MAX_MENTION_BADGE_INDEX = 9;

    function truncateToBytes(value, maxBytes) {
        const text = String(value);
        const encoded = new TextEncoder().encode(text);
        if (encoded.length <= maxBytes) return text;
        let end = maxBytes;
        while (end > 0 && (encoded[end] & 0xc0) === 0x80) end -= 1;
        return new TextDecoder().decode(encoded.slice(0, end));
    }

    // Closed mapping: { kind: "none" } clears, { kind: "unread" } is
    // badge-11, { kind: "mentions", count } is badge-N capped at badge-10.
    // Titles are the only input and never leave this function as text.
    function badgeForTitle(title) {
        const bounded = truncateToBytes(title, MAX_TITLE_BYTES);
        if (bounded.startsWith("•")) return { kind: "unread" };
        if (bounded.startsWith("(")) {
            const match = /^[(](\d+)/.exec(bounded);
            if (match) {
                const count = Number.parseInt(match[1], 10);
                if (Number.isSafeInteger(count)) return { kind: "mentions", count };
            }
            return { kind: "unread" };
        }
        return { kind: "none" };
    }

    function badgeAsset(badge) {
        if (!badge || badge.kind === "none") return null;
        if (badge.kind === "unread") return "badge-11.ico";
        if (badge.kind === "mentions") {
            const count = badge.count;
            if (!Number.isSafeInteger(count) || count > MAX_MENTION_BADGE_INDEX) return "badge-10.ico";
            return `badge-${count}.ico`;
        }
        return null;
    }

    try {
        Object.defineProperty(window, "__remcordTrayBadge", {
            configurable: false,
            enumerable: false,
            value: Object.freeze({ installed: true, badgeForTitle, badgeAsset }),
        });
    } catch (error) {
        console.error("[Remcord][TrayBadge] status publish failed", error);
    }
})();

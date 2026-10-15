(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic document-start script.
    "use strict";

    // Non-global keybind matcher (F8 core): Electron fires enabled +
    // non-global keybinds through app-menu accelerators (focused-app-only).
    // Native has no menus (F15), so this document-start script matches
    // `keydown` events against the enabled non-global inventory served by
    // the `keybinds.list` capability and reports matches as one-way
    // `keybind_matched` events carrying only the entry id. The host
    // re-validates (known, enabled, non-global) and executes the stored
    // action itself; a match for a global id is refused host-side.
    //
    // Entry origins for the three release channels (stable/canary/ptb).
    // Exact match only — no subdomains, no other hosts.
    if (
        window.location.origin !== "https://discord.com" &&
        window.location.origin !== "https://canary.discord.com" &&
        window.location.origin !== "https://ptb.discord.com"
    )
        return;
    if (Object.hasOwn(window, "__remcordNativeKeybindMatcher")) return;
    Object.defineProperty(window, "__remcordNativeKeybindMatcher", {
        configurable: false,
        enumerable: false,
        value: true,
    });

    // Modifier canonicalization (Electron Accelerator parity): several
    // spellings map to one slot. Order-insensitive, duplicates refused.
    function parseModifiers(tokens) {
        let mods = 0;
        for (const token of tokens) {
            const lower = token.toLowerCase();
            let flag = 0;
            if (["control", "ctrl", "commandorcontrol", "cmdorctrl", "command", "cmd"].includes(lower)) flag = 1;
            else if (["alt", "option", "altgr"].includes(lower)) flag = 2;
            else if (lower === "shift") flag = 4;
            else if (["super", "meta", "win", "windows"].includes(lower)) flag = 8;
            else return null;
            if (mods & flag) return null;
            mods |= flag;
        }
        return mods;
    }

    // Non-modifier canonicalization. The maker records raw `event.key`
    // values (release-order noise included), so both spellings of every
    // named key parse to one canonical token; single characters compare
    // uppercased. Returns null for modifiers, numpad, and unknowns.
    function canonicalKey(token) {
        if (typeof token !== "string" || token.length === 0) return null;
        if (token.length === 1) {
            const upper = token.toUpperCase();
            if (/^[A-Z0-9]$/.test(upper)) return upper;
            if (token === " ") return "Space";
            return null;
        }
        const upper = token.toUpperCase();
        if (upper === "SPACE") return "Space";
        if (upper === "TAB") return "Tab";
        if (upper === "ENTER" || upper === "RETURN") return "Enter";
        if (upper === "ESC" || upper === "ESCAPE") return "Escape";
        if (upper === "BACKSPACE") return "Backspace";
        if (upper === "DELETE" || upper === "DEL") return "Delete";
        if (upper === "INSERT" || upper === "INS") return "Insert";
        if (upper === "HOME") return "Home";
        if (upper === "END") return "End";
        if (upper === "PAGEUP" || upper === "PGUP") return "PageUp";
        if (upper === "PAGEDOWN" || upper === "PGDN") return "PageDown";
        if (upper === "UP" || upper === "ARROWUP") return "Up";
        if (upper === "DOWN" || upper === "ARROWDOWN") return "Down";
        if (upper === "LEFT" || upper === "ARROWLEFT") return "Left";
        if (upper === "RIGHT" || upper === "ARROWRIGHT") return "Right";
        if (upper === "CAPSLOCK") return "CapsLock";
        if (upper === "NUMLOCK") return "NumLock";
        if (upper === "SCROLLLOCK" || upper === "SCRLOCK") return "ScrollLock";
        if (upper === "PRINTSCREEN" || upper === "PRTSC" || upper === "SNAPSHOT") return "PrintScreen";
        if (upper === "PAUSE" || upper === "BREAK") return "Pause";
        if (upper === "MEDIANEXTTRACK") return "MediaNextTrack";
        if (upper === "MEDIAPREVIOUSTRACK") return "MediaPreviousTrack";
        if (upper === "MEDIASTOP") return "MediaStop";
        if (upper === "MEDIAPLAYPAUSE") return "MediaPlayPause";
        if (upper === "VOLUMEMUTE") return "VolumeMute";
        if (upper === "VOLUMEDOWN") return "VolumeDown";
        if (upper === "VOLUMEUP") return "VolumeUp";
        const fkey = /^F([1-9]|1[0-9]|2[0-4])$/.exec(upper);
        if (fkey) return `F${Number.parseInt(fkey[1], 10)}`;
        return null;
    }

    // Parses one stored accelerator into "mods:key" (or null when the
    // stored string is not executable — hand-edited entries skip loudly).
    function parseAccelerator(accelerator) {
        if (typeof accelerator !== "string" || accelerator.length === 0 || accelerator.length > 32) return null;
        const tokens = accelerator.split("+");
        if (tokens.length > 4 || tokens.some((token) => token.length === 0)) return null;
        const keyTokens = tokens.filter((token) => canonicalKey(token) !== null);
        // Exactly one non-modifier key; every other token must be a known
        // modifier (a token that is neither fails the whole entry).
        if (keyTokens.length !== 1) return null;
        const modTokens = tokens.filter((token) => canonicalKey(token) === null);
        const mods = parseModifiers(modTokens);
        if (mods === null) return null;
        return `${mods}:${canonicalKey(keyTokens[0])}`;
    }

    // Canonical form of one live keydown event for comparison.
    function eventForm(event) {
        let mods = 0;
        if (event.ctrlKey) mods |= 1;
        if (event.altKey) mods |= 2;
        if (event.shiftKey) mods |= 4;
        if (event.metaKey) mods |= 8;
        const key = canonicalKey(event.key);
        if (key === null) return null;
        return `${mods}:${key}`;
    }

    let inventory = [];
    let lastFetch = 0;
    const REFRESH_MS = 30_000;

    async function refreshInventory() {
        try {
            const native = window.remcord?.native;
            if (!native?.supports("keybinds.list")) return;
            native.requireCapability("keybinds.list");
            const entries = await window.remcord.settings.listKeybinds();
            if (!Array.isArray(entries)) return;
            inventory = entries.filter(
                (entry) =>
                    entry &&
                    typeof entry.id === "string" &&
                    typeof entry.accelerator === "string" &&
                    entry.enabled === true &&
                    entry.global === false,
            );
            lastFetch = Date.now();
        } catch {
            // A failed refresh keeps the previous inventory: matches stay
            // available, never silently unbound.
        }
    }

    function reportMatch(id) {
        try {
            window.ipc.postMessage(JSON.stringify({ version: 1, event: { type: "keybind_matched", payload: { id } } }));
        } catch (error) {
            console.error("[Keybinds native][adapter_initialization] could not report match", error);
        }
    }

    window.addEventListener(
        "keydown",
        (event) => {
            try {
                if (event.defaultPrevented) return;
                if (Date.now() - lastFetch > REFRESH_MS) void refreshInventory();
                const form = eventForm(event);
                if (form === null || inventory.length === 0) return;
                for (const entry of inventory) {
                    if (parseAccelerator(entry.accelerator) === form) {
                        // Menu-accelerator parity: the match shadows the
                        // page, so Discord's own binding for the same chord
                        // does not also fire.
                        event.preventDefault();
                        event.stopPropagation();
                        reportMatch(entry.id);
                        return;
                    }
                }
            } catch {
                // Never break page input.
            }
        },
        true,
    );

    void refreshInventory();
})();

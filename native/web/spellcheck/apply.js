(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic script, not a module.
    "use strict";
    // F18 spellcheck enforcement (Electron `spellcheck` webPreference +
    // `session.setSpellCheckerLanguages` parity). The ONLY host-supplied
    // values are the two JSON literals below (closed boolean + one BCP-47
    // tag, spliced host-side by `native/src/spellcheck.rs`). No bridge,
    // fetch, storage, URL, path, or eval surface: the renderer supplies
    // nothing.
    const spellcheckEnabled = __REMCORD_SPELLCHECK_ENABLED__;
    const spellcheckLang = __REMCORD_SPELLCHECK_LANG__;
    const ENTRY_ORIGINS = ["https://discord.com", "https://canary.discord.com", "https://ptb.discord.com"];
    const publishStatus = () => {
        try {
            Object.defineProperty(window, "__remcordSpellcheck", {
                configurable: false,
                enumerable: false,
                value: Object.freeze({
                    installed: true,
                    enabled: spellcheckEnabled === true,
                    lang: typeof spellcheckLang === "string" ? spellcheckLang : "en-US",
                }),
            });
        } catch (error) {
            console.error("[Remcord][Spellcheck] status publish failed", error);
        }
    };
    publishStatus();
    if (ENTRY_ORIGINS.indexOf(window.location.origin) === -1) return;
    const registryKey = Symbol.for("remcord.spellcheck.apply");
    try {
        if (globalThis[registryKey]?.installed) return;
    } catch {}
    const markInstalled = () => {
        try {
            globalThis[registryKey] = { installed: true };
        } catch {}
    };
    const enabled = spellcheckEnabled === true;
    const lang = typeof spellcheckLang === "string" && spellcheckLang ? spellcheckLang : "en-US";
    const applyTo = (root) => {
        try {
            if (root?.documentElement && enabled) {
                if (!root.documentElement.hasAttribute("lang")) {
                    root.documentElement.setAttribute("lang", lang);
                }
            }
            const scope = root?.querySelectorAll ? root : document;
            const editables = scope.querySelectorAll
                ? scope.querySelectorAll(
                      'input[type="text"], input[type="search"], input:not([type]), textarea, [contenteditable="true"], [contenteditable=""]',
                  )
                : [];
            for (const node of editables) {
                try {
                    node.setAttribute("spellcheck", enabled ? "true" : "false");
                    if (enabled && !node.hasAttribute("lang")) {
                        node.setAttribute("lang", lang);
                    }
                } catch {}
            }
        } catch {}
    };
    markInstalled();
    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", () => applyTo(document), { once: true });
    } else {
        applyTo(document);
    }
    try {
        const observer = new MutationObserver((mutations) => {
            for (const mutation of mutations) {
                for (const node of mutation.addedNodes || []) {
                    if (node && node.nodeType === 1) {
                        if (node.matches?.("input, textarea, [contenteditable]")) {
                            applyTo(node.ownerDocument || document);
                        } else if (node.querySelectorAll) {
                            const hits = node.querySelectorAll("input, textarea, [contenteditable]");
                            if (hits?.length) applyTo(node.ownerDocument || document);
                        }
                    }
                }
            }
        });
        observer.observe(document.documentElement || document, { childList: true, subtree: true });
    } catch {}
})();

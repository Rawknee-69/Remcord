(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic document-start script.
    "use strict";

    // Entry origins for the three release channels (stable/canary/ptb).
    // Exact match only — no subdomains, no other hosts.
    if (
        window.location.origin !== "https://discord.com" &&
        window.location.origin !== "https://canary.discord.com" &&
        window.location.origin !== "https://ptb.discord.com"
    )
        return;
    if (Object.hasOwn(window, "__remcordNativePluginLoader")) return;

    const REQUEST_TIMEOUT_MS = 120_000;
    const TARGETS = ["preload", "renderer"];

    function reportResult(pluginId, target, ok, error) {
        try {
            window.ipc.postMessage(
                JSON.stringify({
                    version: 1,
                    event: {
                        type: "plugin_result",
                        payload: {
                            plugin_id: pluginId,
                            target,
                            ok,
                            error: ok ? null : String(error?.message ?? error ?? "unknown").slice(0, 2048),
                        },
                    },
                }),
            );
        } catch (reportError) {
            console.error(`[Native plugins][${pluginId}] could not report execution result`, reportError);
        }
    }

    // Minimal page-world method patcher (one uniform shape for both
    // targets). Electron gives preload entries spitroast
    // (`before/after/instead(parent, key, fn)`) and renderer entries a
    // different `(name, parent, callback)` proxy patcher; neither exists
    // natively, so the loader defines this documented subset instead:
    // - before(parent, key, fn): fn(args) runs first, original return kept.
    // - after(parent, key, fn): fn(args, returnValue) runs after; a
    //   non-undefined return replaces the value.
    // - instead(parent, key, fn): fn(args, originalBound) fully controls it.
    // Every patch targets an existing function property and returns an
    // idempotent unpatch, which is also registered as a cleanup so a
    // re-executed plugin never stacks wrappers.
    function createPatcher(label, addCleanup) {
        const wrap = (parent, method, kind, fn) => {
            if ((typeof parent !== "object" && typeof parent !== "function") || parent === null) {
                throw new TypeError(`[Native plugins][${label}] patch parent must be an object`);
            }
            if (typeof method !== "string" && typeof method !== "symbol") {
                throw new TypeError(`[Native plugins][${label}] patch key must be a string or symbol`);
            }
            const original = parent[method];
            if (typeof original !== "function") {
                throw new TypeError(`[Native plugins][${label}] can only patch an existing function`);
            }
            if (typeof fn !== "function") {
                throw new TypeError(`[Native plugins][${label}] patch callback must be a function`);
            }
            let wrapped;
            if (kind === "before") {
                wrapped = function (...args) {
                    fn.apply(this, args);
                    return original.apply(this, args);
                };
            } else if (kind === "after") {
                wrapped = function (...args) {
                    const returned = original.apply(this, args);
                    const replacement = fn.call(this, args, returned);
                    return replacement === undefined ? returned : replacement;
                };
            } else {
                wrapped = function (...args) {
                    return fn.call(this, args, original.bind(this));
                };
            }
            parent[method] = wrapped;
            let active = true;
            const unpatch = () => {
                if (!active) return;
                active = false;
                if (parent[method] === wrapped) parent[method] = original;
            };
            addCleanup(unpatch);
            return unpatch;
        };
        return {
            before: (parent, method, fn) => wrap(parent, method, "before", fn),
            after: (parent, method, fn) => wrap(parent, method, "after", fn),
            instead: (parent, method, fn) => wrap(parent, method, "instead", fn),
        };
    }

    // Per-plugin cleanup registry (unpatch functions + onCleanup hooks),
    // mirroring Electron's preload cleanup map. Cleared before every
    // (re-)execution OF THE SAME TARGET so a reloaded entry never stacks
    // patches or hooks — while the other target's patches are untouched
    // (Electron's renderer entries share nothing with the preload map).
    const cleanups = new Map();
    const cleanupKey = (target, pluginId) => `${target}:${pluginId}`;
    function addCleanup(key, cleanup) {
        if (typeof cleanup !== "function") return;
        const list = cleanups.get(key) ?? [];
        list.push(cleanup);
        cleanups.set(key, list);
    }
    function clearCleanups(key) {
        const list = cleanups.get(key) ?? [];
        cleanups.set(key, []);
        for (const cleanup of list) {
            try {
                cleanup();
            } catch (error) {
                console.error(`[Native plugins][${key}] cleanup failed`, error);
            }
        }
    }

    function createApi(pluginId, pluginName, target) {
        const prefix = `[Plugin:${pluginId}]`;
        const key = cleanupKey(target, pluginId);
        return {
            id: pluginId,
            name: pluginName,
            logger: {
                log: (...args) => console.log(prefix, ...args),
                warn: (...args) => console.warn(prefix, ...args),
                error: (...args) => console.error(prefix, ...args),
            },
            patcher: createPatcher(pluginId, (cleanup) => addCleanup(key, cleanup)),
            onCleanup: (cleanup) => addCleanup(key, cleanup),
        };
    }

    // CommonJS shim mirroring Electron's preload/renderer runners
    // (`mod`/`module`/`exports` plus the
    // `activate ?? default ?? globalThis.activatePlugin` lookup). Built
    // with string concatenation rather than a template literal so plugin
    // sources containing backticks or `${` cannot corrupt the wrapper
    // (Electron interpolates and would break on those bytes).
    function executePluginSource(pluginId, target, source, api) {
        const prefix =
            '"use strict";\n' +
            "const mod = { exports: {} };\n" +
            "const module = mod;\n" +
            "const exports = mod.exports;\n";
        const suffix =
            "\nconst activate = mod.exports.activate ?? mod.exports.default ?? globalThis.activatePlugin;\n" +
            'if (typeof activate === "function") { return activate(api); }\n' +
            `//# sourceURL=remcord-plugin-${target}-${pluginId}.js`;
        const runner = new Function("api", prefix + source + suffix);
        return runner(api);
    }

    const statuses = new Map();
    function setStatus(pluginId, status) {
        statuses.set(pluginId, status);
    }

    async function loadTarget(native, target) {
        let entries;
        try {
            entries = await native.getPluginEntries(target, REQUEST_TIMEOUT_MS);
        } catch (error) {
            console.error(`[Native plugins] could not list ${target} entries`, error);
            return;
        }
        if (!Array.isArray(entries)) return;
        for (const entry of entries) {
            if (!entry || typeof entry.id !== "string" || typeof entry.name !== "string") continue;
            const key = cleanupKey(target, entry.id);
            clearCleanups(key);
            try {
                const result = await native.getPluginScript(entry.id, target, REQUEST_TIMEOUT_MS);
                const source = result?.source;
                if (!source) continue;
                await Promise.resolve(
                    executePluginSource(entry.id, target, source, createApi(entry.id, entry.name, target)),
                );
                setStatus(entry.id, { status: "executed", target, error: null });
                reportResult(entry.id, target, true, null);
            } catch (error) {
                console.error(`[Native plugins][${entry.id}] ${target} entry failed`, error);
                setStatus(entry.id, {
                    status: "failed",
                    target,
                    error: String(error?.message ?? error).slice(0, 2048),
                });
                reportResult(entry.id, target, false, error);
            }
        }
    }

    async function loadPlugins() {
        const native = window.remcord?.native;
        if (!native) {
            console.error("[Native plugins] compatibility facade is unavailable");
            return;
        }
        try {
            native.requireCapability("plugins.get_runtime_entries");
            native.requireCapability("plugins.get_runtime_script");
        } catch (error) {
            console.error("[Native plugins] runtime capabilities are unavailable", error);
            return;
        }
        // Electron order: preload entries first, renderer entries second,
        // each sequential so one plugin's failure never skips another.
        for (const target of TARGETS) {
            await loadTarget(native, target);
        }
    }

    Object.defineProperty(window, "__remcordNativePluginLoader", {
        configurable: false,
        enumerable: false,
        value: true,
    });
    Object.defineProperty(window, "__remcordNativePlugins", {
        configurable: false,
        enumerable: false,
        get: () => Object.freeze(Object.fromEntries(statuses.entries())),
    });

    void loadPlugins();
})();

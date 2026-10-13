(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic document-start script.
    "use strict";

    if (window.location.origin !== "https://discord.com") return;
    if (Object.hasOwn(window, "__remcordNativeModSubstrate")) return;

    /**
     * Generic document-start adapter factory for native client mods.
     *
     * The factory owns request correlation, one-start-per-document behavior,
     * bootstrap verdict handling, typed result reporting, and error reporting.
     * Each mod supplies only its identity and typed wire vocabulary; the
     * renderer can never pick URLs, paths, executable source, or host methods.
     *
     * config:
     *   capability          capability name required in the boot snapshot
     *   requestBundle       (native) => Promise resolving the host metadata
     *   requestTimeoutMs    host round-trip budget (default 120_000)
     *   adapterFlag         document-local once-guard property name
     *   stateName           document-local readable state property name
     *   bootstrapEvent      document-local DOM event carrying the verdict
     *   expectedMarker      success marker string from the host verifier
     *   resultEvent         typed host event for the bootstrap result
     *   ackEvent            typed host event acknowledging the host response
     *   errorEvent          typed host event for adapter-level failures
     *   cssResultEvent      typed host event for the CSS result (CSS mods only)
     *   logTag              console prefix, e.g. "[Vencord native]"
     *   modId               logical mod id echoed on every typed host event
     *   validateResponse    (payload) => void, throws on invalid host metadata
     *   resultExtra         (detail) => object merged into the result payload
     *   responseExtra       (payload) => object merged into adapter state
     *   onExecuted          () => void, optional post-bootstrap hook ran once
     *                       the execution marker verifies (after state update,
     *                       before the result post; failures are logged, never
     *                       thrown, so the result always reports)
     */
    function createModAdapter(config) {
        let state;
        // A mod id is only echoed when the adapter declares one. The
        // mod-agnostic event family requires it so the host can refuse a
        // verdict claimed for another mod; the legacy Vencord event family
        // predates it and stays byte-identical.
        function hostEvent(type, payload) {
            const body = config.modId ? { mod_id: config.modId, ...payload } : payload;
            return JSON.stringify({ version: 1, event: { type, payload: body } });
        }
        function reportAdapterError(error) {
            const message = String(error?.message ?? error).slice(0, 2048);
            if (state) {
                state.status = "failed";
                state.error = message;
            }
            try {
                window.ipc.postMessage(hostEvent(config.errorEvent, { message }));
            } catch (reportError) {
                console.error(`${config.logTag}[adapter_initialization] could not report failure`, reportError);
            }
        }

        if (!window.remcord?.native) {
            reportAdapterError(new Error("Compatibility facade is unavailable"));
            return null;
        }
        if (Object.hasOwn(window, config.adapterFlag)) return null;

        const native = window.remcord.native;
        state = { status: "pending", marker: null, executionCount: 0 };
        let requestId = null;

        Object.defineProperty(window, config.adapterFlag, {
            configurable: false,
            enumerable: false,
            value: true,
        });
        Object.defineProperty(window, config.stateName, {
            configurable: false,
            enumerable: false,
            get: () => Object.freeze({ ...state }),
        });

        window.addEventListener(config.bootstrapEvent, (event) => {
            const result = event.detail;
            if (!result || result.request_id !== requestId) return;
            if (result.ok === true && result.marker === config.expectedMarker) {
                state.status = "executed";
                state.marker = result.marker;
                state.executionCount = 1;
                Object.assign(state, config.resultExtra ? config.resultExtra(result) : null);
                if (typeof config.onExecuted === "function") {
                    try {
                        config.onExecuted();
                    } catch (error) {
                        console.error(`${config.logTag}[adapter_initialization] post-bootstrap hook failed`, error);
                    }
                }
            } else {
                state.status = "failed";
                state.error = String(result.error ?? "Mod bootstrap marker was not observed");
            }
            try {
                window.ipc.postMessage(
                    hostEvent(config.resultEvent, {
                        request_id: requestId,
                        ok: state.status === "executed",
                        marker: state.marker ?? "",
                        execution_ms: Number.isFinite(state.executionMs) ? state.executionMs : 0,
                        ...(config.resultPayloadExtra ? config.resultPayloadExtra(result, state) : null),
                        error: state.status === "executed" ? null : state.error,
                    }),
                );
            } catch (error) {
                reportAdapterError(error);
            }
        });

        try {
            native.requireCapability(config.capability);
            const request = config.requestBundle(native);
            requestId = request.requestId;
            request
                .then((result) => {
                    if (config.validateResponse) config.validateResponse(result);
                    if (state.status !== "executed") {
                        throw new Error("Native mod response arrived before the execution marker");
                    }
                    Object.assign(state, config.responseExtra ? config.responseExtra(result) : null);
                    console.info(`${config.logTag} bootstrap executed`, {
                        marker: state.marker,
                        executionMs: state.executionMs,
                    });
                    window.ipc.postMessage(hostEvent(config.ackEvent, { request_id: requestId }));
                })
                .catch((error) => {
                    state.status = "failed";
                    state.error = String(error?.message ?? error).slice(0, 2048);
                    if (!error?.code || error.code === "REQUEST_TIMEOUT") reportAdapterError(error);
                });
        } catch (error) {
            reportAdapterError(error);
        }

        return {
            get requestId() {
                return requestId;
            },
            get state() {
                return Object.freeze({ ...state });
            },
        };
    }

    /**
     * Replaces a Vencord-family `VencordNative.quickCss.openEditor` popup
     * flow with a handoff to the Remcord in-Discord editor.
     *
     * The upstream `openEditor` calls `open("about:blank", ...)` and then
     * `document.write`s the editor into the child. The native host denies
     * the popup at the WebView2 layer and manages its own single-slot
     * child, so the opener's window proxy is dead: the call alerts about
     * popups and strands a blank orphan window. This override dispatches a
     * cancelable `remcord-native:open-quickcss-editor` DOM event carrying
     * the mod mode instead; the Remcord settings plugin (which owns the
     * editor modal) takes the handoff and calls `preventDefault` to signal
     * it. When nothing takes the handoff, an honest error names the real
     * cause instead of falling through to the dead popup path.
     *
     * `mode` is the owning mod ("vencord" | "equicord"); the settings
     * plugin titles the editor and resolves storage from it. Returns true
     * when the override installed, false when there was nothing to patch
     * (bundle absent or already patched). Never throws, never opens a
     * window, never touches the network or the host.
     */
    function overrideQuickCssEditor(mode) {
        try {
            const api = window.VencordNative?.quickCss;
            if (!api || typeof api.openEditor !== "function" || api.openEditor.__remcordNativePatched) {
                return false;
            }
            const patched = () => {
                let handled = false;
                try {
                    handled =
                        window.dispatchEvent(
                            new CustomEvent("remcord-native:open-quickcss-editor", {
                                cancelable: true,
                                detail: { mode },
                            }),
                        ) === false;
                } catch {
                    handled = false;
                }
                if (handled) return;
                try {
                    alert("Could not open the QuickCSS editor: the Remcord settings plugin is not running.");
                } catch {
                    console.error("[Remcord native][quickcss] editor handoff was not handled");
                }
            };
            try {
                patched.__remcordNativePatched = true;
            } catch {
                return false;
            }
            api.openEditor = patched;
            return true;
        } catch {
            return false;
        }
    }

    Object.defineProperty(window, "__remcordNativeModSubstrate", {
        configurable: false,
        enumerable: false,
        value: Object.freeze({ createModAdapter, overrideQuickCssEditor }),
    });
})();

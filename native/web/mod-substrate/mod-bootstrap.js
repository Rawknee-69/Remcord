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

    Object.defineProperty(window, "__remcordNativeModSubstrate", {
        configurable: false,
        enumerable: false,
        value: Object.freeze({ createModAdapter }),
    });
})();

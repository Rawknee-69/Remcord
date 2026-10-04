(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic document-start script.
    "use strict";

    if (window.location.origin !== "https://discord.com") return;
    let state;
    function reportAdapterError(error) {
        const message = String(error?.message ?? error).slice(0, 2048);
        if (state) {
            state.status = "failed";
            state.error = message;
        }
        try {
            window.ipc.postMessage(
                JSON.stringify({
                    version: 1,
                    event: { type: "vencord_adapter_error", payload: { message } },
                }),
            );
        } catch (reportError) {
            console.error("[Vencord native][adapter_initialization] could not report adapter failure", reportError);
        }
    }

    if (!window.legcord?.native) {
        reportAdapterError(new Error("Compatibility facade is unavailable"));
        return;
    }
    if (Object.hasOwn(window, "__legcordNativeVencordAdapter")) return;

    const native = window.legcord.native;
    state = { status: "pending", marker: null, executionCount: 0 };
    let requestId = null;

    Object.defineProperty(window, "__legcordNativeVencordAdapter", {
        configurable: false,
        enumerable: false,
        value: true,
    });
    Object.defineProperty(window, "__legcordNativeVencord", {
        configurable: false,
        enumerable: false,
        get: () => Object.freeze({ ...state }),
    });

    window.addEventListener("legcord-native:vencord-bootstrap", (event) => {
        const result = event.detail;
        if (!result || result.request_id !== requestId) return;
        if (result.ok === true && result.marker === "vencord-bootstrap-executed") {
            state.status = "executed";
            state.marker = result.marker;
            state.executionCount = 1;
            state.executionMs = result.execution_ms;
            state.localStorageApi = result.local_storage_api;
            state.settingsState = result.settings_state;
        } else {
            state.status = "failed";
            state.error = String(result.error ?? "Vencord bootstrap marker was not observed");
        }
        try {
            window.ipc.postMessage(
                JSON.stringify({
                    version: 1,
                    event: {
                        type: "vencord_bootstrap_result",
                        payload: {
                            request_id: requestId,
                            ok: state.status === "executed",
                            marker: state.marker ?? "",
                            execution_ms: Number.isFinite(state.executionMs) ? state.executionMs : 0,
                            local_storage_api: state.localStorageApi ?? "blocked",
                            settings_state: state.settingsState ?? "storage_unavailable",
                            error: state.status === "executed" ? null : state.error,
                        },
                    },
                }),
            );
        } catch (error) {
            reportAdapterError(error);
        }
    });

    try {
        native.requireCapability("mod.vencord.get_bundle");
        const request = native.getVencordBundle();
        requestId = request.requestId;
        request
            .then((result) => {
                if (result.marker !== "vencord-bootstrap-executed") {
                    throw new Error("Native Vencord bootstrap marker is invalid");
                }
                if (state.status !== "executed") {
                    throw new Error("Native Vencord response arrived before the execution marker");
                }
                state.bundleVersion = result.bundle_version;
                state.bundleBytes = result.bundle_bytes;
                state.cssBytes = result.css_bytes;
                state.requestLatencyMs = result.request_latency_ms;
                state.retrievalMs = result.retrieval_ms;
                state.cssInjectionMs = result.css_injection_ms;
                console.info("[Vencord native] bootstrap executed", {
                    version: state.bundleVersion,
                    bundleBytes: state.bundleBytes,
                    cssBytes: state.cssBytes,
                    requestLatencyMs: state.requestLatencyMs,
                    retrievalMs: state.retrievalMs,
                    executionMs: state.executionMs,
                    cssInjectionMs: state.cssInjectionMs,
                });
                window.ipc.postMessage(
                    JSON.stringify({
                        version: 1,
                        event: { type: "vencord_response_ack", payload: { request_id: requestId } },
                    }),
                );
            })
            .catch((error) => {
                state.status = "failed";
                state.error = String(error?.message ?? error).slice(0, 2048);
                if (!error?.code || error.code === "REQUEST_TIMEOUT") reportAdapterError(error);
            });
    } catch (error) {
        reportAdapterError(error);
    }
})();

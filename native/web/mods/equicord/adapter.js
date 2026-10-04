(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic document-start script.
    "use strict";

    if (window.location.origin !== "https://discord.com") return;
    const substrate = window.__legcordNativeModSubstrate;
    if (!substrate) {
        try {
            window.ipc.postMessage(
                JSON.stringify({
                    version: 1,
                    event: {
                        type: "mod_adapter_error",
                        payload: {
                            mod_id: "equicord",
                            message: "Native mod substrate is unavailable",
                        },
                    },
                }),
            );
        } catch {
            console.error("[Equicord native][adapter_initialization] mod substrate is unavailable");
        }
        return;
    }

    // Equicord owns only its identity and typed wire vocabulary here. Unlike
    // Shelter, Equicord DOES require CSS: its Electron preload calls
    // `webFrame.insertCSS(bundle.css!)` with an explicit "Equicord requires CSS"
    // note, so the host runs a full CSS stage and this adapter reports its
    // result before acknowledging.
    //
    // Equicord is a Vencord fork and publishes the same `Vencord` page-world
    // global its own upstream bundle defines. It is also gated on
    // `settings.mods`, matching Electron's
    // `getConfig("mods").includes("equicord")` — so exactly one of the two
    // globals-sharing consumers can be active in a given run.
    substrate.createModAdapter({
        capability: "mod.get_bundle",
        requestBundle: (native) => native.getModBundle("equicord"),
        requestTimeoutMs: 120_000,
        adapterFlag: "__legcordNativeEquicordAdapter",
        stateName: "__legcordNativeEquicord",
        bootstrapEvent: "legcord-native:equicord-bootstrap",
        expectedMarker: "equicord-bootstrap-executed",
        // The host injects Equicord's stylesheet and reports its own result;
        // this adapter therefore names NO CSS event. The host's CSS result
        // carries its own mod id and cannot be claimed by another mod.
        resultEvent: "mod_bootstrap_result",
        ackEvent: "mod_response_ack",
        errorEvent: "mod_adapter_error",
        logTag: "[Equicord native]",
        modId: "equicord",
        validateResponse: (result) => {
            if (result.mod_id !== "equicord") {
                throw new Error("Native mod bundle response is not for Equicord");
            }
            if (result.marker !== "equicord-bootstrap-executed") {
                throw new Error("Native Equicord bootstrap marker is invalid");
            }
        },
        resultExtra: (result) => ({
            executionMs: result.execution_ms,
            localStorageApi: result.local_storage_api,
            settingsState: result.settings_state,
        }),
        resultPayloadExtra: (_result, state) => ({
            local_storage_api: state.localStorageApi ?? "blocked",
            settings_state: state.settingsState ?? "storage_unavailable",
        }),
        responseExtra: (result) => ({
            modId: result.mod_id,
            bundleVersion: result.bundle_version,
            bundleBytes: result.bundle_bytes,
            cssBytes: result.css_bytes,
            cssInjectionMs: result.css_injection_ms,
            requestLatencyMs: result.request_latency_ms,
            retrievalMs: result.retrieval_ms,
        }),
    });
})();

(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic document-start script.
    "use strict";

    if (window.location.origin !== "https://discord.com") return;
    const substrate = window.__remcordNativeModSubstrate;
    if (!substrate) {
        try {
            window.ipc.postMessage(
                JSON.stringify({
                    version: 1,
                    event: {
                        type: "vencord_adapter_error",
                        payload: { message: "Native mod substrate is unavailable" },
                    },
                }),
            );
        } catch {
            console.error("[Vencord native][adapter_initialization] mod substrate is unavailable");
        }
        return;
    }

    // Vencord owns only its identity and typed wire vocabulary here. Bundle
    // source, URLs, cache paths, execution, CSS, and lifecycle all remain
    // host-controlled through the shared substrate and the typed capability.
    substrate.createModAdapter({
        capability: "mod.get_bundle",
        requestBundle: (native) => native.getModBundle("vencord"),
        requestTimeoutMs: 120_000,
        adapterFlag: "__remcordNativeVencordAdapter",
        stateName: "__remcordNativeVencord",
        bootstrapEvent: "remcord-native:vencord-bootstrap",
        expectedMarker: "vencord-bootstrap-executed",
        resultEvent: "vencord_bootstrap_result",
        ackEvent: "vencord_response_ack",
        errorEvent: "vencord_adapter_error",
        logTag: "[Vencord native]",
        validateResponse: (result) => {
            // The shared `mod.get_bundle` response is mod-agnostic, so this
            // adapter must confirm BOTH the echoed registry id and its own
            // execution marker: a host answering for another registered mod
            // can never pass as Vencord.
            if (result.mod_id !== "vencord") {
                throw new Error("Native mod bundle response is not for Vencord");
            }
            if (result.marker !== "vencord-bootstrap-executed") {
                throw new Error("Native Vencord bootstrap marker is invalid");
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
            requestLatencyMs: result.request_latency_ms,
            retrievalMs: result.retrieval_ms,
            cssInjectionMs: result.css_injection_ms,
        }),
        // After the bootstrap marker proves the bundle ran, replace
        // Vencord's about:blank popup editor (dead under the native
        // managed-child popup policy) with the handoff to the Remcord
        // in-Discord editor. Runs at most once per document; a missing
        // VencordNative simply leaves the hook a no-op.
        onExecuted: () => {
            substrate.overrideQuickCssEditor("vencord");
        },
    });
})();

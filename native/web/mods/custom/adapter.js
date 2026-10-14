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
    const substrate = window.__remcordNativeModSubstrate;
    if (!substrate) {
        try {
            window.ipc.postMessage(
                JSON.stringify({
                    version: 1,
                    event: {
                        type: "mod_adapter_error",
                        payload: {
                            mod_id: "custom",
                            message: "Native mod substrate is unavailable",
                        },
                    },
                }),
            );
        } catch {
            console.error("[Custom native][adapter_initialization] mod substrate is unavailable");
        }
        return;
    }

    // Custom owns only its identity and typed wire vocabulary here. The
    // bundle bytes, the profile-root file paths, execution, CSS, and
    // lifecycle all remain host-controlled through the shared substrate.
    //
    // Two traced facts shape this adapter and nothing else:
    // 1. Custom is gated on `settings.mods` containing `"custom"`
    //    (Electron's `getConfig("mods").includes("custom")` in
    //    `src/discord/ipc.ts`). The host injects this adapter only for
    //    runs where that gate holds, so its presence IS the enablement.
    // 2. Electron verifies nothing after execution: the bundle runs via
    //    `webFrame.executeJavaScript` and CSS lands via `insertCSS`, and a
    //    throw is only a console error. Natively the host marker therefore
    //    proves "the host context plus the user JS evaluated without the
    //    delivery failing" rather than a page-world global no custom bundle
    //    could be required to publish.
    substrate.createModAdapter({
        capability: "mod.get_bundle",
        requestBundle: (native) => native.getModBundle("custom"),
        requestTimeoutMs: 120_000,
        adapterFlag: "__remcordNativeCustomAdapter",
        stateName: "__remcordNativeCustom",
        bootstrapEvent: "remcord-native:custom-bootstrap",
        expectedMarker: "custom-bootstrap-executed",
        resultEvent: "mod_bootstrap_result",
        ackEvent: "mod_response_ack",
        errorEvent: "mod_adapter_error",
        logTag: "[Custom native]",
        modId: "custom",
        validateResponse: (result) => {
            // The shared `mod.get_bundle` response is mod-agnostic, so this
            // adapter must confirm BOTH the echoed registry id and its own
            // execution marker: a host answering for another registered mod
            // can never pass as Custom.
            if (result.mod_id !== "custom") {
                throw new Error("Native mod bundle response is not for Custom");
            }
            if (result.marker !== "custom-bootstrap-executed") {
                throw new Error("Native Custom bootstrap marker is invalid");
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

(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Test-only adapter, exercised in node vm tests.
    "use strict";

    // Synthetic second consumer of the shared mod substrate. It proves the
    // factory is not Vencord-specific without loading Shelter or Equicord.
    // This file is never included in the native document-start script, never
    // touches Discord UI, and never submits account state.
    if (window.location.origin !== "https://discord.com") return;
    const substrate = window.__remcordNativeModSubstrate;
    if (!substrate) return;

    substrate.createModAdapter({
        capability: "mod.test.get_bundle",
        requestBundle: (native) => native.getTestModBundle(),
        requestTimeoutMs: 5000,
        adapterFlag: "__remcordNativeTestModAdapter",
        stateName: "__remcordNativeTestMod",
        bootstrapEvent: "remcord-native:testmod-bootstrap",
        expectedMarker: "testmod-bootstrap-executed",
        resultEvent: "testmod_bootstrap_result",
        ackEvent: "testmod_response_ack",
        errorEvent: "testmod_adapter_error",
        logTag: "[TestMod native]",
        validateResponse: (result) => {
            if (result.marker !== "testmod-bootstrap-executed") {
                throw new Error("Native TestMod bootstrap marker is invalid");
            }
        },
        resultExtra: (result) => ({ executionMs: result.execution_ms }),
        resultPayloadExtra: (_result, state) => ({
            storage_api: state.storageApi ?? "blocked",
        }),
        responseExtra: (result) => ({ bundleVersion: result.bundle_version }),
    });
})();

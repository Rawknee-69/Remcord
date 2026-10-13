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
                        payload: { mod_id: "shelter", message: "Native mod substrate is unavailable" },
                    },
                }),
            );
        } catch {
            console.error("[Shelter native][adapter_initialization] mod substrate is unavailable");
        }
        return;
    }

    // Shelter owns only its identity and typed wire vocabulary here. The
    // bundle source, its fixed upstream source, the cache file, artifact
    // identity, execution world, readiness deadline, lifecycle, and duplicate
    // guards all remain host-controlled through the shared substrate.
    //
    // Three traced facts shape this adapter and nothing else:
    // 1. Shelter ships NO stylesheet (the bundle handler returns only `js`, and
    //    no stylesheet exists for it anywhere in the repo), so there is no CSS
    //    result to report and no CSS stage in the host pipeline.
    // 2. Shelter publishes `window.shelter` asynchronously, only after
    //    Discord's `#app-mount` exists and has children, so the host verdict is
    //    awaited rather than read synchronously.
    // 3. Electron wraps Shelter's source in an IIFE that declares an injector
    //    plugin map. That wrapper is deliberately NOT reproduced here: the
    //    preload plugin loader is unimplemented natively (the local-asset
    //    protocol it would load from now exists as `remcord://` with a legacy
    //    `remcord://` rewrite), and Shelter skips the injector entirely when
    //    the global is absent, so omitting it is exact native behavior rather
    //    than a stub.
    substrate.createModAdapter({
        capability: "mod.get_bundle",
        requestBundle: (native) => native.getModBundle("shelter"),
        requestTimeoutMs: 180_000,
        adapterFlag: "__remcordNativeShelterAdapter",
        stateName: "__remcordNativeShelter",
        bootstrapEvent: "remcord-native:shelter-bootstrap",
        expectedMarker: "shelter-bootstrap-executed",
        resultEvent: "mod_bootstrap_result",
        ackEvent: "mod_response_ack",
        errorEvent: "mod_adapter_error",
        logTag: "[Shelter native]",
        modId: "shelter",
        validateResponse: (result) => {
            // The shared `mod.get_bundle` response is mod-agnostic, so this
            // adapter must confirm BOTH the echoed registry id and its own
            // execution marker: a host answering for another registered mod
            // can never pass as Shelter.
            if (result.mod_id !== "shelter") {
                throw new Error("Native mod bundle response is not for Shelter");
            }
            if (result.marker !== "shelter-bootstrap-executed") {
                throw new Error("Native Shelter bootstrap marker is invalid");
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
    });
})();

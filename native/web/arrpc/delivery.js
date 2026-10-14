(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic document-start script.
    "use strict";

    // arRPC presence delivery bridge (F11 core): forwards host-delivered
    // presence events to the page-world consumer. Electron delivers activity
    // through `executeJavaScript("window.remcordRPC.listen(...)")`; native
    // delivers the same payload as a typed `remcord-native:rpc-activity`
    // CustomEvent, and this script (running in the same document) hands it
    // to `window.remcordRPC.listen` when the Shelter RPC plugin installed
    // it. Invite codes are recorded (bounded) for the F11b UI; the Discord
    // invite navigation itself stays deferred with the F11b socket source.
    //
    // Entry origins for the three release channels (stable/canary/ptb).
    // Exact match only — no subdomains, no other hosts.
    if (
        window.location.origin !== "https://discord.com" &&
        window.location.origin !== "https://canary.discord.com" &&
        window.location.origin !== "https://ptb.discord.com"
    )
        return;
    if (Object.hasOwn(window, "__remcordNativeRpcDelivery")) return;
    Object.defineProperty(window, "__remcordNativeRpcDelivery", {
        configurable: false,
        enumerable: false,
        value: true,
    });

    function publishStatus() {
        try {
            Object.defineProperty(window, "__remcordRpcDelivery", {
                configurable: false,
                enumerable: false,
                value: Object.freeze({ installed: true }),
            });
        } catch (error) {
            console.error("[Remcord][RpcDelivery] status publish failed", error);
        }
    }
    publishStatus();

    function validDetail(detail) {
        return !!detail && typeof detail === "object" && detail.version === 1;
    }

    // Activity pulses: the host-validated Discord activity object, handed to
    // the Shelter RPC plugin exactly like Electron's executeJavaScript call.
    // A missing plugin (or a misshapen payload) drops the pulse loudly —
    // presence never throws into the page.
    window.addEventListener("remcord-native:rpc-activity", (event) => {
        try {
            const detail = event?.detail;
            if (!validDetail(detail) || !detail.activity || typeof detail.activity !== "object") return;
            const listen = window.remcordRPC?.listen;
            if (typeof listen !== "function") return;
            listen.call(window.remcordRPC, { activity: detail.activity });
        } catch (error) {
            console.error("[Remcord][RpcDelivery] activity forward failed", error);
        }
    });

    // Invite codes: recorded (bounded token + timestamp) for the F11b UI.
    // No navigation happens here — routing needs the settings router
    // context, which the socket source owns.
    window.addEventListener("remcord-native:rpc-invite", (event) => {
        try {
            const detail = event?.detail;
            if (!validDetail(detail) || typeof detail.code !== "string") return;
            if (!/^[A-Za-z0-9_-]{1,64}$/.test(detail.code)) return;
            Object.defineProperty(window, "__remcordLastInvite", {
                configurable: true,
                enumerable: false,
                value: Object.freeze({ code: detail.code, at: Date.now() }),
            });
        } catch (error) {
            console.error("[Remcord][RpcDelivery] invite record failed", error);
        }
    });
})();

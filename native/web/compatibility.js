(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic script, not a module.
    "use strict";

    const startedAt = performance.now();
    const bootSnapshot = __LEGCORD_BOOT_SNAPSHOT__;
    const protocolVersion = 1;
    const pending = new Map();
    const textEncoder = new TextEncoder();
    let sequence = 0;

    if (window.location.origin !== "https://discord.com") return;

    function freezeDeep(value) {
        if (value && typeof value === "object" && !Object.isFrozen(value)) {
            for (const child of Object.values(value)) freezeDeep(child);
            Object.freeze(value);
        }
        return value;
    }

    function nextId() {
        sequence += 1;
        return `legcord-${Date.now().toString(36)}-${sequence}`;
    }

    function post(message) {
        if (!window.ipc || typeof window.ipc.postMessage !== "function") {
            throw new Error("Native bridge transport is unavailable");
        }
        window.ipc.postMessage(JSON.stringify({ version: protocolVersion, ...message }));
    }

    function sendEvent(event) {
        post({ event });
    }

    function truncateUtf8(value, maxBytes) {
        let result = "";
        let byteLength = 0;
        for (const character of String(value)) {
            const characterBytes = textEncoder.encode(character).length;
            if (byteLength + characterBytes > maxBytes) break;
            result += character;
            byteLength += characterBytes;
        }
        return result;
    }

    function reportRendererError(kind, message) {
        try {
            sendEvent({
                type: "renderer_error",
                payload: { kind, message: truncateUtf8(message, 2048) },
            });
        } catch {
            // Error reporting must not interfere with Discord's error handling.
        }
    }

    function ping(nonce) {
        if (typeof nonce !== "string" || nonce.length === 0 || textEncoder.encode(nonce).length > 128) {
            return Promise.reject(new TypeError("nonce must be a non-empty string of at most 128 UTF-8 bytes"));
        }

        const requestId = nextId();
        return new Promise((resolve, reject) => {
            const timer = setTimeout(() => {
                pending.delete(requestId);
                reject(new Error("Native bridge request timed out"));
            }, 5000);
            pending.set(requestId, { resolve, reject, timer });
            try {
                post({
                    request_id: requestId,
                    command: { type: "ping", payload: { nonce } },
                });
            } catch (error) {
                clearTimeout(timer);
                pending.delete(requestId);
                reject(error);
            }
        });
    }

    window.addEventListener("legcord-native:response", (event) => {
        const response = event.detail;
        if (!response || response.version !== protocolVersion || typeof response.request_id !== "string") {
            return;
        }
        const request = pending.get(response.request_id);
        if (!request) return;

        clearTimeout(request.timer);
        pending.delete(response.request_id);
        if (response.ok === true && response.payload && typeof response.payload.nonce === "string") {
            request.resolve(response.payload);
            try {
                sendEvent({
                    type: "round_trip_ack",
                    payload: { nonce: response.payload.nonce },
                });
            } catch (error) {
                reportRendererError("bridge_event", error);
            }
        } else {
            request.reject(new Error("Native bridge request failed"));
        }
    });

    window.addEventListener(
        "pagehide",
        () => {
            for (const request of pending.values()) {
                clearTimeout(request.timer);
                request.reject(new Error("Native bridge document was disposed"));
            }
            pending.clear();
        },
        { once: true },
    );

    window.addEventListener("error", (event) => {
        reportRendererError("uncaught", event.message || "Uncaught renderer error");
    });
    window.addEventListener("unhandledrejection", (event) => {
        reportRendererError("unhandled_rejection", event.reason);
    });

    const originalConsoleError = console.error.bind(console);
    console.error = (...values) => {
        originalConsoleError(...values);
        reportRendererError(
            "console.error",
            values
                .map((value) => {
                    try {
                        return String(value);
                    } catch {
                        return "[unprintable renderer error]";
                    }
                })
                .join(" "),
        );
    };

    const snapshot = freezeDeep(bootSnapshot);
    const capabilities = new Set(snapshot.capabilities);
    const native = Object.freeze({
        ping,
        reportCss(value) {
            sendEvent({
                type: "css_probe",
                payload: { value: truncateUtf8(value, 128) },
            });
        },
        supports(capability) {
            return typeof capability === "string" && capabilities.has(capability);
        },
        requireCapability(capability) {
            if (typeof capability !== "string" || !capabilities.has(capability)) {
                const error = new Error(`Native capability is not available: ${String(capability)}`);
                error.code = "UNSUPPORTED_CAPABILITY";
                throw error;
            }
            return true;
        },
    });

    Object.defineProperty(window, "legcord", {
        configurable: false,
        enumerable: true,
        value: Object.freeze({ boot: snapshot, native }),
    });

    try {
        sendEvent({
            type: "bootstrap_metrics",
            payload: { execution_ms: Math.max(0, performance.now() - startedAt) },
        });
    } catch (error) {
        reportRendererError("bridge_event", error);
    }
})();

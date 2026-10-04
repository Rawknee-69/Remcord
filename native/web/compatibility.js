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
            console.error("[Legcord native] renderer error could not be sent to the host", {
                kind,
                message,
            });
        }
    }

    function describeRendererError(error) {
        try {
            if (error && typeof error.stack === "string") return error.stack;
            return String(error);
        } catch {
            return "[unprintable renderer error]";
        }
    }

    function request(command, timeoutMs = 5000) {
        const requestId = nextId();
        const pendingRequest = new Promise((resolve, reject) => {
            const timer = setTimeout(() => {
                pending.delete(requestId);
                const error = new Error("Native bridge request timed out");
                error.code = "REQUEST_TIMEOUT";
                reject(error);
            }, timeoutMs);
            pending.set(requestId, { resolve, reject, timer, command: command.type });
            try {
                post({
                    request_id: requestId,
                    command,
                });
            } catch (error) {
                clearTimeout(timer);
                pending.delete(requestId);
                reject(error);
            }
        });
        Object.defineProperty(pendingRequest, "requestId", { value: requestId });
        return pendingRequest;
    }

    function ping(nonce) {
        if (typeof nonce !== "string" || nonce.length === 0 || textEncoder.encode(nonce).length > 128) {
            return Promise.reject(new TypeError("nonce must be a non-empty string of at most 128 UTF-8 bytes"));
        }

        return request({ type: "ping", payload: { nonce } });
    }

    function getVencordBundle() {
        requireCapability("mod.vencord.get_bundle");
        return request({ type: "mod_vencord_get_bundle", payload: {} }, 120_000);
    }

    function isSettingsSnapshot(value) {
        return (
            !!value &&
            typeof value === "object" &&
            typeof value.blockPowerSavingInVoiceChat === "boolean" &&
            typeof value.minimizeToTray === "boolean" &&
            typeof value.windowStyle === "string" &&
            typeof value.transparency === "string" &&
            Array.isArray(value.mods) &&
            value.mods.every((entry) => typeof entry === "string") &&
            !!value.audio &&
            typeof value.audio === "object"
        );
    }

    function getConfig(timeoutMs = 5000) {
        requireCapability("settings.get_config");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        return request({ type: "settings_get_config", payload: {} }, timeoutMs).then((payload) => payload.settings);
    }

    function getLang(key, timeoutMs = 5000) {
        requireCapability("localization.get_lang");
        if (typeof key !== "string" || key.length === 0 || textEncoder.encode(key).length > 128) {
            return Promise.reject(new TypeError("key must be a non-empty string of at most 128 UTF-8 bytes"));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        return request({ type: "localization_get_lang", payload: { key } }, timeoutMs).then((payload) => payload.value);
    }

    function setBlockPowerSavingInVoiceChat(value, timeoutMs = 5000) {
        requireCapability("settings.set_block_power_saving_in_voice_chat");
        if (typeof value !== "boolean") {
            return Promise.reject(new TypeError("value must be a boolean"));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        return request({ type: "settings_set_block_power_saving", payload: { value } }, timeoutMs).then(
            (payload) => payload.block_power_saving_in_voice_chat,
        );
    }

    function getVersion(timeoutMs = 5000) {
        requireCapability("app.get_version");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        return request({ type: "app_get_version", payload: {} }, timeoutMs).then((payload) => payload.version);
    }

    function requireCapability(capability) {
        if (typeof capability !== "string" || !capabilities.has(capability)) {
            const error = new Error(`Native capability is not available: ${String(capability)}`);
            error.code = "UNSUPPORTED_CAPABILITY";
            throw error;
        }
        return true;
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
        if (response.ok === true && response.payload?.type === "ping") {
            const payload = response.payload.payload;
            if (!payload || typeof payload.nonce !== "string" || typeof payload.message !== "string") {
                request.reject(new Error("Native ping response schema is invalid"));
                return;
            }
            request.resolve(payload);
            try {
                sendEvent({
                    type: "round_trip_ack",
                    payload: { nonce: payload.nonce },
                });
            } catch (error) {
                reportRendererError("bridge_event", error);
            }
        } else if (response.ok === true && response.payload?.type === "vencord_bundle") {
            const payload = response.payload.payload;
            if (
                !payload ||
                typeof payload.bundle_version !== "string" ||
                !Number.isSafeInteger(payload.bundle_bytes) ||
                !Number.isSafeInteger(payload.css_bytes) ||
                !Number.isFinite(payload.request_latency_ms) ||
                !Number.isFinite(payload.retrieval_ms) ||
                !Number.isFinite(payload.execution_ms) ||
                !Number.isFinite(payload.css_injection_ms) ||
                payload.marker !== "vencord-bootstrap-executed" ||
                payload.document_execution_count !== 1
            ) {
                request.reject(new Error("Native Vencord response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "settings_config") {
            const payload = response.payload.payload;
            if (
                !payload ||
                !isSettingsSnapshot(payload.settings) ||
                (payload.source !== "file" && payload.source !== "defaults") ||
                !Number.isSafeInteger(payload.payload_bytes)
            ) {
                request.reject(new Error("Native settings response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "settings_mutation") {
            const payload = response.payload.payload;
            if (
                !payload ||
                typeof payload.block_power_saving_in_voice_chat !== "boolean" ||
                payload.source !== "file" ||
                !Number.isSafeInteger(payload.payload_bytes)
            ) {
                request.reject(new Error("Native settings mutation response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "localization_string") {
            const payload = response.payload.payload;
            if (
                !payload ||
                typeof payload.value !== "string" ||
                textEncoder.encode(payload.value).length > 4096 ||
                typeof payload.lang !== "string" ||
                payload.lang.length === 0 ||
                textEncoder.encode(payload.lang).length > 32
            ) {
                request.reject(new Error("Native localization response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "app_version") {
            const payload = response.payload.payload;
            if (
                !payload ||
                typeof payload.version !== "string" ||
                payload.version.length === 0 ||
                textEncoder.encode(payload.version).length > 64
            ) {
                request.reject(new Error("Native app version response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else {
            const error = new Error(
                typeof response.error?.message === "string" ? response.error.message : "Native bridge request failed",
            );
            if (typeof response.error?.code === "string") error.code = response.error.code;
            request.reject(error);
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
        const location = event.filename ? ` (${event.filename}:${event.lineno}:${event.colno})` : "";
        reportRendererError("uncaught", `${event.message || "Uncaught renderer error"}${location}`);
    });
    window.addEventListener("unhandledrejection", (event) => {
        reportRendererError("unhandled_rejection", describeRendererError(event.reason));
    });

    const originalConsoleError = console.error.bind(console);
    console.error = (...values) => {
        originalConsoleError(...values);
        reportRendererError(
            "console.error",
            values
                .map((value) => {
                    return describeRendererError(value);
                })
                .join(" "),
        );
    };

    const snapshot = freezeDeep(bootSnapshot);
    const capabilities = new Set(snapshot.capabilities);
    const native = Object.freeze({
        ping,
        getVencordBundle,
        getVersion,
        reportCss(value) {
            sendEvent({
                type: "css_probe",
                payload: { value: truncateUtf8(value, 128) },
            });
        },
        supports(capability) {
            return typeof capability === "string" && capabilities.has(capability);
        },
        requireCapability,
    });
    const settings = Object.freeze({ getConfig, setBlockPowerSavingInVoiceChat });

    Object.defineProperty(window, "legcord", {
        configurable: false,
        enumerable: true,
        value: Object.freeze({ boot: snapshot, native, settings, getLang }),
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

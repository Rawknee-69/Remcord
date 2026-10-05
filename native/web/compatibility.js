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

    function getModBundle(modId, timeoutMs = 120_000) {
        requireCapability("mod.get_bundle");
        if (typeof modId !== "string" || modId.length === 0 || !/^[A-Za-z0-9._-]{1,64}$/.test(modId)) {
            return Promise.reject(new TypeError("modId must match [A-Za-z0-9._-]{1,64}"));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        return request({ type: "mod_get_bundle", payload: { mod_id: modId } }, timeoutMs);
    }

    function getVencordBundle() {
        return getModBundle("vencord", 120_000);
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

    const PLUGIN_ID_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;
    const MAX_PLUGIN_STORAGE_BYTES = 1048576;

    function checkPluginStorageArgs(pluginId, relativePath) {
        if (typeof pluginId !== "string" || !PLUGIN_ID_PATTERN.test(pluginId)) {
            return "pluginId must match [A-Za-z0-9_-]{1,64}";
        }
        if (
            typeof relativePath !== "string" ||
            relativePath.length === 0 ||
            textEncoder.encode(relativePath).length > 256
        ) {
            return "relativePath must be a non-empty string of at most 256 UTF-8 bytes";
        }
        return null;
    }

    function readPluginFile(pluginId, relativePath, timeoutMs = 5000) {
        requireCapability("plugin_storage.read");
        const invalid = checkPluginStorageArgs(pluginId, relativePath);
        if (invalid) return Promise.reject(new TypeError(invalid));
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        return request(
            { type: "plugin_storage_read", payload: { plugin_id: pluginId, relative_path: relativePath } },
            timeoutMs,
        ).then((payload) => payload.data);
    }

    function writePluginFile(pluginId, relativePath, data, timeoutMs = 5000) {
        requireCapability("plugin_storage.write");
        const invalid = checkPluginStorageArgs(pluginId, relativePath);
        if (invalid) return Promise.reject(new TypeError(invalid));
        if (typeof data !== "string" || textEncoder.encode(data).length > MAX_PLUGIN_STORAGE_BYTES) {
            return Promise.reject(new TypeError("data must be a string of at most 1048576 UTF-8 bytes"));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        return request(
            {
                type: "plugin_storage_write",
                payload: { plugin_id: pluginId, relative_path: relativePath, data },
            },
            timeoutMs,
        ).then((payload) => payload.payload_bytes);
    }

    function requireCapability(capability) {
        if (typeof capability !== "string" || !capabilities.has(capability)) {
            const error = new Error(`Native capability is not available: ${String(capability)}`);
            error.code = "UNSUPPORTED_CAPABILITY";
            throw error;
        }
        return true;
    }

    // Exact Electron BUNDLE_FILES allowlist (src/common/backup.ts), in the
    // same order. custom.js/custom.css are inventory-only and stay
    // non-executable natively; they are listed so the inventory reasons
    // about the same set Electron reasons about.
    const BUNDLE_NAMES = [
        "vencord.js",
        "vencord.css",
        "equicord.js",
        "equicord.css",
        "shelter.js",
        "custom.js",
        "custom.css",
    ];

    // Closed location enum for the three host-controlled roots. The
    // renderer selects a variant only; the host builds the directory.
    const FOLDER_LOCATIONS = ["storage", "themes", "plugins"];
    // Same rule the host enforces for theme ids ([A-Za-z0-9._-]{1,64},
    // never "." or ".."); separators are impossible by charset.
    const THEME_ID_PATTERN = /^[A-Za-z0-9._-]{1,64}$/;
    // Maximum installable CSS text in UTF-8 bytes, mirroring the host
    // theme bound. Larger stylesheets are rejected before dispatch.
    const MAX_THEME_CSS_BYTES = 1048576;

    function openFolder(location, timeoutMs = 5000) {
        requireCapability("folders.open");
        if (typeof location !== "string" || !FOLDER_LOCATIONS.includes(location)) {
            return Promise.reject(new TypeError("location must be one of storage, themes, plugins"));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Reveal semantics (the Explorer location behavior Electron gets
        // from its folder-reveal call): the host opens the parent with this
        // fixed root selected.
        return request({ type: "folders_open", payload: { location } }, timeoutMs).then((payload) => {
            if (payload.location !== location) {
                throw new Error("Native folder response schema is invalid");
            }
            return payload.location;
        });
    }

    function openThemeFolder(id, timeoutMs = 5000) {
        requireCapability("folders.open_theme");
        if (typeof id !== "string" || !THEME_ID_PATTERN.test(id) || id === "." || id === "..") {
            return Promise.reject(new TypeError("id must match [A-Za-z0-9._-]{1,64} and not be . or .."));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Open semantics (the Explorer behavior Electron gets from opening
        // a directory path): Explorer opens inside the validated theme
        // folder itself.
        return request({ type: "folders_open_theme", payload: { id } }, timeoutMs).then((payload) => {
            if (payload.id !== id) {
                throw new Error("Native theme folder response schema is invalid");
            }
            return payload.id;
        });
    }

    function installThemeFromCode(css, timeoutMs = 5000) {
        requireCapability("themes.install_code");
        if (typeof css !== "string" || css.length === 0 || textEncoder.encode(css).length > MAX_THEME_CSS_BYTES) {
            return Promise.reject(new TypeError("css must be a non-empty string of at most 1048576 UTF-8 bytes"));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // CSS text only: the host derives the theme id from `@name`, fixes
        // the `src.css` filename, and keeps the theme disabled. No URL,
        // path, filename, id, or destination may be supplied.
        return request({ type: "theme_install_code", payload: { css } }, timeoutMs).then((payload) => payload);
    }

    function writeThemeCss(id, css, timeoutMs = 5000) {
        requireCapability("themes.write_css");
        if (typeof id !== "string" || !THEME_ID_PATTERN.test(id) || id === "." || id === "..") {
            return Promise.reject(new TypeError("id must match [A-Za-z0-9._-]{1,64} and not be . or .."));
        }
        if (typeof css !== "string" || textEncoder.encode(css).length > MAX_THEME_CSS_BYTES) {
            return Promise.reject(new TypeError("css must be a string of at most 1048576 UTF-8 bytes"));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Id plus CSS text only: the host resolves the manifest-declared
        // file under its own themes root and preserves the manifest. The
        // response echoes the validated id so the caller can confirm the
        // target. No path, filename, URL, destination, or enablement flag
        // may be supplied.
        return request({ type: "theme_write_css", payload: { id, css } }, timeoutMs).then((payload) => {
            if (payload.id !== id) {
                throw new Error("Native theme write response schema is invalid");
            }
            return payload;
        });
    }

    function listBundles(timeoutMs = 5000) {
        requireCapability("backup.list_bundles");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Parameter-free by protocol: no path, URL, include flag, or
        // selector may be supplied. The host inventories the fixed
        // allowlist only.
        return request({ type: "backup_list_bundles", payload: {} }, timeoutMs).then(
            (payload) => payload.bundles,
        );
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
        } else if (response.ok === true && response.payload?.type === "mod_bundle") {
            const payload = response.payload.payload;
            // A mod that ships no stylesheet reports BOTH css fields as null.
            // Requiring the pair to agree keeps "ships no CSS" distinguishable
            // from "shipped a stylesheet" without inventing an empty-stylesheet
            // state, and rejects a half-populated response either way.
            const shipsNoCss = payload?.css_bytes === null && payload?.css_injection_ms === null;
            const shipsCss =
                Number.isSafeInteger(payload?.css_bytes) &&
                payload.css_bytes >= 0 &&
                Number.isFinite(payload?.css_injection_ms) &&
                payload.css_injection_ms >= 0;
            if (
                !payload ||
                typeof payload.mod_id !== "string" ||
                payload.mod_id.length === 0 ||
                typeof payload.bundle_version !== "string" ||
                payload.bundle_version.length === 0 ||
                !Number.isSafeInteger(payload.bundle_bytes) ||
                payload.bundle_bytes < 0 ||
                (!shipsNoCss && !shipsCss) ||
                !Number.isFinite(payload.request_latency_ms) ||
                !Number.isFinite(payload.retrieval_ms) ||
                !Number.isFinite(payload.execution_ms) ||
                typeof payload.marker !== "string" ||
                payload.marker.length === 0 ||
                payload.document_execution_count !== 1
            ) {
                request.reject(new Error("Native mod bundle response schema is invalid"));
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
        } else if (response.ok === true && response.payload?.type === "plugin_storage_read") {
            const payload = response.payload.payload;
            if (
                !payload ||
                typeof payload.data !== "string" ||
                textEncoder.encode(payload.data).length > MAX_PLUGIN_STORAGE_BYTES ||
                !Number.isSafeInteger(payload.payload_bytes) ||
                payload.payload_bytes !== textEncoder.encode(payload.data).length
            ) {
                request.reject(new Error("Native plugin storage response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "plugin_storage_write") {
            const payload = response.payload.payload;
            if (
                !payload ||
                !Number.isSafeInteger(payload.payload_bytes) ||
                payload.payload_bytes < 0 ||
                payload.payload_bytes > MAX_PLUGIN_STORAGE_BYTES
            ) {
                request.reject(new Error("Native plugin storage response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "bundles_list") {
            const payload = response.payload.payload;
            const entries = payload?.bundles;
            const namesMatch =
                Array.isArray(entries) &&
                entries.length === BUNDLE_NAMES.length &&
                entries.every((entry, index) => entry?.name === BUNDLE_NAMES[index]);
            const shapesValid =
                Array.isArray(entries) &&
                entries.every(
                    (entry) =>
                        entry &&
                        typeof entry.present === "boolean" &&
                        Number.isSafeInteger(entry.bytes) &&
                        entry.bytes >= 0 &&
                        // Absent entries carry no size; present entries carry
                        // the host-observed length (no upper bound: nothing
                        // is allocated from this metadata).
                        (entry.present || entry.bytes === 0) &&
                        // Metadata only: no path, URL, or content may ride
                        // along with an entry.
                        entry.path === undefined &&
                        entry.url === undefined &&
                        entry.data === undefined,
                );
            if (!payload || !namesMatch || !shapesValid) {
                request.reject(new Error("Native bundle inventory response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "folders_opened") {
            const payload = response.payload.payload;
            if (
                !payload ||
                Object.keys(payload).length !== 1 ||
                typeof payload.location !== "string" ||
                !FOLDER_LOCATIONS.includes(payload.location)
            ) {
                request.reject(new Error("Native folder response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "theme_folder_opened") {            const payload = response.payload.payload;
            if (
                !payload ||
                Object.keys(payload).length !== 1 ||
                typeof payload.id !== "string" ||
                !THEME_ID_PATTERN.test(payload.id) ||
                payload.id === "." ||
                payload.id === ".." ||
                textEncoder.encode(payload.id).length > 64
            ) {
                request.reject(new Error("Native theme folder response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "theme_installed") {
            const payload = response.payload.payload;
            // The id is host-derived (`<name>-BD`); the theme stays
            // disabled by Electron parity, so `enabled` must be false.
            if (
                !payload ||
                Object.keys(payload).length !== 3 ||
                typeof payload.id !== "string" ||
                !THEME_ID_PATTERN.test(payload.id) ||
                !payload.id.endsWith("-BD") ||
                textEncoder.encode(payload.id).length > 64 ||
                payload.enabled !== false ||
                !Number.isSafeInteger(payload.css_bytes) ||
                payload.css_bytes < 0 ||
                payload.css_bytes > MAX_THEME_CSS_BYTES
            ) {
                request.reject(new Error("Native theme install response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "theme_written") {
            const payload = response.payload.payload;
            if (
                !payload ||
                Object.keys(payload).length !== 3 ||
                typeof payload.id !== "string" ||
                !THEME_ID_PATTERN.test(payload.id) ||
                payload.id === "." ||
                payload.id === ".." ||
                textEncoder.encode(payload.id).length > 64 ||
                typeof payload.enabled !== "boolean" ||
                !Number.isSafeInteger(payload.css_bytes) ||
                payload.css_bytes < 0 ||
                payload.css_bytes > MAX_THEME_CSS_BYTES
            ) {
                request.reject(new Error("Native theme write response schema is invalid"));
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
        getModBundle,
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
    const pluginStorage = Object.freeze({ readFile: readPluginFile, writeFile: writePluginFile });
    // Backup discovery only: save/restore (dialogs + zip) are deferred, so
    // no save/restore member may exist on this namespace.
    const backup = Object.freeze({ listBundles });
    // Folder opening only: reveal for the three fixed roots, open for one
    // validated theme id. No path, dialog, picker, or shell surface.
    const folders = Object.freeze({ open: openFolder, openTheme: openThemeFolder });
    // Theme installation only: CSS text in, bounded metadata out. No URL,
    // path, dialog, picker, editor, or list surface.
    // Theme CSS replacement only: validated id plus CSS text in, bounded
    // metadata out. No path, filename, URL, destination, or enablement flag.
    const themes = Object.freeze({ installFromCode: installThemeFromCode, writeCss: writeThemeCss });

    Object.defineProperty(window, "legcord", {
        configurable: false,
        enumerable: true,
        value: Object.freeze({ boot: snapshot, native, settings, getLang, pluginStorage, backup, folders, themes }),
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

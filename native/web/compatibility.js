(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic script, not a module.
    "use strict";

    const startedAt = performance.now();
    const bootSnapshot = __REMCORD_BOOT_SNAPSHOT__;
    const settingsBootData = __REMCORD_SETTINGS_BOOT_SNAPSHOT__;
    const hostPlatformData = __REMCORD_HOST_PLATFORM__;
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
        return `remcord-${Date.now().toString(36)}-${sequence}`;
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

    // Error-forward flood guard: identical consecutive renderer errors
    // (chatty WebRTC/Discord internals during voice/screenshare, repeated
    // mod-boot logs) collapse into one host event plus a repeat count, and
    // sustained floods of distinct messages are capped per window with an
    // overflow summary. Kinds and first-seen messages are preserved; only
    // exact duplicates collapse and only over-cap distinct messages drop —
    // and both are counted exactly in the flushed summary, never silently.
    // Counters are observable via the overflow summaries in host logs.
    const rendererErrorFlood = { key: "", repeats: 0, dropped: 0, sent: 0, windowStart: 0 };
    const RENDERER_ERROR_FLOOD_WINDOW_MS = 5000;
    const RENDERER_ERROR_FLOOD_CAP = 20;
    function reportRendererError(kind, message) {
        try {
            const text = String(message);
            const key = `${kind}\n${text}`;
            const now = Date.now();
            if (now - rendererErrorFlood.windowStart > RENDERER_ERROR_FLOOD_WINDOW_MS) {
                rendererErrorFlood.windowStart = now;
                rendererErrorFlood.sent = 0;
            }
            if (key === rendererErrorFlood.key) {
                rendererErrorFlood.repeats += 1;
                return;
            }
            if (rendererErrorFlood.repeats > 0 || rendererErrorFlood.dropped > 0) {
                // Flush the collapsed/dropped run before the new distinct
                // message so host logs keep exact counts, never silent drops.
                const collapsed = rendererErrorFlood.repeats;
                const dropped = rendererErrorFlood.dropped;
                rendererErrorFlood.repeats = 0;
                rendererErrorFlood.dropped = 0;
                sendEvent({
                    type: "renderer_error",
                    payload: {
                        kind: "error_flood_summary",
                        message: truncateUtf8(
                            `collapsed ${collapsed} duplicate(s), capped ${dropped} (kind=${kind})`,
                            2048,
                        ),
                    },
                });
            }
            rendererErrorFlood.key = key;
            if (rendererErrorFlood.sent >= RENDERER_ERROR_FLOOD_CAP) {
                rendererErrorFlood.dropped += 1;
                return;
            }
            rendererErrorFlood.sent += 1;
            sendEvent({
                type: "renderer_error",
                payload: { kind, message: truncateUtf8(text, 2048) },
            });
        } catch {
            console.error("[Remcord native] renderer error could not be sent to the host", {
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

    // Per-document mutable working copies for the closed settings shim (see
    // below): same identity on every call of one document, fresh and
    // independent on the next. Declared here because getConfig is defined
    // alongside the other capabilities; assigned on first use after the
    // frozen snapshot exists. Calls happen after full script evaluation, so
    // there is no ordering hazard.
    let settingsWorkingCopy;
    let themesWorkingCopy;

    function getConfig(timeoutMs = 5000) {
        requireCapability("settings.get_config");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Dual contract for the closed settings shim: synchronous callers
        // (the real settings plugin) receive the launch-frozen snapshot as a
        // per-document mutable working copy — the same identity on every call
        // of this document, mirroring Electron's cached config object — while
        // awaited callers keep the exact live async capability. The live
        // request still fires eagerly, so async behavior, metrics, timeouts,
        // disposal, and response-schema validation are unchanged.
        if (settingsWorkingCopy === undefined) {
            // A null snapshot (corrupt settings on disk) must still hand the
            // plugin a working object: an empty one, honestly empty, so boot
            // degrades instead of throwing on the first property read. Full
            // execution stays gated on settingsSource upstream.
            const clone = cloneJson(settingsBootSnapshot.settings, {});
            settingsWorkingCopy = clone !== null && typeof clone === "object" ? clone : {};
        }
        const snapshot = settingsWorkingCopy;
        const live = request({ type: "settings_get_config", payload: {} }, timeoutMs).then(
            (payload) => payload.settings,
        );
        // Settle only this eager branch: awaited callers attach their own
        // branch through `then` below and still observe host failures.
        // Without this, a never-awaited synchronous call could surface an
        // unhandled rejection when the host reports unavailable settings.
        live.catch(() => {});
        // Intentional thenable for the dual contract (see above): the
        // snapshot value stays synchronously readable while awaited callers
        // keep exact async behavior; sync callers ignore `then`.
        // biome-ignore lint/suspicious/noThenProperty: intentional thenable for the closed settings shim
        Object.defineProperty(snapshot, "then", {
            configurable: true,
            writable: true,
            value: (resolve, reject) => {
                live.then(resolve, reject);
            },
        });
        return snapshot;
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

    // Closed allowlist for the `settings.set` capability: the exact wire keys
    // the host enum accepts. Anything else is rejected before dispatch, and
    // the host re-validates the enum plus the key/value pairing.
    const SETTINGS_SET_KEYS = [
        "mods",
        "support_banner_dismissed",
        "show_experimental_plugin_menu",
        "extended_plugin_abilities",
        "csp",
    ];
    // Exact wire types the host echoes in `window_ack` responses. The echo
    // must be one of these; an acknowledgement for any other (or missing)
    // command never resolves a window call.
    const WINDOW_ACK_COMMANDS = [
        "window_show",
        "window_hide",
        "window_minimize",
        "window_maximize",
        "window_unmaximize",
        "window_drag",
        "window_toggle_fullscreen",
        "window_quit",
    ];
    // Exact ValidMods union (src/@types/settings.d.ts). The renderer cannot
    // invent a mod id through this capability.
    const SETTINGS_VALID_MODS = ["vencord", "equicord", "custom", "shelter"];
    const MAX_SETTINGS_MODS_ENTRIES = 8;
    // Exact CSP modes the host implements (Electron csp.ts parity). The
    // renderer cannot invent a policy through this capability.
    const SETTINGS_VALID_CSP_MODES = ["none", "strict", "vanilla"];

    function setSetting(key, value, timeoutMs = 5000) {
        requireCapability("settings.set");
        if (typeof key !== "string" || !SETTINGS_SET_KEYS.includes(key)) {
            return Promise.reject(new TypeError("key must be one of the allowlisted settings keys"));
        }
        if (key === "mods") {
            if (
                !Array.isArray(value) ||
                value.length > MAX_SETTINGS_MODS_ENTRIES ||
                !value.every((entry) => typeof entry === "string" && SETTINGS_VALID_MODS.includes(entry))
            ) {
                return Promise.reject(new TypeError("value must be an array of known mod ids"));
            }
        } else if (key === "csp") {
            if (typeof value !== "string" || !SETTINGS_VALID_CSP_MODES.includes(value)) {
                return Promise.reject(new TypeError("value must be one of the supported CSP modes"));
            }
        } else if (typeof value !== "boolean") {
            return Promise.reject(new TypeError("value must be a boolean for this key"));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Key plus bounded value only: the host owns the settings path,
        // merges the single field, preserves unknown keys, and persists
        // atomically. No path, URL, object, or second field may be supplied.
        return request({ type: "settings_set", payload: { key, value } }, timeoutMs).then((payload) => {
            const echoedValue =
                key === "mods"
                    ? Array.isArray(payload.value) &&
                      payload.value.length <= MAX_SETTINGS_MODS_ENTRIES &&
                      payload.value.every((entry) => typeof entry === "string" && SETTINGS_VALID_MODS.includes(entry))
                        ? payload.value
                        : undefined
                    : key === "csp"
                      ? typeof payload.value === "string" && SETTINGS_VALID_CSP_MODES.includes(payload.value)
                          ? payload.value
                          : undefined
                      : typeof payload.value === "boolean"
                        ? payload.value
                        : undefined;
            if (
                !payload ||
                Object.keys(payload).length !== 4 ||
                payload.key !== key ||
                echoedValue === undefined ||
                payload.source !== "file" ||
                !Number.isSafeInteger(payload.payload_bytes)
            ) {
                throw new Error("Native settings set response schema is invalid");
            }
            return { key: payload.key, value: echoedValue };
        });
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

    function setThemeEnabled(id, enabled, timeoutMs = 5000) {
        requireCapability("themes.set_enabled");
        if (typeof id !== "string" || !THEME_ID_PATTERN.test(id) || id === "." || id === "..") {
            return Promise.reject(new TypeError("id must match [A-Za-z0-9._-]{1,64} and not be . or .."));
        }
        if (typeof enabled !== "boolean") {
            return Promise.reject(new TypeError("enabled must be a boolean"));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Id plus boolean only: the host derives the manifest path from
        // the validated id and flips only the `enabled` field. No path,
        // filename, manifest, CSS, URL, or destination may be supplied.
        // The existing host lifecycle observes the rewrite and applies the
        // managed CSS; this call never touches styles directly.
        return request({ type: "theme_set_enabled", payload: { id, enabled } }, timeoutMs).then((payload) => {
            if (!payload || Object.keys(payload).length !== 2 || payload.id !== id || payload.enabled !== enabled) {
                throw new Error("Native theme enable response schema is invalid");
            }
            return payload;
        });
    }

    function importFiles(timeoutMs = 120_000) {
        requireCapability("themes.import_files");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Parameter-free by protocol: the host owns the entire picker
        // configuration (translated title, Import action label, filters,
        // multi-selection). No path, URL, filter, or dialog option may be
        // supplied. The modal picker stays open as long as the user needs,
        // so this uses the long-operation timeout like the mod bundle
        // retrieval rather than the ordinary 5-second command timeout.
        // Cancellation resolves { canceled: true }, never a rejection.
        return request({ type: "theme_import_files", payload: {} }, timeoutMs).then((payload) => {
            if (
                !payload ||
                Object.keys(payload).length !== 3 ||
                typeof payload.canceled !== "boolean" ||
                !Number.isSafeInteger(payload.imported) ||
                payload.imported < 0 ||
                !Number.isSafeInteger(payload.failed) ||
                payload.failed < 0
            ) {
                throw new Error("Native theme import response schema is invalid");
            }
            return payload;
        });
    }

    // Maximum inventoried themes per response and maximum single display
    // string, mirroring the host bounds. A hostile themes directory cannot
    // widen the bridge response past these.
    const MAX_THEME_LIST_ENTRIES = 256;
    const MAX_THEME_LIST_FIELD_BYTES = 4096;
    // Maximum backup-save request in UTF-8 bytes, mirroring the host
    // transport bound (4 MiB clientMods envelope plus slack). Larger
    // payloads are rejected before dispatch.
    const MAX_BACKUP_SAVE_MESSAGE_BYTES = 4259840;
    // Maximum single client-mod string in UTF-8 bytes, mirroring the host
    // per-string bound. Larger snapshots are rejected before dispatch.
    const MAX_CLIENT_MOD_STRING_BYTES = 1048576;
    // The exact seven include flags Electron's backup UI sends. Unknown
    // keys are rejected: the renderer cannot invent an archive section.
    const BACKUP_INCLUDE_KEYS = [
        "remcordConfig",
        "remcordThemesAndQuickCss",
        "remcordExtensionPlugins",
        "vencordModData",
        "equicordModData",
        "shelterModData",
        "modBundles",
    ];
    // The exact client-mod envelope keys. No additional field may be
    // supplied (no path, filename, archive path, source, or root).
    const BACKUP_CLIENT_MOD_KEYS = ["vencordLocalStorage", "equicordLocalStorage", "shelter"];

    function listThemes(timeoutMs = 5000) {
        requireCapability("themes.list");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Parameter-free by protocol: the host owns the root, enumeration,
        // ordering, and bounds. No path, root, selector, URL, or filter may
        // be supplied. The host returns bounded display metadata only.
        return request({ type: "themes_list", payload: {} }, timeoutMs).then((payload) => payload.themes);
    }

    function listBundles(timeoutMs = 5000) {
        requireCapability("backup.list_bundles");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Parameter-free by protocol: no path, URL, include flag, or
        // selector may be supplied. The host inventories the fixed
        // allowlist only.
        return request({ type: "bundles_list", payload: {} }, timeoutMs).then((payload) => payload.bundles);
    }

    function saveBackup(payloadString, timeoutMs = 120_000) {
        requireCapability("backup.save");
        if (typeof payloadString !== "string") {
            return Promise.reject(new TypeError("payload must be a JSON string"));
        }
        if (textEncoder.encode(payloadString).length > MAX_BACKUP_SAVE_MESSAGE_BYTES) {
            return Promise.reject(new TypeError("payload exceeds the backup size limit"));
        }
        let parsed;
        try {
            parsed = JSON.parse(payloadString);
        } catch {
            return Promise.reject(new TypeError("payload must be valid JSON"));
        }
        // The exact Electron save shape ({ includes, clientMods }): no
        // output path, filename, archive path, source path, dialog option,
        // or root may ride along. The host owns collection, the manifest,
        // ZIP creation, the dialog, and the final write.
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
            return Promise.reject(new TypeError("payload must be an object"));
        }
        const topKeys = Object.keys(parsed);
        if (topKeys.length !== 2 || !topKeys.includes("includes") || !topKeys.includes("clientMods")) {
            return Promise.reject(new TypeError("payload must carry exactly includes and clientMods"));
        }
        const includes = parsed.includes;
        if (!includes || typeof includes !== "object" || Array.isArray(includes)) {
            return Promise.reject(new TypeError("includes must be an object"));
        }
        const includeKeys = Object.keys(includes);
        if (
            includeKeys.length !== BACKUP_INCLUDE_KEYS.length ||
            !BACKUP_INCLUDE_KEYS.every((key) => typeof includes[key] === "boolean")
        ) {
            return Promise.reject(new TypeError("includes must carry exactly the seven known booleans"));
        }
        const clientMods = parsed.clientMods;
        if (!clientMods || typeof clientMods !== "object" || Array.isArray(clientMods)) {
            return Promise.reject(new TypeError("clientMods must be an object"));
        }
        for (const key of Object.keys(clientMods)) {
            if (!BACKUP_CLIENT_MOD_KEYS.includes(key)) {
                return Promise.reject(new TypeError("clientMods carries an unknown field"));
            }
        }
        for (const key of ["vencordLocalStorage", "equicordLocalStorage"]) {
            const value = clientMods[key];
            if (value !== undefined && value !== null) {
                if (typeof value !== "string" || textEncoder.encode(value).length > MAX_CLIENT_MOD_STRING_BYTES) {
                    return Promise.reject(new TypeError(`${key} must be a string of at most 1048576 UTF-8 bytes`));
                }
            }
        }
        if (clientMods.shelter !== undefined && clientMods.shelter !== null && typeof clientMods.shelter !== "object") {
            return Promise.reject(new TypeError("shelter must be an object"));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // The modal picker stays open as long as the user needs, so this
        // uses the long-operation timeout like the mod bundle retrieval and
        // the theme import rather than the ordinary 5-second command
        // timeout. The Electron result shape is preserved ({ ok } /
        // { ok: false, error }): cancellation resolves { ok: false, error:
        // "CANCELLED" } (silent typed control flow, never a rejection), and
        // host failures resolve { ok: false, error: <typed code> } so the
        // existing backup UI keeps working without a native branch.
        return request({ type: "backup_save", payload: { includes, clientMods } }, timeoutMs).then(
            (payload) => (payload.canceled === true ? { ok: false, error: "CANCELLED" } : { ok: true }),
            (error) => ({
                ok: false,
                error: typeof error?.code === "string" ? error.code : "UNKNOWN",
            }),
        );
    }

    // The exact client-mod envelope keys the host may return (the same three
    // the save path sends, in BACKUP_CLIENT_MOD_KEYS above). No additional
    // field may ride along.
    function isBoundedClientMods(value) {
        if (!value || typeof value !== "object" || Array.isArray(value)) return false;
        for (const key of Object.keys(value)) {
            if (!BACKUP_CLIENT_MOD_KEYS.includes(key)) return false;
        }
        for (const key of ["vencordLocalStorage", "equicordLocalStorage"]) {
            const entry = value[key];
            if (entry !== undefined && entry !== null) {
                if (typeof entry !== "string" || textEncoder.encode(entry).length > MAX_CLIENT_MOD_STRING_BYTES) {
                    return false;
                }
            }
        }
        const shelter = value.shelter;
        if (shelter !== undefined && shelter !== null && (typeof shelter !== "object" || Array.isArray(shelter))) {
            return false;
        }
        return true;
    }

    function restoreBackup(timeoutMs = 120_000) {
        requireCapability("backup.restore");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Parameter-free by protocol: the host owns the entire open picker
        // plus validation, staging, and commit. No archive path, filename,
        // URL, filter, or dialog option may be supplied. The modal picker
        // stays open as long as the user needs, so this uses the
        // long-operation timeout like save and the theme import.
        // Cancellation resolves { ok: false, error: "CANCELLED" } (silent
        // typed control flow, never a rejection); host failures resolve
        // { ok: false, error: <typed code> }. Success resolves
        // { ok: true, clientMods } for the existing renderer mechanism to
        // apply — the host never applies clientMods itself.
        return request({ type: "backup_restore", payload: {} }, timeoutMs).then(
            (payload) => {
                if (payload.canceled === true) return { ok: false, error: "CANCELLED" };
                if (!isBoundedClientMods(payload.client_mods)) {
                    throw new Error("Native backup restore response schema is invalid");
                }
                return { ok: true, clientMods: payload.client_mods };
            },
            (error) => ({
                ok: false,
                error: typeof error?.code === "string" ? error.code : "UNKNOWN",
            }),
        );
    }

    window.addEventListener("remcord-native:response", (event) => {
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
        } else if (response.ok === true && response.payload?.type === "settings_set") {
            const payload = response.payload.payload;
            // Key echo plus normalized value only: the echo must be an
            // allowlisted key, the value must match that key's shape, and no
            // path, URL, snapshot, or extra member may ride along.
            const valueValid =
                payload?.key === "mods"
                    ? Array.isArray(payload?.value) &&
                      payload.value.length <= MAX_SETTINGS_MODS_ENTRIES &&
                      payload.value.every((entry) => typeof entry === "string" && SETTINGS_VALID_MODS.includes(entry))
                    : payload?.key === "csp"
                      ? typeof payload?.value === "string" && SETTINGS_VALID_CSP_MODES.includes(payload.value)
                      : SETTINGS_SET_KEYS.includes(payload?.key) && typeof payload?.value === "boolean";
            if (
                !payload ||
                Object.keys(payload).length !== 4 ||
                !SETTINGS_SET_KEYS.includes(payload.key) ||
                !valueValid ||
                payload.source !== "file" ||
                !Number.isSafeInteger(payload.payload_bytes)
            ) {
                request.reject(new Error("Native settings set response schema is invalid"));
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
        } else if (response.ok === true && response.payload?.type === "theme_folder_opened") {
            const payload = response.payload.payload;
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
        } else if (response.ok === true && response.payload?.type === "theme_imported") {
            const payload = response.payload.payload;
            // Bounded summary only: cancellation plus counts. No ids,
            // arrays, paths, URLs, or source echo may ride along.
            if (
                !payload ||
                Object.keys(payload).length !== 3 ||
                typeof payload.canceled !== "boolean" ||
                !Number.isSafeInteger(payload.imported) ||
                payload.imported < 0 ||
                !Number.isSafeInteger(payload.failed) ||
                payload.failed < 0
            ) {
                request.reject(new Error("Native theme import response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "theme_enabled") {
            const payload = response.payload.payload;
            // Id echo plus persisted state only. The echo must equal the
            // request so one theme's flip can never resolve another's call.
            if (
                !payload ||
                Object.keys(payload).length !== 2 ||
                typeof payload.id !== "string" ||
                !THEME_ID_PATTERN.test(payload.id) ||
                payload.id === "." ||
                payload.id === ".." ||
                textEncoder.encode(payload.id).length > 64 ||
                typeof payload.enabled !== "boolean"
            ) {
                request.reject(new Error("Native theme enable response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "themes_list") {
            const payload = response.payload.payload;
            // Bounded display metadata only: an array of exact-shape
            // entries. No paths, no CSS contents, no URLs, no extra
            // members anywhere in the envelope.
            const entries = payload?.themes;
            const shapesValid =
                Array.isArray(entries) &&
                entries.length <= MAX_THEME_LIST_ENTRIES &&
                entries.every(
                    (entry) =>
                        entry &&
                        Object.keys(entry).length === 6 &&
                        typeof entry.id === "string" &&
                        THEME_ID_PATTERN.test(entry.id) &&
                        entry.id !== "." &&
                        entry.id !== ".." &&
                        textEncoder.encode(entry.id).length <= 64 &&
                        typeof entry.name === "string" &&
                        textEncoder.encode(entry.name).length <= MAX_THEME_LIST_FIELD_BYTES &&
                        typeof entry.author === "string" &&
                        textEncoder.encode(entry.author).length <= MAX_THEME_LIST_FIELD_BYTES &&
                        typeof entry.description === "string" &&
                        textEncoder.encode(entry.description).length <= MAX_THEME_LIST_FIELD_BYTES &&
                        typeof entry.version === "string" &&
                        textEncoder.encode(entry.version).length <= MAX_THEME_LIST_FIELD_BYTES &&
                        typeof entry.enabled === "boolean",
                );
            if (!payload || !shapesValid) {
                request.reject(new Error("Native theme list response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "window_ack") {
            const payload = response.payload.payload;
            // Command echo only: it must be one of the closed window wire
            // types, and no state, handle, or geometry may ride along.
            // (Previously this branch did not exist, so every window-button
            // click rejected as "Native bridge request failed" and logged a
            // bogus renderer error via the unhandled-rejection reporter.)
            if (!payload || Object.keys(payload).length !== 1 || !WINDOW_ACK_COMMANDS.includes(payload.command)) {
                request.reject(new Error("Native window acknowledgement response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "backup_saved") {
            const payload = response.payload.payload;
            // Bounded save outcome only: cancellation plus the final archive
            // size. No path, filename, URL, source echo, or filesystem error
            // may ride along.
            if (
                !payload ||
                Object.keys(payload).length !== 2 ||
                typeof payload.canceled !== "boolean" ||
                !Number.isSafeInteger(payload.bytes) ||
                payload.bytes < 0
            ) {
                request.reject(new Error("Native backup save response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "backup_restored") {
            const payload = response.payload.payload;
            // Bounded restore outcome only: cancellation plus the filtered
            // clientMods envelope. No archive path, profile path, filename,
            // URL, or filesystem error may ride along. Cancellation carries
            // a null envelope; success carries the bounded object the
            // existing renderer mechanism applies (the host never applies it).
            const envelopeValid =
                payload?.canceled === true
                    ? payload.client_mods === null || payload.client_mods === undefined
                    : isBoundedClientMods(payload?.client_mods);
            if (
                !payload ||
                Object.keys(payload).length !== 2 ||
                typeof payload.canceled !== "boolean" ||
                !envelopeValid
            ) {
                request.reject(new Error("Native backup restore response schema is invalid"));
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
    // Local window state seeded from boot snapshot. The window is created
    // hidden, so both focused and maximized start as false. The typed
    // `window_state` host event converges the authoritative state.
    let _windowFocused = snapshot.window?.focused === true;
    let windowMaximized = snapshot.window?.maximized === true;

    window.addEventListener("remcord-native:window-state", (event) => {
        const detail = event?.detail;
        if (!detail || typeof detail !== "object") return;
        if (detail.version !== protocolVersion) return;
        if (typeof detail.focused !== "boolean" || typeof detail.maximized !== "boolean") return;
        _windowFocused = detail.focused;
        windowMaximized = detail.maximized;
        // Mirror Electron's body-attribute writes (window.ts focus/blur and
        // maximize/unmaximize handlers) in the same event — no polling, no
        // extra listener — so titlebar CSS ([isMaximized], [unFocused])
        // reflects maximized/restored/focused state.
        if (typeof document !== "undefined" && document && document.body) {
            if (detail.maximized) document.body.setAttribute("isMaximized", "");
            else document.body.removeAttribute("isMaximized");
            if (detail.focused) document.body.removeAttribute("unFocused");
            else document.body.setAttribute("unFocused", "");
        }
    });

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
        window: Object.freeze({
            show() {
                requireCapability("window.show");
                return request({ type: "window_show", payload: {} });
            },
            hide() {
                requireCapability("window.hide");
                return request({ type: "window_hide", payload: {} });
            },
            minimize() {
                requireCapability("window.minimize");
                return request({ type: "window_minimize", payload: {} });
            },
            maximize() {
                requireCapability("window.maximize");
                return request({ type: "window_maximize", payload: {} });
            },
            unmaximize() {
                requireCapability("window.unmaximize");
                return request({ type: "window_unmaximize", payload: {} });
            },
            drag() {
                requireCapability("window.drag");
                return request({ type: "window_drag", payload: {} });
            },
            toggleFullscreen() {
                requireCapability("window.toggle_fullscreen");
                return request({ type: "window_toggle_fullscreen", payload: {} });
            },
            quit() {
                requireCapability("window.quit");
                return request({ type: "window_quit", payload: {} });
            },
            // Synchronous local-state reads — NO synchronous native bridge
            // call, NO polling. Seeded from boot snapshot and converged by
            // the typed window_state host event.
            maximized() {
                return windowMaximized;
            },
            isNormal() {
                return !windowMaximized;
            },
        }),
    });
    // Frameless chrome has no system menu, so F11 is bound here (capture
    // phase, every document) to the single fixed toggle_fullscreen
    // operation. The host owns the fullscreen state; the key carries no
    // payload and no other key is intercepted.
    window.addEventListener(
        "keydown",
        (event) => {
            if (event?.key !== "F11" || event.defaultPrevented) return;
            event.preventDefault();
            let pending;
            try {
                pending = native.window.toggleFullscreen();
            } catch {
                return;
            }
            if (pending && typeof pending.catch === "function") pending.catch(() => {});
        },
        true,
    );
    // (settingsShim/themesShim below replace these narrow namespaces with
    // the shimmed versions; the originals are kept out of the surface so no
    // duplicate member can drift.)
    const pluginStorage = Object.freeze({ readFile: readPluginFile, writeFile: writePluginFile });
    // Backup save plus discovery: `save` takes the existing Electron JSON
    // string ({ includes, clientMods }) and resolves the existing result
    // shape ({ ok } / { ok: false, error }); `restore` is parameter-free and
    // resolves { ok: true, clientMods } / { ok: false, error } with the same
    // silent CANCELLED control flow.
    const backup = Object.freeze({ listBundles, save: saveBackup, restore: restoreBackup });
    // Folder opening only: reveal for the three fixed roots, open for one
    // validated theme id. No path, dialog, picker, or shell surface.
    const folders = Object.freeze({ open: openFolder, openTheme: openThemeFolder });
    // Theme installation only: CSS text in, bounded metadata out. No URL,
    // path, dialog, picker, editor, or list surface.
    // Theme CSS replacement only: validated id plus CSS text in, bounded
    // metadata out. No path, filename, URL, destination, or enablement flag.
    // Theme file import only: an exact empty payload in, a bounded
    // cancellation/counts summary out. No path, URL, filter, dialog-option,
    // picker, or filesystem surface.
    // Theme enable flip only: validated id plus boolean in, id echo plus
    // persisted state out. No path, filename, manifest, CSS, URL, or
    // destination surface. The host applies the change through its own
    // existing lifecycle.
    // Theme inventory only: an exact empty payload in, a bounded display
    // array out. No path, root, selector, URL, filter, CSS, or filesystem
    // surface. (The frozen namespace is assembled as themesShim below, which
    // additionally carries the closed synchronous getThemes.)

    // Synchronous settings boot snapshot (3d-2 prerequisite B). Host-created
    // data, spliced per launch before the mod adapters and deeply frozen
    // here, so it exists before the settings plugin could execute. The five
    // data members are startup values only; the two functions are narrow
    // fire-and-forget senders over closed one-way events (one bounded locale,
    // one empty payload). No generic invocation, no eval, no renderer-supplied
    // values, no paths, no URLs.
    const LOCALE_PATTERN = /^[A-Za-z0-9_-]{1,32}$/;

    function bootSetLang(locale) {
        if (typeof locale !== "string" || !LOCALE_PATTERN.test(locale) || textEncoder.encode(locale).length > 32) {
            throw new TypeError("locale must match [A-Za-z0-9_-]{1,32}");
        }
        sendEvent({ type: "settings_boot_set_lang", payload: { locale } });
    }

    function bootRestart() {
        sendEvent({ type: "settings_boot_restart", payload: {} });
    }

    const settingsBootSnapshot = freezeDeep(settingsBootData);
    const settingsBoot = Object.freeze({
        settingsSource: settingsBootSnapshot.settingsSource,
        settings: settingsBootSnapshot.settings,
        themes: settingsBootSnapshot.themes,
        lang: settingsBootSnapshot.lang,
        i18n: settingsBootSnapshot.i18n,
        setLang: bootSetLang,
        restart: bootRestart,
    });

    // Closed settings compatibility shim (3d-2): the exact synchronous names
    // the real settings plugin reads during onLoad(), backed ONLY by the
    // frozen snapshot above. No filesystem, no synchronous IPC, no fabricated
    // values, no generic invocation. Per-document mutable working copies give
    // the plugin Electron-faithful identity (the main-process cache object is
    // equally shared and mutable) while the host snapshot stays frozen.
    function cloneJson(value, fallback) {
        try {
            const text = JSON.stringify(value);
            if (text === undefined) return fallback;
            return JSON.parse(text);
        } catch {
            return fallback;
        }
    }

    // Project one raw snapshot manifest onto the closed display shape. Only
    // entries with a usable id/name/enabled survive; missing optional text
    // becomes "" so every installed theme stays inventoried (Electron lists
    // manifests with missing optionals too). Only the six display fields
    // cross — never paths, stylesheets, remote references, or any other
    // manifest property.
    function toShimTheme(entry) {
        if (!entry || typeof entry !== "object") return null;
        if (typeof entry.id !== "string" || typeof entry.name !== "string") return null;
        if (typeof entry.enabled !== "boolean") return null;
        const text = (value) => (typeof value === "string" ? value : "");
        return {
            id: entry.id,
            name: entry.name,
            author: text(entry.author),
            description: text(entry.description),
            version: text(entry.version),
            enabled: entry.enabled,
        };
    }

    function getThemesSync() {
        if (themesWorkingCopy === undefined) {
            const raw = Array.isArray(settingsBootSnapshot.themes) ? settingsBootSnapshot.themes : [];
            const projected = [];
            for (const entry of raw) {
                const theme = toShimTheme(entry);
                if (theme) projected.push(theme);
            }
            themesWorkingCopy = projected;
        }
        return themesWorkingCopy;
    }

    // Host platform identity for the closed shim: static launch-time strings
    // only (process.platform / os.release() parity), no capability, no
    // request, no renderer input. `platform` renders section gates;
    // `osRelease` keeps render-time version comparisons from throwing.
    const hostPlatform = freezeDeep(hostPlatformData);
    const settingsShim = Object.freeze({
        getConfig,
        setBlockPowerSavingInVoiceChat,
        set: setSetting,
        setLang: bootSetLang,
    });
    const themesShim = Object.freeze({
        installFromCode: installThemeFromCode,
        writeCss: writeThemeCss,
        importFiles,
        setEnabled: setThemeEnabled,
        listThemes,
        getThemes: getThemesSync,
    });

    Object.defineProperty(window, "remcord", {
        configurable: false,
        enumerable: true,
        value: Object.freeze({
            boot: snapshot,
            native,
            // Window API at top level for Electron compatibility.
            // The native path uses `window.remcord.native.window.*` for
            // capability-gated async operations and `window.remcord.window.*`
            // for synchronous local-state reads. Electron's bridge exposes
            // all under `window.remcord.window` directly; the native facade
            // mirrors both for exactly-one-path dispatch.
            window: Object.freeze({
                show: native.window.show,
                hide: native.window.hide,
                minimize: native.window.minimize,
                maximize: native.window.maximize,
                unmaximize: native.window.unmaximize,
                drag: native.window.drag,
                toggleFullscreen: native.window.toggleFullscreen,
                quit: native.window.quit,
                maximized: native.window.maximized,
                isNormal: native.window.isNormal,
            }),
            settings: settingsShim,
            getLang,
            pluginStorage,
            backup,
            folders,
            themes: themesShim,
            settingsBoot,
            translations: settingsBootSnapshot.i18n,
            restart: bootRestart,
            platform: hostPlatform.platform,
            osRelease: hostPlatform.osRelease,
        }),
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

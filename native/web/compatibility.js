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

    // Entry origins for the three release channels (stable/canary/ptb):
    // the channel setting selects which one the host navigates to, and the
    // facade must initialize on any of them. Exact match only — no
    // subdomains, no other hosts.
    if (
        window.location.origin !== "https://discord.com" &&
        window.location.origin !== "https://canary.discord.com" &&
        window.location.origin !== "https://ptb.discord.com"
    )
        return;

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
        "quick_css",
        "channel",
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
    // Exact release channels the host implements (redirect.html parity).
    // The renderer cannot invent a channel or smuggle a URL through this
    // capability.
    const SETTINGS_VALID_CHANNELS = ["stable", "canary", "ptb"];

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
        } else if (key === "channel") {
            if (typeof value !== "string" || !SETTINGS_VALID_CHANNELS.includes(value)) {
                return Promise.reject(new TypeError("value must be one of the supported release channels"));
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
                      : key === "channel"
                        ? typeof payload.value === "string" && SETTINGS_VALID_CHANNELS.includes(payload.value)
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

    // Exact importable preference keys (settings_import.rs parity). The
    // response echoes subsets of this list only; anything else is rejected.
    // Restart-requiring keys mirror RESTART_REQUIRED_KEYS so the UI can
    // raise the restart banner without a second round trip.
    const SETTINGS_IMPORT_KEYS = [
        "mods",
        "csp",
        "channel",
        "supportBannerDismissed",
        "showExperimentalPluginMenu",
        "extendedPluginAbilities",
        "quickCss",
        "spellcheck",
        "smoothScroll",
        "autoScroll",
        "mobileMode",
        "skipSplash",
        "sleepInBackground",
        "popoutPiP",
        "bounceOnPing",
        "useSystemCssEditor",
        "minimizeToTray",
        "disableAutogain",
        "spellcheckLanguage",
        "scanInterval",
        "overlayButtonColor",
        "noBundleUpdates",
        "pluginStates",
    ];
    const SETTINGS_IMPORT_RESTART_KEYS = ["mods", "channel", "csp"];

    function importElectronSettings(timeoutMs = 30000) {
        requireCapability("settings.import_electron");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Parameter-free by protocol: the host builds the fixed Electron
        // source path itself and applies exactly its own allowlist. No
        // source path, URL, key list, or value may be supplied. Copy-only:
        // the Electron file is never written, and the import runs once
        // per explicit call — never at startup.
        return request({ type: "settings_import_electron", payload: {} }, timeoutMs).then((payload) => {
            const listsValid =
                payload &&
                Object.keys(payload).length === 2 &&
                Array.isArray(payload.applied) &&
                Array.isArray(payload.skipped) &&
                payload.applied.length + payload.skipped.length <= SETTINGS_IMPORT_KEYS.length &&
                [...payload.applied, ...payload.skipped].every(
                    (key) => typeof key === "string" && SETTINGS_IMPORT_KEYS.includes(key),
                ) &&
                new Set([...payload.applied, ...payload.skipped]).size ===
                    payload.applied.length + payload.skipped.length;
            if (!listsValid) {
                throw new Error("Native settings import response schema is invalid");
            }
            return {
                applied: [...payload.applied],
                skipped: [...payload.skipped],
                restartRequired: payload.applied.some((key) => SETTINGS_IMPORT_RESTART_KEYS.includes(key)),
            };
        });
    }

    function getVersion(timeoutMs = 5000) {
        requireCapability("app.get_version");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        return request({ type: "app_get_version", payload: {} }, timeoutMs).then((payload) => payload.version);
    }

    // Keybind management (F8 core): same member names as the Electron
    // bridge (`settings.addKeybind/toggleKeybind/removeKeybind`, which are
    // fire-and-forget sends), but async with typed outcomes — the host
    // validates, persists, and re-registers the global set. `listKeybinds`
    // serves the validated inventory the in-page matcher filters.
    const KEYBIND_ID_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;
    const MAX_KEYBIND_ACCELERATOR_BYTES = 32;
    const MAX_KEYBIND_LIST_ENTRIES = 64;
    // The eight executable actions (keybinds.rs parity). `runJavascript`
    // is structurally excluded: it has no wire shape and never executes.
    const KEYBIND_ACTIONS = [
        "mute",
        "deafen",
        "leaveCall",
        "pushToTalk",
        "navigateBack",
        "navigateForward",
        "openQuickCss",
        "openSettings",
    ];

    function checkKeybindId(id) {
        if (typeof id !== "string" || !KEYBIND_ID_PATTERN.test(id)) {
            return "id must match [A-Za-z0-9_-]{1,64}";
        }
        return null;
    }

    function checkKeybindAccelerator(accelerator) {
        if (
            typeof accelerator !== "string" ||
            accelerator.length === 0 ||
            textEncoder.encode(accelerator).length > MAX_KEYBIND_ACCELERATOR_BYTES
        ) {
            return "accelerator must be a non-empty string of at most 32 UTF-8 bytes";
        }
        return null;
    }

    function listKeybinds(timeoutMs = 5000) {
        requireCapability("keybinds.list");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Parameter-free by protocol: the host owns the inventory,
        // validation, and bounds. Entries carry metadata only. The
        // validator resolves the bare array for exact Electron parity.
        return request({ type: "keybinds_list", payload: {} }, timeoutMs);
    }

    function addKeybind(keybind, timeoutMs = 5000) {
        requireCapability("keybinds.add");
        if (!keybind || typeof keybind !== "object" || Array.isArray(keybind)) {
            return Promise.reject(new TypeError("keybind must be an object"));
        }
        const badId = checkKeybindId(keybind.id);
        if (badId) return Promise.reject(new TypeError(badId));
        const badAccelerator = checkKeybindAccelerator(keybind.accelerator);
        if (badAccelerator) return Promise.reject(new TypeError(badAccelerator));
        if (typeof keybind.action !== "string" || !KEYBIND_ACTIONS.includes(keybind.action)) {
            return Promise.reject(new TypeError("action must be one of the eight executable actions"));
        }
        if (typeof keybind.global !== "boolean" || typeof keybind.enabled !== "boolean") {
            return Promise.reject(new TypeError("global and enabled must be booleans"));
        }
        if (keybind.js !== undefined) {
            // Stored code can never cross: runJavascript entries are
            // refused outright, not sanitized.
            return Promise.reject(new TypeError("keybind must not carry executable code"));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Id plus accelerator plus action plus flags only: the host
        // validates the grammar, refuses duplicates, persists, and
        // re-registers. Typed failures resolve { ok: false, error } so
        // the maker can toast the reason.
        return request(
            {
                type: "keybinds_add",
                payload: {
                    id: keybind.id,
                    accelerator: keybind.accelerator,
                    action: keybind.action,
                    global: keybind.global,
                    enabled: keybind.enabled,
                },
            },
            timeoutMs,
        ).then((payload) => {
            // Id echo must equal the request: one entry's save can never
            // resolve another entry's call.
            if (payload.ok === true && payload.id !== keybind.id) {
                throw new Error("Native keybind save response schema is invalid");
            }
            return payload;
        });
    }

    function toggleKeybind(id, timeoutMs = 5000) {
        requireCapability("keybinds.toggle");
        const badId = checkKeybindId(id);
        if (badId) return Promise.reject(new TypeError(badId));
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Id only: the host flips the stored flag and re-registers.
        // Unknown ids resolve { ok: false } without writing.
        return request({ type: "keybinds_toggle", payload: { id } }, timeoutMs);
    }

    function removeKeybind(id, timeoutMs = 5000) {
        requireCapability("keybinds.remove");
        const badId = checkKeybindId(id);
        if (badId) return Promise.reject(new TypeError(badId));
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Id only: the host removes the entry and re-registers. Unknown
        // ids resolve { ok: false } without writing.
        return request({ type: "keybinds_remove", payload: { id } }, timeoutMs);
    }

    // Read-only host facts for the debug/GPU copy actions: app version,
    // WebView2 Runtime version, profile-kind label, channel, csp, mod ids.
    // Fixed-vocabulary strings plus the filtered id list only — no paths,
    // tokens, or file contents. The caller performs the clipboard write
    // with `navigator.clipboard`; there is no host clipboard surface.
    function getDiagnosticsInfo(timeoutMs = 5000) {
        requireCapability("diagnostics.get_info");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        return request({ type: "diagnostics_get_info", payload: {} }, timeoutMs);
    }

    // CDP-backed graphics report: WebView2 Runtime version, the exact
    // additional browser arguments, the environment label, plus the
    // Browser.getVersion / System.getInfo sections (each marked measured,
    // unsupported, or unavailable — never invented). Parameter-free by
    // protocol: the two probed methods and their empty parameters are fixed
    // host-side. The caller performs the clipboard write with
    // `navigator.clipboard`; there is no host clipboard surface. The longer
    // timeout covers two async runtime round trips.
    function getGraphicsInfo(timeoutMs = 15000) {
        requireCapability("graphics.get_info");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        return request({ type: "graphics_get_info", payload: {} }, timeoutMs);
    }

    // Client-mod cache wipe: the five host-managed downloaded bundle
    // caches plus the stored `modCache` map reset. Parameter-free by
    // protocol — user-owned custom bundles can never be selected.
    function clearModCache(timeoutMs = 30000) {
        requireCapability("mod.clear_cache");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        return request({ type: "mod_clear_cache", payload: {} }, timeoutMs);
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

    // Closed location enum for the four host-controlled roots. The
    // renderer selects a variant only; the host builds the directory.
    const FOLDER_LOCATIONS = ["storage", "themes", "plugins", "gallery"];
    // Host-managed downloaded bundle caches the `mod.clear_cache`
    // wipe may delete. User-owned custom.js/custom.css are deliberately
    // absent: they can never be selected for deletion.
    const BUNDLE_MOD_CACHE_NAMES = ["vencord.js", "vencord.css", "equicord.js", "equicord.css", "shelter.js"];
    // Same rule the host enforces for theme ids ([A-Za-z0-9._-]{1,64},
    // never "." or ".."); separators are impossible by charset.
    const THEME_ID_PATTERN = /^[A-Za-z0-9._-]{1,64}$/;
    // Maximum installable CSS text in UTF-8 bytes, mirroring the host
    // theme bound. Larger stylesheets are rejected before dispatch.
    const MAX_THEME_CSS_BYTES = 1048576;

    function openFolder(location, timeoutMs = 5000) {
        requireCapability("folders.open");
        if (typeof location !== "string" || !FOLDER_LOCATIONS.includes(location)) {
            return Promise.reject(new TypeError("location must be one of storage, themes, plugins, gallery"));
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

    // Gallery disk cache (F5 core): save/browse/search/categories/disk
    // usage/inline preview over the host-owned <profile>/gallery/ root.
    // Method names and result shapes mirror the Electron bridge exactly
    // (src/discord/preload/bridge.ts), so GalleryPage.tsx needs no
    // environment branch. Import/export/root-pick need modal host dialogs
    // (F5b slice): their methods exist for signature parity but their
    // capabilities are absent, so they throw UNSUPPORTED_CAPABILITY
    // honestly and the page gates those buttons.
    const GALLERY_ID_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;
    const MAX_GALLERY_URL_BYTES = 2048;
    const MAX_GALLERY_IDS = 2000;
    const MAX_GALLERY_NAME_BYTES = 256;
    // Maximum inline base64 preview text: 48 MiB of media bytes inflate by
    // 4/3 over the wire. Mirrors the host transport cap.
    const MAX_GALLERY_FILE_BYTES = 67108864;

    function checkGalleryMeta(meta) {
        if (meta === undefined) return null;
        if (!meta || typeof meta !== "object" || Array.isArray(meta)) {
            return "meta must be an object";
        }
        return null;
    }

    function checkGalleryUrl(url) {
        if (typeof url !== "string" || url.length === 0 || textEncoder.encode(url).length > MAX_GALLERY_URL_BYTES) {
            return "url must be a non-empty string of at most 2048 UTF-8 bytes";
        }
        // Only http(s) URLs may be saved: the host refuses every other
        // scheme before any network or filesystem work (a renderer URL
        // must never read local files), so reject here like Electron's
        // own save paths do.
        if (!/^https?:\/\//i.test(url)) {
            return "url must be an http(s) URL";
        }
        return null;
    }

    function checkGalleryId(id) {
        if (typeof id !== "string" || !GALLERY_ID_PATTERN.test(id)) {
            return "id must match [A-Za-z0-9_-]{1,64}";
        }
        return null;
    }

    function checkGalleryName(name) {
        if (typeof name !== "string" || name.length === 0 || textEncoder.encode(name).length > MAX_GALLERY_NAME_BYTES) {
            return "name must be a non-empty string of at most 256 UTF-8 bytes";
        }
        return null;
    }

    // Gallery-local timeout shape check (string-or-null, like the theme
    // section helpers): a separate name so hoisting can never rebind it
    // to the plugins-section promise-style checker.
    function checkGalleryTimeout(timeoutMs) {
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return "timeout must be a positive finite number";
        }
        return null;
    }

    function listGalleryItems(timeoutMs = 5000) {
        requireCapability("gallery.list");
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        // Parameter-free by protocol: the host owns the root, the read,
        // and the newest-first ordering. No path, selector, URL, or
        // filter may be supplied.
        return request({ type: "gallery_list", payload: {} }, timeoutMs);
    }

    function addGalleryUrl(url, meta, timeoutMs = 120_000) {
        requireCapability("gallery.add_url");
        const badUrl = checkGalleryUrl(url);
        if (badUrl) return Promise.reject(new TypeError(badUrl));
        const badMeta = checkGalleryMeta(meta);
        if (badMeta) return Promise.reject(new TypeError(badMeta));
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        // URL plus metadata only: the host validates the scheme, derives
        // kind/file/id, and downloads image/video bytes itself. Non-http(s)
        // URLs are refused before any network or filesystem work. Downloads
        // use the long-operation timeout like mod bundle retrieval.
        return request({ type: "gallery_add_url", payload: { url, meta: meta ?? {} } }, timeoutMs);
    }

    function addGalleryLink(url, meta, timeoutMs = 30_000) {
        requireCapability("gallery.add_link");
        const badUrl = checkGalleryUrl(url);
        if (badUrl) return Promise.reject(new TypeError(badUrl));
        const badMeta = checkGalleryMeta(meta);
        if (badMeta) return Promise.reject(new TypeError(badMeta));
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        // Metadata-only save: no download, no bytes. Same shape rules as
        // addUrl.
        return request({ type: "gallery_add_link", payload: { url, meta: meta ?? {} } }, timeoutMs);
    }

    function importGalleryDialog(_meta, _timeoutMs = 120_000) {
        // F5b dialog slice: no host file picker exists yet, so the
        // capability is absent and this throws UNSUPPORTED_CAPABILITY
        // honestly. The Gallery page gates the Import button natively.
        requireCapability("gallery.import_files");
        return Promise.reject(new Error("unreachable"));
    }

    function removeGalleryItem(id, timeoutMs = 5000) {
        requireCapability("gallery.remove");
        const badId = checkGalleryId(id);
        if (badId) return Promise.reject(new TypeError(badId));
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        // Id only: the host resolves the cached file from its own index
        // and deletes it only inside the gallery root.
        return request({ type: "gallery_remove", payload: { id } }, timeoutMs);
    }

    function bulkRemoveGalleryItems(ids, timeoutMs = 30_000) {
        requireCapability("gallery.bulk_remove");
        if (
            !Array.isArray(ids) ||
            ids.length === 0 ||
            ids.length > MAX_GALLERY_IDS ||
            !ids.every((id) => typeof id === "string")
        ) {
            return Promise.reject(new TypeError("ids must be a non-empty array of at most 2000 id strings"));
        }
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        // Ids only (at most 2000, mirroring Electron's slice); invalid
        // ids are skipped host-side, never dispatched as paths.
        return request({ type: "gallery_bulk_remove", payload: { ids } }, timeoutMs);
    }

    function updateGalleryItem(id, patch, timeoutMs = 5000) {
        requireCapability("gallery.update");
        const badId = checkGalleryId(id);
        if (badId) return Promise.reject(new TypeError(badId));
        if (!patch || typeof patch !== "object" || Array.isArray(patch)) {
            return Promise.reject(new TypeError("patch must be an object"));
        }
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        // Id plus title/categories patch only: only the title and the tag
        // list are mutable, mirroring Electron's patch shape. No kind,
        // URL, file, size, mime, timestamp, or path member may be supplied
        // (the host rejects unknown members).
        const clean = {};
        if (patch.title !== undefined) clean.title = patch.title;
        if (patch.categories !== undefined) clean.categories = patch.categories;
        return request({ type: "gallery_update", payload: { id, ...clean } }, timeoutMs);
    }

    function listGalleryCategories(timeoutMs = 5000) {
        requireCapability("gallery.categories");
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        // The validator resolves the bare array for exact Electron parity.
        return request({ type: "gallery_categories", payload: {} }, timeoutMs);
    }

    function addGalleryCategory(name, timeoutMs = 5000) {
        requireCapability("gallery.category_add");
        const badName = checkGalleryName(name);
        if (badName) return Promise.reject(new TypeError(badName));
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        return request({ type: "gallery_category_add", payload: { name } }, timeoutMs);
    }

    function renameGalleryCategory(oldName, newName, timeoutMs = 5000) {
        requireCapability("gallery.category_rename");
        const badOld = checkGalleryName(oldName);
        if (badOld) return Promise.reject(new TypeError(badOld));
        const badNew = checkGalleryName(newName);
        if (badNew) return Promise.reject(new TypeError(badNew));
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        return request(
            { type: "gallery_category_rename", payload: { old_name: oldName, new_name: newName } },
            timeoutMs,
        );
    }

    function removeGalleryCategory(name, timeoutMs = 5000) {
        requireCapability("gallery.category_remove");
        const badName = checkGalleryName(name);
        if (badName) return Promise.reject(new TypeError(badName));
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        return request({ type: "gallery_category_remove", payload: { name } }, timeoutMs);
    }

    function getGalleryUsage(timeoutMs = 5000) {
        requireCapability("gallery.usage");
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        return request({ type: "gallery_usage", payload: {} }, timeoutMs);
    }

    function getGalleryRoot(timeoutMs = 5000) {
        requireCapability("gallery.root");
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        // Fixed profile-relative label only; an absolute path never
        // crosses (deliberate difference: Electron reports its custom
        // root, native fixes the root until the F5b dialog slice).
        return request({ type: "gallery_root", payload: {} }, timeoutMs);
    }

    function pickGalleryRoot(_timeoutMs = 120_000) {
        // F5b dialog slice: no host directory picker exists yet, so the
        // capability is absent and this throws UNSUPPORTED_CAPABILITY
        // honestly. The Gallery page gates the Choose-folder button.
        requireCapability("gallery.root_pick");
        return Promise.reject(new Error("unreachable"));
    }

    function openGalleryRoot() {
        // Reveal semantics through the shared folders.open capability on
        // the fixed gallery root (Electron's send is fire-and-forget, so
        // the resolution value is dropped the same way).
        return openFolder("gallery").then(() => undefined);
    }

    function exportGalleryItems(_ids, _timeoutMs = 120_000) {
        // F5b dialog slice: no host directory picker exists yet, so the
        // capability is absent and this throws UNSUPPORTED_CAPABILITY
        // honestly. The Gallery page gates the Copy-to-folder button.
        requireCapability("gallery.export_items");
        return Promise.reject(new Error("unreachable"));
    }

    function getGalleryFile(id, timeoutMs = 30_000) {
        requireCapability("gallery.file");
        const badId = checkGalleryId(id);
        if (badId) return Promise.reject(new TypeError(badId));
        const badTimeout = checkGalleryTimeout(timeoutMs);
        if (badTimeout) return Promise.reject(new TypeError(badTimeout));
        // Id only: the host resolves, bounds (48 MiB transport cap), and
        // base64-encodes the cached bytes itself. Larger files play via
        // their remote sourceUrl (Electron parity).
        return request({ type: "gallery_file", payload: { id } }, timeoutMs);
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

    // Runtime plugin bounds, mirroring the host. Plugin ids share the
    // mod-id charset plus dots ([A-Za-z0-9._-]{1,64}, never "." or "..");
    // targets are the closed preload/renderer pair (`main` has no native
    // runtime and fails closed at parse time).
    const RUNTIME_PLUGIN_ID_PATTERN = /^[A-Za-z0-9._-]{1,64}$/;
    const PLUGIN_TARGETS = ["preload", "renderer"];
    const MAX_PLUGIN_LIST_ENTRIES = 256;
    const MAX_PLUGIN_LIST_FIELD_BYTES = 4096;
    const MAX_PLUGIN_SOURCE_BYTES = 1048576;

    function checkPluginId(pluginId) {
        if (
            typeof pluginId !== "string" ||
            !RUNTIME_PLUGIN_ID_PATTERN.test(pluginId) ||
            pluginId === "." ||
            pluginId === ".." ||
            textEncoder.encode(pluginId).length > 64
        ) {
            return Promise.reject(new TypeError("pluginId must match [A-Za-z0-9._-]{1,64} and not be . or .."));
        }
        return null;
    }

    function checkTimeout(timeoutMs) {
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        return null;
    }

    function listPlugins(timeoutMs = 5000) {
        requireCapability("plugins.list");
        const badTimeout = checkTimeout(timeoutMs);
        if (badTimeout) return badTimeout;
        // Parameter-free by protocol: the host owns the plugins root,
        // enumeration, manifest reads, enablement, compatibility, and
        // bounds. No path, root, selector, URL, or filter may be supplied.
        // The host returns bounded display metadata only.
        return request({ type: "plugins_list", payload: {} }, timeoutMs).then((payload) => payload.plugins);
    }

    function setPluginEnabled(pluginId, enabled, timeoutMs = 5000) {
        requireCapability("plugins.set_enabled");
        const badId = checkPluginId(pluginId);
        if (badId) return badId;
        if (typeof enabled !== "boolean") {
            return Promise.reject(new TypeError("enabled must be a boolean"));
        }
        const badTimeout = checkTimeout(timeoutMs);
        if (badTimeout) return badTimeout;
        // Id plus boolean only: the host resolves the id against fresh
        // discovery and persists the flip into the pluginStates settings
        // map. Unknown ids and incompatible enables resolve { ok: false }
        // (Electron parity); the echo must equal the request so one
        // plugin's flip can never resolve another's call. No path,
        // manifest, entry, URL, or filesystem surface may be supplied.
        return request({ type: "plugins_set_enabled", payload: { plugin_id: pluginId, enabled } }, timeoutMs).then(
            (payload) => {
                if (!payload || typeof payload.ok !== "boolean") {
                    throw new Error("Native plugin flip response schema is invalid");
                }
                if (payload.ok && (payload.plugin_id !== pluginId || payload.enabled !== enabled)) {
                    throw new Error("Native plugin flip response schema is invalid");
                }
                return payload;
            },
        );
    }

    function reloadPlugin(pluginId, timeoutMs = 5000) {
        requireCapability("plugins.reload");
        const badId = checkPluginId(pluginId);
        if (badId) return badId;
        const badTimeout = checkTimeout(timeoutMs);
        if (badTimeout) return badTimeout;
        // Id only: the host re-discovers and re-validates that plugin's
        // manifest and files, reporting whether it is currently servable.
        // The loader picks the result up on the next document; no live
        // in-place re-execution is manufactured. No path, manifest, entry,
        // or selector may be supplied.
        return request({ type: "plugins_reload", payload: { plugin_id: pluginId } }, timeoutMs).then((payload) => {
            if (!payload || typeof payload.ok !== "boolean") {
                throw new Error("Native plugin reload response schema is invalid");
            }
            if (payload.ok && payload.plugin_id !== pluginId) {
                throw new Error("Native plugin reload response schema is invalid");
            }
            return payload;
        });
    }

    function getPluginEntries(target, timeoutMs = 120_000) {
        requireCapability("plugins.get_runtime_entries");
        if (typeof target !== "string" || !PLUGIN_TARGETS.includes(target)) {
            return Promise.reject(new TypeError("target must be preload or renderer"));
        }
        const badTimeout = checkTimeout(timeoutMs);
        if (badTimeout) return badTimeout;
        // Closed target only: the host returns every enabled+compatible
        // plugin's logical { id, name } for that target. No id, path, URL,
        // or selector may be supplied, and none may ride back (Electron's
        // absolute path member is dropped at the host).
        return request({ type: "plugins_get_runtime_entries", payload: { target } }, timeoutMs).then((payload) => {
            if (!payload || payload.target !== target || !Array.isArray(payload.entries)) {
                throw new Error("Native plugin entries response schema is invalid");
            }
            for (const entry of payload.entries) {
                if (
                    !entry ||
                    typeof entry.id !== "string" ||
                    !RUNTIME_PLUGIN_ID_PATTERN.test(entry.id) ||
                    typeof entry.name !== "string" ||
                    entry.name.length === 0 ||
                    textEncoder.encode(entry.name).length > MAX_PLUGIN_LIST_FIELD_BYTES ||
                    entry.path !== undefined ||
                    entry.url !== undefined
                ) {
                    throw new Error("Native plugin entries response schema is invalid");
                }
            }
            return payload.entries;
        });
    }

    function getPluginScript(pluginId, target, timeoutMs = 120_000) {
        requireCapability("plugins.get_runtime_script");
        const badId = checkPluginId(pluginId);
        if (badId) return badId;
        if (typeof target !== "string" || !PLUGIN_TARGETS.includes(target)) {
            return Promise.reject(new TypeError("target must be preload or renderer"));
        }
        const badTimeout = checkTimeout(timeoutMs);
        if (badTimeout) return badTimeout;
        // Logical id plus closed target only: the host re-resolves the
        // manifest entry under its own plugins root. An undeclared,
        // unresolvable, missing, or empty entry resolves { source: null }
        // (the loader skips falsy sources like Electron); over-limit,
        // non-UTF-8, or unreadable entries are typed rejections. No path,
        // URL, filename, or entry selector may be supplied.
        return request(
            { type: "plugins_get_runtime_script", payload: { plugin_id: pluginId, target } },
            timeoutMs,
        ).then((payload) => {
            if (
                !payload ||
                payload.plugin_id !== pluginId ||
                payload.target !== target ||
                (payload.source !== null &&
                    (typeof payload.source !== "string" ||
                        textEncoder.encode(payload.source).length > MAX_PLUGIN_SOURCE_BYTES))
            ) {
                throw new Error("Native plugin source response schema is invalid");
            }
            return payload;
        });
    }

    // Maximum Quick CSS editor buffer in UTF-8 bytes, mirroring the host
    // bound. Larger buffers are rejected before dispatch.
    const MAX_QUICK_CSS_BYTES = 1048576;

    function readQuickCss(timeoutMs = 5000) {
        requireCapability("quickcss.read");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Parameter-free by protocol: the host reads its own fixed file
        // only. No path, filename, URL, or selector may be supplied. The
        // host returns the text plus its byte length.
        return request({ type: "quick_css_read", payload: {} }, timeoutMs).then((payload) => {
            if (
                !payload ||
                Object.keys(payload).length !== 2 ||
                typeof payload.css !== "string" ||
                textEncoder.encode(payload.css).length > MAX_QUICK_CSS_BYTES ||
                !Number.isSafeInteger(payload.bytes) ||
                payload.bytes < 0 ||
                payload.bytes > MAX_QUICK_CSS_BYTES
            ) {
                throw new Error("Native Quick CSS read response schema is invalid");
            }
            return payload;
        });
    }

    function writeQuickCss(css, timeoutMs = 5000) {
        requireCapability("quickcss.write");
        if (typeof css !== "string" || textEncoder.encode(css).length > MAX_QUICK_CSS_BYTES) {
            return Promise.reject(new TypeError("css must be a string of at most 1048576 UTF-8 bytes"));
        }
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // CSS text only: the host always replaces its own fixed file
        // atomically and re-pushes the managed style, so no reload or
        // restart is needed. No path, filename, URL, or destination may be
        // supplied. The host returns the persisted byte length.
        return request({ type: "quick_css_write", payload: { css } }, timeoutMs).then((payload) => {
            if (
                !payload ||
                Object.keys(payload).length !== 1 ||
                !Number.isSafeInteger(payload.bytes) ||
                payload.bytes < 0 ||
                payload.bytes > MAX_QUICK_CSS_BYTES
            ) {
                throw new Error("Native Quick CSS write response schema is invalid");
            }
            return payload;
        });
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

    // The exact seven include flags an inspect preview carries. Unknown
    // keys are rejected: the renderer cannot invent an archive section.
    const BACKUP_PREVIEW_INCLUDE_KEYS = [
        "remcordConfig",
        "remcordThemesAndQuickCss",
        "remcordExtensionPlugins",
        "vencordModData",
        "equicordModData",
        "shelterModData",
        "modBundles",
    ];
    // Maximum preview listing length, mirroring the host bound. Longer
    // listings are rejected: the host truncates past this bound itself.
    const MAX_BACKUP_PREVIEW_ENTRIES = 128;
    const MAX_BACKUP_PREVIEW_ENTRY_BYTES = 512;

    function isBackupPreviewIncludes(value) {
        return (
            value &&
            typeof value === "object" &&
            !Array.isArray(value) &&
            Object.keys(value).length === BACKUP_PREVIEW_INCLUDE_KEYS.length &&
            BACKUP_PREVIEW_INCLUDE_KEYS.every((key) => typeof value[key] === "boolean")
        );
    }

    function inspectBackup(timeoutMs = 120_000) {
        requireCapability("backup.inspect");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Parameter-free by protocol: the host owns the entire open picker
        // plus validation and staging. No archive path, filename, URL,
        // filter, or dialog option may be supplied. The modal picker stays
        // open as long as the user needs, so this uses the long-operation
        // timeout like save and the theme import. The staged bytes are HELD
        // host-side for one explicit commit or discard; only the preview
        // crosses back here. Cancellation resolves { ok: false, error:
        // "CANCELLED" } (silent typed control flow, never a rejection);
        // host failures resolve { ok: false, error: <typed code> }. Success
        // resolves { ok: true, preview } for the preview UI — the commit
        // step applies exactly these held bytes.
        return request({ type: "backup_inspect", payload: {} }, timeoutMs).then(
            (payload) => {
                if (payload.canceled === true) return { ok: false, error: "CANCELLED" };
                if (
                    !payload ||
                    Object.keys(payload).length !== 11 ||
                    payload.canceled !== false ||
                    !isBackupPreviewIncludes(payload.includes) ||
                    !Number.isSafeInteger(payload.archive_bytes) ||
                    payload.archive_bytes < 0 ||
                    !Number.isSafeInteger(payload.uncompressed_bytes) ||
                    payload.uncompressed_bytes < 0 ||
                    !Number.isSafeInteger(payload.entry_count) ||
                    payload.entry_count < 0 ||
                    !Number.isSafeInteger(payload.client_mods_bytes) ||
                    payload.client_mods_bytes < 0 ||
                    typeof payload.has_vencord_mod_data !== "boolean" ||
                    typeof payload.has_equicord_mod_data !== "boolean" ||
                    typeof payload.has_shelter_mod_data !== "boolean" ||
                    !Array.isArray(payload.entries) ||
                    payload.entries.length > MAX_BACKUP_PREVIEW_ENTRIES ||
                    !payload.entries.every(
                        (entry) =>
                            typeof entry === "string" &&
                            entry.length > 0 &&
                            textEncoder.encode(entry).length <= MAX_BACKUP_PREVIEW_ENTRY_BYTES,
                    ) ||
                    typeof payload.entries_truncated !== "boolean"
                ) {
                    throw new Error("Native backup inspect response schema is invalid");
                }
                return { ok: true, preview: payload };
            },
            (error) => ({
                ok: false,
                error: typeof error?.code === "string" ? error.code : "UNKNOWN",
            }),
        );
    }

    function commitRestore(timeoutMs = 120_000) {
        requireCapability("backup.commit_restore");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Parameter-free by protocol: the commit applies exactly the held
        // staged bytes (validated at inspect time, never out of host
        // hands), so preview and commit cannot disagree. No archive path,
        // bytes, manifest, or selector may be supplied. Committing with
        // nothing held resolves { ok: false, error: "NO_STAGED_RESTORE" },
        // never a silent no-op. Success resolves { ok: true, clientMods }
        // for the existing renderer mechanism to apply — the host never
        // applies clientMods itself.
        return request({ type: "backup_commit_restore", payload: {} }, timeoutMs).then(
            (payload) => {
                if (payload.canceled === true) return { ok: false, error: "CANCELLED" };
                if (!isBoundedClientMods(payload.client_mods)) {
                    throw new Error("Native backup commit response schema is invalid");
                }
                return { ok: true, clientMods: payload.client_mods };
            },
            (error) => ({
                ok: false,
                error: typeof error?.code === "string" ? error.code : "UNKNOWN",
            }),
        );
    }

    function discardRestore(timeoutMs = 5000) {
        requireCapability("backup.discard_restore");
        if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
            return Promise.reject(new TypeError("timeout must be a positive finite number"));
        }
        // Parameter-free by protocol: drops the held staged restore without
        // touching the profile. Resolves { discarded } — false when nothing
        // was held, never an error for the empty case.
        return request({ type: "backup_discard_restore", payload: {} }, timeoutMs).then((payload) => {
            if (!payload || Object.keys(payload).length !== 1 || typeof payload.discarded !== "boolean") {
                throw new Error("Native backup discard response schema is invalid");
            }
            return payload;
        });
    }

    // Bounds mirroring the host graphics report. Kept strings are
    // identifiers and driver text only — never paths (the host never sends
    // the raw Chromium command line).
    const GRAPHICS_STATUSES = ["measured", "unsupported", "unavailable"];
    const MAX_GRAPHICS_DEVICES = 16;
    const MAX_GRAPHICS_FEATURES = 64;
    const MAX_GRAPHICS_STRING_BYTES = 512;
    const MAX_GRAPHICS_FEATURE_STRING_BYTES = 128;
    const MAX_GRAPHICS_FLAGS = 12;
    const MAX_GRAPHICS_FLAG_BYTES = 64;
    const MAX_GRAPHICS_FEATURE_LIST_BYTES = 1024;

    function isGraphicsStringOrNull(value, maxBytes) {
        return value === null || (typeof value === "string" && textEncoder.encode(value).length <= maxBytes);
    }

    function isGraphicsDevice(entry) {
        return (
            entry &&
            typeof entry === "object" &&
            Object.keys(entry).length === 5 &&
            isGraphicsStringOrNull(entry.vendor_id, MAX_GRAPHICS_STRING_BYTES) &&
            isGraphicsStringOrNull(entry.device_id, MAX_GRAPHICS_STRING_BYTES) &&
            isGraphicsStringOrNull(entry.vendor_string, MAX_GRAPHICS_STRING_BYTES) &&
            isGraphicsStringOrNull(entry.device_string, MAX_GRAPHICS_STRING_BYTES) &&
            isGraphicsStringOrNull(entry.driver_version, MAX_GRAPHICS_STRING_BYTES)
        );
    }

    function isGraphicsFeature(entry) {
        return (
            entry &&
            typeof entry === "object" &&
            Object.keys(entry).length === 2 &&
            typeof entry.name === "string" &&
            textEncoder.encode(entry.name).length <= MAX_GRAPHICS_FEATURE_STRING_BYTES &&
            typeof entry.status === "string" &&
            textEncoder.encode(entry.status).length <= MAX_GRAPHICS_FEATURE_STRING_BYTES
        );
    }

    function isGraphicsInfo(payload) {
        if (!payload || typeof payload !== "object") return false;
        if (Object.keys(payload).length !== 5) return false;
        if (
            typeof payload.webview2_runtime !== "string" ||
            payload.webview2_runtime.length === 0 ||
            textEncoder.encode(payload.webview2_runtime).length > 64 ||
            typeof payload.additional_browser_arguments !== "string" ||
            textEncoder.encode(payload.additional_browser_arguments).length > 512 ||
            typeof payload.environment !== "string" ||
            payload.environment.length === 0 ||
            textEncoder.encode(payload.environment).length > 128
        ) {
            return false;
        }
        const browser = payload.browser;
        if (
            !browser ||
            typeof browser !== "object" ||
            Object.keys(browser).length !== 8 ||
            !GRAPHICS_STATUSES.includes(browser.status) ||
            browser.method !== "Browser.getVersion" ||
            !isGraphicsStringOrNull(browser.product, MAX_GRAPHICS_STRING_BYTES) ||
            !isGraphicsStringOrNull(browser.revision, MAX_GRAPHICS_STRING_BYTES) ||
            !isGraphicsStringOrNull(browser.user_agent, MAX_GRAPHICS_STRING_BYTES) ||
            !isGraphicsStringOrNull(browser.js_version, MAX_GRAPHICS_STRING_BYTES) ||
            !isGraphicsStringOrNull(browser.protocol_version, MAX_GRAPHICS_STRING_BYTES) ||
            !isGraphicsStringOrNull(browser.error, MAX_GRAPHICS_STRING_BYTES)
        ) {
            return false;
        }
        const system = payload.system;
        if (
            !system ||
            typeof system !== "object" ||
            Object.keys(system).length !== 9 ||
            !GRAPHICS_STATUSES.includes(system.status) ||
            system.method !== "System.getInfo" ||
            !isGraphicsStringOrNull(system.model_name, MAX_GRAPHICS_STRING_BYTES) ||
            !Array.isArray(system.devices) ||
            system.devices.length > MAX_GRAPHICS_DEVICES ||
            !system.devices.every(isGraphicsDevice) ||
            !Array.isArray(system.feature_status) ||
            system.feature_status.length > MAX_GRAPHICS_FEATURES ||
            !system.feature_status.every(isGraphicsFeature) ||
            !Array.isArray(system.command_line_flags) ||
            system.command_line_flags.length > MAX_GRAPHICS_FLAGS ||
            !system.command_line_flags.every(
                (flag) =>
                    typeof flag === "string" &&
                    flag.length > 0 &&
                    textEncoder.encode(flag).length <= MAX_GRAPHICS_FLAG_BYTES,
            ) ||
            !isGraphicsStringOrNull(system.disable_features, MAX_GRAPHICS_FEATURE_LIST_BYTES) ||
            !isGraphicsStringOrNull(system.enable_features, MAX_GRAPHICS_FEATURE_LIST_BYTES) ||
            !isGraphicsStringOrNull(system.error, MAX_GRAPHICS_STRING_BYTES)
        ) {
            return false;
        }
        return true;
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
                      : payload?.key === "channel"
                        ? typeof payload?.value === "string" && SETTINGS_VALID_CHANNELS.includes(payload.value)
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
        } else if (response.ok === true && response.payload?.type === "settings_imported") {
            const payload = response.payload.payload;
            // Key names only: applied/skipped must be disjoint subsets of
            // the import allowlist. No values, no paths may ride along —
            // the UI re-reads display state through settings.get_config.
            const keysValid =
                payload &&
                Object.keys(payload).length === 2 &&
                Array.isArray(payload.applied) &&
                Array.isArray(payload.skipped) &&
                [...payload.applied, ...payload.skipped].every(
                    (key) => typeof key === "string" && SETTINGS_IMPORT_KEYS.includes(key),
                ) &&
                new Set([...payload.applied, ...payload.skipped]).size ===
                    payload.applied.length + payload.skipped.length;
            if (!keysValid) {
                request.reject(new Error("Native settings import response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "keybinds_list") {
            const payload = response.payload.payload;
            // Validated inventory, metadata only: bounded entries with the
            // closed id/accelerator/action/flag shape. No js, path, URL, or
            // code member may ride along.
            const entriesValid =
                Array.isArray(payload?.keybinds) &&
                payload.keybinds.length <= MAX_KEYBIND_LIST_ENTRIES &&
                payload.keybinds.every(
                    (entry) =>
                        entry &&
                        typeof entry.id === "string" &&
                        KEYBIND_ID_PATTERN.test(entry.id) &&
                        typeof entry.accelerator === "string" &&
                        textEncoder.encode(entry.accelerator).length <= MAX_KEYBIND_ACCELERATOR_BYTES &&
                        typeof entry.action === "string" &&
                        KEYBIND_ACTIONS.includes(entry.action) &&
                        typeof entry.global === "boolean" &&
                        typeof entry.enabled === "boolean" &&
                        entry.js === undefined &&
                        entry.path === undefined &&
                        entry.url === undefined,
                );
            if (!payload || Object.keys(payload).length !== 1 || !entriesValid) {
                request.reject(new Error("Native keybind list response schema is invalid"));
                return;
            }
            request.resolve(payload.keybinds);
        } else if (response.ok === true && response.payload?.type === "keybinds_add") {
            const payload = response.payload.payload;
            // Typed echo: success carries the saved id (which must equal
            // the request — one entry's save can never resolve another's),
            // failure carries the code for the maker toast.
            if (
                !payload ||
                typeof payload.ok !== "boolean" ||
                (payload.ok
                    ? typeof payload.id !== "string" || payload.error !== undefined
                    : typeof payload.error !== "string" || payload.id !== undefined)
            ) {
                request.reject(new Error("Native keybind save response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "keybinds_toggle") {
            const payload = response.payload.payload;
            if (
                !payload ||
                typeof payload.ok !== "boolean" ||
                (payload.ok ? typeof payload.enabled !== "boolean" : payload.enabled !== undefined)
            ) {
                request.reject(new Error("Native keybind toggle response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "keybinds_remove") {
            const payload = response.payload.payload;
            if (!payload || Object.keys(payload).length !== 1 || typeof payload.ok !== "boolean") {
                request.reject(new Error("Native keybind removal response schema is invalid"));
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
        } else if (response.ok === true && response.payload?.type === "diagnostics_info") {
            const payload = response.payload.payload;
            // Host-owned facts only: fixed-vocabulary strings plus the
            // filtered mod-id list. No paths, tokens, or file contents may
            // ride along.
            const modsValid =
                Array.isArray(payload?.mods) &&
                payload.mods.length <= MAX_SETTINGS_MODS_ENTRIES &&
                payload.mods.every((entry) => typeof entry === "string" && SETTINGS_VALID_MODS.includes(entry));
            if (
                !payload ||
                Object.keys(payload).length !== 6 ||
                typeof payload.app_version !== "string" ||
                payload.app_version.length === 0 ||
                textEncoder.encode(payload.app_version).length > 64 ||
                typeof payload.webview2_runtime !== "string" ||
                payload.webview2_runtime.length === 0 ||
                textEncoder.encode(payload.webview2_runtime).length > 64 ||
                typeof payload.profile_kind !== "string" ||
                !["dev", "release", "override"].includes(payload.profile_kind) ||
                typeof payload.channel !== "string" ||
                !SETTINGS_VALID_CHANNELS.includes(payload.channel) ||
                typeof payload.csp !== "string" ||
                !SETTINGS_VALID_CSP_MODES.includes(payload.csp) ||
                !modsValid
            ) {
                request.reject(new Error("Native diagnostics response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "graphics_info") {
            const payload = response.payload.payload;
            // CDP-backed graphics report: fixed keys, bounded strings, and
            // explicit measured/unsupported/unavailable sections. No paths,
            // command lines, or file contents may ride along.
            if (!isGraphicsInfo(payload)) {
                request.reject(new Error("Native graphics response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "mod_cache_cleared") {
            const payload = response.payload.payload;
            // Fixed allowlisted cache names plus the map-reset flag only.
            // No paths, sizes, or contents may ride along.
            const clearedValid =
                Array.isArray(payload?.cleared) &&
                payload.cleared.length <= 5 &&
                payload.cleared.every((entry) => typeof entry === "string" && BUNDLE_MOD_CACHE_NAMES.includes(entry));
            if (
                !payload ||
                Object.keys(payload).length !== 2 ||
                !clearedValid ||
                typeof payload.mod_cache_cleared !== "boolean"
            ) {
                request.reject(new Error("Native mod cache response schema is invalid"));
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
        } else if (response.ok === true && response.payload?.type === "gallery_list") {
            const payload = response.payload.payload;
            // Items newest-first plus categories in Electron's shape.
            // localFile values stay profile-relative (files/<name>);
            // no path, content, or absolute reference may ride along.
            const itemsValid =
                Array.isArray(payload?.items) &&
                payload.items.length <= 20000 &&
                payload.items.every(
                    (entry) =>
                        entry &&
                        typeof entry.id === "string" &&
                        GALLERY_ID_PATTERN.test(entry.id) &&
                        typeof entry.kind === "string" &&
                        (entry.localFile === "" ||
                            (typeof entry.localFile === "string" && entry.localFile.startsWith("files/"))) &&
                        entry.path === undefined &&
                        entry.data === undefined &&
                        entry.base64 === undefined,
                );
            const categoriesValid =
                Array.isArray(payload?.categories) &&
                payload.categories.length <= 512 &&
                payload.categories.every(
                    (entry) =>
                        entry &&
                        typeof entry.name === "string" &&
                        entry.path === undefined &&
                        entry.url === undefined &&
                        entry.data === undefined,
                );
            if (!payload || Object.keys(payload).length !== 2 || !itemsValid || !categoriesValid) {
                request.reject(new Error("Native gallery list response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (
            response.ok === true &&
            (response.payload?.type === "gallery_add" || response.payload?.type === "gallery_update")
        ) {
            const payload = response.payload.payload;
            // Electron's { ok, item } | { ok, error } shape, resolved (never
            // rejected) by the host so the page can toast the reason.
            const itemValid =
                payload.item === undefined ||
                (payload.item && typeof payload.item.id === "string" && GALLERY_ID_PATTERN.test(payload.item.id));
            if (
                !payload ||
                typeof payload.ok !== "boolean" ||
                (payload.ok
                    ? !payload.item || payload.error !== undefined
                    : !payload.error || payload.item !== undefined) ||
                !itemValid
            ) {
                request.reject(new Error("Native gallery save response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (
            response.ok === true &&
            (response.payload?.type === "gallery_remove" || response.payload?.type === "gallery_category_remove")
        ) {
            const payload = response.payload.payload;
            if (!payload || Object.keys(payload).length !== 1 || typeof payload.ok !== "boolean") {
                request.reject(new Error("Native gallery removal response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "gallery_bulk_remove") {
            const payload = response.payload.payload;
            if (
                !payload ||
                Object.keys(payload).length !== 1 ||
                !Number.isSafeInteger(payload.removed) ||
                payload.removed < 0
            ) {
                request.reject(new Error("Native gallery bulk removal response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "gallery_categories") {
            const payload = response.payload.payload;
            if (
                !payload ||
                Object.keys(payload).length !== 1 ||
                !Array.isArray(payload.categories) ||
                payload.categories.length > 512 ||
                !payload.categories.every((entry) => entry && typeof entry.name === "string")
            ) {
                request.reject(new Error("Native gallery categories response schema is invalid"));
                return;
            }
            // Electron's bridge resolves the bare array (not the
            // `{categories}` envelope), so extract here for exact parity.
            request.resolve(payload.categories);
        } else if (response.ok === true && response.payload?.type === "gallery_category_add") {
            const payload = response.payload.payload;
            if (
                !payload ||
                typeof payload.ok !== "boolean" ||
                (payload.ok
                    ? !payload.category || typeof payload.category.name !== "string" || payload.error !== undefined
                    : !payload.error || payload.category !== undefined)
            ) {
                request.reject(new Error("Native gallery category response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "gallery_category_rename") {
            const payload = response.payload.payload;
            if (
                !payload ||
                typeof payload.ok !== "boolean" ||
                (payload.ok ? payload.error !== undefined : typeof payload.error !== "string")
            ) {
                request.reject(new Error("Native gallery category rename response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "gallery_usage") {
            const payload = response.payload.payload;
            if (
                !payload ||
                Object.keys(payload).length !== 3 ||
                !Number.isSafeInteger(payload.bytes) ||
                payload.bytes < 0 ||
                !Number.isSafeInteger(payload.files) ||
                payload.files < 0 ||
                !Number.isSafeInteger(payload.items) ||
                payload.items < 0
            ) {
                request.reject(new Error("Native gallery usage response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "gallery_file") {
            const payload = response.payload.payload;
            // Bounded inline preview (48 MiB of bytes at most, base64
            // inflates by 4/3) or a typed error the thumbnail falls back
            // from. No path may ride along.
            const fileValid =
                payload.ok === true
                    ? typeof payload.base64 === "string" &&
                      payload.base64.length <= MAX_GALLERY_FILE_BYTES &&
                      typeof payload.mime === "string" &&
                      typeof payload.fileName === "string" &&
                      payload.error === undefined &&
                      payload.path === undefined
                    : typeof payload.error === "string" && payload.base64 === undefined && payload.path === undefined;
            if (!payload || typeof payload.ok !== "boolean" || !fileValid) {
                request.reject(new Error("Native gallery file response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "gallery_root") {
            const payload = response.payload.payload;
            // Fixed profile-relative label only; an absolute path never
            // crosses.
            if (!payload || Object.keys(payload).length !== 1 || payload.root !== "gallery") {
                request.reject(new Error("Native gallery root response schema is invalid"));
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
        } else if (response.ok === true && response.payload?.type === "plugins_list") {
            const payload = response.payload.payload;
            // Bounded display metadata only: an array of exact-shape
            // entries in sorted directory order. No paths, URLs, sources,
            // or extra members anywhere in the envelope (Electron's
            // absolute entry path is dropped at the host).
            const entries = payload?.plugins;
            const textField = (value) =>
                typeof value === "string" && textEncoder.encode(value).length <= MAX_PLUGIN_LIST_FIELD_BYTES;
            const shapesValid =
                Array.isArray(entries) &&
                entries.length <= MAX_PLUGIN_LIST_ENTRIES &&
                entries.every(
                    (entry) =>
                        entry &&
                        typeof entry.id === "string" &&
                        RUNTIME_PLUGIN_ID_PATTERN.test(entry.id) &&
                        entry.id !== "." &&
                        entry.id !== ".." &&
                        textEncoder.encode(entry.id).length <= 64 &&
                        typeof entry.name === "string" &&
                        entry.name.length > 0 &&
                        textEncoder.encode(entry.name).length <= 128 &&
                        (entry.description === undefined || textField(entry.description)) &&
                        (entry.author === undefined || textField(entry.author)) &&
                        typeof entry.enabled === "boolean" &&
                        typeof entry.compatible === "boolean" &&
                        (entry.compatibility_message === undefined ||
                            (typeof entry.compatibility_message === "string" &&
                                textEncoder.encode(entry.compatibility_message).length <= 512)) &&
                        Array.isArray(entry.compatible_versions) &&
                        entry.compatible_versions.length <= 32 &&
                        entry.compatible_versions.every(
                            (pattern) => typeof pattern === "string" && textEncoder.encode(pattern).length <= 64,
                        ) &&
                        typeof entry.has_main === "boolean" &&
                        typeof entry.has_preload === "boolean" &&
                        typeof entry.has_renderer === "boolean" &&
                        entry.path === undefined &&
                        entry.url === undefined &&
                        entry.source === undefined,
                );
            if (!payload || !shapesValid) {
                request.reject(new Error("Native plugin list response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "plugins_set_enabled") {
            const payload = response.payload.payload;
            // Electron-parity { ok } plus the validated echo on success so
            // one plugin's flip can never resolve another's call.
            if (
                !payload ||
                typeof payload.ok !== "boolean" ||
                (payload.ok &&
                    (Object.keys(payload).length !== 5 ||
                        typeof payload.plugin_id !== "string" ||
                        !RUNTIME_PLUGIN_ID_PATTERN.test(payload.plugin_id) ||
                        typeof payload.enabled !== "boolean" ||
                        payload.source !== "file" ||
                        !Number.isSafeInteger(payload.payload_bytes)))
            ) {
                request.reject(new Error("Native plugin flip response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "plugins_reloaded") {
            const payload = response.payload.payload;
            // Same { ok } contract as the flip: success echoes the
            // re-validated id, failure echoes nothing.
            if (
                !payload ||
                typeof payload.ok !== "boolean" ||
                (payload.ok &&
                    (Object.keys(payload).length !== 2 ||
                        typeof payload.plugin_id !== "string" ||
                        !RUNTIME_PLUGIN_ID_PATTERN.test(payload.plugin_id)))
            ) {
                request.reject(new Error("Native plugin reload response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "plugins_runtime_entries") {
            const payload = response.payload.payload;
            // Closed target echo plus logical { id, name } entries only.
            // The per-entry shape is re-checked by getPluginEntries before
            // resolving; this branch only gates the envelope.
            if (
                !payload ||
                !PLUGIN_TARGETS.includes(payload.target) ||
                !Array.isArray(payload.entries) ||
                payload.entries.length > MAX_PLUGIN_LIST_ENTRIES
            ) {
                request.reject(new Error("Native plugin entries response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "plugins_runtime_script") {
            const payload = response.payload.payload;
            // Id+target echo plus bounded source text, or an explicit null
            // source for undeclared/missing/empty entries (the loader skips
            // falsy sources like Electron). The source bound is re-checked
            // by getPluginScript before resolving; this branch gates shape.
            if (
                !payload ||
                typeof payload.plugin_id !== "string" ||
                !PLUGIN_TARGETS.includes(payload.target) ||
                (payload.source !== null && typeof payload.source !== "string")
            ) {
                request.reject(new Error("Native plugin source response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "quick_css_read") {
            const payload = response.payload.payload;
            // Fixed-file text plus its byte length only. No path, flag, or
            // filesystem error may ride along; bounds are re-checked here so
            // a hostile host reply cannot widen the editor buffer.
            if (
                !payload ||
                Object.keys(payload).length !== 2 ||
                typeof payload.css !== "string" ||
                textEncoder.encode(payload.css).length > MAX_QUICK_CSS_BYTES ||
                !Number.isSafeInteger(payload.bytes) ||
                payload.bytes < 0 ||
                payload.bytes > MAX_QUICK_CSS_BYTES
            ) {
                request.reject(new Error("Native Quick CSS read response schema is invalid"));
                return;
            }
            request.resolve(payload);
        } else if (response.ok === true && response.payload?.type === "quick_css_written") {
            const payload = response.payload.payload;
            // Persisted byte length only. No path, text, or filesystem error
            // may ride along.
            if (
                !payload ||
                Object.keys(payload).length !== 1 ||
                !Number.isSafeInteger(payload.bytes) ||
                payload.bytes < 0 ||
                payload.bytes > MAX_QUICK_CSS_BYTES
            ) {
                request.reject(new Error("Native Quick CSS write response schema is invalid"));
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
        } else if (response.ok === true && response.payload?.type === "window_state") {
            const payload = response.payload.payload;
            // F16 persisted geometry: clamped integer bounds plus the live
            // maximized flag, with the optional best-effort monitor hint.
            // No handle, path, URL, or error text may ride along.
            const geometryValid =
                payload &&
                Number.isSafeInteger(payload.width) &&
                payload.width >= 400 &&
                payload.width <= 16384 &&
                Number.isSafeInteger(payload.height) &&
                payload.height >= 300 &&
                payload.height <= 16384 &&
                Number.isSafeInteger(payload.x) &&
                payload.x >= -32768 &&
                payload.x <= 32768 &&
                Number.isSafeInteger(payload.y) &&
                payload.y >= -32768 &&
                payload.y <= 32768 &&
                typeof payload.is_maximized === "boolean" &&
                (payload.display_id === undefined ||
                    (Number.isSafeInteger(payload.display_id) && payload.display_id >= 0));
            if (
                !geometryValid ||
                !Object.keys(payload).every((key) =>
                    ["width", "height", "x", "y", "is_maximized", "display_id"].includes(key),
                )
            ) {
                request.reject(new Error("Native window state response schema is invalid"));
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
        } else if (response.ok === true && response.payload?.type === "backup_inspected") {
            const payload = response.payload.payload;
            // Bounded preview only: cancellation plus counts, flags, and the
            // bounded display listing. No contents, no paths beyond display
            // names, no envelope may ride along. Cancellation carries the
            // zeroed shape; success carries the held restore's preview.
            if (
                !payload ||
                Object.keys(payload).length !== 11 ||
                typeof payload.canceled !== "boolean" ||
                !isBackupPreviewIncludes(payload.includes) ||
                !Number.isSafeInteger(payload.archive_bytes) ||
                payload.archive_bytes < 0 ||
                !Number.isSafeInteger(payload.uncompressed_bytes) ||
                payload.uncompressed_bytes < 0 ||
                !Number.isSafeInteger(payload.entry_count) ||
                payload.entry_count < 0 ||
                !Number.isSafeInteger(payload.client_mods_bytes) ||
                payload.client_mods_bytes < 0 ||
                typeof payload.has_vencord_mod_data !== "boolean" ||
                typeof payload.has_equicord_mod_data !== "boolean" ||
                typeof payload.has_shelter_mod_data !== "boolean" ||
                !Array.isArray(payload.entries) ||
                payload.entries.length > MAX_BACKUP_PREVIEW_ENTRIES ||
                !payload.entries.every(
                    (entry) =>
                        typeof entry === "string" &&
                        entry.length > 0 &&
                        textEncoder.encode(entry).length <= MAX_BACKUP_PREVIEW_ENTRY_BYTES,
                ) ||
                typeof payload.entries_truncated !== "boolean"
            ) {
                request.reject(new Error("Native backup inspect response schema is invalid"));
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
        } else if (response.ok === true && response.payload?.type === "backup_restore_discarded") {
            const payload = response.payload.payload;
            // Discard acknowledgement only: whether a held restore was
            // actually dropped. Nothing else may ride along.
            if (!payload || Object.keys(payload).length !== 1 || typeof payload.discarded !== "boolean") {
                request.reject(new Error("Native backup discard response schema is invalid"));
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
        getPluginEntries,
        getPluginScript,
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
            // F16 persisted-geometry read: the host captured the bounds
            // from its own window events, so the renderer supplies no
            // parameter — the response is validated below before it
            // resolves. There is deliberately no setState/move/resize:
            // placement is host-owned, never renderer-driven.
            getState() {
                requireCapability("window.get_state");
                return request({ type: "window_get_state", payload: {} });
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
    const backup = Object.freeze({
        listBundles,
        save: saveBackup,
        inspect: inspectBackup,
        commitRestore,
        discardRestore,
    });
    // Folder opening only: reveal for the four fixed roots, open for one
    // validated theme id. No path, dialog, picker, or shell surface.
    const folders = Object.freeze({ open: openFolder, openTheme: openThemeFolder });
    // Gallery disk cache only: Electron-identical method names and result
    // shapes over host-owned storage. Import/export/root-pick need modal
    // host dialogs (F5b slice): their methods exist for signature parity
    // but throw UNSUPPORTED_CAPABILITY until their capabilities exist.
    const gallery = Object.freeze({
        list: listGalleryItems,
        addUrl: addGalleryUrl,
        addLink: addGalleryLink,
        importDialog: importGalleryDialog,
        remove: removeGalleryItem,
        bulkRemove: bulkRemoveGalleryItems,
        update: updateGalleryItem,
        categories: listGalleryCategories,
        categoryAdd: addGalleryCategory,
        categoryRename: renameGalleryCategory,
        categoryRemove: removeGalleryCategory,
        usage: getGalleryUsage,
        root: getGalleryRoot,
        pickRoot: pickGalleryRoot,
        openRoot: openGalleryRoot,
        exportItems: exportGalleryItems,
        getFile: getGalleryFile,
    });
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
        importElectron: importElectronSettings,
        addKeybind,
        toggleKeybind,
        removeKeybind,
        listKeybinds,
    });
    const themesShim = Object.freeze({
        installFromCode: installThemeFromCode,
        writeCss: writeThemeCss,
        importFiles,
        setEnabled: setThemeEnabled,
        listThemes,
        getThemes: getThemesSync,
    });
    // Quick CSS editor pair: the fixed file only, bounded both ways. The
    // host re-pushes the managed style on write, so the editor needs no
    // reload or restart round trip of its own.
    const quickcssShim = Object.freeze({
        read: readQuickCss,
        write: writeQuickCss,
    });
    // Diagnostics readout only: host-owned facts in, no parameters out.
    // The caller performs the clipboard write with `navigator.clipboard`.
    const diagnostics = Object.freeze({ getInfo: getDiagnosticsInfo });
    // CDP-backed graphics report only: the two fixed probes in, the bounded
    // measured-or-explicitly-missing report out. The caller performs the
    // clipboard write with `navigator.clipboard`.
    const graphics = Object.freeze({ getInfo: getGraphicsInfo });
    // Client-mod cache wipe only: an exact empty payload in, the fixed
    // allowlist outcome out. User-owned bundles can never be selected.
    const mod = Object.freeze({ clearCache: clearModCache });
    // Runtime plugin management only: bounded inventory in, id+boolean
    // flips and id-only reloads with Electron-parity { ok } outcomes out.
    // Folder opening reuses the typed folders.open capability on the fixed
    // plugins root. No manifest, entry, path, URL, or source surface.
    const plugins = Object.freeze({
        list: listPlugins,
        setEnabled: setPluginEnabled,
        reload: reloadPlugin,
        openFolder: () => openFolder("plugins"),
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
                getState: native.window.getState,
                maximized: native.window.maximized,
                isNormal: native.window.isNormal,
            }),
            settings: settingsShim,
            getLang,
            pluginStorage,
            plugins,
            backup,
            folders,
            gallery,
            themes: themesShim,
            quickcss: quickcssShim,
            diagnostics,
            graphics,
            mod,
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

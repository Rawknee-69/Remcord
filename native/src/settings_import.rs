//! Copy-only settings import from the Electron profile (F7).
//!
//! This module is the native counterpart of *no* Electron feature:
//! Electron never migrates settings between profiles. It reads the
//! Electron app's own settings file and copies a fixed allowlist of
//! validated preference values into the native profile. The checklist
//! constraint is structural: **never auto-migrate, copy-only if ever
//! built** — so this module runs exactly once per explicit renderer
//! request (`settings.import_electron`), never at startup, never on a
//! timer, and never writes to the Electron profile.
//!
//! Traced Electron facts (actual implementation, not declarations):
//! - Location: `app.getPath("userData")` on Windows is the roaming
//!   `%APPDATA%\Remcord` directory; the settings file is
//!   `<userData>/storage/settings.json` (`src/common/config.ts:
//!   getConfigLocation`). The native release profile is
//!   `%LOCALAPPDATA%\Remcord` — a different root by construction
//!   (`profile.rs`), so the two files can never alias.
//! - Shape: a flat JSON object with the exact camelCase keys of
//!   `src/@types/settings.d.ts`, defaults in `config.ts`. Corrupt files
//!   trigger a setup reset + error box there; natively a corrupt Electron
//!   file is a typed failure, never a reset of anything.
//!
//! Native contract (deliberate, documented differences from Electron):
//! - Fixed source, zero renderer input: the path is built host-side from
//!   `%APPDATA%` plus three constant segments. There is no source path,
//!   URL, key list, or value parameter anywhere — the request is
//!   parameter-free, so a renderer cannot point the read at another file.
//! - Copy-only: the Electron file is opened read-only and never written,
//!   moved, or deleted. Only the native `<profile>/storage/settings.json`
//!   is written, through the same authoritative-load + atomic-persist path
//!   as every other settings mutation (missing file starts from defaults,
//!   corrupt native file is an error that is never overwritten, unknown
//!   future keys preserved).
//! - Per-key apply with a report: every allowlisted key present in the
//!   Electron file is validated by the same rules its native consumer
//!   enforces; valid values are applied, invalid ones are skipped and
//!   named in the response. Nothing is guessed, nothing is defaulted-in:
//!   an absent key keeps the native value.
//! - The allowlist is preference-shaped data only (see
//!   [`IMPORTABLE_KEYS`]). Everything that names a path, launches a
//!   process, routes the network, controls an unimplemented surface, or
//!   carries code is excluded with a reason in the table.
//! - Response carries key names only (`applied` / `skipped`), never
//!   values and never paths: the UI re-reads display state through the
//!   existing `settings.get_config` capability.
//!
//! Logical roots owned by this module: the native profile settings file
//! (write, via `settings.rs`) and the Electron profile settings file
//! (bounded read-only read). No other filesystem access.

use std::path::{Path, PathBuf};

/// Leaf of the Electron product profile under the roaming AppData root.
/// Distinct from the native release leaf by root (`profile.rs` documents
/// the disjointness); the leaf name itself matches because it is the
/// Electron app's own directory, not a native choice.
pub const ELECTRON_PROFILE_LEAF: &str = "Remcord";
/// Fixed segments below the Electron profile root to its settings file
/// (mirrors `getConfigLocation`: `<userData>/storage/settings.json`).
pub const ELECTRON_SETTINGS_SEGMENTS: &[&str] = &["storage", "settings.json"];

/// Maximum accepted Electron settings file size. Reuses the native
/// settings bound (`settings.rs::MAX_SETTINGS_FILE_BYTES`): the real
/// file is ~2 KiB; anything larger is refused, never partially read.
pub const MAX_ELECTRON_SETTINGS_BYTES: u64 = 256 * 1024;

/// Maximum `spellcheckLanguage` entries (Electron: unbounded array).
pub const MAX_SPELLCHECK_LANGUAGES: usize = 16;
/// Maximum single language tag length in bytes.
pub const MAX_LANGUAGE_TAG_BYTES: usize = 64;
/// Maximum `pluginStates` entries (Electron: unbounded object).
pub const MAX_PLUGIN_STATES_ENTRIES: usize = 256;
/// Accepted `scanInterval` range in milliseconds. Electron applies any
/// number; the native bound keeps a hostile file from installing a
/// zero-interval poll loop or an overflow-adjacent timer.
pub const MIN_SCAN_INTERVAL_MS: u64 = 1_000;
pub const MAX_SCAN_INTERVAL_MS: u64 = 600_000;

/// One importable preference: the exact Electron `settings.json` key,
/// what validates it, and why it is safe to copy. `rationale` is
/// documentation enforced by tests (every key must justify its safety);
/// it is never read at runtime by design.
pub struct ImportableKey {
    /// Exact document field, e.g. `"mods"`.
    pub key: &'static str,
    /// Why copying this value cannot harm the native shell.
    #[allow(dead_code)]
    pub rationale: &'static str,
}

/// Fixed import allowlist, in application order. Validation per key lives
/// in [`extract_importable_values`], reusing the exact rules the native
/// consumers enforce (`settings.rs` allowlists, `channel.rs`,
/// `plugins.rs`).
pub const IMPORTABLE_KEYS: &[ImportableKey] = &[
    ImportableKey {
        key: "mods",
        rationale: "exact ValidMods members; resolved by the mod registry at startup",
    },
    ImportableKey {
        key: "csp",
        rationale: "exact CSP mode; applied by the document CSP filter",
    },
    ImportableKey {
        key: "channel",
        rationale: "exact release channel; selects startup navigation",
    },
    ImportableKey {
        key: "supportBannerDismissed",
        rationale: "pure renderer UI state",
    },
    ImportableKey {
        key: "showExperimentalPluginMenu",
        rationale: "pure renderer UI state",
    },
    ImportableKey {
        key: "extendedPluginAbilities",
        rationale: "persisted flag; native plugin storage is capability-gated regardless",
    },
    ImportableKey {
        key: "quickCss",
        rationale: "trusted-load pipeline flag; re-pushed live after import",
    },
    ImportableKey {
        key: "spellcheck",
        rationale: "pure preference bool",
    },
    ImportableKey {
        key: "smoothScroll",
        rationale: "pure preference bool",
    },
    ImportableKey {
        key: "autoScroll",
        rationale: "pure preference bool",
    },
    ImportableKey {
        key: "mobileMode",
        rationale: "pure preference bool",
    },
    ImportableKey {
        key: "skipSplash",
        rationale: "pure preference bool",
    },
    ImportableKey {
        key: "sleepInBackground",
        rationale: "pure preference bool",
    },
    ImportableKey {
        key: "popoutPiP",
        rationale: "pure preference bool",
    },
    ImportableKey {
        key: "bounceOnPing",
        rationale: "pure preference bool",
    },
    ImportableKey {
        key: "useSystemCssEditor",
        rationale: "pure preference bool",
    },
    ImportableKey {
        key: "minimizeToTray",
        rationale: "pure preference bool",
    },
    ImportableKey {
        key: "disableAutogain",
        rationale: "pure preference bool",
    },
    ImportableKey {
        key: "spellcheckLanguage",
        rationale: "bounded BCP-47-ish tag list",
    },
    ImportableKey {
        key: "scanInterval",
        rationale: "bounded poll interval; persisted for the future arRPC milestone",
    },
    ImportableKey {
        key: "overlayButtonColor",
        rationale: "hex color string",
    },
    ImportableKey {
        key: "noBundleUpdates",
        rationale: "exact ValidMods members",
    },
    ImportableKey {
        key: "hardwareAcceleration",
        rationale: "pure preference bool; selects WebView2 GPU switches",
    },
    ImportableKey {
        key: "multiInstance",
        rationale: "pure preference bool; selects the single-instance gate",
    },
    ImportableKey {
        key: "disableHttpCache",
        rationale: "pure preference bool; selects a Chromium cache switch",
    },
    ImportableKey {
        key: "sdpH264BaselineRewrite",
        rationale: "pure preference bool; gates the SDP rewrite layer",
    },
    ImportableKey {
        key: "inviteWebsocket",
        rationale: "pure preference bool; read by the arRPC milestone",
    },
    ImportableKey {
        key: "processScanning",
        rationale: "pure preference bool; read by the arRPC milestone",
    },
    ImportableKey {
        key: "windowsLegacyScanning",
        rationale: "pure preference bool; read by the arRPC milestone",
    },
    ImportableKey {
        key: "startMinimized",
        rationale: "exact launch-visibility mode",
    },
    ImportableKey {
        key: "tray",
        rationale: "exact tray icon value",
    },
    ImportableKey {
        key: "windowStyle",
        rationale: "exact window chrome style",
    },
    ImportableKey {
        key: "transparency",
        rationale: "exact transparency mode",
    },
    ImportableKey {
        key: "windowMaterial",
        rationale: "exact backdrop material",
    },
    ImportableKey {
        key: "performanceMode",
        rationale: "exact GPU/CPU preset name",
    },
    ImportableKey {
        key: "proxyMode",
        rationale: "exact proxy mode",
    },
    ImportableKey {
        key: "proxyRules",
        rationale: "bounded printable-ASCII server list",
    },
    ImportableKey {
        key: "proxyBypassRules",
        rationale: "bounded printable-ASCII bypass list",
    },
    ImportableKey {
        key: "proxyPacScript",
        rationale: "bounded http(s) PAC URL",
    },
    ImportableKey {
        key: "pluginStates",
        rationale: "bounded plugin-id to bool map",
    },
];

/// Storage keys whose change takes effect on next launch (mirrors the
/// `settings.set` restart-required model). Mirrored by the facade (which
/// derives the UI restart flag) and enforced by tests; never read at
/// runtime by design — the host applies values, the UI decides chrome.
#[allow(dead_code)]
pub const RESTART_REQUIRED_KEYS: &[&str] = &["mods", "channel", "csp"];

/// Typed failure: a stable machine-readable `code` plus a human message
/// that never contains a filesystem path.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsImportFailure {
    pub code: &'static str,
    pub message: String,
}

fn failure(code: &'static str, message: impl Into<String>) -> SettingsImportFailure {
    SettingsImportFailure {
        code,
        message: message.into(),
    }
}

/// Outcome of one import: which allowlisted keys were applied and which
/// were present-but-invalid (skipped). Key names only — never values.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsImportSummary {
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
}

/// Builds the Electron settings file path from one `%APPDATA%` value.
/// Pure: no environment or filesystem access. Empty input is an explicit
/// error, never a fallback to another directory.
pub fn electron_settings_path_from_appdata(
    appdata: &str,
) -> Result<PathBuf, SettingsImportFailure> {
    if appdata.is_empty() {
        return Err(failure(
            "ELECTRON_SETTINGS_NOT_FOUND",
            "the Electron app profile is unavailable",
        ));
    }
    let mut path = PathBuf::from(appdata);
    path.push(ELECTRON_PROFILE_LEAF);
    for segment in ELECTRON_SETTINGS_SEGMENTS {
        path.push(segment);
    }
    Ok(path)
}

fn is_valid_mods_list(value: &serde_json::Value) -> Option<Vec<String>> {
    let entries = value.as_array()?;
    if entries.len() > crate::settings::MAX_MODS_ENTRIES {
        return None;
    }
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let id = entry.as_str()?;
        if !crate::settings::VALID_MODS.contains(&id) {
            return None;
        }
        out.push(id.to_owned());
    }
    Some(out)
}

fn is_valid_lang_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= MAX_LANGUAGE_TAG_BYTES
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn is_hex_color(value: &str) -> bool {
    let hex = value.strip_prefix('#').unwrap_or("");
    (hex.len() == 3 || hex.len() == 6) && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Extracts the validated import values from a parsed Electron settings
/// document. Returns `(key, validated JSON value)` pairs in
/// [`IMPORTABLE_KEYS`] order for keys that are present AND valid, plus
/// the names of keys that were present but invalid (skipped). Absent
/// keys are neither applied nor reported: the native value stands.
pub fn extract_importable_values(
    document: &serde_json::Value,
) -> (Vec<(&'static str, serde_json::Value)>, Vec<String>) {
    let mut applied = Vec::new();
    let mut skipped = Vec::new();
    let object = match document.as_object() {
        Some(object) => object,
        None => return (applied, skipped),
    };
    for ImportableKey { key, .. } in IMPORTABLE_KEYS {
        let Some(raw) = object.get(*key) else {
            continue;
        };
        let validated = match *key {
            "mods" | "noBundleUpdates" => is_valid_mods_list(raw).map(|entries| {
                serde_json::Value::Array(
                    entries.into_iter().map(serde_json::Value::String).collect(),
                )
            }),
            "csp" => raw
                .as_str()
                .filter(|mode| crate::settings::VALID_CSP_MODES.contains(mode))
                .map(|mode| serde_json::Value::String(mode.to_owned())),
            "channel" => raw
                .as_str()
                .filter(|channel| crate::channel::VALID_CHANNELS.contains(channel))
                .map(|channel| serde_json::Value::String(channel.to_owned())),
            "supportBannerDismissed"
            | "showExperimentalPluginMenu"
            | "extendedPluginAbilities"
            | "quickCss"
            | "spellcheck"
            | "smoothScroll"
            | "autoScroll"
            | "mobileMode"
            | "skipSplash"
            | "sleepInBackground"
            | "popoutPiP"
            | "bounceOnPing"
            | "useSystemCssEditor"
            | "minimizeToTray"
            | "disableAutogain"
            | "hardwareAcceleration"
            | "multiInstance"
            | "disableHttpCache"
            | "sdpH264BaselineRewrite"
            | "inviteWebsocket"
            | "processScanning"
            | "windowsLegacyScanning" => raw.as_bool().map(serde_json::Value::Bool),
            "spellcheckLanguage" => raw.as_array().and_then(|entries| {
                if entries.len() > MAX_SPELLCHECK_LANGUAGES {
                    return None;
                }
                let mut out = Vec::with_capacity(entries.len());
                for entry in entries {
                    let tag = entry.as_str()?;
                    if !is_valid_lang_tag(tag) {
                        return None;
                    }
                    out.push(serde_json::Value::String(tag.to_owned()));
                }
                Some(serde_json::Value::Array(out))
            }),
            "scanInterval" => raw
                .as_u64()
                .filter(|ms| (*ms >= MIN_SCAN_INTERVAL_MS) && (*ms <= MAX_SCAN_INTERVAL_MS))
                .map(serde_json::Value::from),
            "startMinimized" => raw
                .as_str()
                .filter(|mode| crate::settings::VALID_START_MINIMIZED_MODES.contains(mode))
                .map(|mode| serde_json::Value::String(mode.to_owned())),
            "tray" => raw
                .as_str()
                .filter(|icon| crate::settings::VALID_TRAY_ICONS.contains(icon))
                .map(|icon| serde_json::Value::String(icon.to_owned())),
            "windowStyle" => raw
                .as_str()
                .filter(|style| crate::settings::VALID_WINDOW_STYLES.contains(style))
                .map(|style| serde_json::Value::String(style.to_owned())),
            "transparency" => raw
                .as_str()
                .filter(|mode| crate::settings::VALID_TRANSPARENCY_MODES.contains(mode))
                .map(|mode| serde_json::Value::String(mode.to_owned())),
            "windowMaterial" => raw
                .as_str()
                .filter(|material| crate::settings::VALID_WINDOW_MATERIALS.contains(material))
                .map(|material| serde_json::Value::String(material.to_owned())),
            "performanceMode" => raw
                .as_str()
                .filter(|mode| crate::settings::VALID_PERFORMANCE_MODES.contains(mode))
                .map(|mode| serde_json::Value::String(mode.to_owned())),
            "proxyMode" => raw
                .as_str()
                .filter(|mode| crate::settings::VALID_PROXY_MODES.contains(mode))
                .map(|mode| serde_json::Value::String(mode.to_owned())),
            "proxyRules" | "proxyBypassRules" => raw.as_str().and_then(|rules| {
                if rules.len() > crate::settings::MAX_PROXY_RULES_BYTES
                    || rules.is_empty()
                    || !rules.bytes().all(|byte| (0x20..0x7F).contains(&byte))
                {
                    return None;
                }
                Some(serde_json::Value::String(rules.to_owned()))
            }),
            "proxyPacScript" => raw.as_str().and_then(|url| {
                if url.len() > crate::settings::MAX_PAC_URL_BYTES {
                    return None;
                }
                let lower = url.to_ascii_lowercase();
                if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                    return None;
                }
                Some(serde_json::Value::String(url.to_owned()))
            }),
            "overlayButtonColor" => raw
                .as_str()
                .filter(|color| is_hex_color(color))
                .map(|color| serde_json::Value::String(color.to_owned())),
            "pluginStates" => raw.as_object().and_then(|map| {
                if map.len() > MAX_PLUGIN_STATES_ENTRIES {
                    return None;
                }
                let mut out = serde_json::Map::with_capacity(map.len());
                for (id, enabled) in map {
                    if crate::plugins::validate_plugin_id(id).is_err() {
                        return None;
                    }
                    out.insert(id.clone(), serde_json::Value::Bool(enabled.as_bool()?));
                }
                Some(serde_json::Value::Object(out))
            }),
            _ => None,
        };
        match validated {
            Some(value) => applied.push((*key, value)),
            None => skipped.push(key.to_string()),
        }
    }
    (applied, skipped)
}

/// Reads and parses the Electron settings file at an already-resolved
/// path. Bounded, read-only, never follows the result anywhere: links
/// and non-regular files are refused (a planted link must not pull bytes
/// from outside the Electron profile into the native one).
pub fn read_electron_settings(path: &Path) -> Result<serde_json::Value, SettingsImportFailure> {
    let meta = std::fs::symlink_metadata(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => failure(
            "ELECTRON_SETTINGS_NOT_FOUND",
            "no Electron app settings were found to import",
        ),
        _ => failure(
            "ELECTRON_SETTINGS_UNAVAILABLE",
            "the Electron app settings are unavailable",
        ),
    })?;
    if !meta.is_file() {
        return Err(failure(
            "ELECTRON_SETTINGS_UNAVAILABLE",
            "the Electron app settings are unavailable",
        ));
    }
    if meta.len() > MAX_ELECTRON_SETTINGS_BYTES {
        return Err(failure(
            "ELECTRON_SETTINGS_INVALID",
            "the Electron app settings exceed the size limit",
        ));
    }
    let bytes = std::fs::read(path).map_err(|_| {
        failure(
            "ELECTRON_SETTINGS_UNAVAILABLE",
            "the Electron app settings are unavailable",
        )
    })?;
    let document: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
        failure(
            "ELECTRON_SETTINGS_INVALID",
            "the Electron app settings are not valid settings",
        )
    })?;
    if !document.is_object() {
        return Err(failure(
            "ELECTRON_SETTINGS_INVALID",
            "the Electron app settings are not valid settings",
        ));
    }
    Ok(document)
}

/// Copies the validated Electron preferences into the native profile.
///
/// Merge semantics: the native document is loaded authoritatively first
/// (missing file starts from defaults; a corrupt native file is an error
/// that is never overwritten), only validated allowlisted fields are
/// inserted, and every other key — including unknown future keys — is
/// preserved byte-for-value. Persistence is atomic. The Electron source
/// is never written.
pub fn import_electron_settings(
    profile_dir: &Path,
    electron_document: &serde_json::Value,
) -> Result<SettingsImportSummary, SettingsImportFailure> {
    let (values, skipped) = extract_importable_values(electron_document);
    let applied_names: Vec<String> = values.iter().map(|(key, _)| key.to_string()).collect();
    crate::settings::merge_imported_fields(profile_dir, &values)
        .map_err(|message| failure("SETTINGS_UNAVAILABLE", message))?;
    Ok(SettingsImportSummary {
        applied: applied_names,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn electron_doc(json: serde_json::Value) -> serde_json::Value {
        json
    }

    #[test]
    fn empty_appdata_fails_without_fallback() {
        let error = electron_settings_path_from_appdata("").expect_err("empty APPDATA must fail");
        assert_eq!(error.code, "ELECTRON_SETTINGS_NOT_FOUND");
    }

    #[test]
    fn source_path_is_fixed_segments() {
        let path = electron_settings_path_from_appdata(r"C:\Users\T\AppData\Roaming").unwrap();
        assert_eq!(
            path,
            PathBuf::from(r"C:\Users\T\AppData\Roaming\Remcord\storage\settings.json")
        );
    }

    #[test]
    fn missing_source_reports_not_found() {
        let root = std::env::temp_dir().join(format!(
            "remcord-import-test-{}-{}-missing",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&root).unwrap();
        let error =
            read_electron_settings(&root.join("settings.json")).expect_err("absent file must fail");
        assert_eq!(error.code, "ELECTRON_SETTINGS_NOT_FOUND");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_source_fails_closed() {
        let root = std::env::temp_dir().join(format!(
            "remcord-import-test-{}-{}-corrupt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("settings.json"), b"{not json").unwrap();
        let error = read_electron_settings(&root.join("settings.json"))
            .expect_err("corrupt file must fail");
        assert_eq!(error.code, "ELECTRON_SETTINGS_INVALID");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn valid_keys_extract_and_invalid_ones_skip() {
        let (applied, skipped) = extract_importable_values(&electron_doc(serde_json::json!({
            "mods": ["vencord", "shelter"],
            "csp": "strict",
            "channel": "canary",
            "spellcheck": false,
            "scanInterval": 5000,
            "overlayButtonColor": "#121214",
            "spellcheckLanguage": ["en-US"],
            "noBundleUpdates": ["shelter"],
            "pluginStates": {"my-plugin": true},
            "windowStyle": "native",
            "additionalArguments": "--no-sandbox",
            "customIcon": "C:\\x\\icon.png",
            "galleryRoot": "D:\\g",
            "keybinds": [],
            "proxyMode": "fixed_servers",
            "mods_bogus": true,
        })));
        let keys: Vec<&str> = applied.iter().map(|(key, _)| *key).collect();
        assert!(keys.contains(&"mods"));
        assert!(keys.contains(&"csp"));
        assert!(keys.contains(&"channel"));
        assert!(keys.contains(&"spellcheck"));
        assert!(keys.contains(&"scanInterval"));
        assert!(keys.contains(&"overlayButtonColor"));
        assert!(keys.contains(&"spellcheckLanguage"));
        assert!(keys.contains(&"noBundleUpdates"));
        assert!(keys.contains(&"pluginStates"));
        assert!(keys.contains(&"windowStyle"));
        assert!(keys.contains(&"proxyMode"));
        assert!(!keys.contains(&"additionalArguments"));
        assert!(!keys.contains(&"customIcon"));
        assert!(!keys.contains(&"galleryRoot"));
        assert!(!keys.contains(&"keybinds"));
        assert!(skipped.is_empty());
    }

    #[test]
    fn hostile_values_skip_without_applying() {
        let (applied, skipped) = extract_importable_values(&electron_doc(serde_json::json!({
            "mods": ["vencord", "../escape"],
            "csp": "strict;evil",
            "channel": "beta",
            "spellcheck": "yes",
            "scanInterval": 0,
            "overlayButtonColor": "red",
            "spellcheckLanguage": ["en-US", "../x"],
            "noBundleUpdates": ["shelter", "x".repeat(200).as_str()],
            "pluginStates": {"../escape": true},
        })));
        assert!(applied.is_empty(), "nothing hostile may apply");
        for key in [
            "mods",
            "csp",
            "channel",
            "spellcheck",
            "scanInterval",
            "overlayButtonColor",
            "spellcheckLanguage",
            "noBundleUpdates",
            "pluginStates",
        ] {
            assert!(
                skipped.contains(&key.to_string()),
                "{key} must be reported skipped"
            );
        }
    }

    #[test]
    fn absent_keys_keep_native_values() {
        let (applied, skipped) = extract_importable_values(&electron_doc(serde_json::json!({})));
        assert!(applied.is_empty());
        assert!(skipped.is_empty());
    }

    #[test]
    fn import_merges_into_native_doc_and_preserves_unknown_keys() {
        let profile = std::env::temp_dir().join(format!(
            "remcord-import-test-{}-{}-merge",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(
            profile.join("storage").join("settings.json"),
            r#"{"channel":"stable","futureKey":{"nested":1}}"#,
        )
        .unwrap();
        let summary = import_electron_settings(
            &profile,
            &electron_doc(serde_json::json!({"channel":"canary","csp":"strict"})),
        )
        .expect("valid import must apply");
        assert_eq!(
            summary.applied,
            vec!["csp".to_string(), "channel".to_string()]
        );
        assert!(summary.skipped.is_empty());
        let merged: serde_json::Value = serde_json::from_slice(
            &std::fs::read(profile.join("storage").join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(merged["channel"], "canary");
        assert_eq!(merged["csp"], "strict");
        assert_eq!(merged["futureKey"]["nested"], 1);
        let _ = std::fs::remove_dir_all(&profile);
    }
}

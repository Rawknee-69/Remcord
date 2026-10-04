//! Native authoritative settings source for the `settings.get_config` capability.
//!
//! This module is the read-only Rust counterpart of `src/common/config.ts`.
//! The renderer never supplies a filesystem path or a config key: the host
//! builds `<profile>/storage/settings.json` itself, deserializes it into the
//! strongly typed [`SettingsSnapshot`], and returns a validated snapshot.
//!
//! Behavior contract:
//! - File present and valid JSON with correct types → `source: "file"`.
//! - File missing → compiled-in defaults (mirroring `config.ts`) with
//!   `source: "defaults"`. No file is created; this milestone is read-only.
//! - File present but invalid (bad JSON or wrong field types) → `Err`, and
//!   the capability responds with typed `SETTINGS_UNAVAILABLE` instead of
//!   guessing. Unknown extra keys are ignored so a newer Electron config
//!   cannot break the native reader.
//! - Renderer input plays no role: the request payload is empty by protocol.

use std::{collections::HashMap, path::Path};

use serde::{Deserialize, Serialize};

/// Maximum accepted `settings.json` size. The real file is ~2 KiB; anything
/// larger is treated as corrupt rather than parsed.
pub const MAX_SETTINGS_FILE_BYTES: u64 = 256 * 1024;

fn default_spellcheck_language() -> Vec<String> {
    vec!["en-US".to_owned()]
}

fn default_proxy_bypass_rules() -> String {
    "<local>".to_owned()
}

fn default_overlay_button_color() -> String {
    "#121214".to_owned()
}

fn default_custom_js_bundle() -> String {
    "https://legcord.app/placeholder.js".to_owned()
}

fn default_custom_css_bundle() -> String {
    "https://legcord.app/placeholder.css".to_owned()
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSettings {
    #[serde(default)]
    pub workaround: bool,
    #[serde(default = "audio_true")]
    pub device_select: bool,
    #[serde(default = "audio_true")]
    pub granular_select: bool,
    #[serde(default)]
    pub ignore_virtual: bool,
    #[serde(default)]
    pub ignore_devices: bool,
    #[serde(default)]
    pub ignore_input_media: bool,
    #[serde(default)]
    pub only_speakers: bool,
    #[serde(default = "audio_true")]
    pub only_default_speakers: bool,
    #[serde(default = "default_loopback_type")]
    pub loopback_type: String,
}

fn audio_true() -> bool {
    true
}

fn default_loopback_type() -> String {
    "loopback".to_owned()
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            workaround: false,
            device_select: true,
            granular_select: true,
            ignore_virtual: false,
            ignore_devices: false,
            ignore_input_media: false,
            only_speakers: false,
            only_default_speakers: true,
            loopback_type: "loopback".to_owned(),
        }
    }
}

/// Validated native settings snapshot.
///
/// Field names use the exact Electron `settings.json` keys (camelCase) so the
/// native response is shape-compatible with
/// `window.legcord.settings.getConfig()` in Electron. Every field has a
/// default mirroring `src/common/config.ts`; `keybinds` entries are opaque
/// JSON because this read-only milestone validates the envelope, not the
/// keybind schema. `customIcon` defaults to empty (no host path is invented).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    #[serde(rename = "0", default, skip_serializing_if = "Option::is_none")]
    pub broken_marker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore_protocol_warning: Option<bool>,
    #[serde(default)]
    pub custom_icon: String,
    #[serde(default = "default_window_style")]
    pub window_style: String,
    #[serde(default = "default_channel")]
    pub channel: String,
    #[serde(default = "default_transparency")]
    pub transparency: String,
    #[serde(default = "default_window_material")]
    pub window_material: String,
    #[serde(default)]
    pub audio: AudioSettings,
    #[serde(default = "default_csp")]
    pub csp: String,
    #[serde(default = "bool_true")]
    pub minimize_to_tray: bool,
    #[serde(default)]
    pub multi_instance: bool,
    #[serde(default = "bool_true")]
    pub spellcheck: bool,
    #[serde(default)]
    pub mods: Vec<String>,
    #[serde(default)]
    pub mobile_mode: bool,
    #[serde(default)]
    pub skip_splash: bool,
    #[serde(default = "default_performance_mode")]
    pub performance_mode: String,
    #[serde(default = "default_custom_js_bundle")]
    pub custom_js_bundle: String,
    #[serde(default = "default_custom_css_bundle")]
    pub custom_css_bundle: String,
    #[serde(default = "default_start_minimized")]
    pub start_minimized: String,
    /// Opaque passthrough: validated as an array envelope, not per-entry.
    #[serde(default)]
    pub keybinds: Vec<serde_json::Value>,
    #[serde(default = "bool_true")]
    pub hardware_acceleration: bool,
    #[serde(default = "bool_true")]
    pub sdp_h264_baseline_rewrite: bool,
    #[serde(default)]
    pub use_mac_system_picker: bool,
    #[serde(default = "bool_true")]
    pub invite_websocket: bool,
    #[serde(default)]
    pub disable_autogain: bool,
    #[serde(default = "bool_true")]
    pub auto_hide_menu_bar: bool,
    #[serde(default)]
    pub vaapi: bool,
    #[serde(default)]
    pub block_power_saving_in_voice_chat: bool,
    #[serde(default)]
    pub disable_http_cache: bool,
    #[serde(default = "default_tray")]
    pub tray: String,
    #[serde(default)]
    pub done_setup: bool,
    #[serde(default = "default_spellcheck_language")]
    pub spellcheck_language: Vec<String>,
    #[serde(default = "bool_true")]
    pub smooth_scroll: bool,
    #[serde(default)]
    pub bounce_on_ping: bool,
    #[serde(default)]
    pub popout_pip: bool,
    #[serde(default)]
    pub sleep_in_background: bool,
    #[serde(default)]
    pub use_system_css_editor: bool,
    #[serde(default = "bool_true")]
    pub quick_css: bool,
    #[serde(default)]
    pub auto_scroll: bool,
    #[serde(default)]
    pub additional_arguments: String,
    #[serde(default = "default_proxy_mode")]
    pub proxy_mode: String,
    #[serde(default)]
    pub proxy_rules: String,
    #[serde(default = "default_proxy_bypass_rules")]
    pub proxy_bypass_rules: String,
    #[serde(default)]
    pub proxy_pac_script: String,
    #[serde(default)]
    pub no_bundle_updates: Vec<String>,
    #[serde(default)]
    pub automatic_updates: bool,
    #[serde(default = "default_overlay_button_color")]
    pub overlay_button_color: String,
    #[serde(default = "bool_true")]
    pub process_scanning: bool,
    #[serde(default)]
    pub windows_legacy_scanning: bool,
    #[serde(default = "default_scan_interval")]
    pub scan_interval: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mod_cache: Option<HashMap<String, String>>,
    #[serde(default)]
    pub extended_plugin_abilities: bool,
    #[serde(default)]
    pub support_banner_dismissed: bool,
    #[serde(default)]
    pub plugin_states: HashMap<String, bool>,
    #[serde(default)]
    pub show_experimental_plugin_menu: bool,
}

fn bool_true() -> bool {
    true
}

fn default_window_style() -> String {
    "overlay".to_owned()
}

fn default_channel() -> String {
    "stable".to_owned()
}

fn default_transparency() -> String {
    "none".to_owned()
}

fn default_window_material() -> String {
    "mica".to_owned()
}

fn default_csp() -> String {
    "none".to_owned()
}

fn default_performance_mode() -> String {
    "none".to_owned()
}

fn default_start_minimized() -> String {
    "off".to_owned()
}

fn default_tray() -> String {
    "dynamic".to_owned()
}

fn default_proxy_mode() -> String {
    "system".to_owned()
}

fn default_scan_interval() -> u64 {
    5000
}

impl Default for SettingsSnapshot {
    fn default() -> Self {
        serde_json::from_value(serde_json::json!({}))
            .expect("an empty object must deserialize into snapshot defaults")
    }
}

/// Where the returned snapshot came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSource {
    /// Read from `<profile>/storage/settings.json`.
    File,
    /// No settings file exists; compiled-in defaults were returned.
    Defaults,
}

impl SettingsSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SettingsSource::File => "file",
            SettingsSource::Defaults => "defaults",
        }
    }
}

#[derive(Debug)]
pub struct SettingsLoad {
    pub snapshot: SettingsSnapshot,
    pub source: SettingsSource,
    /// Serialized snapshot size in bytes (for DEV metrics).
    pub payload_bytes: usize,
}

/// Host-constructed settings path. The renderer cannot influence this: the
/// profile directory is fixed and the remaining segments are constants.
pub fn settings_file_path(profile_dir: &Path) -> std::path::PathBuf {
    profile_dir.join("storage").join("settings.json")
}

/// Load and validate the authoritative native settings.
///
/// Returns [`SettingsLoad`] on success or a human-readable error (surfaced as
/// typed `SETTINGS_UNAVAILABLE`, never a raw IO error to the renderer).
pub fn load_settings(profile_dir: &Path) -> Result<SettingsLoad, String> {
    let path = settings_file_path(profile_dir);
    let raw = match std::fs::read(&path) {
        Ok(bytes) => {
            if bytes.len() as u64 > MAX_SETTINGS_FILE_BYTES {
                return Err("settings file exceeds the size limit".to_owned());
            }
            Some(bytes)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("settings file is unreadable: {error}")),
    };

    let (snapshot, source) = match raw {
        None => (SettingsSnapshot::default(), SettingsSource::Defaults),
        Some(bytes) => {
            let snapshot: SettingsSnapshot = serde_json::from_slice(&bytes)
                .map_err(|error| format!("settings file is invalid: {error}"))?;
            (snapshot, SettingsSource::File)
        }
    };

    let payload_bytes = serde_json::to_string(&snapshot)
        .map_err(|error| format!("settings snapshot cannot be serialized: {error}"))?
        .len();
    Ok(SettingsLoad {
        snapshot,
        source,
        payload_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::{load_settings, SettingsSnapshot, SettingsSource};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_profile_dir(tag: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be available")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "legcord-settings-{tag}-{nanos}-{}",
            std::process::id()
        ))
    }

    #[test]
    fn missing_file_returns_compiled_defaults() {
        let profile = unique_profile_dir("missing");
        let load = load_settings(&profile).expect("a missing file must yield defaults");
        assert_eq!(load.source, SettingsSource::Defaults);
        assert_eq!(load.source.as_str(), "defaults");
        assert!(!load.snapshot.block_power_saving_in_voice_chat);
        assert_eq!(load.snapshot.window_style, "overlay");
        assert!(load.snapshot.minimize_to_tray);
        assert!(load.payload_bytes > 100);
    }

    #[test]
    fn file_snapshot_round_trips_real_caller_fields() {
        let profile = unique_profile_dir("file");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(
            profile.join("storage").join("settings.json"),
            r#"{"windowStyle":"default","minimizeToTray":false,"transparency":"modern","blockPowerSavingInVoiceChat":true,"mods":["shelter"],"audio":{"granularSelect":false}}"#,
        )
        .unwrap();
        let load = load_settings(&profile).expect("a valid file must load");
        assert_eq!(load.source, SettingsSource::File);
        assert_eq!(load.snapshot.window_style, "default");
        assert!(!load.snapshot.minimize_to_tray);
        assert_eq!(load.snapshot.transparency, "modern");
        assert!(load.snapshot.block_power_saving_in_voice_chat);
        assert_eq!(load.snapshot.mods, vec!["shelter".to_owned()]);
        assert!(!load.snapshot.audio.granular_select);
        // Unspecified fields fall back to defaults.
        assert_eq!(load.snapshot.channel, "stable");
    }

    #[test]
    fn unknown_future_fields_are_ignored() {
        let profile = unique_profile_dir("unknown");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(
            profile.join("storage").join("settings.json"),
            r#"{"blockPowerSavingInVoiceChat":true,"someFutureElectronKey":{"nested":1}}"#,
        )
        .unwrap();
        let load = load_settings(&profile).expect("unknown fields must not break the reader");
        assert_eq!(load.source, SettingsSource::File);
        assert!(load.snapshot.block_power_saving_in_voice_chat);
        let serialized = serde_json::to_value(&load.snapshot).unwrap();
        assert!(serialized.get("someFutureElectronKey").is_none());
    }

    #[test]
    fn wrong_field_types_are_rejected() {
        let profile = unique_profile_dir("wrongtype");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(
            profile.join("storage").join("settings.json"),
            r#"{"blockPowerSavingInVoiceChat":"yes"}"#,
        )
        .unwrap();
        assert!(load_settings(&profile).is_err());
    }

    #[test]
    fn corrupt_json_is_rejected() {
        let profile = unique_profile_dir("corrupt");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(profile.join("storage").join("settings.json"), b"{not json").unwrap();
        let error = load_settings(&profile).expect_err("corrupt JSON must fail");
        assert!(error.contains("invalid"));
    }

    #[test]
    fn snapshot_serializes_with_electron_compatible_keys() {
        let value = serde_json::to_value(SettingsSnapshot::default()).unwrap();
        assert_eq!(value["blockPowerSavingInVoiceChat"], false);
        assert_eq!(value["windowStyle"], "overlay");
        assert_eq!(value["minimizeToTray"], true);
        assert_eq!(value["audio"]["granularSelect"], true);
        assert!(value["mods"].as_array().is_some());
    }

    #[test]
    fn settings_reread_after_reload_returns_fresh_values() {
        let profile = unique_profile_dir("reload");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        let path = profile.join("storage").join("settings.json");
        std::fs::write(&path, r#"{"blockPowerSavingInVoiceChat":false}"#).unwrap();
        assert!(
            !load_settings(&profile)
                .unwrap()
                .snapshot
                .block_power_saving_in_voice_chat
        );
        // Simulate an external settings change between documents.
        std::fs::write(&path, r#"{"blockPowerSavingInVoiceChat":true}"#).unwrap();
        let reread = load_settings(&profile).expect("reread after reload must succeed");
        assert_eq!(reread.source, SettingsSource::File);
        assert!(reread.snapshot.block_power_saving_in_voice_chat);
    }
}

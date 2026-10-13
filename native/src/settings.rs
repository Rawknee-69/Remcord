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
    // Unset by default: never point at another product's bundle endpoint.
    String::new()
}

fn default_custom_css_bundle() -> String {
    // Unset by default: never point at another product's bundle endpoint.
    String::new()
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
/// `window.remcord.settings.getConfig()` in Electron. Every field has a
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

/// Result of the single allowlisted native settings mutation
/// (`blockPowerSavingInVoiceChat`, see [`apply_block_power_saving`]).
/// `source` is always [`SettingsSource::File`]: a successful mutation has
/// just persisted the file. `payload_bytes` is the serialized file size so
/// the renderer can sanity-check the write without re-reading the snapshot.
#[derive(Debug)]
pub struct SettingsMutation {
    pub block_power_saving_in_voice_chat: bool,
    pub source: SettingsSource,
    pub payload_bytes: usize,
}

/// Closed allowlist of renderer-mutable settings keys for the generic
/// `settings.set` capability.
///
/// This enum IS the allowlist: serde rejects any other key string at parse
/// time, before any filesystem work. Every other `Settings` field is
/// deliberately absent:
/// - window chrome / tray / startup (`windowStyle`, `transparency`,
///   `windowMaterial`, `tray`, `minimizeToTray`, `startMinimized`,
///   `skipSplash`, `mobileMode`, `autoHideMenuBar`, `overlayButtonColor`,
///   `bounceOnPing`, `popoutPiP`, `sleepInBackground`): ELECTRON-ONLY, no
///   native window shell consumes them yet. `channel` is allowlisted below
///   (the native boot maps it to its Discord entry URL); `startMinimized`
///   stays out (only its `"minimized"` arm has a native consumer and it
///   rides on window visibility, not on a mutated value).
/// - network / process surface (`proxyMode`, `proxyRules`,
///   `proxyBypassRules`, `proxyPacScript`, `customJsBundle`,
///   `customCssBundle`, `additionalArguments`, `customIcon`,
///   `automaticUpdates`, `disableHttpCache`): UNSAFE to accept from the
///   renderer (URL / path / CLI-switch / proxy injection).
/// - voice / capture (`audio`, `useMacSystemPicker`, `disableAutogain`,
///   `sdpH264BaselineRewrite`, `vaapi`, `hardwareAcceleration`,
///   `performanceMode`, `smoothScroll`, `autoScroll`, `spellcheck`,
///   `spellcheckLanguage`): DEFERRED with the media/voice milestone.
/// - RPC / scanning (`inviteWebsocket`, `processScanning`,
///   `windowsLegacyScanning`, `scanInterval`, `keybinds`): DEFERRED with
///   arRPC; `keybinds` additionally carries arbitrary renderer JS and is
///   UNSAFE as a value.
/// - themes / QuickCSS (`quickCss`, `csp`): owned by their own native
///   capabilities (themes list/toggle/import, Quick CSS host push); the
///   QuickCSS switch is disabled in the native Themes UI, so it has no
///   native caller.
/// - opaque host state (`modCache`, `noBundleUpdates`, `doneSetup`,
///   `channel`, `multiInstance`, `useSystemCssEditor`): DEFERRED.
///
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsSetKey {
    /// Client-mod enablement (`ValidMods[]`). Native consumer: the mod
    /// registry resolves adapters from `settings.mods` at startup; the
    /// Settings UI mod toggles are enabled natively and would otherwise
    /// throw. Takes effect on next launch (restart-required model).
    Mods,
    /// Support-banner dismissal flag. Pure renderer UI state; the banner
    /// is rendered natively and its dismiss button would otherwise throw.
    SupportBannerDismissed,
    /// Experimental plugin-menu flag. Read by the settings plugin at
    /// registration from the boot snapshot; takes effect on next launch.
    ShowExperimentalPluginMenu,
    /// Extended plugin-abilities flag. Persisted for Electron parity (it
    /// gates `pluginWriteFile`/`pluginReadFile` there); the native
    /// plugin-storage capabilities are already capability-gated, so this
    /// carries no host side effect beyond persistence.
    ExtendedPluginAbilities,
    /// Content-Security-Policy mode (`"none"` | `"strict"` | `"vanilla"`).
    /// Native consumer: the document CSP filter applies the selected mode to
    /// every Discord document response (Electron parity with
    /// `src/discord/extensions/csp.ts`, whose default is also `"none"`).
    /// Takes effect on next launch (restart-required model).
    Csp,
    /// Quick CSS enablement flag. Native consumer: the trusted-load pipeline
    /// pushes (or removes) the managed style from this flag, and a
    /// post-mutation re-push applies the flip live, so no restart is needed.
    QuickCss,
    /// Discord release channel (`"stable"` | `"canary"` | `"ptb"`).
    /// Native consumer: startup navigation selects the channel's entry URL
    /// (`src/splash/redirect.html` parity, whose default is also stable).
    /// Takes effect on next launch (restart-required model).
    Channel,
}

impl SettingsSetKey {
    /// Exact `settings.json` document field this key mutates.
    pub fn storage_key(self) -> &'static str {
        match self {
            SettingsSetKey::Mods => "mods",
            SettingsSetKey::SupportBannerDismissed => "supportBannerDismissed",
            SettingsSetKey::ShowExperimentalPluginMenu => "showExperimentalPluginMenu",
            SettingsSetKey::ExtendedPluginAbilities => "extendedPluginAbilities",
            SettingsSetKey::Csp => "csp",
            SettingsSetKey::QuickCss => "quickCss",
            SettingsSetKey::Channel => "channel",
        }
    }
}

/// Bounded value union for the allowlist. The key selects which variant is
/// legal; anything else is a type error. There is deliberately no number,
/// object, or null variant: no allowlisted key needs one. The two string
/// variants serve exactly one key each (`csp` mode, `channel` id), so a
/// value for one can never land in the other's document field.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SettingsSetValue {
    Bool(bool),
    Mods(Vec<String>),
    Csp(String),
    Channel(String),
}

/// Maximum accepted mod-enablement entries. Electron's `ValidMods` has four
/// members; this bound leaves headroom without accepting a spam array.
pub const MAX_MODS_ENTRIES: usize = 8;

/// Exact `ValidMods` union from `src/@types/settings.d.ts`. The renderer
/// cannot invent a mod id through this capability.
pub const VALID_MODS: &[&str] = &["vencord", "equicord", "custom", "shelter"];

/// Exact CSP modes the native document filter implements, mirroring
/// Electron's `src/discord/extensions/csp.ts` switch (`"none"` strips the
/// main-frame policy, `"strict"` replaces it, anything else keeps Discord's
/// vanilla policy). Bounded so the renderer cannot smuggle a policy string
/// through this capability.
pub const VALID_CSP_MODES: &[&str] = &["none", "strict", "vanilla"];

/// Validate one allowlisted mutation before any filesystem work: the value
/// variant must match the key, `mods` entries must be exact `ValidMods`
/// members within the length bound, and `csp` must be an exact mode.
pub fn validate_settings_set(key: SettingsSetKey, value: &SettingsSetValue) -> Result<(), String> {
    match (key, value) {
        (SettingsSetKey::Mods, SettingsSetValue::Mods(entries)) => {
            if entries.len() > MAX_MODS_ENTRIES {
                return Err("mods list exceeds the size limit".to_owned());
            }
            for entry in entries {
                if !VALID_MODS.contains(&entry.as_str()) {
                    return Err("mods list carries an unknown mod id".to_owned());
                }
            }
            Ok(())
        }
        (
            SettingsSetKey::SupportBannerDismissed
            | SettingsSetKey::ShowExperimentalPluginMenu
            | SettingsSetKey::ExtendedPluginAbilities
            | SettingsSetKey::QuickCss,
            SettingsSetValue::Bool(_),
        ) => Ok(()),
        (SettingsSetKey::Csp, SettingsSetValue::Csp(mode)) => {
            if !VALID_CSP_MODES.contains(&mode.as_str()) {
                return Err("csp mode is not a supported policy".to_owned());
            }
            Ok(())
        }
        (SettingsSetKey::Channel, SettingsSetValue::Channel(channel)) => {
            if !crate::channel::VALID_CHANNELS.contains(&channel.as_str()) {
                return Err("channel is not a supported release channel".to_owned());
            }
            Ok(())
        }
        _ => Err("settings value does not match the allowlisted key".to_owned()),
    }
}

/// Result of one allowlisted `settings.set` mutation. `value` is the
/// normalized applied value (the exact `mods` array or bool the host
/// persisted) so the renderer can update local state instead of holding
/// stale data. `source` is always [`SettingsSource::File`].
#[derive(Debug)]
pub struct SettingsSetMutation {
    pub key: SettingsSetKey,
    pub value: SettingsSetValue,
    pub source: SettingsSource,
    pub payload_bytes: usize,
}

/// Load the authoritative JSON document for a write.
///
/// Returns the document to mutate: the parsed live file when settings came
/// from disk, or the defaults snapshot serialized to a document when no file
/// exists. A corrupt/unreadable file is an error and is NEVER converted into
/// defaults here: overwriting corruption with a fresh document would destroy
/// evidence and (via the Electron corrupt-cache fallback) persist a
/// near-empty file.
fn load_document_for_write(
    profile_dir: &Path,
    load: SettingsLoad,
) -> Result<serde_json::Value, String> {
    match load.source {
        SettingsSource::Defaults => serde_json::to_value(&load.snapshot)
            .map_err(|error| format!("settings snapshot cannot be serialized: {error}")),
        SettingsSource::File => {
            let path = settings_file_path(profile_dir);
            let raw = std::fs::read(&path)
                .map_err(|error| format!("settings file is unreadable: {error}"))?;
            let document: serde_json::Value = serde_json::from_slice(&raw)
                .map_err(|error| format!("settings file is invalid: {error}"))?;
            if !document.is_object() {
                return Err("settings file is invalid: top-level JSON must be an object".to_owned());
            }
            Ok(document)
        }
    }
}

/// Persist a mutated settings document atomically: same-directory sibling
/// temp file plus rename, so readers never observe a truncated
/// `settings.json`. Returns the serialized byte length. A temp file is never
/// left behind on failure.
fn persist_settings_document(
    profile_dir: &Path,
    document: &serde_json::Value,
) -> Result<usize, String> {
    let serialized = serde_json::to_string_pretty(document)
        .map_err(|error| format!("settings snapshot cannot be serialized: {error}"))?;
    let path = settings_file_path(profile_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("settings file cannot be written: {error}"))?;
    }
    // Same-directory temp file + rename: the rename is atomic on one
    // filesystem, so readers never observe a truncated `settings.json`.
    let temp_path = path.with_extension("json.tmp");
    if let Err(error) = std::fs::write(&temp_path, serialized.as_bytes()) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("settings file cannot be written: {error}"));
    }
    if let Err(error) = std::fs::rename(&temp_path, &path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("settings file cannot be written: {error}"));
    }
    Ok(serialized.len())
}

/// Apply the ONE allowlisted native settings mutation.
///
/// This is the write half of the read-only [`load_settings`] capability and
/// deliberately NOT a generic `setConfig(key, value)`:
/// - The field is fixed at compile time (`blockPowerSavingInVoiceChat`).
///   There is no key, path, JSON-patch, or object parameter anywhere.
/// - The value is a plain `bool`, already type-checked by the protocol
///   parser before this function runs.
/// - The current settings are loaded authoritatively first: a missing file
///   starts from compiled-in defaults, while a corrupt file (bad JSON or
///   wrong field types) is an error and is NEVER overwritten with defaults.
/// - Unknown future keys are preserved: the write merges the single field
///   into the existing JSON document instead of re-serializing the snapshot.
/// - Persistence is atomic via [`persist_settings_document`].
///
/// Returns [`SettingsMutation`] carrying the applied value for the renderer
/// to update its local state (Electron's synchronous setter returns
/// `undefined` after persisting; the async native caller needs the applied
/// value back to avoid holding stale state).
pub fn apply_block_power_saving(
    profile_dir: &Path,
    value: bool,
) -> Result<SettingsMutation, String> {
    // Authoritative load: validates JSON shape and every known field type.
    let load = load_settings(profile_dir)?;
    let mut document = load_document_for_write(profile_dir, load)?;
    let object = document
        .as_object_mut()
        .ok_or_else(|| "settings file is invalid: top-level JSON must be an object".to_owned())?;
    // ONLY the allowlisted field is touched; every other key
    // (including unknown future keys) is preserved byte-for-value.
    object.insert(
        "blockPowerSavingInVoiceChat".to_owned(),
        serde_json::Value::Bool(value),
    );
    let payload_bytes = persist_settings_document(profile_dir, &document)?;

    Ok(SettingsMutation {
        block_power_saving_in_voice_chat: value,
        source: SettingsSource::File,
        payload_bytes,
    })
}

/// Apply one allowlisted `settings.set` mutation.
///
/// Closed twin of [`apply_block_power_saving`] for the [`SettingsSetKey`]
/// allowlist: the key is an enum (unknown keys never parse) and the value
/// union is validated by [`validate_settings_set`] before any filesystem
/// work. Same ownership semantics: authoritative load first, missing file
/// starts from defaults, corrupt file is an error that is never overwritten,
/// unknown future keys preserved, atomic temp+rename persistence.
///
/// Returns [`SettingsSetMutation`] carrying the normalized applied value so
/// the renderer can update its local state from host-confirmed data.
pub fn apply_settings_set(
    profile_dir: &Path,
    key: SettingsSetKey,
    value: &SettingsSetValue,
) -> Result<SettingsSetMutation, String> {
    validate_settings_set(key, value)?;
    // Authoritative load: validates JSON shape and every known field type.
    let load = load_settings(profile_dir)?;
    let mut document = load_document_for_write(profile_dir, load)?;
    let object = document
        .as_object_mut()
        .ok_or_else(|| "settings file is invalid: top-level JSON must be an object".to_owned())?;
    // ONLY the allowlisted field is touched, with the value variant the
    // key owns: a key/variant mismatch is refused here even if validation
    // were ever bypassed, so a bool can never land in the `csp` document
    // field (or vice versa).
    let stored = match (key, value) {
        (SettingsSetKey::Mods, SettingsSetValue::Mods(entries)) => serde_json::Value::Array(
            entries
                .iter()
                .map(|entry| serde_json::Value::String(entry.clone()))
                .collect(),
        ),
        (SettingsSetKey::Csp, SettingsSetValue::Csp(mode)) => {
            serde_json::Value::String(mode.clone())
        }
        (SettingsSetKey::Channel, SettingsSetValue::Channel(channel)) => {
            serde_json::Value::String(channel.clone())
        }
        (
            SettingsSetKey::SupportBannerDismissed
            | SettingsSetKey::ShowExperimentalPluginMenu
            | SettingsSetKey::ExtendedPluginAbilities
            | SettingsSetKey::QuickCss,
            SettingsSetValue::Bool(flag),
        ) => serde_json::Value::Bool(*flag),
        _ => return Err("settings value does not match the allowlisted key".to_owned()),
    };
    // ONLY the allowlisted field is touched; every other key
    // (including unknown future keys) is preserved byte-for-value.
    object.insert(key.storage_key().to_owned(), stored);
    let payload_bytes = persist_settings_document(profile_dir, &document)?;

    Ok(SettingsSetMutation {
        key,
        value: value.clone(),
        source: SettingsSource::File,
        payload_bytes,
    })
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
    use super::{
        apply_block_power_saving, apply_settings_set, load_settings, validate_settings_set,
        SettingsSetKey, SettingsSetValue, SettingsSnapshot, SettingsSource,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_profile_dir(tag: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be available")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "remcord-settings-{tag}-{nanos}-{}",
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
    fn custom_bundle_defaults_are_empty_and_never_point_off_host() {
        // Remcord ships no bundle endpoint: an empty default must deserialize
        // cleanly and must never trigger a download from another product.
        let profile = unique_profile_dir("bundle-defaults");
        let load = load_settings(&profile).expect("a missing file must yield defaults");
        assert_eq!(load.snapshot.custom_js_bundle, "");
        assert_eq!(load.snapshot.custom_css_bundle, "");
        assert!(!load.snapshot.custom_js_bundle.contains("://"));
        assert!(!load.snapshot.custom_css_bundle.contains("://"));
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

    #[test]
    fn mutation_applies_the_allowlisted_field_and_persists() {
        let profile = unique_profile_dir("mutate-valid");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        let path = profile.join("storage").join("settings.json");
        std::fs::write(
            &path,
            r#"{"windowStyle":"overlay","blockPowerSavingInVoiceChat":false}"#,
        )
        .unwrap();
        let mutation =
            apply_block_power_saving(&profile, true).expect("a valid mutation must persist");
        assert!(mutation.block_power_saving_in_voice_chat);
        assert_eq!(mutation.source, SettingsSource::File);
        assert!(mutation.payload_bytes > 0);
        // No temp file may be left behind and the file must parse.
        assert!(!profile.join("storage").join("settings.json.tmp").exists());
        let reread = load_settings(&profile).expect("the mutated file must load");
        assert!(reread.snapshot.block_power_saving_in_voice_chat);
        assert_eq!(reread.snapshot.window_style, "overlay");
    }

    #[test]
    fn mutation_on_a_missing_file_starts_from_defaults() {
        let profile = unique_profile_dir("mutate-missing");
        let mutation = apply_block_power_saving(&profile, true)
            .expect("a missing file must start from defaults");
        assert!(mutation.block_power_saving_in_voice_chat);
        // A second read reflects the mutation: the file now exists.
        let reread = load_settings(&profile).expect("the created file must load");
        assert_eq!(reread.source, SettingsSource::File);
        assert!(reread.snapshot.block_power_saving_in_voice_chat);
        assert_eq!(reread.snapshot.window_style, "overlay");
    }

    #[test]
    fn mutation_rejects_a_corrupt_file_without_overwriting_it() {
        let profile = unique_profile_dir("mutate-corrupt");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        let path = profile.join("storage").join("settings.json");
        std::fs::write(&path, b"{not json").unwrap();
        let error = apply_block_power_saving(&profile, true).expect_err("a corrupt file must fail");
        assert!(error.contains("invalid"));
        // The corrupt bytes are untouched: no reset-to-defaults after failure.
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
    }

    #[test]
    fn mutation_rejects_wrong_field_types_without_overwriting() {
        let profile = unique_profile_dir("mutate-wrongtype");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        let path = profile.join("storage").join("settings.json");
        let before = r#"{"blockPowerSavingInVoiceChat":"yes"}"#;
        std::fs::write(&path, before).unwrap();
        assert!(apply_block_power_saving(&profile, true).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn mutation_preserves_unknown_future_keys() {
        let profile = unique_profile_dir("mutate-unknown");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        let path = profile.join("storage").join("settings.json");
        std::fs::write(
            &path,
            r#"{"blockPowerSavingInVoiceChat":false,"someFutureElectronKey":{"nested":1}}"#,
        )
        .unwrap();
        apply_block_power_saving(&profile, true).expect("unknown keys must survive a mutation");
        let document: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(document["blockPowerSavingInVoiceChat"], true);
        assert_eq!(document["someFutureElectronKey"]["nested"], 1);
    }

    #[test]
    fn rapid_sequential_mutations_leave_deterministic_valid_state() {
        // The host dispatches bridge requests on one event-loop thread, so
        // native mutations serialize; this test proves repeated writes never
        // corrupt the document and the last write wins deterministically.
        let profile = unique_profile_dir("mutate-rapid");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(
            profile.join("storage").join("settings.json"),
            r#"{"blockPowerSavingInVoiceChat":false}"#,
        )
        .unwrap();
        for value in [true, false, true, false, true] {
            let mutation =
                apply_block_power_saving(&profile, value).expect("sequential writes must succeed");
            assert_eq!(mutation.block_power_saving_in_voice_chat, value);
        }
        let raw = std::fs::read_to_string(profile.join("storage").join("settings.json")).unwrap();
        let document: serde_json::Value =
            serde_json::from_str(&raw).expect("rapid writes must stay valid JSON");
        assert_eq!(document["blockPowerSavingInVoiceChat"], true);
        assert!(
            load_settings(&profile)
                .unwrap()
                .snapshot
                .block_power_saving_in_voice_chat
        );
    }

    #[test]
    fn set_applies_each_bool_key_and_persists() {
        for (key, field) in [
            (
                SettingsSetKey::SupportBannerDismissed,
                "supportBannerDismissed",
            ),
            (
                SettingsSetKey::ShowExperimentalPluginMenu,
                "showExperimentalPluginMenu",
            ),
            (
                SettingsSetKey::ExtendedPluginAbilities,
                "extendedPluginAbilities",
            ),
            (SettingsSetKey::QuickCss, "quickCss"),
        ] {
            let profile = unique_profile_dir("set-bool");
            std::fs::create_dir_all(profile.join("storage")).unwrap();
            let path = profile.join("storage").join("settings.json");
            std::fs::write(&path, r#"{"windowStyle":"overlay"}"#).unwrap();
            let mutation = apply_settings_set(&profile, key, &SettingsSetValue::Bool(true))
                .expect("an allowlisted bool mutation must persist");
            assert_eq!(mutation.key, key);
            assert_eq!(mutation.value, SettingsSetValue::Bool(true));
            assert_eq!(mutation.source, SettingsSource::File);
            assert!(mutation.payload_bytes > 0);
            assert!(!profile.join("storage").join("settings.json.tmp").exists());
            let document: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(document[field], true);
            // Sibling keys survive the merge.
            assert_eq!(document["windowStyle"], "overlay");
        }
    }

    #[test]
    fn set_applies_mods_and_round_trips() {
        let profile = unique_profile_dir("set-mods");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        let path = profile.join("storage").join("settings.json");
        std::fs::write(&path, r#"{"mods":[]}"#).unwrap();
        let mutation = apply_settings_set(
            &profile,
            SettingsSetKey::Mods,
            &SettingsSetValue::Mods(vec!["vencord".to_owned()]),
        )
        .expect("an allowlisted mods mutation must persist");
        assert_eq!(mutation.key, SettingsSetKey::Mods);
        let reread = load_settings(&profile).expect("the mutated file must load");
        assert_eq!(reread.snapshot.mods, vec!["vencord".to_owned()]);
        // Electron's mutual-exclusion shape (exactly what toggleMod builds)
        // persists verbatim.
        apply_settings_set(
            &profile,
            SettingsSetKey::Mods,
            &SettingsSetValue::Mods(vec!["equicord".to_owned()]),
        )
        .expect("replacing the mods array must persist");
        let reread = load_settings(&profile).expect("the replaced file must load");
        assert_eq!(reread.snapshot.mods, vec!["equicord".to_owned()]);
    }

    #[test]
    fn set_on_a_missing_file_starts_from_defaults() {
        let profile = unique_profile_dir("set-missing");
        let mutation = apply_settings_set(
            &profile,
            SettingsSetKey::SupportBannerDismissed,
            &SettingsSetValue::Bool(true),
        )
        .expect("a missing file must start from defaults");
        assert_eq!(mutation.value, SettingsSetValue::Bool(true));
        let reread = load_settings(&profile).expect("the created file must load");
        assert_eq!(reread.source, SettingsSource::File);
        assert_eq!(reread.snapshot.window_style, "overlay");
    }

    #[test]
    fn set_rejects_a_corrupt_file_without_overwriting_it() {
        let profile = unique_profile_dir("set-corrupt");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        let path = profile.join("storage").join("settings.json");
        std::fs::write(&path, b"{not json").unwrap();
        let error = apply_settings_set(
            &profile,
            SettingsSetKey::Mods,
            &SettingsSetValue::Mods(vec!["vencord".to_owned()]),
        )
        .expect_err("a corrupt file must fail");
        assert!(error.contains("invalid"));
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
    }

    #[test]
    fn set_rejects_mismatched_value_types() {
        // Bool keys refuse the mods array; the mods key refuses a bool; the
        // csp key refuses both, and every other key refuses a csp string.
        assert!(validate_settings_set(
            SettingsSetKey::SupportBannerDismissed,
            &SettingsSetValue::Mods(vec!["vencord".to_owned()]),
        )
        .is_err());
        assert!(
            validate_settings_set(SettingsSetKey::Mods, &SettingsSetValue::Bool(true)).is_err()
        );
        assert!(
            validate_settings_set(SettingsSetKey::Csp, &SettingsSetValue::Bool(true),).is_err()
        );
        assert!(validate_settings_set(
            SettingsSetKey::Csp,
            &SettingsSetValue::Mods(vec!["vencord".to_owned()]),
        )
        .is_err());
        assert!(validate_settings_set(
            SettingsSetKey::Mods,
            &SettingsSetValue::Csp("none".to_owned()),
        )
        .is_err());
        assert!(validate_settings_set(
            SettingsSetKey::SupportBannerDismissed,
            &SettingsSetValue::Csp("none".to_owned()),
        )
        .is_err());
    }

    #[test]
    fn set_accepts_only_the_three_csp_modes() {
        for mode in ["none", "strict", "vanilla"] {
            assert!(
                validate_settings_set(
                    SettingsSetKey::Csp,
                    &SettingsSetValue::Csp(mode.to_owned()),
                )
                .is_ok(),
                "{mode} must validate"
            );
        }
        for hostile in [
            "",
            "NONE",
            "Strict",
            "none ",
            "strict;",
            "unsafe-inline",
            "none; script-src *",
            "vanilla\0",
        ] {
            assert!(
                validate_settings_set(
                    SettingsSetKey::Csp,
                    &SettingsSetValue::Csp(hostile.to_owned()),
                )
                .is_err(),
                "{hostile:?} must be refused"
            );
        }
        assert_eq!(SettingsSetKey::Csp.storage_key(), "csp");
    }

    #[test]
    fn set_applies_csp_and_round_trips() {
        let profile = unique_profile_dir("set-csp");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        let path = profile.join("storage").join("settings.json");
        std::fs::write(&path, r#"{"csp":"none"}"#).unwrap();
        let mutation = apply_settings_set(
            &profile,
            SettingsSetKey::Csp,
            &SettingsSetValue::Csp("strict".to_owned()),
        )
        .expect("an allowlisted csp mutation must persist");
        assert_eq!(mutation.key, SettingsSetKey::Csp);
        assert_eq!(mutation.value, SettingsSetValue::Csp("strict".to_owned()));
        let document: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(document["csp"], "strict");
        let reread = load_settings(&profile).expect("the mutated file must load");
        assert_eq!(reread.snapshot.csp, "strict");
    }

    #[test]
    fn set_applies_channel_and_round_trips() {
        for channel in ["stable", "canary", "ptb"] {
            let profile = unique_profile_dir("set-channel");
            std::fs::create_dir_all(profile.join("storage")).unwrap();
            let path = profile.join("storage").join("settings.json");
            std::fs::write(&path, r#"{"channel":"stable"}"#).unwrap();
            let mutation = apply_settings_set(
                &profile,
                SettingsSetKey::Channel,
                &SettingsSetValue::Channel(channel.to_owned()),
            )
            .expect("an allowlisted channel mutation must persist");
            assert_eq!(mutation.key, SettingsSetKey::Channel);
            assert_eq!(
                mutation.value,
                SettingsSetValue::Channel(channel.to_owned())
            );
            let document: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(document["channel"], channel);
            let reread = load_settings(&profile).expect("the mutated file must load");
            assert_eq!(reread.snapshot.channel, channel);
        }
        assert_eq!(SettingsSetKey::Channel.storage_key(), "channel");
    }

    #[test]
    fn set_rejects_unknown_channels_and_crossed_string_variants() {
        for hostile in [
            "",
            "STABLE",
            "beta",
            "stable ",
            "canary.discord.com",
            "ptb\0",
        ] {
            assert!(
                validate_settings_set(
                    SettingsSetKey::Channel,
                    &SettingsSetValue::Channel(hostile.to_owned()),
                )
                .is_err(),
                "{hostile:?} must be refused"
            );
        }
        // A csp string can never validate as a channel (or vice versa): the
        // key owns its variant, so values cannot cross document fields.
        assert!(validate_settings_set(
            SettingsSetKey::Channel,
            &SettingsSetValue::Csp("none".to_owned()),
        )
        .is_err());
        assert!(validate_settings_set(
            SettingsSetKey::Csp,
            &SettingsSetValue::Channel("stable".to_owned()),
        )
        .is_err());
        assert!(
            validate_settings_set(SettingsSetKey::Channel, &SettingsSetValue::Bool(true),).is_err()
        );
        assert!(validate_settings_set(
            SettingsSetKey::Channel,
            &SettingsSetValue::Mods(vec!["vencord".to_owned()]),
        )
        .is_err());
    }

    #[test]
    fn set_rejects_unknown_mod_ids_and_oversized_lists() {
        assert!(validate_settings_set(
            SettingsSetKey::Mods,
            &SettingsSetValue::Mods(vec!["../escape".to_owned()]),
        )
        .is_err());
        assert!(validate_settings_set(
            SettingsSetKey::Mods,
            &SettingsSetValue::Mods(vec!["https://attacker.invalid".to_owned()]),
        )
        .is_err());
        assert!(validate_settings_set(
            SettingsSetKey::Mods,
            &SettingsSetValue::Mods(vec!["Vencord".to_owned()]),
        )
        .is_err());
        let oversized = vec!["vencord".to_owned(); super::MAX_MODS_ENTRIES + 1];
        assert!(
            validate_settings_set(SettingsSetKey::Mods, &SettingsSetValue::Mods(oversized),)
                .is_err()
        );
    }

    #[test]
    fn set_preserves_unknown_future_keys() {
        let profile = unique_profile_dir("set-unknown");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        let path = profile.join("storage").join("settings.json");
        std::fs::write(
            &path,
            r#"{"supportBannerDismissed":false,"someFutureElectronKey":{"nested":1}}"#,
        )
        .unwrap();
        apply_settings_set(
            &profile,
            SettingsSetKey::SupportBannerDismissed,
            &SettingsSetValue::Bool(true),
        )
        .expect("unknown keys must survive a set mutation");
        let document: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(document["supportBannerDismissed"], true);
        assert_eq!(document["someFutureElectronKey"]["nested"], 1);
    }
}

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::backup::{MAX_CLIENT_MODS_BYTES, MAX_CLIENT_MOD_STRING_BYTES, MAX_SHELTER_MOD_BYTES};
use crate::folders::{FolderLocation, MAX_FOLDER_THEME_ID_BYTES};
use crate::localization::MAX_LANG_KEY_BYTES;
use crate::plugin_storage::{
    MAX_PLUGIN_FILE_BYTES, MAX_PLUGIN_ID_BYTES, MAX_PLUGIN_REL_PATH_BYTES,
};
use crate::settings::{SettingsSetKey, SettingsSetValue, SettingsSnapshot};

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_IPC_MESSAGE_BYTES: usize = 8 * 1024;
/// Single large-payload exception to the 8 KiB transport bound: theme code
/// payloads (install and CSS write) carry up to 1 MiB of CSS text plus JSON
/// envelope slack in a single message, so real stylesheets fit without a
/// chunked sub-protocol.
/// Every other command (including the 1 MiB-capable plugin-storage write,
/// whose effective bound stays 8 KiB) is held to `MAX_IPC_MESSAGE_BYTES`.
/// Each CSS body itself is re-checked against `MAX_THEME_CSS_BYTES` below.
pub const MAX_THEME_INSTALL_MESSAGE_BYTES: usize = 1024 * 1024 + 4096;
/// Single large-payload exception for the backup-save request: the
/// renderer-built `clientMods` envelope carries up to 4 MiB plus JSON slack
/// in one message, so real localStorage snapshots fit without a chunked
/// sub-protocol. Every other command keeps the legacy bound.
pub const MAX_BACKUP_SAVE_MESSAGE_BYTES: usize = 4 * 1024 * 1024 + 64 * 1024;
/// Absolute transport ceiling: the larger of the two large-payload
/// exceptions above. Named so both bounds stay live code.
pub const MAX_BRIDGE_MESSAGE_BYTES: usize =
    if MAX_BACKUP_SAVE_MESSAGE_BYTES > MAX_THEME_INSTALL_MESSAGE_BYTES {
        MAX_BACKUP_SAVE_MESSAGE_BYTES
    } else {
        MAX_THEME_INSTALL_MESSAGE_BYTES
    };
pub const MAX_REQUEST_ID_BYTES: usize = 128;
pub const MAX_ERROR_BYTES: usize = 2 * 1024;
/// Maximum bytes per graphics-probe summary string (device/feature facts
/// reflected from the bounded host report; sized above the smoke script's
/// 1024-character slice so multi-byte text never trips the bound).
pub const MAX_GRAPHICS_PROBE_SUMMARY_BYTES: usize = 2048;
pub const MAX_TRACKED_REQUEST_IDS: usize = 4096;
/// Maximum logical mod bundle id length in UTF-8 bytes. Registry ids are
/// short lowercase names (`"vencord"`); the bound mirrors the theme id cap.
pub const MAX_MOD_BUNDLE_ID_BYTES: usize = 64;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeRequest {
    pub version: u8,
    pub request_id: String,
    pub command: BridgeCommand,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum BridgeCommand {
    Ping(PingRequest),
    ModGetBundle(ModGetBundleRequest),
    SettingsGetConfig(SettingsGetConfigRequest),
    SettingsSetBlockPowerSaving(SettingsSetBlockPowerSavingRequest),
    SettingsSet(SettingsSetRequest),
    LocalizationGetLang(LocalizationGetLangRequest),
    AppGetVersion(AppGetVersionRequest),
    PluginStorageRead(PluginStorageReadRequest),
    PluginStorageWrite(PluginStorageWriteRequest),
    BundlesList(BundlesListRequest),
    FoldersOpen(FoldersOpenRequest),
    FoldersOpenTheme(FoldersOpenThemeRequest),
    ThemeInstallCode(ThemeInstallCodeRequest),
    ThemeWriteCss(ThemeWriteCssRequest),
    ThemeImportFiles(ThemeImportFilesRequest),
    ThemeSetEnabled(ThemeSetEnabledRequest),
    ThemesList(ThemesListRequest),
    QuickCssRead(QuickCssReadRequest),
    QuickCssWrite(QuickCssWriteRequest),
    DiagnosticsGetInfo(DiagnosticsGetInfoRequest),
    GraphicsGetInfo(GraphicsGetInfoRequest),
    ModClearCache(ModClearCacheRequest),
    BackupSave(BackupSaveRequest),
    BackupInspect(BackupInspectRequest),
    BackupCommitRestore(BackupCommitRestoreRequest),
    BackupDiscardRestore(BackupDiscardRestoreRequest),
    WindowShow(WindowShowRequest),
    WindowHide(WindowHideRequest),
    WindowMinimize(WindowMinimizeRequest),
    WindowMaximize(WindowMaximizeRequest),
    WindowUnmaximize(WindowUnmaximizeRequest),
    WindowDrag(WindowDragRequest),
    WindowToggleFullscreen(WindowToggleFullscreenRequest),
    WindowQuit(WindowQuitRequest),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PingRequest {
    pub nonce: String,
}

/// Scoped client-mod bundle request: exactly one logical bundle id. There
/// is deliberately NO URL, path, filename, channel, or executable source:
/// the host resolves the id by exact match against its compile-time known
/// mod registry (`KNOWN_MODS`) and rejects everything else. Any extra
/// payload member is rejected by `deny_unknown_fields`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModGetBundleRequest {
    pub mod_id: String,
}

fn is_mod_bundle_id_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
}

/// Shape validation for a logical bundle id: non-empty, bounded, and
/// confined to `[A-Za-z0-9._-]`. This rejects path separators, drive/UNC
/// syntax, URLs, and `javascript:` shapes at parse time; registry
/// membership (exact match) is enforced at dispatch.
pub fn validate_mod_bundle_id(mod_id: &str) -> Result<(), String> {
    if mod_id.is_empty() {
        return Err("mod bundle id must not be empty".to_owned());
    }
    if mod_id.len() > MAX_MOD_BUNDLE_ID_BYTES {
        return Err("mod bundle id exceeds the size limit".to_owned());
    }
    if !mod_id.bytes().all(is_mod_bundle_id_char) {
        return Err("mod bundle id uses characters outside [A-Za-z0-9._-]".to_owned());
    }
    Ok(())
}

/// Validates the echoed registry id on a legacy Vencord CSS result. `None`
/// is the original legacy shape (no echo) and stays accepted; `Some("vencord")`
/// is the shared CSS builder's echo and is accepted; anything else claims a
/// different mod's gate and is refused before any handler runs.
pub fn validate_vencord_css_mod_id(mod_id: &Option<String>) -> Result<(), String> {
    match mod_id {
        None => Ok(()),
        Some(id) if id == "vencord" => Ok(()),
        Some(_) => Err("Vencord CSS result claims a different mod".to_owned()),
    }
}

/// Empty request for the read-only `settings.get_config` capability. There
/// are no renderer-supplied parameters by design: no config key, no file
/// path, no selector. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsGetConfigRequest {}

/// The single allowlisted `settings.set_block_power_saving_in_voice_chat`
/// mutation parameter: the new boolean value. There is deliberately NO key,
/// field name, path, JSON patch, or object: the mutated setting is fixed at
/// compile time, so arbitrary renderer input can never select another field.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsSetBlockPowerSavingRequest {
    pub value: bool,
}

/// The closed `settings.set` mutation: exactly one allowlisted key plus its
/// bounded value. The key is an enum (unknown keys fail deserialization
/// before dispatch); the value is keyed to the key by the custom
/// [`Deserialize`] impl below, so a `csp` string can never parse as a
/// `channel` value (or vice versa) even though both are JSON strings —
/// untagged string variants would accept either. There is deliberately NO
/// path, JSON patch, object, number, or null shape: no allowlisted key
/// needs one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsSetRequest {
    pub key: SettingsSetKey,
    pub value: SettingsSetValue,
}

impl<'de> serde::Deserialize<'de> for SettingsSetRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawRequest {
            key: SettingsSetKey,
            value: serde_json::Value,
        }
        let raw = RawRequest::deserialize(deserializer)?;
        // The key selects the ONLY legal value shape: a bare JSON string
        // parses into the key's own variant, never the other string key's.
        // `serde_json::from_value` rejects wrong JSON types (bool/array/
        // null/object) with a type error before validation or dispatch.
        let value = match raw.key {
            SettingsSetKey::Mods => SettingsSetValue::Mods(
                serde_json::from_value(raw.value).map_err(serde::de::Error::custom)?,
            ),
            SettingsSetKey::Csp => SettingsSetValue::Csp(
                serde_json::from_value(raw.value).map_err(serde::de::Error::custom)?,
            ),
            SettingsSetKey::Channel => SettingsSetValue::Channel(
                serde_json::from_value(raw.value).map_err(serde::de::Error::custom)?,
            ),
            SettingsSetKey::SupportBannerDismissed
            | SettingsSetKey::ShowExperimentalPluginMenu
            | SettingsSetKey::ExtendedPluginAbilities
            | SettingsSetKey::QuickCss => SettingsSetValue::Bool(
                serde_json::from_value(raw.value).map_err(serde::de::Error::custom)?,
            ),
        };
        Ok(SettingsSetRequest {
            key: raw.key,
            value,
        })
    }
}

/// The single allowed `localization.get_lang` parameter: one catalog key.
/// No locale, file, path, URL, or object may be supplied.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalizationGetLangRequest {
    pub key: String,
}

/// Empty request for the read-only `app.get_version` capability. There are no
/// renderer-supplied parameters by design: no channel, key, path, or
/// selector. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppGetVersionRequest {}

/// Scoped plugin-storage read: a logical plugin id plus a logical relative
/// path. There is deliberately NO absolute path, root selector, URL, drive,
/// or file-handle parameter: the host builds the root itself.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginStorageReadRequest {
    pub plugin_id: String,
    pub relative_path: String,
}

/// Scoped plugin-storage write: the same logical addressing as the read plus
/// UTF-8 text data. No binary envelope, no path, no directory-creation flag.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginStorageWriteRequest {
    pub plugin_id: String,
    pub relative_path: String,
    pub data: String,
}

/// Empty request for the read-only `backup.list_bundles` discovery
/// capability. There are no renderer-supplied parameters by design: no path,
/// no URL, no include flags, no selector. Any extra field is rejected, so a
/// renderer cannot narrow, widen, or redirect the fixed allowlist inventory.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundlesListRequest {}

/// Reveal request for the `folders.open` capability: exactly one closed
/// location enum. There is deliberately NO path, URL, directory name, or
/// selector: the host builds the fixed root itself. Unknown variants are
/// rejected during deserialization, before any filesystem or shell work.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoldersOpenRequest {
    pub location: FolderLocation,
}

/// Open request for the `folders.open_theme` capability: exactly one
/// validated theme id. There is deliberately NO path, URL, or directory
/// parameter: the host joins the id onto its own themes root and enforces
/// containment. Charset and length are re-checked at dispatch.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoldersOpenThemeRequest {
    pub id: String,
}

/// Install request for the `themes.install_code` capability: exactly one
/// bounded CSS text. There is deliberately NO URL, path, filename, theme
/// id, or destination: the host derives the id from `@name`, fixes the
/// `src.css` filename, and builds the directory itself. Length is checked
/// here; marker/name validation lives in `themes` at dispatch.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeInstallCodeRequest {
    pub css: String,
}

/// Write request for the `themes.write_css` capability: exactly one
/// validated theme id plus bounded CSS text. There is deliberately NO path,
/// filename, URL, destination, or enablement flag: the host resolves the
/// manifest-declared file under its own themes root. Charset and length are
/// re-checked at dispatch; manifest and containment rules live in `themes`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeWriteCssRequest {
    pub id: String,
    pub css: String,
}

/// Empty request for the `themes.import_files` capability: the host-owned
/// file picker plus bounded local ingestion. There is deliberately NO path,
/// filename, URL, directory, filter, or dialog option: the host defines the
/// entire picker configuration. Any extra field is rejected, so a renderer
/// cannot select, narrow, or redirect what the host reads.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeImportFilesRequest {}

/// Flip request for the `themes.set_enabled` capability: exactly one
/// validated theme id plus the new boolean state. There is deliberately NO
/// path, filename, manifest path, CSS filename, destination, or arbitrary
/// filesystem selector: the host derives the manifest path from the
/// validated id under its own themes root. Charset and length are re-checked
/// at dispatch; manifest and containment rules live in `themes`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeSetEnabledRequest {
    pub id: String,
    pub enabled: bool,
}

/// Empty request for the read-only `themes.list` inventory capability. There
/// is deliberately NO path, root, selector, URL, or filter: the host owns
/// the themes root, enumeration, manifest reads, ordering, and bounds. Any
/// extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemesListRequest {}

/// Empty request for the `quickcss.read` editor capability: returns the
/// fixed Quick CSS file text. There is deliberately NO path, filename, URL,
/// or selector: the host reads its own fixed file only. Any extra field is
/// rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuickCssReadRequest {}

/// Write request for the `quickcss.write` editor capability: exactly one
/// bounded CSS text. There is deliberately NO path, filename, URL, or
/// destination: the host always replaces its own fixed file atomically.
/// Length is checked here and re-enforced at dispatch.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuickCssWriteRequest {
    pub css: String,
}

/// Empty request for the read-only `diagnostics.get_info` capability.
/// Returns the host-owned facts from `crate::diagnostics` (app version,
/// WebView2 Runtime version, profile kind, channel, csp, mod ids). There
/// are no renderer-supplied parameters by design: no key, path, selector,
/// or detail flag. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsGetInfoRequest {}

/// Empty request for the `graphics.get_info` capability. Returns the
/// CDP-backed graphics report from `crate::graphics` (WebView2 Runtime
/// version, the exact additional browser arguments, the environment label,
/// plus the `Browser.getVersion` / `System.getInfo` sections, each marked
/// `measured`, `unsupported`, or `unavailable`). There are no
/// renderer-supplied parameters by design: no method name, target, session,
/// or detail flag — the two probed methods and their empty parameters are
/// fixed at compile time. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphicsGetInfoRequest {}

/// Empty request for the `mod.clear_cache` capability. Deletes the five
/// host-managed downloaded bundle caches and resets the stored `modCache`
/// map (`crate::mod_cache`). There are no renderer-supplied parameters by
/// design: no mod id, path, filename, or selector. Any extra field is
/// rejected, so user-owned `custom.js`/`custom.css` can never be selected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModClearCacheRequest {}

/// Save request for the `backup.save` capability: exactly the include flags
/// plus the bounded renderer-built client-mod envelope. There is deliberately
/// NO path, filename, archive path, source path, dialog option, or root: the
/// host owns collection, the manifest, ZIP creation, the dialog, and the
/// final write. Unknown keys at any level are rejected, and the envelope
/// bounds live in `backup` and are re-enforced at dispatch.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BackupSaveRequest {
    pub includes: crate::backup::BackupIncludes,
    pub client_mods: crate::backup::BackupClientMods,
}

/// Empty request for the `backup.inspect` capability: the host-owned open
/// dialog plus bounded validation and staging, WITHOUT committing. The
/// staged bytes are held host-side for one explicit commit or discard, so
/// the preview the renderer shows cannot drift from what the commit would
/// apply. Same no-parameter contract as the restore picker it replaces.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupInspectRequest {}

/// Empty request for the `backup.commit_restore` capability: commits the
/// single held staged restore into the profile. There is deliberately NO
/// archive path, bytes, manifest, or selector: the commit applies exactly
/// the held staged bytes (validated at inspect time, never out of host
/// hands), so preview and commit cannot disagree. Committing with nothing
/// held is a typed error, never a silent no-op.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupCommitRestoreRequest {}

/// Empty request for the `backup.discard_restore` capability: drops the
/// single held staged restore without touching the profile. Discarding with
/// nothing held resolves `discarded: false`, never an error.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupDiscardRestoreRequest {}

/// Empty request for the `window.show` capability: reveal the single host
/// window (restore when minimized, clear skip-taskbar where supported, show,
/// focus). There is deliberately NO window id, handle, coordinate, style, or
/// message parameter: the host owns the one Remcord window. Any extra field
/// is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowShowRequest {}

/// Empty request for the `window.hide` capability: hide the single host
/// window. No parameter of any kind; any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowHideRequest {}

/// Empty request for the `window.minimize` capability: minimize the single
/// host window. No parameter of any kind; any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowMinimizeRequest {}

/// Empty request for the `window.maximize` capability: maximize the single
/// host window. No parameter of any kind; any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowMaximizeRequest {}

/// Empty request for the `window.unmaximize` capability: restore the single
/// host window from maximized state. No parameter of any kind; any extra
/// field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowUnmaximizeRequest {}

/// Empty request for the `window.drag` capability: begin a native
/// titlebar drag of the single host window (frameless chrome has no
/// OS-provided drag region; the titlebar plugin calls this from an
/// explicit drag handle). No parameter of any kind; any extra field is
/// rejected. This is a single fixed operation, not a generic host-control
/// RPC: no window id, coordinate, or message can be supplied.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowDragRequest {}

/// Empty request for the `window.toggle_fullscreen` capability: flip the
/// single host window between windowed and borderless fullscreen (the F11
/// contract; the renderer owns the key binding, the host owns the state).
/// No parameter of any kind; any extra field is rejected. Like `window.drag`,
/// this is one fixed operation, not a generic host-control RPC.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowToggleFullscreenRequest {}

/// Empty request for the `window.quit` capability: terminate the application
/// through the existing native shutdown path. No parameter of any kind; any
/// extra field is rejected. There is deliberately NO `window.close`: Electron
/// exposes no renderer `win-close` operation, so none is added.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowQuitRequest {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeEventMessage {
    pub version: u8,
    pub event: BridgeEvent,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum BridgeEvent {
    RoundTripAck(RoundTripAck),
    CssProbe(CssProbe),
    RendererError(RendererError),
    BootstrapMetrics(BootstrapMetrics),
    /// Mod-consumer events. Unlike the historically Vencord-named variants
    /// below, every variant here is mod-agnostic and carries an explicit
    /// `mod_id`, so one event family serves every registered mod and adding a
    /// new consumer requires no protocol change. The host resolves the id
    /// against its registry AND cross-checks it against the mod that owns the
    /// correlated request, so a renderer cannot report another mod's verdict.
    ModCssResult(ModCssResult),
    ModBootstrapResult(ModBootstrapResult),
    ModAdapterError(ModAdapterError),
    ModResponseAck(ModResponseAck),
    VencordCssResult(VencordCssResult),
    VencordBootstrapResult(VencordBootstrapResult),
    VencordAdapterError(VencordAdapterError),
    VencordResponseAck(VencordResponseAck),
    InteractionProbe(InteractionProbe),
    /// DEV-only diagnostic posted by the settings smoke probe
    /// (`REMCORD_NATIVE_SETTINGS_SMOKE=1`). Never sent by shipped adapters.
    SettingsProbe(SettingsProbe),
    /// DEV-only diagnostic posted by the settings mutation smoke probe
    /// (`REMCORD_NATIVE_SETTINGS_MUTATION_SMOKE=1`). Never sent by shipped
    /// adapters.
    SettingsMutationProbe(SettingsMutationProbe),
    /// DEV-only diagnostic posted by the settings set smoke probe
    /// (`REMCORD_NATIVE_SETTINGS_SET_SMOKE=1`). Never sent by shipped
    /// adapters. `key` is the allowlisted wire key exercised
    /// (`support_banner_dismissed`); `attacks_blocked` counts the
    /// unknown-key / wrong-type / extra-member attempts the facade or host
    /// rejected. The probe never carries settings values or host paths.
    SettingsSetProbe(SettingsSetProbe),
    /// DEV-only diagnostic posted by the localization smoke probe
    /// (`REMCORD_NATIVE_GETLANG_SMOKE=1`). Never sent by shipped adapters.
    GetLangProbe(GetLangProbe),
    /// DEV-only diagnostic posted by the app-version smoke probe
    /// (`REMCORD_NATIVE_APP_VERSION_SMOKE=1`). Never sent by shipped code.
    AppVersionProbe(AppVersionProbe),
    /// DEV-only diagnostic posted by the graphics smoke probe
    /// (`REMCORD_NATIVE_GRAPHICS_SMOKE=1`). Never sent by shipped code.
    GraphicsProbe(GraphicsProbe),
    /// DEV-only diagnostic posted by the plugin-storage smoke probe
    /// (`REMCORD_NATIVE_PLUGIN_STORAGE_SMOKE=1`). Never sent by shipped
    /// code.
    PluginStorageProbe(PluginStorageProbe),
    /// DEV-only diagnostic posted by the theme install smoke probe
    /// (`REMCORD_NATIVE_THEME_INSTALL_SMOKE=1`). Never sent by shipped
    /// code.
    ThemeInstallProbe(ThemeInstallProbe),
    /// DEV-only diagnostic posted by the theme write smoke probe
    /// (`REMCORD_NATIVE_THEME_WRITE_SMOKE=1`). Never sent by shipped
    /// code.
    ThemeWriteProbe(ThemeWriteProbe),
    /// DEV-only diagnostic posted by the theme import smoke probe
    /// (`REMCORD_NATIVE_THEME_IMPORT_SMOKE=1`). Never sent by shipped
    /// code.
    ThemeImportProbe(ThemeImportProbe),
    /// DEV-only diagnostic posted by the theme enable smoke probe
    /// (`REMCORD_NATIVE_THEME_SET_ENABLED_SMOKE=1`). Never sent by shipped
    /// code.
    ThemeSetEnabledProbe(ThemeSetEnabledProbe),
    /// DEV-only diagnostic posted by the theme list smoke probe
    /// (`REMCORD_NATIVE_THEME_LIST_SMOKE=1`). Never sent by shipped code.
    ThemeListProbe(ThemeListProbe),
    /// DEV-only diagnostic posted by the backup save probe
    /// (`REMCORD_NATIVE_BACKUP_SAVE_SMOKE=1`). Never sent by shipped code.
    BackupSaveProbe(BackupSaveProbe),
    /// DEV-only diagnostic posted by the backup restore probe
    /// (`REMCORD_NATIVE_BACKUP_RESTORE_SMOKE=1`). Never sent by shipped
    /// code.
    BackupRestoreProbe(BackupRestoreProbe),
    /// DEV-only diagnostic posted by the Shelter loader probe
    /// (`REMCORD_NATIVE_LOADER_PROBE=1`). Never sent by shipped code.
    ShelterLoaderProbe(ShelterLoaderProbe),
    /// Shipped one-way event posted by the `settingsBoot.setLang(locale)`
    /// boot-surface function. Fire-and-forget like Electron's
    /// `ipcRenderer.send("setLang", lang)`: no response is ever sent. The
    /// locale is the only renderer-supplied value and is strictly validated
    /// (non-empty, at most 32 bytes, `[A-Za-z0-9_-]`); no path, URL, file, or
    /// catalog selector can cross this event.
    SettingsBootSetLang(SettingsBootSetLang),
    /// Shipped one-way event posted by the `settingsBoot.restart()` boot-surface
    /// function. Empty payload (any member is rejected); the host relaunches
    /// the application executable and then shuts down orderly, matching
    /// Electron's relaunch semantics (`app.relaunch()` + `app.exit()`). A
    /// document reload would be insufficient: mod adapters are resolved once
    /// at startup and baked into the document-start script, so only a new
    /// process activates newly enabled mods. A spawn failure is loud and
    /// keeps the running app alive instead of stranding the user.
    SettingsBootRestart(SettingsBootRestart),
    /// DEV-only diagnostic posted by the settings boot snapshot smoke probe
    /// (`REMCORD_NATIVE_SETTINGS_BOOT_SMOKE=1`). Never sent by shipped code.
    SettingsBootProbe(SettingsBootProbe),
    /// DEV-only staged report from the settings UI workflow probe
    /// (`REMCORD_NATIVE_SETTINGS_UI_SMOKE=1`). Never sent by shipped code.
    /// `stage` names the workflow step reached (`begin`, `gear`, `open`,
    /// `themes`, `cards`, `enable`, `second`, `disable`, `import`, `folder`,
    /// `reload`, `verify`, `restore`, `done`); `detail` carries small
    /// counts/flags only — never settings values, manifests, translations,
    /// paths, or URLs.
    SettingsUiProbe(SettingsUiProbe),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundTripAck {
    pub nonce: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CssProbe {
    pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererError {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapMetrics {
    pub execution_ms: f64,
}

/// CSS injection result for a registered mod that declares a stylesheet.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModCssResult {
    pub mod_id: String,
    pub request_id: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// Page-world execution verdict for a registered mod. `marker` must equal the
/// descriptor's own `success_marker`; the host rejects any other value, and
/// the adapter independently re-checks it before acknowledging.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModBootstrapResult {
    pub mod_id: String,
    pub request_id: String,
    pub ok: bool,
    pub marker: String,
    pub execution_ms: f64,
    pub local_storage_api: String,
    pub settings_state: String,
    pub error: Option<String>,
}

/// Adapter-level failure for a registered mod. Never carries a source, path,
/// URL, or filename — only a bounded message.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModAdapterError {
    pub mod_id: String,
    pub message: String,
}

/// Renderer acknowledgement that the host response was accepted.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModResponseAck {
    pub mod_id: String,
    pub request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VencordCssResult {
    pub request_id: String,
    pub ok: bool,
    pub error: Option<String>,
    /// Registry id echoed by the shared CSS builder (which always echoes it,
    /// exactly like the mod-agnostic family). Absent on the original legacy
    /// shape, which still parses. When present it must name Vencord — any
    /// other value is a cross-mod claim and is rejected at dispatch.
    #[serde(default)]
    pub mod_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VencordBootstrapResult {
    pub request_id: String,
    pub ok: bool,
    pub marker: String,
    pub execution_ms: f64,
    pub local_storage_api: String,
    pub settings_state: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VencordAdapterError {
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VencordResponseAck {
    pub request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionProbe {
    pub phase: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsProbe {
    pub ok: bool,
    pub source: String,
    pub payload_bytes: usize,
    pub block_power_saving: bool,
    pub error: Option<String>,
}

/// DEV-only report for one stage of the settings mutation smoke sequence:
/// `stage` is `"mutated"` (write flipped value, re-read, verify) or
/// `"restored"` (write the original value back, re-read, verify). `before`
/// is the value read before the stage's write, `value` the applied value,
/// and `reread_matches` whether a fresh `getConfig` confirmed it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsMutationProbe {
    pub ok: bool,
    pub stage: String,
    pub before: bool,
    pub value: bool,
    pub reread_matches: bool,
    pub error: Option<String>,
}

/// DEV-only report for one stage of the settings set smoke sequence:
/// `stage` is `"mutated"` (flip the allowlisted flag, re-read, verify, and
/// block three attack shapes) or `"restored"` (write the original value
/// back, re-read, verify). `key` is the wire key exercised; `before` is the
/// value read before the stage's write; `reread_matches` whether a fresh
/// `getConfig` confirmed the write; `attacks_blocked` counts the
/// unknown-key / wrong-type / extra-member attempts rejected (exactly 3 on
/// success). The probe never carries host paths.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsSetProbe {
    pub ok: bool,
    pub stage: String,
    pub key: String,
    pub before: bool,
    pub reread_matches: bool,
    pub attacks_blocked: u8,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetLangProbe {
    pub ok: bool,
    pub lang: String,
    pub value: String,
    pub missing_is_empty: bool,
    pub error: Option<String>,
}

/// DEV-only report for the app-version smoke probe: `version` is the exact
/// string the migrated update-checker caller compares against the remote
/// release.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppVersionProbe {
    pub ok: bool,
    pub version: String,
    pub error: Option<String>,
}

/// DEV-only report for the graphics smoke probe. `stage` is `"report"`
/// (run the real `graphics.get_info` capability from the live document and
/// reflect the bounded report back). `browser_status`/`system_status` carry
/// the measured section outcomes (`measured`, `unsupported`, or
/// `unavailable` — unsupported/unavailable sections are still a successful
/// probe of runtime behavior); `device_count`/`feature_count` count the
/// reported inventory; `device_summary`/`feature_summary` carry bounded
/// parsed GPU facts only (never the raw command line, paths, or file
/// contents). `ok` is true exactly when the facade call resolved with a
/// shape-valid report.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphicsProbe {
    pub ok: bool,
    pub stage: String,
    pub browser_status: String,
    pub system_status: String,
    pub device_count: usize,
    pub feature_count: usize,
    pub device_summary: String,
    pub feature_summary: String,
    pub webgl_status: String,
    pub webgl_summary: String,
    pub error: Option<String>,
}

/// DEV-only report for the plugin-storage smoke probe. `stage` is
/// `"write_read"` (write a fixture, read it back, verify bytes, then block
/// three attack paths). `attacks_blocked` counts the traversal/absolute/
/// drive attempts the facade or host rejected. The probe never carries file
/// contents or host paths.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginStorageProbe {
    pub ok: bool,
    pub stage: String,
    pub payload_bytes: usize,
    pub round_trip: bool,
    pub attacks_blocked: u8,
    pub error: Option<String>,
}

/// DEV-only report for the theme install smoke probe. `stage` is
/// `"install"` (install a disposable fixture through the real capability,
/// verify the id/enabled/bytes, then block three attack shapes).
/// `attacks_blocked` counts the missing-marker/traversal/oversized attempts
/// the facade or host rejected. `fetch_observed` records the separate CSP
/// network observation (`"allowed"`, `"blocked"`, or `"error"`); it never
/// affects the install verdict. The probe never carries CSS contents or
/// host paths.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeInstallProbe {
    pub ok: bool,
    pub stage: String,
    pub id: String,
    pub css_bytes: usize,
    pub enabled: bool,
    pub attacks_blocked: u8,
    pub fetch_observed: String,
    pub error: Option<String>,
}

/// DEV-only report for the theme write smoke probe. `stage` is `"write"`
/// (replace a pre-placed enabled fixture's CSS through the real capability,
/// verify the id/enabled/bytes plus in-WebView reflection, then block three
/// attack shapes). `reflected` reports whether the new CSS custom property
/// was observed via `getComputedStyle`. The probe never carries CSS
/// contents or host paths.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeWriteProbe {
    pub ok: bool,
    pub stage: String,
    pub id: String,
    pub css_bytes: usize,
    pub enabled: bool,
    pub attacks_blocked: u8,
    pub reflected: bool,
    pub error: Option<String>,
}

/// DEV-only report for the theme import smoke probe. `stage` is `"import"`
/// (open the real host picker through the capability; user completion or
/// dismissal resolves the bounded summary). Unlike the install/write probes
/// there is no `attacks_blocked`: the facade takes no input, so there is no
/// client-side attack shape to block. The probe never carries CSS contents,
/// theme ids, or host paths.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeImportProbe {
    pub ok: bool,
    pub stage: String,
    pub canceled: bool,
    pub imported: u32,
    pub failed: u32,
    pub error: Option<String>,
}

/// DEV-only report for the theme enable smoke probe. `stage` is
/// `"set_enabled"` (flip a disposable theme's flag through the real
/// capability, prove the managed style node appears with the exact CSS
/// bytes, prove a same-state flip is a no-op, unflip and prove removal,
/// then block three attack shapes). `reflected` reports whether the managed
/// node carried the exact installed bytes; `multi` reports whether a second
/// enabled disposable theme coexisted (no mutual exclusion). `clearvision`
/// reports the observed-problem fixture when present (`"reflected"`),
/// `"absent"` when its directory does not exist, or `"failed"`. The probe
/// never carries CSS contents or host paths.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeSetEnabledProbe {
    pub ok: bool,
    pub stage: String,
    pub id: String,
    pub enabled: bool,
    pub attacks_blocked: u8,
    pub reflected: bool,
    pub multi: bool,
    pub clearvision: String,
    pub error: Option<String>,
}

/// DEV-only report for the theme list smoke probe. `stage` is `"list"`
/// (inventory the host themes root through the real capability and verify
/// the bounded display shape). `count` is the inventoried entry count;
/// `clearvision` reports the observed-problem fixture when present
/// (`"present-enabled"` / `"present-disabled"`), `"absent"` when its
/// directory does not exist, or `"failed"`. The probe never carries CSS
/// contents or host paths.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeListProbe {
    pub ok: bool,
    pub stage: String,
    pub count: u32,
    pub clearvision: String,
    pub error: Option<String>,
}

/// DEV-only report for the backup save probe. `stage` is `"save"` (run the
/// real `backup.save` capability with all includes through the host dialog;
/// user completion or dismissal resolves the bounded outcome). Unlike the
/// install/write probes there is no `attacks_blocked`: the probe input is a
/// fixed payload, never user data. The probe never carries settings, themes,
/// paths, URLs, or client-mod contents — only counts and flags.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupSaveProbe {
    pub ok: bool,
    pub stage: String,
    pub canceled: bool,
    pub bytes: u64,
    pub error: Option<String>,
}

/// DEV-only report for the backup restore probe. `stage` is `"restore"`
/// (open the real host picker through the capability; user completion or
/// dismissal resolves the bounded outcome). Unlike the install/write probes
/// there is no `attacks_blocked`: the facade takes no input, so there is no
/// client-side attack shape to block. `client_mods_bytes` is the serialized
/// envelope size (0 when canceled); the probe never carries settings,
/// themes, paths, URLs, or client-mod contents — only counts and flags.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupRestoreProbe {
    pub ok: bool,
    pub stage: String,
    pub canceled: bool,
    pub client_mods_bytes: u64,
    pub error: Option<String>,
}

/// DEV-only report for the Shelter plugin-loader probe. `stage` is one of
/// `"presence"`, `"synthetic"`, or `"cleanup"`: the probe first verifies the
/// upstream `shelter.plugins` loader surface exists (and that the URL-based
/// `ensureLoaderPlugin` is NOT exposed), then round-trips one synthetic
/// host-fixed plugin through add/start/stop/remove, then verifies no residue
/// remains. `synthetic` is `"verified"` only on the full cycle, `"skipped"`
/// otherwise. The probe never carries plugin source, paths, URLs, or
/// filesystem surface — only fixed stage strings and booleans.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShelterLoaderProbe {
    pub ok: bool,
    pub stage: String,
    pub shelter: bool,
    pub plugins: bool,
    pub add_local_plugin: bool,
    pub start_plugin: bool,
    pub stop_plugin: bool,
    pub remove_plugin: bool,
    pub url_loader_absent: bool,
    pub synthetic: String,
    pub error: Option<String>,
}

/// Narrow `setLang` event for the settings boot surface. Exactly one bounded
/// locale string; structural validation (plus charset/length checks in the
/// localization owner) rejects everything else before any write.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsBootSetLang {
    pub locale: String,
}

/// Narrow `restart` event for the settings boot surface. The payload must be
/// the exact empty object; any member is rejected by `deny_unknown_fields`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsBootRestart {}

/// DEV-only report for the settings boot snapshot smoke probe.
/// `settings_source` is the snapshot's own source enum; `settings_bytes` is
/// the serialized config length (0 when unavailable); `themes_count` and
/// `i18n_keys` are bounded counts (never contents); `frozen` reports whether
/// the snapshot object is deeply frozen; `keys_exact` reports whether the
/// surface carries exactly the seven approved members. The probe never
/// carries settings values, theme manifests, translations, paths, or URLs.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsBootProbe {
    pub ok: bool,
    pub settings_source: String,
    pub settings_bytes: u32,
    pub themes_count: u32,
    pub i18n_keys: u32,
    pub lang: String,
    pub frozen: bool,
    pub keys_exact: bool,
    pub snapshot_bytes: u32,
    pub error: Option<String>,
}

/// DEV-only staged report from the settings UI workflow probe. Bounded
/// stage/detail strings plus one success flag; inconsistency between `ok`
/// and `error` is rejected like every other probe.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsUiProbe {
    pub stage: String,
    pub ok: bool,
    pub detail: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BridgeMessage {
    Request(BridgeRequest),
    Event(BridgeEventMessage),
}

#[derive(Serialize)]
pub struct BridgeResponse {
    pub version: u8,
    pub request_id: String,
    pub ok: bool,
    pub payload: Option<BridgeResponsePayload>,
    pub error: Option<BridgeError>,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum BridgeResponsePayload {
    Ping(PingResponse),
    ModBundle(ModBundleResponse),
    SettingsConfig(Box<SettingsConfigResponse>),
    SettingsMutation(SettingsMutationResponse),
    SettingsSet(SettingsSetResponse),
    LocalizationString(LocalizationStringResponse),
    AppVersion(AppVersionResponse),
    PluginStorageRead(PluginStorageReadResponse),
    PluginStorageWrite(PluginStorageWriteResponse),
    BundlesList(BundlesListResponse),
    FoldersOpened(FoldersOpenedResponse),
    ThemeFolderOpened(ThemeFolderOpenedResponse),
    ThemeInstalled(ThemeInstalledResponse),
    ThemeWritten(ThemeWrittenResponse),
    ThemeImported(ThemeImportedResponse),
    ThemeEnabled(ThemeEnabledResponse),
    ThemesList(ThemesListResponse),
    QuickCssRead(QuickCssReadResponse),
    QuickCssWritten(QuickCssWrittenResponse),
    DiagnosticsInfo(DiagnosticsInfoResponse),
    GraphicsInfo(Box<crate::graphics::GraphicsInfoResponse>),
    ModCacheCleared(ModCacheClearedResponse),
    BackupSaved(BackupSavedResponse),
    BackupInspected(BackupInspectedResponse),
    BackupRestored(BackupRestoredResponse),
    BackupRestoreDiscarded(BackupRestoreDiscardedResponse),
    WindowAck(WindowAckResponse),
}

#[derive(Serialize)]
pub struct BridgeError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Serialize)]
pub struct PingResponse {
    pub nonce: String,
    pub message: &'static str,
}

#[derive(Default)]
pub struct RequestTracker {
    seen: HashSet<String>,
    in_flight: HashSet<String>,
}

/// Validated bundle metadata response for the scoped `mod.get_bundle`
/// capability. Carries ONLY what the substrate pipeline needs: the registry
/// id echo (so the adapter can confirm identity), the validated build
/// version, byte sizes, timing measurements, the descriptor's success
/// marker, and the per-document execution count. There is deliberately NO
/// bundle source, URL, path, filename, cache layout, or executable content:
/// the multi-hundred-kilobyte sources travel host-side only (disk cache +
/// page-world execution) and never cross the bridge.
#[derive(Serialize)]
pub struct ModBundleResponse {
    pub mod_id: &'static str,
    pub bundle_version: String,
    pub bundle_bytes: usize,
    /// `None` for a mod whose descriptor declares no stylesheet, so "ships no
    /// CSS" is distinguishable from "ships an empty stylesheet".
    pub css_bytes: Option<usize>,
    pub request_latency_ms: f64,
    pub retrieval_ms: f64,
    pub execution_ms: f64,
    /// `None` when no CSS stage ran; otherwise the measured injection time.
    pub css_injection_ms: Option<f64>,
    pub marker: &'static str,
    pub document_execution_count: u8,
}

/// Read-only validated settings snapshot response. `source` is `"file"` when
/// the snapshot came from `<profile>/storage/settings.json` and `"defaults"`
/// when no settings file exists.
#[derive(Serialize)]
pub struct SettingsConfigResponse {
    pub settings: SettingsSnapshot,
    pub source: &'static str,
    pub payload_bytes: usize,
}

/// Validated response for the allowlisted `blockPowerSavingInVoiceChat`
/// mutation. Carries the applied value (so the renderer can update local
/// state instead of holding stale data), the persistence source (always
/// `"file"` after a successful write), and the persisted file size.
/// Electron's synchronous setter returns `undefined`; the async native
/// caller needs the applied value back, hence this minimal shape instead of
/// a full snapshot.
#[derive(Serialize)]
pub struct SettingsMutationResponse {
    pub block_power_saving_in_voice_chat: bool,
    pub source: &'static str,
    pub payload_bytes: usize,
}

/// Validated response for one allowlisted `settings.set` mutation: the
/// applied key echo (so one key's write can never resolve another key's
/// call), the normalized applied value, the persistence source (always
/// `"file"` after a successful write), and the persisted file size.
#[derive(Serialize)]
pub struct SettingsSetResponse {
    pub key: SettingsSetKey,
    pub value: SettingsSetValue,
    pub source: &'static str,
    pub payload_bytes: usize,
}

/// Single localized string response. `value` is `""` for a missing key
/// (Electron parity); `lang` is the resolved language selection.
#[derive(Serialize)]
pub struct LocalizationStringResponse {
    pub value: String,
    pub lang: String,
}

/// Single application version response. `version` is the exact string
/// Electron serves for `window.remcord.version` (`"0.0.0"` on dev builds,
/// otherwise the package version).
#[derive(Serialize)]
pub struct AppVersionResponse {
    pub version: String,
}

/// Scoped plugin-storage read response: the file text plus its byte length.
/// No path, no metadata, no directory listing.
#[derive(Serialize)]
pub struct PluginStorageReadResponse {
    pub data: String,
    pub payload_bytes: usize,
}

/// Scoped plugin-storage write response: the persisted byte length. The
/// envelope `ok` already signals success; no path is echoed back.
#[derive(Serialize)]
pub struct PluginStorageWriteResponse {
    pub payload_bytes: usize,
}

/// One inventoried bundle file for the read-only `backup.list_bundles`
/// capability. Only the fixed allowlist name, presence, and byte length
/// cross the bridge: no absolute path, no profile path, no URL, no arbitrary
/// filename, and no filesystem error text containing paths. `bytes` is 0
/// whenever `present` is false.
#[derive(Serialize)]
pub struct BundleFileEntry {
    pub name: String,
    pub present: bool,
    pub bytes: u64,
}

/// Read-only bundle inventory response: exactly one entry per allowlisted
/// bundle name, in allowlist order. Carries metadata only, never contents.
#[derive(Serialize)]
pub struct BundlesListResponse {
    pub bundles: Vec<BundleFileEntry>,
}

/// Reveal response for the `folders.open` capability: the validated location
/// echo, so the renderer can confirm which fixed root was revealed. No path
/// crosses the boundary.
#[derive(Serialize)]
pub struct FoldersOpenedResponse {
    pub location: FolderLocation,
}

/// Open response for the `folders.open_theme` capability: the validated
/// theme id echo. No path crosses the boundary.
#[derive(Serialize)]
pub struct ThemeFolderOpenedResponse {
    pub id: String,
}

/// Install response for the `themes.install_code` capability: the
/// host-derived directory id, the persisted enablement (always `false` —
/// Electron parity for fresh installs), and the stored CSS size. No path,
/// URL, or source echo crosses the boundary.
#[derive(Serialize)]
pub struct ThemeInstalledResponse {
    pub id: String,
    pub enabled: bool,
    pub css_bytes: usize,
}

/// Write response for the `themes.write_css` capability: the validated id
/// echo, the preserved enablement (read from the untouched manifest), and
/// the stored CSS size. No path or source echo crosses the boundary.
#[derive(Serialize)]
pub struct ThemeWrittenResponse {
    pub id: String,
    pub enabled: bool,
    pub css_bytes: usize,
}

/// Bounded summary response for the `themes.import_files` capability:
/// cancellation plus counts only. No theme ids (a future list refresh owns
/// those), no arrays, no paths, no filesystem error text.
#[derive(Serialize)]
pub struct ThemeImportedResponse {
    /// True when the user dismissed the picker (or it returned nothing).
    /// `imported` and `failed` are then both zero.
    pub canceled: bool,
    /// Files installed through the shared installer.
    pub imported: u32,
    /// Files skipped with a typed host-side code.
    pub failed: u32,
}

/// Flip response for the `themes.set_enabled` capability: the validated id
/// echo plus the persisted state (which equals the request, including the
/// idempotent same-state no-op). No path crosses the boundary.
#[derive(Serialize)]
pub struct ThemeEnabledResponse {
    pub id: String,
    pub enabled: bool,
}

/// Read-only inventory response for the `themes.list` capability: bounded
/// display metadata only, in sorted-id order. No ids beyond the validated
/// set, no arrays beyond the entry cap, no paths, no CSS contents, no
/// update URLs, no filesystem error text.
#[derive(Serialize)]
pub struct ThemesListResponse {
    pub themes: Vec<crate::themes::ThemeListEntry>,
}

/// Read response for the `quickcss.read` editor capability: the fixed file
/// text plus its byte length. No path, no enablement flag, no filesystem
/// error text.
#[derive(Serialize)]
pub struct QuickCssReadResponse {
    pub css: String,
    pub bytes: usize,
}

/// Write response for the `quickcss.write` editor capability: the persisted
/// byte length. The envelope `ok` already signals success; no path or text
/// is echoed back.
#[derive(Serialize)]
pub struct QuickCssWrittenResponse {
    pub bytes: usize,
}

/// Readout response for the `diagnostics.get_info` capability: the
/// host-owned facts from `crate::diagnostics` re-exported verbatim.
/// Fixed-vocabulary strings plus the filtered mod-id list only — no
/// paths, tokens, cookies, or file contents.
#[derive(Serialize)]
pub struct DiagnosticsInfoResponse {
    pub app_version: String,
    pub webview2_runtime: String,
    pub profile_kind: &'static str,
    pub channel: String,
    pub csp: String,
    pub mods: Vec<String>,
}

/// Outcome response for the `mod.clear_cache` capability: which
/// host-managed cache files were deleted plus whether a stored `modCache`
/// map existed and was reset. Fixed allowlisted names only — no paths.
#[derive(Serialize)]
pub struct ModCacheClearedResponse {
    pub cleared: Vec<String>,
    pub mod_cache_cleared: bool,
}

/// Bounded save response for the `backup.save` capability: cancellation plus
/// the final archive size. No theme ids, no paths, no filenames, no source
/// echo, no filesystem error text — the smallest response that distinguishes
/// cancel from success.
#[derive(Serialize)]
pub struct BackupSavedResponse {
    /// True when the user dismissed the picker. `bytes` is then zero.
    pub canceled: bool,
    /// Final archive size in bytes (zero when canceled).
    pub bytes: u64,
}

/// Bounded inspect response for the `backup.inspect` capability: the
/// user-facing preview of the held staged restore, or cancellation. Counts,
/// flags, and the bounded display listing only — never host paths, never
/// file contents, never the clientMods envelope itself.
#[derive(Serialize)]
pub struct BackupInspectedResponse {
    /// True when the user dismissed the picker. Every other field is then
    /// zeroed/empty (no restore is held).
    pub canceled: bool,
    /// Manifest include flags: which sections the commit would apply.
    pub includes: crate::backup::BackupIncludes,
    /// Picked archive size in bytes.
    pub archive_bytes: usize,
    /// Total staged uncompressed content in bytes.
    pub uncompressed_bytes: usize,
    /// Staged file entries (manifest excluded).
    pub entry_count: usize,
    /// Serialized clientMods envelope size in bytes.
    pub client_mods_bytes: usize,
    /// Envelope presence flags (contents never cross here).
    pub has_vencord_mod_data: bool,
    /// Envelope presence flags (contents never cross here).
    pub has_equicord_mod_data: bool,
    /// Envelope presence flags (contents never cross here).
    pub has_shelter_mod_data: bool,
    /// Accepted archive names in archive order (display only, bounded).
    pub entries: Vec<String>,
    /// True when `entry_count` exceeds the listing length.
    pub entries_truncated: bool,
}

/// Discard response for the `backup.discard_restore` capability: whether a
/// held staged restore was actually dropped. Discarding with nothing held
/// resolves `discarded: false`, never an error.
#[derive(Serialize)]
pub struct BackupRestoreDiscardedResponse {
    pub discarded: bool,
}

/// Bounded restore response for the `backup.commit_restore` capability:
/// cancellation plus the filtered clientMods envelope for the renderer to
/// apply through the existing mechanism. No archive path, profile path,
/// filename, or filesystem error text — the host never applies clientMods
/// itself (no localStorage, no remote-plugin fetch, no script execution).
#[derive(Serialize)]
pub struct BackupRestoredResponse {
    /// True when the user dismissed the picker. `client_mods` is then null.
    pub canceled: bool,
    /// Filtered `clientMods` envelope (null when canceled).
    pub client_mods: serde_json::Value,
}

/// Acknowledgement for one typed window chrome command. `command` echoes the
/// exact wire type the renderer sent (`window_show`, `window_hide`,
/// `window_minimize`, `window_maximize`, `window_unmaximize`, `window_drag`,
/// `window_toggle_fullscreen`, `window_quit`),
/// so one window's acknowledgement can never resolve another command's call.
/// Electron's `send` commands return `void`; the async bridge needs this
/// minimal correlation instead of a value. No state, handle, or geometry
/// crosses the boundary.
#[derive(Serialize)]
pub struct WindowAckResponse {
    pub command: &'static str,
}

/// Typed host-to-renderer window state event. Replaces Electron's
/// `executeJavaScript` attribute writes for focus/blur/maximize/unmaximize
/// with schema-validated data: the renderer owns the DOM attributes and no
/// generic script execution crosses the boundary. Emitted only when the
/// effective state actually changes — never polled, never per-frame.
#[allow(dead_code)]
#[derive(Serialize)]
pub struct WindowStateEvent {
    pub version: u8,
    pub focused: bool,
    pub maximized: bool,
}

/// Serializes one `window_state` host event detail for delivery as a
/// `remcord-native:window-state` CustomEvent. The shape is fixed by
/// construction; the renderer re-validates every field before applying it.
pub fn window_state_event_json(focused: bool, maximized: bool) -> String {
    serde_json::json!({
        "version": PROTOCOL_VERSION,
        "focused": focused,
        "maximized": maximized,
    })
    .to_string()
}

fn valid_request_id(request_id: &str) -> bool {
    !request_id.is_empty()
        && request_id.len() <= MAX_REQUEST_ID_BYTES
        && request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

impl RequestTracker {
    pub fn begin(&mut self, request_id: &str) -> Result<(), &'static str> {
        if self.seen.contains(request_id) {
            return Err("duplicate request id");
        }
        if self.seen.len() >= MAX_TRACKED_REQUEST_IDS {
            return Err("request id limit reached for this document");
        }

        self.seen.insert(request_id.to_owned());
        self.in_flight.insert(request_id.to_owned());
        Ok(())
    }

    pub fn complete(&mut self, request_id: &str) -> Result<(), &'static str> {
        if self.in_flight.remove(request_id) {
            Ok(())
        } else {
            Err("response has no matching request")
        }
    }

    pub fn clear(&mut self) {
        self.seen.clear();
        self.in_flight.clear();
    }
}

pub fn parse_bridge_message(body: &str) -> Result<BridgeMessage, String> {
    // Absolute ceiling first (one backup save may legitimately approach
    // 4 MiB); the per-command transport bound is enforced after the command
    // is identified below.
    if body.len() > MAX_BRIDGE_MESSAGE_BYTES {
        return Err("message exceeds the size limit".to_owned());
    }

    let message: BridgeMessage = serde_json::from_str(body).map_err(|error| error.to_string())?;
    // Only a theme code payload or a backup save payload may exceed the 8 KiB
    // transport bound. Every other command — requests and events alike —
    // keeps the legacy bound, so the large-payload exception cannot widen any
    // other surface.
    let is_large_payload = matches!(
        &message,
        BridgeMessage::Request(request)
            if matches!(
                request.command,
                BridgeCommand::ThemeInstallCode(_)
                    | BridgeCommand::ThemeWriteCss(_)
                    | BridgeCommand::BackupSave(_)
            )
    );
    if !is_large_payload && body.len() > MAX_IPC_MESSAGE_BYTES {
        return Err("message exceeds the size limit".to_owned());
    }
    match &message {
        BridgeMessage::Request(request) => {
            if request.version != PROTOCOL_VERSION {
                return Err("unsupported bridge version".to_owned());
            }
            if !valid_request_id(&request.request_id) {
                return Err("invalid request id".to_owned());
            }
            match &request.command {
                BridgeCommand::Ping(payload) => {
                    if payload.nonce.is_empty() || payload.nonce.len() > 128 {
                        return Err("invalid nonce".to_owned());
                    }
                }
                BridgeCommand::ModGetBundle(payload) => {
                    // Shape check only (presence + byte cap + charset);
                    // registry membership (exact match against KNOWN_MODS)
                    // is enforced at dispatch.
                    if validate_mod_bundle_id(&payload.mod_id).is_err() {
                        return Err("invalid mod bundle id".to_owned());
                    }
                }
                BridgeCommand::SettingsGetConfig(_) => {}
                // The mutation value is a plain bool: serde already rejected
                // non-boolean, missing, or extra payload members via
                // `deny_unknown_fields`, so no further shape check is needed.
                BridgeCommand::SettingsSetBlockPowerSaving(_) => {}
                // The key is a closed enum (unknown keys fail deserialization
                // before dispatch) and the value union is typed; the
                // key↔variant pairing plus per-key rules (bool-only flags,
                // ValidMods-only arrays, exact csp modes) are re-enforced by
                // the settings owner here (parse) and at dispatch.
                BridgeCommand::SettingsSet(payload) => {
                    if crate::settings::validate_settings_set(payload.key, &payload.value).is_err()
                    {
                        return Err("invalid settings set mutation".to_owned());
                    }
                }
                BridgeCommand::LocalizationGetLang(payload) => {
                    if payload.key.is_empty() || payload.key.len() > MAX_LANG_KEY_BYTES {
                        return Err("invalid localization key".to_owned());
                    }
                }
                // Parameter-free by protocol (`deny_unknown_fields` already
                // rejected any renderer-supplied member), so no shape check
                // is needed.
                BridgeCommand::AppGetVersion(_) => {}
                BridgeCommand::PluginStorageRead(payload) => {
                    // Shape check only (presence + byte caps); charset,
                    // traversal, drive/UNC, and containment rules live in
                    // `plugin_storage` and are re-enforced at dispatch.
                    if payload.plugin_id.is_empty()
                        || payload.plugin_id.len() > MAX_PLUGIN_ID_BYTES
                        || payload.relative_path.is_empty()
                        || payload.relative_path.len() > MAX_PLUGIN_REL_PATH_BYTES
                    {
                        return Err("invalid plugin storage read".to_owned());
                    }
                }
                BridgeCommand::PluginStorageWrite(payload) => {
                    if payload.plugin_id.is_empty()
                        || payload.plugin_id.len() > MAX_PLUGIN_ID_BYTES
                        || payload.relative_path.is_empty()
                        || payload.relative_path.len() > MAX_PLUGIN_REL_PATH_BYTES
                        || payload.data.len() > MAX_PLUGIN_FILE_BYTES
                    {
                        return Err("invalid plugin storage write".to_owned());
                    }
                }
                // Parameter-free by protocol (`deny_unknown_fields` already
                // rejected any renderer-supplied member), so no shape check
                // is needed. There is nothing to validate: the inventory
                // covers the fixed allowlist only.
                BridgeCommand::BundlesList(_) => {}
                // The location is a closed enum validated by serde itself:
                // unknown variants never reach dispatch. No path or
                // selector exists to shape-check.
                BridgeCommand::FoldersOpen(_) => {}
                BridgeCommand::FoldersOpenTheme(payload) => {
                    // Shape check only (presence + byte cap); charset,
                    // traversal, and containment rules live in `folders`
                    // and are re-enforced at dispatch.
                    if payload.id.is_empty() || payload.id.len() > MAX_FOLDER_THEME_ID_BYTES {
                        return Err("invalid theme folder open".to_owned());
                    }
                }
                BridgeCommand::ThemeInstallCode(payload) => {
                    // Transport already bounded above; the CSS body itself
                    // must fit the theme bound. Marker/name rules live in
                    // `themes` and are enforced at dispatch.
                    if payload.css.len() > crate::themes::MAX_THEME_CSS_BYTES {
                        return Err("theme CSS exceeds the size limit".to_owned());
                    }
                }
                BridgeCommand::ThemeWriteCss(payload) => {
                    // Shape check only (id presence/cap, CSS cap); charset,
                    // manifest, and containment rules live in `themes` and
                    // are re-enforced at dispatch.
                    if payload.id.is_empty()
                        || payload.id.len() > MAX_FOLDER_THEME_ID_BYTES
                        || payload.css.len() > crate::themes::MAX_THEME_CSS_BYTES
                    {
                        return Err("invalid theme CSS write".to_owned());
                    }
                }
                // Parameter-free by protocol (`deny_unknown_fields` already
                // rejected any renderer-supplied member), so no shape check
                // is needed. The picker configuration and the ingestion
                // bounds live in `theme_import` and are enforced at dispatch.
                BridgeCommand::ThemeImportFiles(_) => {}
                BridgeCommand::ThemeSetEnabled(payload) => {
                    // Shape check only (id presence/cap; the boolean is
                    // typed by serde); charset, manifest, and containment
                    // rules live in `themes` and are re-enforced at dispatch.
                    if payload.id.is_empty() || payload.id.len() > MAX_FOLDER_THEME_ID_BYTES {
                        return Err("invalid theme enable flip".to_owned());
                    }
                }
                // Parameter-free by protocol (`deny_unknown_fields` already
                // rejected any renderer-supplied member), so no shape check
                // is needed. Enumeration, bounds, and ordering live in
                // `themes` and are enforced at dispatch.
                BridgeCommand::ThemesList(_) => {}
                // The read half carries no parameters by protocol. The write
                // half carries exactly one CSS text, bounded here and
                // re-enforced at dispatch; the destination is always the
                // host-fixed file, so there is nothing else to shape-check.
                BridgeCommand::QuickCssRead(_) => {}
                BridgeCommand::QuickCssWrite(payload) => {
                    if payload.css.len() > crate::quick_css::MAX_QUICK_CSS_BYTES {
                        return Err("Quick CSS exceeds the size limit".to_owned());
                    }
                }
                // Parameter-free by protocol (`deny_unknown_fields` already
                // rejected any renderer-supplied member), so no shape check
                // is needed. The facts live in `diagnostics` (host-owned
                // version/runtime/profile/mode/mod facts); the graphics
                // report lives in `graphics` (fixed CDP probes only, no
                // renderer-selected method); the cache roots and file
                // allowlist live in `mod_cache`. None takes a selector, so
                // there is nothing to validate.
                BridgeCommand::DiagnosticsGetInfo(_)
                | BridgeCommand::GraphicsGetInfo(_)
                | BridgeCommand::ModClearCache(_) => {}
                // Parameter-free by protocol (`deny_unknown_fields` already
                // rejected any renderer-supplied member), so no shape check
                // is needed. The picker configuration plus the archive,
                // manifest, path, size, and clientMods bounds live in
                // `backup` and are enforced at dispatch. The inspect/commit/
                // discard trio shares the same empty contract: staging is
                // held host-side between inspect and commit, so preview and
                // commit cannot disagree.
                BridgeCommand::BackupInspect(_)
                | BridgeCommand::BackupCommitRestore(_)
                | BridgeCommand::BackupDiscardRestore(_) => {}
                // Window chrome commands are parameter-free by protocol
                // (`deny_unknown_fields` already rejected any
                // renderer-supplied member: no window id, handle,
                // coordinate, style, message, or process selector can cross).
                // The host acts on the one Remcord window only.
                BridgeCommand::WindowShow(_)
                | BridgeCommand::WindowHide(_)
                | BridgeCommand::WindowMinimize(_)
                | BridgeCommand::WindowMaximize(_)
                | BridgeCommand::WindowUnmaximize(_)
                | BridgeCommand::WindowDrag(_)
                | BridgeCommand::WindowToggleFullscreen(_)
                | BridgeCommand::WindowQuit(_) => {}
                BridgeCommand::BackupSave(payload) => {
                    // The seven include flags are typed booleans by serde
                    // (`deny_unknown_fields` rejected anything else); the
                    // clientMods envelope bounds live in `backup` and are
                    // enforced here (parse) and re-enforced at dispatch.
                    if payload
                        .client_mods
                        .vencord_local_storage
                        .as_ref()
                        .is_some_and(|text| text.len() > MAX_CLIENT_MOD_STRING_BYTES)
                        || payload
                            .client_mods
                            .equicord_local_storage
                            .as_ref()
                            .is_some_and(|text| text.len() > MAX_CLIENT_MOD_STRING_BYTES)
                    {
                        return Err("backup client data exceeds the size limit".to_owned());
                    }
                    if let Some(shelter) = payload.client_mods.shelter.as_ref() {
                        let serialized = serde_json::to_string(shelter)
                            .map_err(|_| "backup client data exceeds the size limit".to_owned())?;
                        if serialized.len() > MAX_SHELTER_MOD_BYTES
                            || serialized.len() > MAX_CLIENT_MODS_BYTES
                        {
                            return Err("backup client data exceeds the size limit".to_owned());
                        }
                    }
                    let envelope = serde_json::to_string(&payload.client_mods)
                        .map_err(|_| "backup client data exceeds the size limit".to_owned())?;
                    if envelope.len() > MAX_CLIENT_MODS_BYTES {
                        return Err("backup client data exceeds the size limit".to_owned());
                    }
                }
            }
        }
        BridgeMessage::Event(event) => {
            if event.version != PROTOCOL_VERSION {
                return Err("unsupported bridge version".to_owned());
            }
            match &event.event {
                BridgeEvent::RoundTripAck(payload) => {
                    if payload.nonce.is_empty() || payload.nonce.len() > 128 {
                        return Err("invalid round-trip acknowledgement".to_owned());
                    }
                }
                BridgeEvent::CssProbe(payload) => {
                    if payload.value.len() > 128 {
                        return Err("CSS probe result exceeds the size limit".to_owned());
                    }
                }
                BridgeEvent::RendererError(payload) => {
                    if payload.kind.is_empty()
                        || payload.kind.len() > 64
                        || payload.message.len() > MAX_ERROR_BYTES
                    {
                        return Err("invalid renderer error report".to_owned());
                    }
                }
                BridgeEvent::BootstrapMetrics(payload) => {
                    if !payload.execution_ms.is_finite()
                        || !(0.0..=60_000.0).contains(&payload.execution_ms)
                    {
                        return Err("invalid bootstrap execution time".to_owned());
                    }
                }
                BridgeEvent::ModCssResult(payload) => {
                    if validate_mod_bundle_id(&payload.mod_id).is_err() {
                        return Err("invalid mod id in mod CSS result".to_owned());
                    }
                    if !valid_request_id(&payload.request_id)
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid mod CSS result".to_owned());
                    }
                }
                BridgeEvent::ModBootstrapResult(payload) => {
                    if validate_mod_bundle_id(&payload.mod_id).is_err() {
                        return Err("invalid mod id in mod bootstrap result".to_owned());
                    }
                    if !valid_request_id(&payload.request_id)
                        || payload.marker.len() > 64
                        || !payload.execution_ms.is_finite()
                        || !(0.0..=60_000.0).contains(&payload.execution_ms)
                        || !matches!(
                            payload.local_storage_api.as_str(),
                            "available" | "missing" | "blocked"
                        )
                        // "not_applicable" is the honest state for a mod that
                        // keeps its settings outside localStorage: no invented
                        // key is read, and no key lookup is claimed.
                        || !matches!(
                            payload.settings_state.as_str(),
                            "missing"
                                | "valid_json"
                                | "invalid_json"
                                | "read_error"
                                | "storage_unavailable"
                                | "not_applicable"
                        )
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid mod bootstrap result".to_owned());
                    }
                }
                BridgeEvent::ModAdapterError(payload) => {
                    if validate_mod_bundle_id(&payload.mod_id).is_err() {
                        return Err("invalid mod id in mod adapter error".to_owned());
                    }
                    if payload.message.is_empty() || payload.message.len() > MAX_ERROR_BYTES {
                        return Err("invalid mod adapter error".to_owned());
                    }
                }
                BridgeEvent::ModResponseAck(payload) => {
                    if validate_mod_bundle_id(&payload.mod_id).is_err() {
                        return Err("invalid mod id in mod response acknowledgement".to_owned());
                    }
                    if !valid_request_id(&payload.request_id) {
                        return Err("invalid mod response acknowledgement".to_owned());
                    }
                }
                BridgeEvent::VencordCssResult(payload) => {
                    if !valid_request_id(&payload.request_id)
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid Vencord CSS result".to_owned());
                    }
                }
                BridgeEvent::VencordBootstrapResult(payload) => {
                    if !valid_request_id(&payload.request_id)
                        || payload.marker.len() > 64
                        || !payload.execution_ms.is_finite()
                        || !(0.0..=60_000.0).contains(&payload.execution_ms)
                        || !matches!(
                            payload.local_storage_api.as_str(),
                            "available" | "missing" | "blocked"
                        )
                        || !matches!(
                            payload.settings_state.as_str(),
                            "missing"
                                | "valid_json"
                                | "invalid_json"
                                | "read_error"
                                | "storage_unavailable"
                        )
                        || !matches!(
                            payload.local_storage_api.as_str(),
                            "available" | "missing" | "blocked"
                        )
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid Vencord bootstrap result".to_owned());
                    }
                }
                BridgeEvent::VencordAdapterError(payload) => {
                    if payload.message.is_empty() || payload.message.len() > MAX_ERROR_BYTES {
                        return Err("invalid Vencord adapter error".to_owned());
                    }
                }
                BridgeEvent::VencordResponseAck(payload) => {
                    if !valid_request_id(&payload.request_id) {
                        return Err("invalid Vencord response acknowledgement".to_owned());
                    }
                }
                BridgeEvent::InteractionProbe(payload) => {
                    if !matches!(
                        payload.phase.as_str(),
                        "channel_missing"
                            | "channel_navigation_requested"
                            | "composer_missing"
                            | "composer_not_empty"
                            | "focus_failed"
                            | "composer_focused"
                            | "input_observed"
                            | "character_erased"
                    ) {
                        return Err("invalid interaction diagnostic phase".to_owned());
                    }
                }
                BridgeEvent::SettingsProbe(payload) => {
                    if !matches!(
                        payload.source.as_str(),
                        // "unknown" is renderer-observed (the facade resolves the
                        // settings object only); the authoritative source is in
                        // the host `[Settings]` log for the same request.
                        "file" | "defaults" | "unavailable" | "unknown"
                    ) || payload.payload_bytes > 1024 * 1024
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid settings probe result".to_owned());
                    }
                }
                BridgeEvent::SettingsMutationProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "mutated" | "restored")
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid settings mutation probe result".to_owned());
                    }
                }
                BridgeEvent::SettingsSetProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "mutated" | "restored")
                        || !matches!(
                            payload.key.as_str(),
                            "mods"
                                | "support_banner_dismissed"
                                | "show_experimental_plugin_menu"
                                | "extended_plugin_abilities"
                        )
                        || payload.attacks_blocked > 8
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                        || (payload.ok && (!payload.reread_matches || payload.attacks_blocked != 3))
                    {
                        return Err("invalid settings set probe result".to_owned());
                    }
                }
                BridgeEvent::GetLangProbe(payload) => {
                    if payload.lang.len() > crate::localization::MAX_LANG_CODE_BYTES
                        || payload.value.len() > crate::localization::MAX_LANG_VALUE_BYTES
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid localization probe result".to_owned());
                    }
                }
                BridgeEvent::AppVersionProbe(payload) => {
                    if payload.version.len() > crate::app::MAX_APP_VERSION_BYTES
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid app version probe result".to_owned());
                    }
                }
                BridgeEvent::GraphicsProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "report")
                        || !matches!(
                            payload.browser_status.as_str(),
                            "measured" | "unsupported" | "unavailable"
                        )
                        || !matches!(
                            payload.system_status.as_str(),
                            "measured" | "unsupported" | "unavailable"
                        )
                        || payload.device_count > crate::graphics::MAX_GPU_DEVICES
                        || payload.feature_count > crate::graphics::MAX_FEATURE_ENTRIES
                        || payload.device_summary.len() > MAX_GRAPHICS_PROBE_SUMMARY_BYTES
                        || payload.feature_summary.len() > MAX_GRAPHICS_PROBE_SUMMARY_BYTES
                        || !matches!(payload.webgl_status.as_str(), "measured" | "unavailable")
                        || payload.webgl_summary.len() > MAX_GRAPHICS_PROBE_SUMMARY_BYTES
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid graphics probe result".to_owned());
                    }
                }
                BridgeEvent::PluginStorageProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "write_read")
                        || payload.payload_bytes > MAX_PLUGIN_FILE_BYTES
                        || payload.attacks_blocked > 8
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                        || (payload.ok && (!payload.round_trip || payload.attacks_blocked != 3))
                    {
                        return Err("invalid plugin storage probe result".to_owned());
                    }
                }
                BridgeEvent::ThemeInstallProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "install")
                        || (payload.ok && payload.id.is_empty())
                        || payload.id.len() > MAX_FOLDER_THEME_ID_BYTES
                        || payload.css_bytes > crate::themes::MAX_THEME_CSS_BYTES
                        || payload.attacks_blocked > 8
                        || !matches!(
                            payload.fetch_observed.as_str(),
                            "allowed" | "blocked" | "error"
                        )
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                        || (payload.ok
                            && (payload.enabled
                                || payload.attacks_blocked != 3
                                || !payload.id.ends_with("-BD")))
                    {
                        return Err("invalid theme install probe result".to_owned());
                    }
                }
                BridgeEvent::ThemeWriteProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "write")
                        || (payload.ok && payload.id.is_empty())
                        || payload.id.len() > MAX_FOLDER_THEME_ID_BYTES
                        || payload.css_bytes > crate::themes::MAX_THEME_CSS_BYTES
                        || payload.attacks_blocked > 8
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                        || (payload.ok
                            && (!payload.enabled
                                || !payload.reflected
                                || payload.attacks_blocked != 3))
                    {
                        return Err("invalid theme write probe result".to_owned());
                    }
                }
                BridgeEvent::ThemeImportProbe(payload) => {
                    // Cancellation is a successful probe outcome (the silent
                    // path), so it must still carry zero counts; a failed
                    // probe carries the typed renderer error instead.
                    if !matches!(payload.stage.as_str(), "import")
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                        || (payload.ok
                            && payload.canceled
                            && (payload.imported != 0 || payload.failed != 0))
                    {
                        return Err("invalid theme import probe result".to_owned());
                    }
                }
                BridgeEvent::ThemeSetEnabledProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "set_enabled")
                        || (payload.ok && payload.id.is_empty())
                        || payload.id.len() > MAX_FOLDER_THEME_ID_BYTES
                        || payload.attacks_blocked > 8
                        || !matches!(
                            payload.clearvision.as_str(),
                            "absent" | "reflected" | "failed"
                        )
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                        || (payload.ok
                            && (payload.enabled
                                || !payload.reflected
                                || !payload.multi
                                || payload.clearvision == "failed"
                                || payload.attacks_blocked != 3))
                    {
                        return Err("invalid theme enable probe result".to_owned());
                    }
                }
                BridgeEvent::ThemeListProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "list")
                        || !matches!(
                            payload.clearvision.as_str(),
                            "absent" | "present-enabled" | "present-disabled" | "failed"
                        )
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                        || (payload.ok && payload.clearvision == "failed")
                    {
                        return Err("invalid theme list probe result".to_owned());
                    }
                }
                BridgeEvent::BackupSaveProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "save")
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid backup save probe result".to_owned());
                    }
                }
                BridgeEvent::BackupRestoreProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "restore")
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                        || payload.client_mods_bytes > MAX_CLIENT_MODS_BYTES as u64
                    {
                        return Err("invalid backup restore probe result".to_owned());
                    }
                }
                BridgeEvent::ShelterLoaderProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "presence" | "synthetic" | "cleanup")
                        || !matches!(payload.synthetic.as_str(), "verified" | "skipped")
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                        || (payload.ok
                            && (payload.stage != "cleanup"
                                || payload.synthetic != "verified"
                                || !payload.shelter
                                || !payload.plugins
                                || !payload.add_local_plugin
                                || !payload.start_plugin
                                || !payload.stop_plugin
                                || !payload.remove_plugin
                                || !payload.url_loader_absent))
                    {
                        return Err("invalid shelter loader probe result".to_owned());
                    }
                }
                BridgeEvent::SettingsBootSetLang(payload) => {
                    // Length only: the charset rule lives in the localization
                    // owner (`validate_locale`) and is re-checked at handling.
                    if payload.locale.is_empty()
                        || payload.locale.len() > crate::localization::MAX_LANG_CODE_BYTES
                    {
                        return Err("invalid settings boot locale".to_owned());
                    }
                }
                BridgeEvent::SettingsBootRestart(_) => {}
                BridgeEvent::SettingsBootProbe(payload) => {
                    if !matches!(
                        payload.settings_source.as_str(),
                        "file" | "defaults" | "unavailable"
                    ) || payload.settings_bytes > 1024 * 1024
                        || payload.themes_count > crate::themes::MAX_LISTED_THEMES as u32
                        || payload.i18n_keys > crate::localization::MAX_BOOT_I18N_KEYS as u32
                        || payload.lang.len() > crate::localization::MAX_LANG_CODE_BYTES
                        || payload.snapshot_bytes
                            > crate::settings_boot::MAX_BOOT_SNAPSHOT_BYTES as u32
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                        || (payload.ok && (!payload.frozen || !payload.keys_exact))
                    {
                        return Err("invalid settings boot probe result".to_owned());
                    }
                }
                BridgeEvent::SettingsUiProbe(payload) => {
                    if payload.stage.is_empty()
                        || payload.stage.len() > 64
                        || payload.detail.len() > 512
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid settings UI probe result".to_owned());
                    }
                }
            }
        }
    }

    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::{
        parse_bridge_message, validate_mod_bundle_id, BridgeCommand, BridgeEvent, BridgeMessage,
        BridgeRequest, BridgeResponse, BridgeResponsePayload, ModBundleResponse, RequestTracker,
        MAX_MOD_BUNDLE_ID_BYTES, PROTOCOL_VERSION,
    };

    #[test]
    fn accepts_a_valid_typed_command() {
        let message = r#"{"version":1,"request_id":"test-1","command":{"type":"ping","payload":{"nonce":"hello"}}}"#;
        assert!(matches!(
            parse_bridge_message(message),
            Ok(BridgeMessage::Request(_))
        ));
    }

    #[test]
    fn rejects_unknown_commands_and_extra_payload_fields() {
        let unknown = r#"{"version":1,"request_id":"test-1","command":{"type":"invoke","payload":{"method":"anything"}}}"#;
        let extra = r#"{"version":1,"request_id":"test-1","command":{"type":"ping","payload":{"nonce":"hello","host_api":"filesystem"}}}"#;
        assert!(parse_bridge_message(unknown).is_err());
        assert!(parse_bridge_message(extra).is_err());
    }

    #[test]
    fn rejects_malformed_payloads_and_wrong_versions() {
        let malformed = r#"{"version":1,"request_id":"test-1","command":{"type":"ping","payload":{"nonce":false}}}"#;
        let wrong_version = r#"{"version":2,"request_id":"test-1","command":{"type":"ping","payload":{"nonce":"hello"}}}"#;
        assert!(parse_bridge_message(malformed).is_err());
        assert!(parse_bridge_message(wrong_version).is_err());
    }

    #[test]
    fn accepts_a_scoped_bundle_id_and_rejects_renderer_source_parameters() {
        let request = r#"{"version":1,"request_id":"vencord-1","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord"}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(BridgeRequest {
                command: BridgeCommand::ModGetBundle(_),
                ..
            }))
        ));

        // A URL, path, filename, or executable-source member is never a
        // valid member of this command: it cannot select what the host reads.
        let url = r#"{"version":1,"request_id":"vencord-2","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord","url":"https://attacker.invalid/bundle.js"}}}"#;
        let path = r#"{"version":1,"request_id":"vencord-3","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord","path":"C:\\secret"}}}"#;
        let file = r#"{"version":1,"request_id":"vencord-4","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord","file":"vencord.js"}}}"#;
        let source = r#"{"version":1,"request_id":"vencord-5","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord","source":"globalThis.x=1"}}}"#;
        let channel = r#"{"version":1,"request_id":"vencord-6","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord","channel":"getVencordBundle"}}}"#;
        let generic = r#"{"version":1,"request_id":"vencord-7","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord","method":"readFile"}}}"#;
        for message in [url, path, file, source, channel, generic] {
            assert!(
                parse_bridge_message(message).is_err(),
                "must reject {message}"
            );
        }
    }

    #[test]
    fn rejects_bundle_ids_that_could_escape_the_registry() {
        // Path, URL, and scheme shapes are rejected at parse time by the
        // charset bound, so no id can become a filesystem or URL selector.
        for hostile in [
            r#"{"version":1,"request_id":"m-1","command":{"type":"mod_get_bundle","payload":{"mod_id":"../vencord"}}}"#,
            r#"{"version":1,"request_id":"m-2","command":{"type":"mod_get_bundle","payload":{"mod_id":"..\\vencord"}}}"#,
            r#"{"version":1,"request_id":"m-3","command":{"type":"mod_get_bundle","payload":{"mod_id":"C:\\vencord.js"}}}"#,
            r#"{"version":1,"request_id":"m-4","command":{"type":"mod_get_bundle","payload":{"mod_id":"//server/share/vencord.js"}}}"#,
            r#"{"version":1,"request_id":"m-5","command":{"type":"mod_get_bundle","payload":{"mod_id":"https://attacker.invalid/vencord.js"}}}"#,
            r#"{"version":1,"request_id":"m-6","command":{"type":"mod_get_bundle","payload":{"mod_id":"javascript:alert(1)"}}}"#,
            r#"{"version":1,"request_id":"m-7","command":{"type":"mod_get_bundle","payload":{"mod_id":""}}}"#,
            r#"{"version":1,"request_id":"m-9","command":{"type":"mod_get_bundle","payload":{"mod_id":"ven cord"}}}"#,
        ] {
            assert!(
                parse_bridge_message(hostile).is_err(),
                "must reject {hostile}"
            );
        }
        // Charset-valid but UNREGISTERED ids parse here and are rejected at
        // dispatch with UNKNOWN_MOD: exact registry match only, so a file
        // name or an unregistered mod can never become a source selector.
        for unregistered in ["shelter", "vencord.js", "equicord", "custom", "Vencord"] {
            let message = format!(
                r#"{{"version":1,"request_id":"m-x","command":{{"type":"mod_get_bundle","payload":{{"mod_id":"{unregistered}"}}}}}}"#
            );
            assert!(
                matches!(
                    parse_bridge_message(&message),
                    Ok(BridgeMessage::Request(BridgeRequest {
                        command: BridgeCommand::ModGetBundle(_),
                        ..
                    }))
                ),
                "{unregistered} must parse and be rejected by the host registry"
            );
            assert!(validate_mod_bundle_id(unregistered).is_ok());
        }
    }

    #[test]
    fn validates_bundle_id_bounds_and_charset() {
        assert!(validate_mod_bundle_id("vencord").is_ok());
        assert!(validate_mod_bundle_id("my.mod_2").is_ok());
        assert!(validate_mod_bundle_id("").is_err());
        assert!(validate_mod_bundle_id(&"a".repeat(MAX_MOD_BUNDLE_ID_BYTES + 1)).is_err());
        assert!(validate_mod_bundle_id("../x").is_err());
        assert!(validate_mod_bundle_id("x/y").is_err());
        assert!(validate_mod_bundle_id("x\\y").is_err());
        assert!(validate_mod_bundle_id("https://a.invalid/x").is_err());
        assert!(validate_mod_bundle_id("javascript:alert(1)").is_err());
    }

    #[test]
    fn accepts_renderer_error_event() {
        let error = r#"{"version":1,"event":{"type":"renderer_error","payload":{"kind":"uncaught","message":"failed"}}}"#;
        assert!(matches!(
            parse_bridge_message(error),
            Ok(BridgeMessage::Event(ref message))
                if matches!(&message.event, BridgeEvent::RendererError(_))
        ));
    }

    #[test]
    fn validates_interaction_diagnostic_phases() {
        let focused = r#"{"version":1,"event":{"type":"interaction_probe","payload":{"phase":"composer_focused"}}}"#;
        let erased = r#"{"version":1,"event":{"type":"interaction_probe","payload":{"phase":"character_erased"}}}"#;
        let invalid = r#"{"version":1,"event":{"type":"interaction_probe","payload":{"phase":"send_message"}}}"#;
        assert!(matches!(
            parse_bridge_message(focused),
            Ok(BridgeMessage::Event(ref message))
                if matches!(&message.event, BridgeEvent::InteractionProbe(_))
        ));
        assert!(matches!(
            parse_bridge_message(erased),
            Ok(BridgeMessage::Event(ref message))
                if matches!(&message.event, BridgeEvent::InteractionProbe(_))
        ));
        assert!(parse_bridge_message(invalid).is_err());
    }

    #[test]
    fn validates_vencord_completion_events() {
        let css = r#"{"version":1,"event":{"type":"vencord_css_result","payload":{"request_id":"vencord-1","ok":true,"error":null}}}"#;
        let bootstrap = r#"{"version":1,"event":{"type":"vencord_bootstrap_result","payload":{"request_id":"vencord-1","ok":true,"marker":"vencord-bootstrap-executed","execution_ms":12.5,"local_storage_api":"available","settings_state":"missing","error":null}}}"#;
        let malformed = r#"{"version":1,"event":{"type":"vencord_bootstrap_result","payload":{"request_id":"vencord-1","ok":true,"marker":"vencord-bootstrap-executed","execution_ms":-1,"local_storage_api":"other","settings_state":"missing","error":null}}}"#;
        assert!(matches!(
            parse_bridge_message(css),
            Ok(BridgeMessage::Event(ref message))
                if matches!(&message.event, BridgeEvent::VencordCssResult(_))
        ));
        assert!(matches!(
            parse_bridge_message(bootstrap),
            Ok(BridgeMessage::Event(ref message))
                if matches!(&message.event, BridgeEvent::VencordBootstrapResult(_))
        ));
        assert!(parse_bridge_message(malformed).is_err());
    }

    #[test]
    fn vencord_css_result_accepts_the_builder_echo_and_refuses_foreign_claims() {
        // The exact shape the shared CSS builder posts for Vencord: the
        // echoed registry id plus the legacy members. Before the echo was
        // accepted, this real-world message was rejected and Vencord could
        // never complete its CSS gate (style landed, marker never ran).
        let echoed = r#"{"version":1,"event":{"type":"vencord_css_result","payload":{"mod_id":"vencord","request_id":"vencord-1","ok":true,"error":null}}}"#;
        let parsed = parse_bridge_message(echoed).expect("the builder echo must parse");
        let echo_id = match parsed {
            BridgeMessage::Event(message) => match message.event {
                BridgeEvent::VencordCssResult(payload) => {
                    assert_eq!(payload.request_id, "vencord-1");
                    assert!(payload.ok);
                    payload.mod_id
                }
                _ => panic!("expected the legacy Vencord CSS variant"),
            },
            _ => panic!("expected an event message"),
        };
        assert!(super::validate_vencord_css_mod_id(&echo_id).is_ok());
        // The original echoless legacy shape still parses.
        assert!(super::validate_vencord_css_mod_id(&None).is_ok());
        // A foreign claim is refused before any handler runs.
        assert!(super::validate_vencord_css_mod_id(&Some("shelter".to_owned())).is_err());
        assert!(super::validate_vencord_css_mod_id(&Some(String::new())).is_err());
        // And it never parses as the mod-agnostic family: the tag stays legacy.
        assert!(matches!(
            parse_bridge_message(echoed),
            Ok(BridgeMessage::Event(ref message))
                if matches!(&message.event, BridgeEvent::VencordCssResult(_))
        ));
    }

    #[test]
    fn serializes_a_typed_bundle_response_without_bundle_source() {
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "vencord-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::ModBundle(ModBundleResponse {
                mod_id: "vencord",
                bundle_version: "test-build".to_owned(),
                bundle_bytes: 1024,
                css_bytes: Some(256),
                request_latency_ms: 20.0,
                retrieval_ms: 10.0,
                execution_ms: 5.0,
                css_injection_ms: Some(2.0),
                marker: "vencord-bootstrap-executed",
                document_execution_count: 1,
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "mod_bundle");
        assert_eq!(value["payload"]["payload"]["mod_id"], "vencord");
        assert_eq!(
            value["payload"]["payload"]["marker"],
            "vencord-bootstrap-executed"
        );
        assert_eq!(value["payload"]["payload"]["document_execution_count"], 1);
        // Metadata only: no source, no URL, no cache path, no layout.
        assert!(value["payload"]["payload"].get("js").is_none());
        assert!(value["payload"]["payload"].get("css").is_none());
        assert!(value["payload"]["payload"].get("url").is_none());
        assert!(value["payload"]["payload"].get("path").is_none());
        assert!(value["payload"]["payload"].get("js_url").is_none());
        assert!(value["payload"]["payload"].get("js_file").is_none());
        assert!(value["payload"]["payload"].get("source").is_none());
        assert!(value["payload"]["payload"].get("profile").is_none());
        assert_eq!(
            value["payload"]["payload"]
                .as_object()
                .map(|object| object.len()),
            Some(10)
        );
    }

    #[test]
    fn a_mod_without_css_serializes_both_css_fields_as_null() {
        // "Ships no stylesheet" is a first-class state, distinct from "shipped
        // an empty stylesheet": both CSS fields are null together, so the
        // renderer-side schema check can require them to agree.
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "shelter-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::ModBundle(ModBundleResponse {
                mod_id: "shelter",
                bundle_version: "sha256-ba858e9cfb83503f".to_owned(),
                bundle_bytes: 235_850,
                css_bytes: None,
                request_latency_ms: 40.0,
                retrieval_ms: 30.0,
                execution_ms: 9.0,
                css_injection_ms: None,
                marker: "shelter-bootstrap-executed",
                document_execution_count: 1,
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        let payload = &value["payload"]["payload"];
        assert!(payload["css_bytes"].is_null());
        assert!(payload["css_injection_ms"].is_null());
        assert_eq!(payload["mod_id"], "shelter");
        assert_eq!(
            payload.as_object().map(|object| object.len()),
            Some(10),
            "the bounded metadata shape is unchanged for a CSS-less mod"
        );
        assert!(payload.get("css").is_none());
        assert!(payload.get("source").is_none());
    }

    #[test]
    fn correlates_responses_and_rejects_unknown_or_duplicate_ids() {
        let mut tracker = RequestTracker::default();
        assert!(tracker.begin("request-1").is_ok());
        assert!(tracker.begin("request-1").is_err());
        assert!(tracker.complete("request-unknown").is_err());
        assert!(tracker.complete("request-1").is_ok());
        assert!(tracker.complete("request-1").is_err());
    }

    #[test]
    fn accepts_the_settings_capability_without_renderer_parameters() {
        let request = r#"{"version":1,"request_id":"settings-1","command":{"type":"settings_get_config","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        // No config key, filesystem path, or selector may be supplied.
        let with_key = r#"{"version":1,"request_id":"settings-2","command":{"type":"settings_get_config","payload":{"key":"mods"}}}"#;
        let with_path = r#"{"version":1,"request_id":"settings-3","command":{"type":"settings_get_config","payload":{"path":"C:\\secret"}}}"#;
        assert!(parse_bridge_message(with_key).is_err());
        assert!(parse_bridge_message(with_path).is_err());
    }

    #[test]
    fn rejects_settings_mutation_and_generic_invocation() {
        let set_config = r#"{"version":1,"request_id":"settings-1","command":{"type":"settings_set_config","payload":{}}}"#;
        let invoke = r#"{"version":1,"request_id":"settings-2","command":{"type":"invoke","payload":{"method":"settings.getConfig"}}}"#;
        assert!(parse_bridge_message(set_config).is_err());
        assert!(parse_bridge_message(invoke).is_err());
    }

    #[test]
    fn accepts_all_eight_window_chrome_commands_without_parameters() {
        // Each typed window command carries an exact empty payload: no
        // window id, handle, coordinate, style, message, or process member
        // exists to supply.
        for (wire, request_id) in [
            ("window_show", "win-1"),
            ("window_hide", "win-2"),
            ("window_minimize", "win-3"),
            ("window_maximize", "win-4"),
            ("window_unmaximize", "win-5"),
            ("window_drag", "win-6"),
            ("window_toggle_fullscreen", "win-7"),
            ("window_quit", "win-8"),
        ] {
            let message = format!(
                "{{\"version\":1,\"request_id\":\"{request_id}\",\"command\":{{\"type\":\"{wire}\",\"payload\":{{}}}}}}"
            );
            let parsed = parse_bridge_message(&message);
            assert!(parsed.is_ok(), "{wire} must parse: {parsed:?}");
            let command = match parsed.unwrap() {
                BridgeMessage::Request(request) => request.command,
                BridgeMessage::Event(_) => panic!("{wire} must parse as a request"),
            };
            let expected = match wire {
                "window_show" => matches!(command, BridgeCommand::WindowShow(_)),
                "window_hide" => matches!(command, BridgeCommand::WindowHide(_)),
                "window_minimize" => matches!(command, BridgeCommand::WindowMinimize(_)),
                "window_maximize" => matches!(command, BridgeCommand::WindowMaximize(_)),
                "window_unmaximize" => matches!(command, BridgeCommand::WindowUnmaximize(_)),
                "window_drag" => matches!(command, BridgeCommand::WindowDrag(_)),
                "window_toggle_fullscreen" => {
                    matches!(command, BridgeCommand::WindowToggleFullscreen(_))
                }
                "window_quit" => matches!(command, BridgeCommand::WindowQuit(_)),
                _ => unreachable!(),
            };
            assert!(expected, "{wire} must dispatch to its own typed variant");
        }
    }

    #[test]
    fn rejects_generic_window_control_and_extra_window_members() {
        // There is no generic window dispatcher: no operation string, no
        // args object, no close (Electron exposes no renderer win-close).
        for wire in [
            "window_control",
            "window_invoke",
            "window_close",
            "win_close",
            "window_show_all",
        ] {
            let message = format!(
                "{{\"version\":1,\"request_id\":\"win-x\",\"command\":{{\"type\":\"{wire}\",\"payload\":{{}}}}}}"
            );
            assert!(
                parse_bridge_message(&message).is_err(),
                "{wire} must be rejected"
            );
        }
        let generic = r#"{"version":1,"request_id":"win-y","command":{"type":"window_show","payload":{"op":"maximize","args":{}}}}"#;
        assert!(parse_bridge_message(generic).is_err());
        // No handle, coordinate, style, message, or process member may ride
        // along with any typed window command.
        for payload in [
            r#"{"hwnd":1234}"#,
            r#"{"handle":"0x1234"}"#,
            r#"{"x":10,"y":10}"#,
            r#"{"width":835,"height":600}"#,
            r#"{"style":"borderless"}"#,
            r#"{"message":274}"#,
            r#"{"pid":1234}"#,
            r#"{"id":"main"}"#,
        ] {
            for wire in [
                "window_show",
                "window_hide",
                "window_minimize",
                "window_maximize",
                "window_unmaximize",
                "window_drag",
                "window_toggle_fullscreen",
                "window_quit",
            ] {
                let message = format!(
                    "{{\"version\":1,\"request_id\":\"win-z\",\"command\":{{\"type\":\"{wire}\",\"payload\":{payload}}}}}"
                );
                assert!(
                    parse_bridge_message(&message).is_err(),
                    "{wire} with {payload} must be rejected"
                );
            }
        }
    }

    #[test]
    fn serializes_a_typed_window_ack_without_state_or_handles() {
        use super::{BridgeResponsePayload, WindowAckResponse};
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "win-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::WindowAck(WindowAckResponse {
                command: "window_show",
            })),
            error: None,
        };
        let serialized = serde_json::to_value(&response).unwrap();
        assert_eq!(
            serialized["payload"]["type"],
            serde_json::Value::String("window_ack".to_owned())
        );
        assert_eq!(
            serialized["payload"]["payload"]["command"],
            serde_json::Value::String("window_show".to_owned())
        );
        // The acknowledgement echoes the command only: no handle, geometry,
        // state, path, or error text may ride along.
        let keys: Vec<&str> = serialized["payload"]["payload"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["command"]);
    }

    #[test]
    fn window_state_event_carries_exactly_focused_and_maximized() {
        use super::window_state_event_json;
        for (focused, maximized) in [(true, true), (true, false), (false, true), (false, false)] {
            let detail: serde_json::Value =
                serde_json::from_str(&window_state_event_json(focused, maximized)).unwrap();
            let object = detail.as_object().unwrap();
            let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(keys, ["focused", "maximized", "version"]);
            assert_eq!(object["version"], serde_json::Value::from(PROTOCOL_VERSION));
            assert_eq!(object["focused"], serde_json::Value::from(focused));
            assert_eq!(object["maximized"], serde_json::Value::from(maximized));
        }
    }

    #[test]
    fn accepts_the_bundle_discovery_capability_without_renderer_parameters() {
        let request = r#"{"version":1,"request_id":"bundles-1","command":{"type":"bundles_list","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        // No path, URL, include flag, selector, or generic invocation member
        // may be supplied: the inventory covers the fixed allowlist only.
        let with_path = r#"{"version":1,"request_id":"bundles-2","command":{"type":"bundles_list","payload":{"path":"C:\\secret"}}}"#;
        let with_url = r#"{"version":1,"request_id":"bundles-3","command":{"type":"bundles_list","payload":{"url":"https://attacker.invalid/x.js"}}}"#;
        let with_includes = r#"{"version":1,"request_id":"bundles-4","command":{"type":"bundles_list","payload":{"include_mod_bundles":true}}}"#;
        let with_name = r#"{"version":1,"request_id":"bundles-5","command":{"type":"bundles_list","payload":{"name":"vencord.js"}}}"#;
        let with_method = r#"{"version":1,"request_id":"bundles-6","command":{"type":"bundles_list","payload":{"method":"readFile"}}}"#;
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_url).is_err());
        assert!(parse_bridge_message(with_includes).is_err());
        assert!(parse_bridge_message(with_name).is_err());
        assert!(parse_bridge_message(with_method).is_err());
    }

    #[test]
    fn rejects_the_historical_bundles_wire_type() {
        // P0.4: the facade once sent `backup_list_bundles`; the protocol
        // only ever accepted `bundles_list`, so the old string stays a
        // deserialization error (unknown variant), never a dispatch.
        let stale = r#"{"version":1,"request_id":"bundles-0","command":{"type":"backup_list_bundles","payload":{}}}"#;
        assert!(parse_bridge_message(stale).is_err());
    }

    #[test]
    fn rejects_backup_save_shape_errors_and_generic_invocation() {
        // Save (D4-1) requires the exact `{ includes, clientMods }` shape, so
        // an empty payload is rejected; restore (D4-2) is parameter-free, so
        // its exact empty payload parses while any member is rejected.
        let save = r#"{"version":1,"request_id":"bundles-1","command":{"type":"backup_save","payload":{}}}"#;
        let legacy_restore = r#"{"version":1,"request_id":"bundles-2","command":{"type":"backup_restore","payload":{}}}"#;
        let inspect = r#"{"version":1,"request_id":"bundles-2b","command":{"type":"backup_inspect","payload":{}}}"#;
        let commit = r#"{"version":1,"request_id":"bundles-2c","command":{"type":"backup_commit_restore","payload":{}}}"#;
        let discard = r#"{"version":1,"request_id":"bundles-2d","command":{"type":"backup_discard_restore","payload":{}}}"#;
        let invoke = r#"{"version":1,"request_id":"bundles-3","command":{"type":"invoke","payload":{"method":"backup.save"}}}"#;
        assert!(parse_bridge_message(save).is_err());
        // The single-call restore is gone: preview-then-commit replaced it.
        assert!(parse_bridge_message(legacy_restore).is_err());
        assert!(matches!(
            parse_bridge_message(inspect),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(matches!(
            parse_bridge_message(commit),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(matches!(
            parse_bridge_message(discard),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(parse_bridge_message(invoke).is_err());
        // None of the trio carries an archive path, filename, URL, filter,
        // dialog option, or staged bytes: any member is rejected.
        for payload in [
            r#"{"path":"C:\\x.zip"}"#,
            r#"{"filename":"x.zip"}"#,
            r#"{"url":"https://attacker.invalid/x.zip"}"#,
            r#"{"archive":"e30="}"#,
        ] {
            for wire in [
                "backup_inspect",
                "backup_commit_restore",
                "backup_discard_restore",
            ] {
                let request = format!(
                    "{{\"version\":1,\"request_id\":\"restore-x\",\"command\":{{\"type\":\"{wire}\",\"payload\":{payload}}}}}"
                );
                assert!(
                    parse_bridge_message(&request).is_err(),
                    "{wire} with {payload} must be rejected"
                );
            }
        }
    }

    #[test]
    fn serializes_a_typed_backup_restored_response() {
        use super::BackupRestoredResponse;

        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "restore-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::BackupRestored(
                BackupRestoredResponse {
                    canceled: false,
                    client_mods: serde_json::json!({"vencordLocalStorage": "x"}),
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["payload"]["type"], "backup_restored");
        assert_eq!(value["payload"]["payload"]["canceled"], false);
        assert_eq!(
            value["payload"]["payload"]["client_mods"]["vencordLocalStorage"],
            "x"
        );
    }

    #[test]
    fn serializes_a_typed_backup_inspected_preview() {
        use super::BackupInspectedResponse;
        use crate::backup::BackupIncludes;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "inspect-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::BackupInspected(
                BackupInspectedResponse {
                    canceled: false,
                    includes: BackupIncludes {
                        remcord_config: true,
                        remcord_themes_and_quick_css: true,
                        remcord_extension_plugins: false,
                        vencord_mod_data: true,
                        equicord_mod_data: false,
                        shelter_mod_data: false,
                        mod_bundles: false,
                    },
                    archive_bytes: 100,
                    uncompressed_bytes: 200,
                    entry_count: 2,
                    client_mods_bytes: 10,
                    has_vencord_mod_data: true,
                    has_equicord_mod_data: false,
                    has_shelter_mod_data: false,
                    entries: vec!["data/quickCss.css".to_owned()],
                    entries_truncated: true,
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["payload"]["type"], "backup_inspected");
        let preview = &value["payload"]["payload"];
        assert_eq!(preview["canceled"], false);
        assert_eq!(preview["includes"]["remcordConfig"], true);
        assert_eq!(preview["includes"]["modBundles"], false);
        assert_eq!(preview["entry_count"], 2);
        assert_eq!(preview["entries"], serde_json::json!(["data/quickCss.css"]));
        assert_eq!(preview["entries_truncated"], true);
        assert_eq!(preview["has_vencord_mod_data"], true);
        // No contents, no paths beyond display names, no envelope.
        assert!(preview.get("client_mods").is_none());
        assert_eq!(preview.as_object().map(|object| object.len()), Some(11));
    }

    #[test]
    fn validates_backup_restore_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"backup_restore_probe","payload":{"ok":true,"stage":"restore","canceled":false,"client_mods_bytes":88,"error":null}}}"#;
        let canceled = r#"{"version":1,"event":{"type":"backup_restore_probe","payload":{"ok":true,"stage":"restore","canceled":true,"client_mods_bytes":0,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"backup_restore_probe","payload":{"ok":false,"stage":"restore","canceled":false,"client_mods_bytes":0,"error":"timeout"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"backup_restore_probe","payload":{"ok":true,"stage":"export","canceled":false,"client_mods_bytes":1,"error":null}}}"#;
        let with_path = r#"{"version":1,"event":{"type":"backup_restore_probe","payload":{"ok":true,"stage":"restore","canceled":false,"client_mods_bytes":1,"path":"C:\\x","error":null}}}"#;
        assert!(matches!(
            parse_bridge_message(ok),
            Ok(BridgeMessage::Event(_))
        ));
        assert!(matches!(
            parse_bridge_message(canceled),
            Ok(BridgeMessage::Event(_))
        ));
        assert!(matches!(
            parse_bridge_message(failed),
            Ok(BridgeMessage::Event(_))
        ));
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(with_path).is_err());
    }

    #[test]
    fn accepts_only_the_closed_folder_locations() {
        for location in ["storage", "themes", "plugins"] {
            let request = format!(
                "{{\"version\":1,\"request_id\":\"folders-{location}\",\"command\":{{\"type\":\"folders_open\",\"payload\":{{\"location\":\"{location}\"}}}}}}"
            );
            assert!(
                matches!(
                    parse_bridge_message(&request),
                    Ok(BridgeMessage::Request(_))
                ),
                "{location} must parse"
            );
        }

        // No path, URL, directory name, selector, or second member may ride
        // along; unknown locations never reach dispatch.
        let with_path = r#"{"version":1,"request_id":"folders-4","command":{"type":"folders_open","payload":{"location":"themes","path":"C:\\secret"}}}"#;
        let with_url = r#"{"version":1,"request_id":"folders-5","command":{"type":"folders_open","payload":{"location":"themes","url":"https://attacker.invalid"}}}"#;
        let unknown = r#"{"version":1,"request_id":"folders-6","command":{"type":"folders_open","payload":{"location":"extensions"}}}"#;
        let empty = r#"{"version":1,"request_id":"folders-7","command":{"type":"folders_open","payload":{}}}"#;
        let wrong_case = r#"{"version":1,"request_id":"folders-8","command":{"type":"folders_open","payload":{"location":"Themes"}}}"#;
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_url).is_err());
        assert!(parse_bridge_message(unknown).is_err());
        assert!(parse_bridge_message(empty).is_err());
        assert!(parse_bridge_message(wrong_case).is_err());
    }

    #[test]
    fn accepts_theme_folder_open_with_shape_checked_ids() {
        let request = r#"{"version":1,"request_id":"folders-1","command":{"type":"folders_open_theme","payload":{"id":"my-theme"}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        // Charset/traversal rules live in `folders` at dispatch, but empty,
        // oversized, and extra-member payloads are rejected at parse time.
        let empty = r#"{"version":1,"request_id":"folders-2","command":{"type":"folders_open_theme","payload":{"id":""}}}"#;
        let with_path = r#"{"version":1,"request_id":"folders-3","command":{"type":"folders_open_theme","payload":{"id":"my-theme","path":"C:\\secret"}}}"#;
        let missing = r#"{"version":1,"request_id":"folders-4","command":{"type":"folders_open_theme","payload":{}}}"#;
        assert!(parse_bridge_message(empty).is_err());
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(missing).is_err());
    }

    #[test]
    fn serializes_typed_folder_open_responses_without_paths() {
        use super::{FoldersOpenedResponse, ThemeFolderOpenedResponse};
        use crate::folders::FolderLocation;
        let reveal = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "folders-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::FoldersOpened(
                FoldersOpenedResponse {
                    location: FolderLocation::Themes,
                },
            )),
            error: None,
        };
        let reveal_value = serde_json::to_value(reveal).unwrap();
        assert_eq!(reveal_value["payload"]["type"], "folders_opened");
        assert_eq!(reveal_value["payload"]["payload"]["location"], "themes");
        assert!(reveal_value["payload"]["payload"].get("path").is_none());

        let open = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "folders-2".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::ThemeFolderOpened(
                ThemeFolderOpenedResponse {
                    id: "my-theme".to_owned(),
                },
            )),
            error: None,
        };
        let open_value = serde_json::to_value(open).unwrap();
        assert_eq!(open_value["payload"]["type"], "theme_folder_opened");
        assert_eq!(open_value["payload"]["payload"]["id"], "my-theme");
        assert!(open_value["payload"]["payload"].get("path").is_none());
    }

    #[test]
    fn accepts_theme_install_code_with_a_bounded_body() {
        let request = r#"{"version":1,"request_id":"install-1","command":{"type":"theme_install_code","payload":{"css":"/** @name T */ :root {}"}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        // No URL, path, filename, id, destination, or generic member may
        // ride along; the host derives everything from the CSS text.
        let with_url = r#"{"version":1,"request_id":"install-2","command":{"type":"theme_install_code","payload":{"css":"/** @name T */","url":"https://attacker.invalid/t.css"}}}"#;
        let with_path = r#"{"version":1,"request_id":"install-3","command":{"type":"theme_install_code","payload":{"css":"/** @name T */","path":"C:\\x"}}}"#;
        let with_id = r#"{"version":1,"request_id":"install-4","command":{"type":"theme_install_code","payload":{"css":"/** @name T */","id":"mine"}}}"#;
        let with_file = r#"{"version":1,"request_id":"install-5","command":{"type":"theme_install_code","payload":{"css":"/** @name T */","filename":"x.css"}}}"#;
        let missing = r#"{"version":1,"request_id":"install-6","command":{"type":"theme_install_code","payload":{}}}"#;
        assert!(parse_bridge_message(with_url).is_err());
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_id).is_err());
        assert!(parse_bridge_message(with_file).is_err());
        assert!(parse_bridge_message(missing).is_err());
    }

    #[test]
    fn oversized_bodies_reach_only_the_theme_install_command() {
        // An 8 KiB-plus body with a legacy command is rejected at the
        // transport bound, even though the payload itself would parse.
        let big_css = "x".repeat(9000);
        let ping = format!(
            "{{\"version\":1,\"request_id\":\"big-1\",\"command\":{{\"type\":\"ping\",\"payload\":{{\"nonce\":\"{big_css}\"}}}}}}"
        );
        assert!(parse_bridge_message(&ping).is_err());

        // The same scale of body parses for a theme install (the CSS
        // bound itself is enforced on the payload below).
        let install = format!(
            "{{\"version\":1,\"request_id\":\"big-2\",\"command\":{{\"type\":\"theme_install_code\",\"payload\":{{\"css\":\"/** @name T */ {big_css}\"}}}}}}"
        );
        assert!(matches!(
            parse_bridge_message(&install),
            Ok(BridgeMessage::Request(_))
        ));

        // Past the theme message ceiling, even installs are rejected.
        let huge = "x".repeat(super::MAX_THEME_INSTALL_MESSAGE_BYTES);
        let huge_install = format!(
            "{{\"version\":1,\"request_id\":\"big-3\",\"command\":{{\"type\":\"theme_install_code\",\"payload\":{{\"css\":\"{huge}\"}}}}}}"
        );
        assert!(parse_bridge_message(&huge_install).is_err());
    }

    #[test]
    fn serializes_a_typed_theme_install_response() {
        use super::ThemeInstalledResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "install-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::ThemeInstalled(
                ThemeInstalledResponse {
                    id: "Midnight-Harbor-BD".to_owned(),
                    enabled: false,
                    css_bytes: 128,
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "theme_installed");
        assert_eq!(value["payload"]["payload"]["id"], "Midnight-Harbor-BD");
        assert_eq!(value["payload"]["payload"]["enabled"], false);
        assert_eq!(value["payload"]["payload"]["css_bytes"], 128);
        // Metadata only: no path, URL, source, or destination echo.
        assert!(value["payload"]["payload"].get("path").is_none());
        assert!(value["payload"]["payload"].get("url").is_none());
        assert!(value["payload"]["payload"].get("css").is_none());
    }

    #[test]
    fn validates_theme_install_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"theme_install_probe","payload":{"ok":true,"stage":"install","id":"Probe-BD","css_bytes":64,"enabled":false,"attacks_blocked":3,"fetch_observed":"blocked","error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"theme_install_probe","payload":{"ok":false,"stage":"install","id":"","css_bytes":0,"enabled":false,"attacks_blocked":0,"fetch_observed":"error","error":"timeout"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"theme_install_probe","payload":{"ok":true,"stage":"edited","id":"Probe-BD","css_bytes":64,"enabled":false,"attacks_blocked":3,"fetch_observed":"blocked","error":null}}}"#;
        let enabled = r#"{"version":1,"event":{"type":"theme_install_probe","payload":{"ok":true,"stage":"install","id":"Probe-BD","css_bytes":64,"enabled":true,"attacks_blocked":3,"fetch_observed":"blocked","error":null}}}"#;
        let bad_fetch = r#"{"version":1,"event":{"type":"theme_install_probe","payload":{"ok":true,"stage":"install","id":"Probe-BD","css_bytes":64,"enabled":false,"attacks_blocked":3,"fetch_observed":"unknown","error":null}}}"#;
        let inconsistent = r#"{"version":1,"event":{"type":"theme_install_probe","payload":{"ok":true,"stage":"install","id":"Probe-BD","css_bytes":64,"enabled":false,"attacks_blocked":2,"fetch_observed":"blocked","error":null}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(enabled).is_err());
        assert!(parse_bridge_message(bad_fetch).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
    }

    #[test]
    fn accepts_theme_write_with_id_and_bounded_css_only() {
        let request = r#"{"version":1,"request_id":"write-1","command":{"type":"theme_write_css","payload":{"id":"my-theme","css":":root {}"}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        // No path, filename, URL, destination, or enablement member may
        // ride along; the host owns the destination and preserves state.
        let with_path = r#"{"version":1,"request_id":"write-2","command":{"type":"theme_write_css","payload":{"id":"my-theme","css":":root {}","path":"C:\\x"}}}"#;
        let with_file = r#"{"version":1,"request_id":"write-3","command":{"type":"theme_write_css","payload":{"id":"my-theme","css":":root {}","filename":"x.css"}}}"#;
        let with_url = r#"{"version":1,"request_id":"write-4","command":{"type":"theme_write_css","payload":{"id":"my-theme","css":":root {}","url":"https://attacker.invalid/x.css"}}}"#;
        let with_enabled = r#"{"version":1,"request_id":"write-5","command":{"type":"theme_write_css","payload":{"id":"my-theme","css":":root {}","enabled":true}}}"#;
        let missing_id = r#"{"version":1,"request_id":"write-6","command":{"type":"theme_write_css","payload":{"css":":root {}"}}}"#;
        let missing_css = r#"{"version":1,"request_id":"write-7","command":{"type":"theme_write_css","payload":{"id":"my-theme"}}}"#;
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_file).is_err());
        assert!(parse_bridge_message(with_url).is_err());
        assert!(parse_bridge_message(with_enabled).is_err());
        assert!(parse_bridge_message(missing_id).is_err());
        assert!(parse_bridge_message(missing_css).is_err());
    }

    #[test]
    fn theme_write_shares_the_bounded_css_transport_exception() {
        // Same scale of body parses for a CSS write; anything else at that
        // scale stays rejected.
        let big_css = "x".repeat(9000);
        let write = format!(
            "{{\"version\":1,\"request_id\":\"big-4\",\"command\":{{\"type\":\"theme_write_css\",\"payload\":{{\"id\":\"my-theme\",\"css\":\"{big_css}\"}}}}}}"
        );
        assert!(matches!(
            parse_bridge_message(&write),
            Ok(BridgeMessage::Request(_))
        ));
    }

    #[test]
    fn serializes_a_typed_theme_write_response() {
        use super::ThemeWrittenResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "write-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::ThemeWritten(ThemeWrittenResponse {
                id: "my-theme".to_owned(),
                enabled: true,
                css_bytes: 32,
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "theme_written");
        assert_eq!(value["payload"]["payload"]["id"], "my-theme");
        assert_eq!(value["payload"]["payload"]["enabled"], true);
        assert_eq!(value["payload"]["payload"]["css_bytes"], 32);
        // Metadata only: no path, source, or destination echo.
        assert!(value["payload"]["payload"].get("path").is_none());
        assert!(value["payload"]["payload"].get("css").is_none());
    }

    #[test]
    fn validates_theme_write_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"theme_write_probe","payload":{"ok":true,"stage":"write","id":"WriteProbe","css_bytes":48,"enabled":true,"attacks_blocked":3,"reflected":true,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"theme_write_probe","payload":{"ok":false,"stage":"write","id":"","css_bytes":0,"enabled":false,"attacks_blocked":0,"reflected":false,"error":"timeout"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"theme_write_probe","payload":{"ok":true,"stage":"edited","id":"WriteProbe","css_bytes":48,"enabled":true,"attacks_blocked":3,"reflected":true,"error":null}}}"#;
        let disabled = r#"{"version":1,"event":{"type":"theme_write_probe","payload":{"ok":true,"stage":"write","id":"WriteProbe","css_bytes":48,"enabled":false,"attacks_blocked":3,"reflected":true,"error":null}}}"#;
        let unreflected = r#"{"version":1,"event":{"type":"theme_write_probe","payload":{"ok":true,"stage":"write","id":"WriteProbe","css_bytes":48,"enabled":true,"attacks_blocked":3,"reflected":false,"error":null}}}"#;
        let inconsistent = r#"{"version":1,"event":{"type":"theme_write_probe","payload":{"ok":true,"stage":"write","id":"WriteProbe","css_bytes":48,"enabled":true,"attacks_blocked":2,"reflected":true,"error":null}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(disabled).is_err());
        assert!(parse_bridge_message(unreflected).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
    }

    #[test]
    fn validates_theme_import_probe_diagnostics() {
        let imported = r#"{"version":1,"event":{"type":"theme_import_probe","payload":{"ok":true,"stage":"import","canceled":false,"imported":1,"failed":0,"error":null}}}"#;
        let canceled = r#"{"version":1,"event":{"type":"theme_import_probe","payload":{"ok":true,"stage":"import","canceled":true,"imported":0,"failed":0,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"theme_import_probe","payload":{"ok":false,"stage":"import","canceled":false,"imported":0,"failed":0,"error":"timeout"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"theme_import_probe","payload":{"ok":true,"stage":"install","canceled":false,"imported":1,"failed":0,"error":null}}}"#;
        let inconsistent = r#"{"version":1,"event":{"type":"theme_import_probe","payload":{"ok":true,"stage":"import","canceled":true,"imported":1,"failed":0,"error":null}}}"#;
        let negative = r#"{"version":1,"event":{"type":"theme_import_probe","payload":{"ok":true,"stage":"import","canceled":false,"imported":-1,"failed":0,"error":null}}}"#;
        let with_path = r#"{"version":1,"event":{"type":"theme_import_probe","payload":{"ok":true,"stage":"import","canceled":false,"imported":1,"failed":0,"path":"C:\\x","error":null}}}"#;
        assert!(parse_bridge_message(imported).is_ok());
        assert!(parse_bridge_message(canceled).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
        assert!(parse_bridge_message(negative).is_err());
        assert!(parse_bridge_message(with_path).is_err());
    }

    #[test]
    fn accepts_theme_set_enabled_with_id_and_boolean_only() {
        let request = r#"{"version":1,"request_id":"enable-1","command":{"type":"theme_set_enabled","payload":{"id":"my-theme","enabled":true}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        // No path, filename, manifest, CSS, URL, or arbitrary member may
        // ride along; the host derives the manifest path from the id.
        let with_path = r#"{"version":1,"request_id":"enable-2","command":{"type":"theme_set_enabled","payload":{"id":"my-theme","enabled":true,"path":"C:\\x"}}}"#;
        let with_file = r#"{"version":1,"request_id":"enable-3","command":{"type":"theme_set_enabled","payload":{"id":"my-theme","enabled":true,"filename":"x.css"}}}"#;
        let with_manifest = r#"{"version":1,"request_id":"enable-4","command":{"type":"theme_set_enabled","payload":{"id":"my-theme","enabled":true,"manifest":{}}}}"#;
        let with_css = r#"{"version":1,"request_id":"enable-5","command":{"type":"theme_set_enabled","payload":{"id":"my-theme","enabled":true,"css":":root {}"}}}"#;
        let with_url = r#"{"version":1,"request_id":"enable-6","command":{"type":"theme_set_enabled","payload":{"id":"my-theme","enabled":true,"url":"https://attacker.invalid/x.css"}}}"#;
        let missing_id = r#"{"version":1,"request_id":"enable-7","command":{"type":"theme_set_enabled","payload":{"enabled":true}}}"#;
        let missing_flag = r#"{"version":1,"request_id":"enable-8","command":{"type":"theme_set_enabled","payload":{"id":"my-theme"}}}}"#;
        let string_flag = r#"{"version":1,"request_id":"enable-9","command":{"type":"theme_set_enabled","payload":{"id":"my-theme","enabled":"yes"}}}"#;
        let numeric_id = r#"{"version":1,"request_id":"enable-10","command":{"type":"theme_set_enabled","payload":{"id":42,"enabled":true}}}"#;
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_file).is_err());
        assert!(parse_bridge_message(with_manifest).is_err());
        assert!(parse_bridge_message(with_css).is_err());
        assert!(parse_bridge_message(with_url).is_err());
        assert!(parse_bridge_message(missing_id).is_err());
        assert!(parse_bridge_message(missing_flag).is_err());
        assert!(parse_bridge_message(string_flag).is_err());
        assert!(parse_bridge_message(numeric_id).is_err());
    }

    #[test]
    fn serializes_a_typed_theme_enable_response() {
        use super::ThemeEnabledResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "enable-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::ThemeEnabled(ThemeEnabledResponse {
                id: "my-theme".to_owned(),
                enabled: true,
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "theme_enabled");
        assert_eq!(value["payload"]["payload"]["id"], "my-theme");
        assert_eq!(value["payload"]["payload"]["enabled"], true);
        assert_eq!(
            value["payload"]["payload"]
                .as_object()
                .map(|object| object.len()),
            Some(2)
        );
        // Id echo only: no path, CSS, or manifest echo.
        assert!(value["payload"]["payload"].get("path").is_none());
        assert!(value["payload"]["payload"].get("css").is_none());
        assert!(value["payload"]["payload"].get("manifest").is_none());
    }

    #[test]
    fn validates_theme_set_enabled_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"theme_set_enabled_probe","payload":{"ok":true,"stage":"set_enabled","id":"Probe-BD","enabled":false,"attacks_blocked":3,"reflected":true,"multi":true,"clearvision":"reflected","error":null}}}"#;
        let absent = r#"{"version":1,"event":{"type":"theme_set_enabled_probe","payload":{"ok":true,"stage":"set_enabled","id":"Probe-BD","enabled":false,"attacks_blocked":3,"reflected":true,"multi":true,"clearvision":"absent","error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"theme_set_enabled_probe","payload":{"ok":false,"stage":"set_enabled","id":"","enabled":false,"attacks_blocked":0,"reflected":false,"multi":false,"clearvision":"failed","error":"timeout"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"theme_set_enabled_probe","payload":{"ok":true,"stage":"write","id":"Probe-BD","enabled":false,"attacks_blocked":3,"reflected":true,"multi":true,"clearvision":"reflected","error":null}}}"#;
        let enabled = r#"{"version":1,"event":{"type":"theme_set_enabled_probe","payload":{"ok":true,"stage":"set_enabled","id":"Probe-BD","enabled":true,"attacks_blocked":3,"reflected":true,"multi":true,"clearvision":"reflected","error":null}}}"#;
        let unreflected = r#"{"version":1,"event":{"type":"theme_set_enabled_probe","payload":{"ok":true,"stage":"set_enabled","id":"Probe-BD","enabled":false,"attacks_blocked":3,"reflected":false,"multi":true,"clearvision":"reflected","error":null}}}"#;
        let exclusive = r#"{"version":1,"event":{"type":"theme_set_enabled_probe","payload":{"ok":true,"stage":"set_enabled","id":"Probe-BD","enabled":false,"attacks_blocked":3,"reflected":true,"multi":false,"clearvision":"reflected","error":null}}}"#;
        let cv_failed = r#"{"version":1,"event":{"type":"theme_set_enabled_probe","payload":{"ok":true,"stage":"set_enabled","id":"Probe-BD","enabled":false,"attacks_blocked":3,"reflected":true,"multi":true,"clearvision":"failed","error":null}}}"#;
        let inconsistent = r#"{"version":1,"event":{"type":"theme_set_enabled_probe","payload":{"ok":true,"stage":"set_enabled","id":"Probe-BD","enabled":false,"attacks_blocked":2,"reflected":true,"multi":true,"clearvision":"reflected","error":null}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(absent).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(enabled).is_err());
        assert!(parse_bridge_message(unreflected).is_err());
        assert!(parse_bridge_message(exclusive).is_err());
        assert!(parse_bridge_message(cv_failed).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
    }

    #[test]
    fn accepts_themes_list_with_an_exact_empty_payload() {
        let request =
            r#"{"version":1,"request_id":"list-1","command":{"type":"themes_list","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        // No path, root, selector, URL, or filter may ride along; the host
        // owns enumeration, ordering, and bounds.
        let with_path = r#"{"version":1,"request_id":"list-2","command":{"type":"themes_list","payload":{"path":"C:\\x"}}}"#;
        let with_root = r#"{"version":1,"request_id":"list-3","command":{"type":"themes_list","payload":{"root":"themes"}}}"#;
        let with_url = r#"{"version":1,"request_id":"list-4","command":{"type":"themes_list","payload":{"url":"https://attacker.invalid/"}}}"#;
        let with_filter = r#"{"version":1,"request_id":"list-5","command":{"type":"themes_list","payload":{"filter":"dark"}}}"#;
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_root).is_err());
        assert!(parse_bridge_message(with_url).is_err());
        assert!(parse_bridge_message(with_filter).is_err());
    }

    #[test]
    fn accepts_quickcss_read_and_bounded_write() {
        let read = r#"{"version":1,"request_id":"quickcss-1","command":{"type":"quick_css_read","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(read),
            Ok(BridgeMessage::Request(_))
        ));
        // No path, filename, URL, or selector may ride along; the host owns
        // the fixed file.
        let with_path = r#"{"version":1,"request_id":"quickcss-2","command":{"type":"quick_css_read","payload":{"path":"C:\\x"}}}"#;
        assert!(parse_bridge_message(with_path).is_err());

        let write = r#"{"version":1,"request_id":"quickcss-3","command":{"type":"quick_css_write","payload":{"css":":root{}"}}}"#;
        assert!(matches!(
            parse_bridge_message(write),
            Ok(BridgeMessage::Request(_))
        ));
        // Exactly one CSS text: no path, id, URL, or destination.
        let with_path = r#"{"version":1,"request_id":"quickcss-4","command":{"type":"quick_css_write","payload":{"css":":root{}","path":"C:\\x"}}}"#;
        let missing_css = r#"{"version":1,"request_id":"quickcss-5","command":{"type":"quick_css_write","payload":{}}}"#;
        let null_css = r#"{"version":1,"request_id":"quickcss-6","command":{"type":"quick_css_write","payload":{"css":null}}}"#;
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(missing_css).is_err());
        assert!(parse_bridge_message(null_css).is_err());
        // An oversize body is refused at parse, before any filesystem work.
        let big = "x".repeat(crate::quick_css::MAX_QUICK_CSS_BYTES + 1);
        let oversized = format!(
            "{{\"version\":1,\"request_id\":\"quickcss-7\",\"command\":{{\"type\":\"quick_css_write\",\"payload\":{{\"css\":\"{big}\"}}}}}}"
        );
        assert!(parse_bridge_message(&oversized).is_err());
    }

    #[test]
    fn serializes_quickcss_read_and_written_responses() {
        use super::{QuickCssReadResponse, QuickCssWrittenResponse};
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "quickcss-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::QuickCssRead(QuickCssReadResponse {
                css: ":root{}".to_owned(),
                bytes: 8,
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "quick_css_read");
        assert_eq!(value["payload"]["payload"]["css"], ":root{}");
        assert_eq!(value["payload"]["payload"]["bytes"], 8);
        assert_eq!(
            value["payload"]["payload"]
                .as_object()
                .map(|object| object.len()),
            Some(2)
        );
        let written = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "quickcss-2".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::QuickCssWritten(
                QuickCssWrittenResponse { bytes: 8 },
            )),
            error: None,
        };
        let value = serde_json::to_value(written).unwrap();
        assert_eq!(value["payload"]["type"], "quick_css_written");
        assert_eq!(value["payload"]["payload"]["bytes"], 8);
    }

    #[test]
    fn serializes_a_bounded_themes_list_response() {
        use super::ThemesListResponse;
        use crate::themes::ThemeListEntry;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "list-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::ThemesList(ThemesListResponse {
                themes: vec![ThemeListEntry {
                    id: "Midnight-Harbor-BD".to_owned(),
                    name: "Midnight Harbor".to_owned(),
                    author: "Harbor Line".to_owned(),
                    description: "A dark harbor theme.".to_owned(),
                    version: "1.2.0".to_owned(),
                    enabled: false,
                }],
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "themes_list");
        let entry = &value["payload"]["payload"]["themes"][0];
        assert_eq!(entry["id"], "Midnight-Harbor-BD");
        assert_eq!(entry["name"], "Midnight Harbor");
        assert_eq!(entry["enabled"], false);
        assert_eq!(entry.as_object().map(|object| object.len()), Some(6));
        // Display metadata only: no paths, CSS, URLs, or manifests.
        assert!(entry.get("path").is_none());
        assert!(entry.get("css").is_none());
        assert!(entry.get("url").is_none());
        assert!(entry.get("theme").is_none());
        assert!(entry.get("updateSrc").is_none());
        assert!(value["payload"]["payload"].get("profile").is_none());
    }

    #[test]
    fn validates_theme_list_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"theme_list_probe","payload":{"ok":true,"stage":"list","count":1,"clearvision":"present-disabled","error":null}}}"#;
        let absent = r#"{"version":1,"event":{"type":"theme_list_probe","payload":{"ok":true,"stage":"list","count":0,"clearvision":"absent","error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"theme_list_probe","payload":{"ok":false,"stage":"list","count":0,"clearvision":"failed","error":"timeout"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"theme_list_probe","payload":{"ok":true,"stage":"inventory","count":1,"clearvision":"present-disabled","error":null}}}"#;
        let cv_failed = r#"{"version":1,"event":{"type":"theme_list_probe","payload":{"ok":true,"stage":"list","count":1,"clearvision":"failed","error":null}}}"#;
        let with_path = r#"{"version":1,"event":{"type":"theme_list_probe","payload":{"ok":true,"stage":"list","count":1,"clearvision":"present-disabled","path":"C:\\x","error":null}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(absent).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(cv_failed).is_err());
        assert!(parse_bridge_message(with_path).is_err());
    }

    #[test]
    fn validates_shelter_loader_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"shelter_loader_probe","payload":{"ok":true,"stage":"cleanup","shelter":true,"plugins":true,"add_local_plugin":true,"start_plugin":true,"stop_plugin":true,"remove_plugin":true,"url_loader_absent":true,"synthetic":"verified","error":null}}}"#;
        let presence_failed = r#"{"version":1,"event":{"type":"shelter_loader_probe","payload":{"ok":false,"stage":"presence","shelter":true,"plugins":true,"add_local_plugin":false,"start_plugin":true,"stop_plugin":true,"remove_plugin":true,"url_loader_absent":true,"synthetic":"skipped","error":"loader surface unavailable"}}}"#;
        let synthetic_failed = r#"{"version":1,"event":{"type":"shelter_loader_probe","payload":{"ok":false,"stage":"synthetic","shelter":true,"plugins":true,"add_local_plugin":true,"start_plugin":true,"stop_plugin":true,"remove_plugin":true,"url_loader_absent":true,"synthetic":"skipped","error":"synthetic plugin did not start"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"shelter_loader_probe","payload":{"ok":true,"stage":"loaded","shelter":true,"plugins":true,"add_local_plugin":true,"start_plugin":true,"stop_plugin":true,"remove_plugin":true,"url_loader_absent":true,"synthetic":"verified","error":null}}}"#;
        let ok_unverified = r#"{"version":1,"event":{"type":"shelter_loader_probe","payload":{"ok":true,"stage":"cleanup","shelter":true,"plugins":true,"add_local_plugin":true,"start_plugin":true,"stop_plugin":true,"remove_plugin":true,"url_loader_absent":true,"synthetic":"skipped","error":null}}}"#;
        let ok_url_present = r#"{"version":1,"event":{"type":"shelter_loader_probe","payload":{"ok":true,"stage":"cleanup","shelter":true,"plugins":true,"add_local_plugin":true,"start_plugin":true,"stop_plugin":true,"remove_plugin":true,"url_loader_absent":false,"synthetic":"verified","error":null}}}"#;
        let with_path = r#"{"version":1,"event":{"type":"shelter_loader_probe","payload":{"ok":true,"stage":"cleanup","shelter":true,"plugins":true,"add_local_plugin":true,"start_plugin":true,"stop_plugin":true,"remove_plugin":true,"url_loader_absent":true,"synthetic":"verified","path":"C:\\x","error":null}}}"#;
        let with_source = r#"{"version":1,"event":{"type":"shelter_loader_probe","payload":{"ok":true,"stage":"cleanup","shelter":true,"plugins":true,"add_local_plugin":true,"start_plugin":true,"stop_plugin":true,"remove_plugin":true,"url_loader_absent":true,"synthetic":"verified","js":"({onLoad(){}})","error":null}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(presence_failed).is_ok());
        assert!(parse_bridge_message(synthetic_failed).is_ok());
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(ok_unverified).is_err());
        assert!(parse_bridge_message(ok_url_present).is_err());
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_source).is_err());
    }

    #[test]
    fn accepts_backup_save_with_exact_shape_and_rejects_paths() {
        let request = r#"{"version":1,"request_id":"backup-1","command":{"type":"backup_save","payload":{"includes":{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true},"clientMods":{"vencordLocalStorage":"{\"a\":1}","shelter":{"plugins":{}}}}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        // No path, filename, archive path, source path, dialog option, or
        // arbitrary key may ride along at any level.
        let with_path = r#"{"version":1,"request_id":"backup-2","command":{"type":"backup_save","payload":{"includes":{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true},"clientMods":{},"outputPath":"C:\\x.zip"}}}"#;
        let with_filename = r#"{"version":1,"request_id":"backup-3","command":{"type":"backup_save","payload":{"includes":{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true},"clientMods":{},"filename":"x.zip"}}}"#;
        let with_include = r#"{"version":1,"request_id":"backup-4","command":{"type":"backup_save","payload":{"includes":{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true,"everything":true},"clientMods":{}}}}"#;
        let with_mod = r#"{"version":1,"request_id":"backup-5","command":{"type":"backup_save","payload":{"includes":{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true},"clientMods":{"discordToken":"x"}}}}"#;
        let missing_mods = r#"{"version":1,"request_id":"backup-6","command":{"type":"backup_save","payload":{"includes":{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true}}}}"#;
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_filename).is_err());
        assert!(parse_bridge_message(with_include).is_err());
        assert!(parse_bridge_message(with_mod).is_err());
        assert!(parse_bridge_message(missing_mods).is_err());
    }

    #[test]
    fn backup_save_enforces_client_envelope_bounds() {
        let includes = r#""includes":{"remcordConfig":false,"remcordThemesAndQuickCss":false,"remcordExtensionPlugins":false,"vencordModData":true,"equicordModData":false,"shelterModData":false,"modBundles":false}"#;
        // A 9 KiB client string fits the legacy 8 KiB transport only through
        // the backup exception, and parses (the 1 MiB per-string bound holds).
        let medium = "x".repeat(9000);
        let fits = format!(
            "{{\"version\":1,\"request_id\":\"backup-7\",\"command\":{{\"type\":\"backup_save\",\"payload\":{{{includes},\"clientMods\":{{\"vencordLocalStorage\":\"{medium}\"}}}}}}}}"
        );
        assert!(matches!(
            parse_bridge_message(&fits),
            Ok(BridgeMessage::Request(_))
        ));
        // Past the per-string bound, the request fails.
        let big = "x".repeat(super::MAX_CLIENT_MOD_STRING_BYTES + 1);
        let over = format!(
            "{{\"version\":1,\"request_id\":\"backup-8\",\"command\":{{\"type\":\"backup_save\",\"payload\":{{{includes},\"clientMods\":{{\"vencordLocalStorage\":\"{big}\"}}}}}}}}"
        );
        assert!(parse_bridge_message(&over).is_err());
        // Past the absolute ceiling, even backup saves are rejected.
        let huge = "x".repeat(super::MAX_BACKUP_SAVE_MESSAGE_BYTES);
        let huge_save = format!(
            "{{\"version\":1,\"request_id\":\"backup-9\",\"command\":{{\"type\":\"backup_save\",\"payload\":{{{includes},\"clientMods\":{{\"vencordLocalStorage\":\"{huge}\"}}}}}}}}"
        );
        assert!(parse_bridge_message(&huge_save).is_err());
    }

    #[test]
    fn serializes_a_typed_backup_saved_response() {
        use super::BackupSavedResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "backup-10".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::BackupSaved(BackupSavedResponse {
                canceled: false,
                bytes: 2048,
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "backup_saved");
        assert_eq!(value["payload"]["payload"]["canceled"], false);
        assert_eq!(value["payload"]["payload"]["bytes"], 2048);
        // Bounded response: no path, filename, URL, or source echo.
        assert!(value["payload"]["payload"].get("path").is_none());
        assert!(value["payload"]["payload"].get("filename").is_none());
        assert!(value["payload"]["payload"].get("url").is_none());
    }

    #[test]
    fn validates_backup_save_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"backup_save_probe","payload":{"ok":true,"stage":"save","canceled":false,"bytes":2048,"error":null}}}"#;
        let canceled = r#"{"version":1,"event":{"type":"backup_save_probe","payload":{"ok":true,"stage":"save","canceled":true,"bytes":0,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"backup_save_probe","payload":{"ok":false,"stage":"save","canceled":false,"bytes":0,"error":"timeout"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"backup_save_probe","payload":{"ok":true,"stage":"export","canceled":false,"bytes":1,"error":null}}}"#;
        let with_path = r#"{"version":1,"event":{"type":"backup_save_probe","payload":{"ok":true,"stage":"save","canceled":false,"bytes":1,"path":"C:\\x","error":null}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(canceled).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(with_path).is_err());
    }

    #[test]
    fn accepts_theme_import_files_with_an_exact_empty_payload() {
        let request = r#"{"version":1,"request_id":"import-1","command":{"type":"theme_import_files","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        // No path, filename, URL, directory, filter, or dialog option may
        // ride along; the host defines the entire picker configuration.
        let with_path = r#"{"version":1,"request_id":"import-2","command":{"type":"theme_import_files","payload":{"path":"C:\\x"}}}"#;
        let with_url = r#"{"version":1,"request_id":"import-3","command":{"type":"theme_import_files","payload":{"url":"https://attacker.invalid/x.css"}}}"#;
        let with_file = r#"{"version":1,"request_id":"import-4","command":{"type":"theme_import_files","payload":{"filename":"x.css"}}}"#;
        let with_dir = r#"{"version":1,"request_id":"import-5","command":{"type":"theme_import_files","payload":{"directory":"C:\\themes"}}}"#;
        let with_filters = r#"{"version":1,"request_id":"import-6","command":{"type":"theme_import_files","payload":{"filters":[]}}}"#;
        let with_options = r#"{"version":1,"request_id":"import-7","command":{"type":"theme_import_files","payload":{"options":{}}}}"#;
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_url).is_err());
        assert!(parse_bridge_message(with_file).is_err());
        assert!(parse_bridge_message(with_dir).is_err());
        assert!(parse_bridge_message(with_filters).is_err());
        assert!(parse_bridge_message(with_options).is_err());
    }

    #[test]
    fn serializes_a_bounded_theme_import_summary() {
        use super::ThemeImportedResponse;
        for (canceled, imported, failed) in [(true, 0, 0), (false, 2, 1), (false, 0, 3)] {
            let response = BridgeResponse {
                version: PROTOCOL_VERSION,
                request_id: "import-1".to_owned(),
                ok: true,
                payload: Some(BridgeResponsePayload::ThemeImported(
                    ThemeImportedResponse {
                        canceled,
                        imported,
                        failed,
                    },
                )),
                error: None,
            };
            let value = serde_json::to_value(response).unwrap();
            assert_eq!(value["payload"]["type"], "theme_imported");
            assert_eq!(value["payload"]["payload"]["canceled"], canceled);
            assert_eq!(value["payload"]["payload"]["imported"], imported);
            assert_eq!(value["payload"]["payload"]["failed"], failed);
            // Counts only: no ids, arrays, paths, URLs, or source echo.
            assert_eq!(
                value["payload"]["payload"]
                    .as_object()
                    .map(|object| object.len()),
                Some(3)
            );
            assert!(value["payload"]["payload"].get("id").is_none());
            assert!(value["payload"]["payload"].get("ids").is_none());
            assert!(value["payload"]["payload"].get("path").is_none());
            assert!(value["payload"]["payload"].get("url").is_none());
            assert!(value["payload"]["payload"].get("css").is_none());
            assert!(value["payload"]["payload"].get("error").is_none());
        }
    }

    #[test]
    fn serializes_a_typed_bundle_inventory_response() {
        use super::{BundleFileEntry, BundlesListResponse};
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "bundles-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::BundlesList(BundlesListResponse {
                bundles: vec![
                    BundleFileEntry {
                        name: "vencord.js".to_owned(),
                        present: true,
                        bytes: 698272,
                    },
                    BundleFileEntry {
                        name: "shelter.js".to_owned(),
                        present: false,
                        bytes: 0,
                    },
                ],
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "bundles_list");
        assert_eq!(
            value["payload"]["payload"]["bundles"][0]["name"],
            "vencord.js"
        );
        assert_eq!(value["payload"]["payload"]["bundles"][0]["present"], true);
        assert_eq!(value["payload"]["payload"]["bundles"][0]["bytes"], 698272);
        // The response carries metadata only: never a path, URL, or content.
        assert!(value["payload"]["payload"]["bundles"][0]
            .get("path")
            .is_none());
        assert!(value["payload"]["payload"]["bundles"][0]
            .get("url")
            .is_none());
        assert!(value["payload"]["payload"]["bundles"][0]
            .get("data")
            .is_none());
    }

    #[test]
    fn accepts_only_the_allowlisted_block_power_saving_mutation() {
        let valid = r#"{"version":1,"request_id":"settings-1","command":{"type":"settings_set_block_power_saving","payload":{"value":true}}}"#;
        assert!(matches!(
            parse_bridge_message(valid),
            Ok(BridgeMessage::Request(_))
        ));

        // Wrong value types are rejected before dispatch.
        let string_value = r#"{"version":1,"request_id":"settings-2","command":{"type":"settings_set_block_power_saving","payload":{"value":"yes"}}}"#;
        let null_value = r#"{"version":1,"request_id":"settings-3","command":{"type":"settings_set_block_power_saving","payload":{"value":null}}}"#;
        let int_value = r#"{"version":1,"request_id":"settings-4","command":{"type":"settings_set_block_power_saving","payload":{"value":1}}}"#;
        let missing_value = r#"{"version":1,"request_id":"settings-5","command":{"type":"settings_set_block_power_saving","payload":{}}}"#;
        assert!(parse_bridge_message(string_value).is_err());
        assert!(parse_bridge_message(null_value).is_err());
        assert!(parse_bridge_message(int_value).is_err());
        assert!(parse_bridge_message(missing_value).is_err());
    }

    #[test]
    fn rejects_unauthorized_mutation_shapes() {
        // No key, field name, path, JSON patch, or object may select a
        // different setting: the mutated field is fixed at compile time.
        let with_key = r#"{"version":1,"request_id":"settings-1","command":{"type":"settings_set_block_power_saving","payload":{"value":true,"key":"minimizeToTray"}}}"#;
        let with_field = r#"{"version":1,"request_id":"settings-2","command":{"type":"settings_set_block_power_saving","payload":{"value":true,"field":"tray"}}}"#;
        let with_path = r#"{"version":1,"request_id":"settings-3","command":{"type":"settings_set_block_power_saving","payload":{"value":true,"path":"C:\\secret"}}}"#;
        let with_object = r#"{"version":1,"request_id":"settings-4","command":{"type":"settings_set_block_power_saving","payload":{"settings":{"tray":"disabled"}}}}}"#;
        assert!(parse_bridge_message(with_key).is_err());
        assert!(parse_bridge_message(with_field).is_err());
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_object).is_err());
    }

    #[test]
    fn accepts_only_the_allowlisted_settings_set_keys() {
        let mods = r#"{"version":1,"request_id":"settings-1","command":{"type":"settings_set","payload":{"key":"mods","value":["vencord"]}}}"#;
        let dismissed = r#"{"version":1,"request_id":"settings-2","command":{"type":"settings_set","payload":{"key":"support_banner_dismissed","value":true}}}"#;
        let experimental = r#"{"version":1,"request_id":"settings-3","command":{"type":"settings_set","payload":{"key":"show_experimental_plugin_menu","value":false}}}"#;
        let abilities = r#"{"version":1,"request_id":"settings-4","command":{"type":"settings_set","payload":{"key":"extended_plugin_abilities","value":true}}}"#;
        let csp = r#"{"version":1,"request_id":"settings-5","command":{"type":"settings_set","payload":{"key":"csp","value":"strict"}}}"#;
        let quick_css = r#"{"version":1,"request_id":"settings-5b","command":{"type":"settings_set","payload":{"key":"quick_css","value":true}}}"#;
        let channel = r#"{"version":1,"request_id":"settings-5c","command":{"type":"settings_set","payload":{"key":"channel","value":"canary"}}}"#;
        assert!(matches!(
            parse_bridge_message(mods),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(matches!(
            parse_bridge_message(dismissed),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(matches!(
            parse_bridge_message(experimental),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(matches!(
            parse_bridge_message(abilities),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(matches!(
            parse_bridge_message(csp),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(matches!(
            parse_bridge_message(quick_css),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(matches!(
            parse_bridge_message(channel),
            Ok(BridgeMessage::Request(_))
        ));

        // Unknown keys never parse: the allowlist is the enum.
        let unknown = r#"{"version":1,"request_id":"settings-6","command":{"type":"settings_set","payload":{"key":"minimizeToTray","value":true}}}"#;
        let tray = r#"{"version":1,"request_id":"settings-7","command":{"type":"settings_set","payload":{"key":"tray","value":"disabled"}}}"#;
        let proxy = r#"{"version":1,"request_id":"settings-8","command":{"type":"settings_set","payload":{"key":"proxyRules","value":"http://127.0.0.1:8080"}}}"#;
        let icon = r#"{"version":1,"request_id":"settings-9","command":{"type":"settings_set","payload":{"key":"customIcon","value":"C:\\secret.png"}}}"#;
        // A policy string smuggled through another key never parses: the
        // csp string variant pairs only with the csp key.
        let smuggled = r#"{"version":1,"request_id":"settings-10","command":{"type":"settings_set","payload":{"key":"support_banner_dismissed","value":"none"}}}"#;
        // An unknown csp mode parses the wire shape but fails validation.
        let hostile_mode = r#"{"version":1,"request_id":"settings-11","command":{"type":"settings_set","payload":{"key":"csp","value":"none; script-src *"}}}"#;
        // An unknown channel parses the wire shape but fails validation;
        // a channel string through the csp key (and vice versa) is refused
        // by the key-owned variant pairing.
        let hostile_channel = r#"{"version":1,"request_id":"settings-12","command":{"type":"settings_set","payload":{"key":"channel","value":"beta"}}}"#;
        let crossed_csp = r#"{"version":1,"request_id":"settings-13","command":{"type":"settings_set","payload":{"key":"channel","value":"strict"}}}"#;
        let crossed_channel = r#"{"version":1,"request_id":"settings-14","command":{"type":"settings_set","payload":{"key":"csp","value":"canary"}}}"#;
        assert!(parse_bridge_message(unknown).is_err());
        assert!(parse_bridge_message(tray).is_err());
        assert!(parse_bridge_message(proxy).is_err());
        assert!(parse_bridge_message(icon).is_err());
        assert!(parse_bridge_message(smuggled).is_err());
        assert!(parse_bridge_message(hostile_mode).is_err());
        assert!(parse_bridge_message(hostile_channel).is_err());
        assert!(parse_bridge_message(crossed_csp).is_err());
        assert!(parse_bridge_message(crossed_channel).is_err());
    }

    #[test]
    fn accepts_diagnostics_and_mod_cache_commands_without_parameters() {
        // Both new capabilities are parameter-free reads/wipes of
        // host-owned state: any renderer-supplied member is rejected.
        for (wire, request_id) in [
            ("diagnostics_get_info", "diag-1"),
            ("graphics_get_info", "gfx-1"),
            ("mod_clear_cache", "modcache-1"),
        ] {
            let message = format!(
                "{{\"version\":1,\"request_id\":\"{request_id}\",\"command\":{{\"type\":\"{wire}\",\"payload\":{{}}}}}}"
            );
            let parsed = parse_bridge_message(&message);
            assert!(parsed.is_ok(), "{wire} must parse: {parsed:?}");
            let command = match parsed.unwrap() {
                BridgeMessage::Request(request) => request.command,
                BridgeMessage::Event(_) => panic!("{wire} must parse as a request"),
            };
            let expected = match wire {
                "diagnostics_get_info" => matches!(command, BridgeCommand::DiagnosticsGetInfo(_)),
                "graphics_get_info" => matches!(command, BridgeCommand::GraphicsGetInfo(_)),
                "mod_clear_cache" => matches!(command, BridgeCommand::ModClearCache(_)),
                _ => unreachable!(),
            };
            assert!(expected, "{wire} must dispatch to its own typed variant");
        }
        for payload in [
            r#"{"key":"mods"}"#,
            r#"{"mod_id":"vencord"}"#,
            r#"{"path":"C:\\secret"}"#,
            r#"{"detail":"full"}"#,
            r#"{"targets":["vencord.js"]}"#,
            r#"{"method":"System.getInfo"}"#,
        ] {
            for wire in [
                "diagnostics_get_info",
                "graphics_get_info",
                "mod_clear_cache",
            ] {
                let message = format!(
                    "{{\"version\":1,\"request_id\":\"x\",\"command\":{{\"type\":\"{wire}\",\"payload\":{payload}}}}}"
                );
                assert!(
                    parse_bridge_message(&message).is_err(),
                    "{wire} with {payload} must be rejected"
                );
            }
        }
    }

    #[test]
    fn rejects_mismatched_and_hostile_settings_set_values() {
        // Bool keys refuse arrays; the mods key refuses a bool; the csp key
        // refuses bools and arrays, and bool/mods keys refuse csp strings.
        let mods_bool = r#"{"version":1,"request_id":"settings-1","command":{"type":"settings_set","payload":{"key":"mods","value":true}}}"#;
        let bool_mods = r#"{"version":1,"request_id":"settings-2","command":{"type":"settings_set","payload":{"key":"support_banner_dismissed","value":["vencord"]}}}"#;
        // Strings pair only with the csp key (and only exact modes there):
        // a bool key with a string, a mods key with a string, the csp key
        // with a bool, and an unknown csp mode are all refused.
        let string_value = r#"{"version":1,"request_id":"settings-3","command":{"type":"settings_set","payload":{"key":"support_banner_dismissed","value":"yes"}}}"#;
        let csp_bool = r#"{"version":1,"request_id":"settings-3b","command":{"type":"settings_set","payload":{"key":"csp","value":true}}}"#;
        let csp_mode_unknown = r#"{"version":1,"request_id":"settings-3c","command":{"type":"settings_set","payload":{"key":"csp","value":"permissive"}}}"#;
        // Numbers, null, and objects are never valid values.
        let null_value = r#"{"version":1,"request_id":"settings-4","command":{"type":"settings_set","payload":{"key":"mods","value":null}}}"#;
        let object_value = r#"{"version":1,"request_id":"settings-5","command":{"type":"settings_set","payload":{"key":"mods","value":{"vencord":true}}}}"#;
        // Unknown mod ids, traversal shapes, and oversized lists are refused.
        let unknown_mod = r#"{"version":1,"request_id":"settings-6","command":{"type":"settings_set","payload":{"key":"mods","value":["Vencord"]}}}"#;
        let traversal = r#"{"version":1,"request_id":"settings-7","command":{"type":"settings_set","payload":{"key":"mods","value":["../escape"]}}}"#;
        let missing_key = r#"{"version":1,"request_id":"settings-8","command":{"type":"settings_set","payload":{"value":true}}}"#;
        // Arbitrary key/value, path, and URL members are rejected.
        let generic = r#"{"version":1,"request_id":"settings-9","command":{"type":"settings_set","payload":{"key":"support_banner_dismissed","value":true,"path":"C:\\secret"}}}"#;
        assert!(parse_bridge_message(mods_bool).is_err());
        assert!(parse_bridge_message(bool_mods).is_err());
        assert!(parse_bridge_message(string_value).is_err());
        assert!(parse_bridge_message(csp_bool).is_err());
        assert!(parse_bridge_message(csp_mode_unknown).is_err());
        assert!(parse_bridge_message(null_value).is_err());
        assert!(parse_bridge_message(object_value).is_err());
        assert!(parse_bridge_message(unknown_mod).is_err());
        assert!(parse_bridge_message(traversal).is_err());
        assert!(parse_bridge_message(missing_key).is_err());
        assert!(parse_bridge_message(generic).is_err());
    }

    #[test]
    fn serializes_a_typed_settings_set_response() {
        use super::{SettingsSetKey, SettingsSetResponse, SettingsSetValue};
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "settings-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::SettingsSet(SettingsSetResponse {
                key: SettingsSetKey::Mods,
                value: SettingsSetValue::Mods(vec!["vencord".to_owned()]),
                source: "file",
                payload_bytes: 2048,
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "settings_set");
        assert_eq!(value["payload"]["payload"]["key"], "mods");
        assert_eq!(
            value["payload"]["payload"]["value"],
            serde_json::json!(["vencord"])
        );
        assert_eq!(value["payload"]["payload"]["source"], "file");
        // The response carries the key echo plus the applied value only:
        // never a path, URL, or full snapshot.
        assert!(value["payload"]["payload"].get("path").is_none());
        assert!(value["payload"]["payload"].get("url").is_none());
        assert!(value["payload"]["payload"].get("settings").is_none());
    }

    #[test]
    fn serializes_a_typed_settings_mutation_response() {
        use super::SettingsMutationResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "settings-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::SettingsMutation(
                SettingsMutationResponse {
                    block_power_saving_in_voice_chat: true,
                    source: "file",
                    payload_bytes: 2048,
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "settings_mutation");
        assert_eq!(
            value["payload"]["payload"]["block_power_saving_in_voice_chat"],
            true
        );
        assert_eq!(value["payload"]["payload"]["source"], "file");
        // The response carries only the applied field, never a key/path.
        assert!(value["payload"]["payload"].get("key").is_none());
        assert!(value["payload"]["payload"].get("path").is_none());
    }

    #[test]
    fn validates_settings_mutation_probe_diagnostics() {
        let mutated = r#"{"version":1,"event":{"type":"settings_mutation_probe","payload":{"ok":true,"stage":"mutated","before":false,"value":true,"reread_matches":true,"error":null}}}"#;
        let restored = r#"{"version":1,"event":{"type":"settings_mutation_probe","payload":{"ok":true,"stage":"restored","before":true,"value":false,"reread_matches":true,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"settings_mutation_probe","payload":{"ok":false,"stage":"mutated","before":false,"value":false,"reread_matches":false,"error":"timeout"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"settings_mutation_probe","payload":{"ok":true,"stage":"deleted","before":false,"value":true,"reread_matches":true,"error":null}}}"#;
        let inconsistent = r#"{"version":1,"event":{"type":"settings_mutation_probe","payload":{"ok":true,"stage":"mutated","before":false,"value":true,"reread_matches":true,"error":"extra"}}}"#;
        assert!(parse_bridge_message(mutated).is_ok());
        assert!(parse_bridge_message(restored).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
    }

    #[test]
    fn validates_settings_set_probe_diagnostics() {
        let mutated = r#"{"version":1,"event":{"type":"settings_set_probe","payload":{"ok":true,"stage":"mutated","key":"support_banner_dismissed","before":false,"reread_matches":true,"attacks_blocked":3,"error":null}}}"#;
        let restored = r#"{"version":1,"event":{"type":"settings_set_probe","payload":{"ok":true,"stage":"restored","key":"support_banner_dismissed","before":true,"reread_matches":true,"attacks_blocked":3,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"settings_set_probe","payload":{"ok":false,"stage":"mutated","key":"support_banner_dismissed","before":false,"reread_matches":false,"attacks_blocked":1,"error":"timeout"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"settings_set_probe","payload":{"ok":true,"stage":"deleted","key":"support_banner_dismissed","before":false,"reread_matches":true,"attacks_blocked":3,"error":null}}}"#;
        let bad_key = r#"{"version":1,"event":{"type":"settings_set_probe","payload":{"ok":true,"stage":"mutated","key":"tray","before":false,"reread_matches":true,"attacks_blocked":3,"error":null}}}"#;
        let inconsistent = r#"{"version":1,"event":{"type":"settings_set_probe","payload":{"ok":true,"stage":"mutated","key":"support_banner_dismissed","before":false,"reread_matches":false,"attacks_blocked":3,"error":null}}}"#;
        assert!(parse_bridge_message(mutated).is_ok());
        assert!(parse_bridge_message(restored).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(bad_key).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
    }

    #[test]
    fn mutation_request_ids_reset_with_the_document() {
        let mut tracker = RequestTracker::default();
        assert!(tracker.begin("settings-mutate-1").is_ok());
        // A duplicate within one document is rejected (stale/double-submit).
        assert!(tracker.begin("settings-mutate-1").is_err());
        tracker.clear();
        assert!(tracker.begin("settings-mutate-1").is_ok());
        assert!(tracker.complete("settings-mutate-1").is_ok());
    }
    #[test]
    fn serializes_a_typed_settings_snapshot_response() {
        use super::SettingsConfigResponse;
        use crate::settings::SettingsSnapshot;
        let snapshot = SettingsSnapshot::default();
        let payload_bytes = serde_json::to_string(&snapshot).unwrap().len();
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "settings-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::SettingsConfig(Box::new(
                SettingsConfigResponse {
                    settings: snapshot,
                    source: "defaults",
                    payload_bytes,
                },
            ))),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "settings_config");
        assert_eq!(
            value["payload"]["payload"]["settings"]["blockPowerSavingInVoiceChat"],
            false
        );
        assert_eq!(value["payload"]["payload"]["source"], "defaults");
    }

    #[test]
    fn settings_request_ids_reset_with_the_document() {
        let mut tracker = RequestTracker::default();
        assert!(tracker.begin("settings-1").is_ok());
        tracker.clear();
        // After a document reset the same id shape is usable again and stale
        // completions no longer correlate.
        assert!(tracker.begin("settings-1").is_ok());
        assert!(tracker.complete("settings-1").is_ok());
    }

    #[test]
    fn validates_settings_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"settings_probe","payload":{"ok":true,"source":"file","payload_bytes":2048,"block_power_saving":false,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"settings_probe","payload":{"ok":false,"source":"unavailable","payload_bytes":0,"block_power_saving":false,"error":"timeout"}}}"#;
        let bad_source = r#"{"version":1,"event":{"type":"settings_probe","payload":{"ok":true,"source":"electron","payload_bytes":10,"block_power_saving":false,"error":null}}}"#;
        let inconsistent = r#"{"version":1,"event":{"type":"settings_probe","payload":{"ok":true,"source":"file","payload_bytes":10,"block_power_saving":false,"error":"extra"}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_source).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
    }

    #[test]
    fn accepts_a_valid_get_lang_request() {
        let message = r#"{"version":1,"request_id":"lang-1","command":{"type":"localization_get_lang","payload":{"key":"invite-goBackToApp"}}}"#;
        assert!(matches!(
            parse_bridge_message(message),
            Ok(BridgeMessage::Request(_))
        ));
    }

    #[test]
    fn rejects_empty_and_oversized_localization_keys() {
        let empty = r#"{"version":1,"request_id":"lang-1","command":{"type":"localization_get_lang","payload":{"key":""}}}"#;
        let oversized = format!(
            r#"{{"version":1,"request_id":"lang-2","command":{{"type":"localization_get_lang","payload":{{"key":"{}"}}}}}}"#,
            "k".repeat(129)
        );
        assert!(parse_bridge_message(empty).is_err());
        assert!(parse_bridge_message(&oversized).is_err());
    }

    #[test]
    fn rejects_non_key_localization_parameters() {
        let numeric = r#"{"version":1,"request_id":"lang-1","command":{"type":"localization_get_lang","payload":{"key":42}}}"#;
        let with_path = r#"{"version":1,"request_id":"lang-2","command":{"type":"localization_get_lang","payload":{"key":"a","path":"C:\\secret"}}}"#;
        let with_locale = r#"{"version":1,"request_id":"lang-3","command":{"type":"localization_get_lang","payload":{"key":"a","locale":"en-US"}}}"#;
        assert!(parse_bridge_message(numeric).is_err());
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_locale).is_err());
    }

    #[test]
    fn serializes_a_typed_localization_response() {
        use super::LocalizationStringResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "lang-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::LocalizationString(
                LocalizationStringResponse {
                    value: "Go back to the Discord app".to_owned(),
                    lang: "en-US".to_owned(),
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "localization_string");
        assert_eq!(
            value["payload"]["payload"]["value"],
            "Go back to the Discord app"
        );
        assert_eq!(value["payload"]["payload"]["lang"], "en-US");
    }

    #[test]
    fn localization_request_ids_reset_with_the_document() {
        let mut tracker = RequestTracker::default();
        assert!(tracker.begin("lang-1").is_ok());
        tracker.clear();
        assert!(tracker.begin("lang-1").is_ok());
        assert!(tracker.complete("lang-1").is_ok());
    }

    #[test]
    fn validates_localization_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"get_lang_probe","payload":{"ok":true,"lang":"en-US","value":"Go back to the Discord app","missing_is_empty":true,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"get_lang_probe","payload":{"ok":false,"lang":"en-US","value":"","missing_is_empty":false,"error":"timeout"}}}"#;
        let oversized = format!(
            r#"{{"version":1,"event":{{"type":"get_lang_probe","payload":{{"ok":true,"lang":"en-US","value":"{}","missing_is_empty":true,"error":null}}}}}}"#,
            "v".repeat(5000)
        );
        let inconsistent = r#"{"version":1,"event":{"type":"get_lang_probe","payload":{"ok":true,"lang":"en-US","value":"x","missing_is_empty":true,"error":"extra"}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(&oversized).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
    }

    #[test]
    fn accepts_the_narrow_settings_boot_events() {
        let set_lang = r#"{"version":1,"event":{"type":"settings_boot_set_lang","payload":{"locale":"en-US"}}}"#;
        let restart = r#"{"version":1,"event":{"type":"settings_boot_restart","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(set_lang),
            Ok(BridgeMessage::Event(_))
        ));
        assert!(matches!(
            parse_bridge_message(restart),
            Ok(BridgeMessage::Event(_))
        ));
    }

    #[test]
    fn rejects_boot_event_shapes_outside_the_closed_surface() {
        // Empty/oversized locales, path/URL/file selectors, and any restart
        // payload member are never valid members of these events.
        let empty =
            r#"{"version":1,"event":{"type":"settings_boot_set_lang","payload":{"locale":""}}}"#;
        let oversized = format!(
            r#"{{"version":1,"event":{{"type":"settings_boot_set_lang","payload":{{"locale":"{}"}}}}}}"#,
            "a".repeat(33)
        );
        let with_path = r#"{"version":1,"event":{"type":"settings_boot_set_lang","payload":{"locale":"en-US","path":"C:\\secret"}}}"#;
        let with_url = r#"{"version":1,"event":{"type":"settings_boot_set_lang","payload":{"locale":"en-US","url":"https://attacker.invalid"}}}"#;
        let with_lang_file = r#"{"version":1,"event":{"type":"settings_boot_set_lang","payload":{"locale":"en-US","file":"lang.json"}}}"#;
        let restart_with_arg = r#"{"version":1,"event":{"type":"settings_boot_restart","payload":{"reason":"update"}}}"#;
        let generic_invoke = r#"{"version":1,"request_id":"boot-1","command":{"type":"invoke","payload":{"method":"settings.setLang"}}}"#;
        assert!(parse_bridge_message(empty).is_err());
        assert!(parse_bridge_message(&oversized).is_err());
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_url).is_err());
        assert!(parse_bridge_message(with_lang_file).is_err());
        assert!(parse_bridge_message(restart_with_arg).is_err());
        assert!(parse_bridge_message(generic_invoke).is_err());
    }

    #[test]
    fn validates_settings_boot_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"settings_boot_probe","payload":{"ok":true,"settings_source":"file","settings_bytes":1666,"themes_count":0,"i18n_keys":200,"lang":"en-US","frozen":true,"keys_exact":true,"snapshot_bytes":30000,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"settings_boot_probe","payload":{"ok":false,"settings_source":"unavailable","settings_bytes":0,"themes_count":0,"i18n_keys":0,"lang":"en-US","frozen":false,"keys_exact":false,"snapshot_bytes":0,"error":"snapshot missing"}}}"#;
        let unfrozen_ok = r#"{"version":1,"event":{"type":"settings_boot_probe","payload":{"ok":true,"settings_source":"file","settings_bytes":1666,"themes_count":0,"i18n_keys":200,"lang":"en-US","frozen":false,"keys_exact":true,"snapshot_bytes":30000,"error":null}}}"#;
        let bad_source = r#"{"version":1,"event":{"type":"settings_boot_probe","payload":{"ok":true,"settings_source":"guessed","settings_bytes":1666,"themes_count":0,"i18n_keys":200,"lang":"en-US","frozen":true,"keys_exact":true,"snapshot_bytes":30000,"error":null}}}"#;
        let too_many_themes = r#"{"version":1,"event":{"type":"settings_boot_probe","payload":{"ok":true,"settings_source":"file","settings_bytes":1666,"themes_count":257,"i18n_keys":200,"lang":"en-US","frozen":true,"keys_exact":true,"snapshot_bytes":30000,"error":null}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(unfrozen_ok).is_err());
        assert!(parse_bridge_message(bad_source).is_err());
        assert!(parse_bridge_message(too_many_themes).is_err());
    }

    #[test]
    fn validates_settings_ui_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"settings_ui_probe","payload":{"stage":"enable","ok":true,"detail":"nodes=1 checked=true","error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"settings_ui_probe","payload":{"stage":"gear","ok":false,"detail":"","error":"gear missing"}}}"#;
        let inconsistent = r#"{"version":1,"event":{"type":"settings_ui_probe","payload":{"stage":"open","ok":true,"detail":"","error":"extra"}}}"#;
        let oversized = format!(
            r#"{{"version":1,"event":{{"type":"settings_ui_probe","payload":{{"stage":"open","ok":true,"detail":"{}","error":null}}}}}}"#,
            "d".repeat(513)
        );
        let extra = r#"{"version":1,"event":{"type":"settings_ui_probe","payload":{"stage":"open","ok":true,"detail":"","error":null,"path":"C:\\x"}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(inconsistent).is_err());
        assert!(parse_bridge_message(&oversized).is_err());
        assert!(parse_bridge_message(extra).is_err());
    }

    #[test]
    fn accepts_the_app_version_capability_without_renderer_parameters() {
        let request = r#"{"version":1,"request_id":"app-1","command":{"type":"app_get_version","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        // No channel, key, path, or selector may be supplied.
        let with_channel = r#"{"version":1,"request_id":"app-2","command":{"type":"app_get_version","payload":{"channel":"get-app-version"}}}"#;
        let with_key = r#"{"version":1,"request_id":"app-3","command":{"type":"app_get_version","payload":{"key":"version"}}}"#;
        let with_path = r#"{"version":1,"request_id":"app-4","command":{"type":"app_get_version","payload":{"path":"C:\\secret"}}}"#;
        assert!(parse_bridge_message(with_channel).is_err());
        assert!(parse_bridge_message(with_key).is_err());
        assert!(parse_bridge_message(with_path).is_err());
    }

    #[test]
    fn rejects_generic_invocation_for_the_app_version() {
        let invoke = r#"{"version":1,"request_id":"app-1","command":{"type":"invoke","payload":{"method":"app.getVersion"}}}"#;
        let version_channel = r#"{"version":1,"request_id":"app-2","command":{"type":"get-app-version","payload":{}}}"#;
        assert!(parse_bridge_message(invoke).is_err());
        assert!(parse_bridge_message(version_channel).is_err());
    }

    #[test]
    fn serializes_a_typed_app_version_response() {
        use super::AppVersionResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "app-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::AppVersion(AppVersionResponse {
                version: "1.3.0".to_owned(),
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "app_version");
        assert_eq!(value["payload"]["payload"]["version"], "1.3.0");
        // The response carries only the version string, never a
        // channel/key/path selector.
        assert!(value["payload"]["payload"].get("key").is_none());
        assert!(value["payload"]["payload"].get("path").is_none());
    }

    #[test]
    fn app_version_request_ids_reset_with_the_document() {
        let mut tracker = RequestTracker::default();
        assert!(tracker.begin("app-1").is_ok());
        // A duplicate within one document is rejected (stale/double-submit).
        assert!(tracker.begin("app-1").is_err());
        tracker.clear();
        assert!(tracker.begin("app-1").is_ok());
        assert!(tracker.complete("app-1").is_ok());
    }

    #[test]
    fn validates_app_version_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"app_version_probe","payload":{"ok":true,"version":"1.3.0","error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"app_version_probe","payload":{"ok":false,"version":"","error":"timeout"}}}"#;
        let oversized = format!(
            r#"{{"version":1,"event":{{"type":"app_version_probe","payload":{{"ok":true,"version":"{}","error":null}}}}}}"#,
            "1".repeat(65)
        );
        let inconsistent = r#"{"version":1,"event":{"type":"app_version_probe","payload":{"ok":true,"version":"1.3.0","error":"extra"}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(&oversized).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
    }

    #[test]
    fn validates_graphics_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"graphics_probe","payload":{"ok":true,"stage":"report","browser_status":"measured","system_status":"measured","device_count":2,"feature_count":12,"device_summary":"ANGLE (Intel)","feature_summary":"webgl=hardware_accelerated","webgl_status":"measured","webgl_summary":"ANGLE (Intel)","error":null}}}"#;
        let degraded = r#"{"version":1,"event":{"type":"graphics_probe","payload":{"ok":true,"stage":"report","browser_status":"unavailable","system_status":"unsupported","device_count":0,"feature_count":0,"device_summary":"","feature_summary":"","webgl_status":"unavailable","webgl_summary":"","error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"graphics_probe","payload":{"ok":false,"stage":"report","browser_status":"unavailable","system_status":"unavailable","device_count":0,"feature_count":0,"device_summary":"","feature_summary":"","webgl_status":"unavailable","webgl_summary":"","error":"timeout"}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(degraded).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        // Unknown statuses, wrong stages, over-cap counts, oversized
        // summaries, and ok/error inconsistency are all rejected.
        for hostile in [
            r#"{"version":1,"event":{"type":"graphics_probe","payload":{"ok":true,"stage":"report","browser_status":"turbo","system_status":"measured","device_count":0,"feature_count":0,"device_summary":"","feature_summary":"","webgl_status":"unavailable","webgl_summary":"","error":null}}}"#.to_owned(),
            r#"{"version":1,"event":{"type":"graphics_probe","payload":{"ok":true,"stage":"begin","browser_status":"measured","system_status":"measured","device_count":0,"feature_count":0,"device_summary":"","feature_summary":"","webgl_status":"unavailable","webgl_summary":"","error":null}}}"#.to_owned(),
            r#"{"version":1,"event":{"type":"graphics_probe","payload":{"ok":true,"stage":"report","browser_status":"measured","system_status":"measured","device_count":0,"feature_count":0,"device_summary":"","feature_summary":"","webgl_status":"turbo","webgl_summary":"","error":null}}}"#.to_owned(),
            format!(
                r#"{{"version":1,"event":{{"type":"graphics_probe","payload":{{"ok":true,"stage":"report","browser_status":"measured","system_status":"measured","device_count":{},"feature_count":0,"device_summary":"","feature_summary":"","webgl_status":"unavailable","webgl_summary":"","error":null}}}}}}"#,
                crate::graphics::MAX_GPU_DEVICES + 1
            ),
            format!(
                r#"{{"version":1,"event":{{"type":"graphics_probe","payload":{{"ok":true,"stage":"report","browser_status":"measured","system_status":"measured","device_count":0,"feature_count":0,"device_summary":"{}","feature_summary":"","webgl_status":"unavailable","webgl_summary":"","error":null}}}}}}"#,
                "d".repeat(super::MAX_GRAPHICS_PROBE_SUMMARY_BYTES + 1)
            ),
            r#"{"version":1,"event":{"type":"graphics_probe","payload":{"ok":true,"stage":"report","browser_status":"measured","system_status":"measured","device_count":0,"feature_count":0,"device_summary":"","feature_summary":"","webgl_status":"unavailable","webgl_summary":"","error":"extra"}}}"#.to_owned(),
            r#"{"version":1,"event":{"type":"graphics_probe","payload":{"ok":false,"stage":"report","browser_status":"measured","system_status":"measured","device_count":0,"feature_count":0,"device_summary":"","feature_summary":"","webgl_status":"unavailable","webgl_summary":"","error":null}}}"#.to_owned(),
        ] {
            assert!(
                parse_bridge_message(&hostile).is_err(),
                "hostile probe must be rejected: {hostile}"
            );
        }
    }

    #[test]
    fn accepts_scoped_plugin_storage_requests() {
        let read = r#"{"version":1,"request_id":"ps-1","command":{"type":"plugin_storage_read","payload":{"plugin_id":"my-plugin","relative_path":"cache/note.txt"}}}"#;
        let write = r#"{"version":1,"request_id":"ps-2","command":{"type":"plugin_storage_write","payload":{"plugin_id":"my-plugin","relative_path":"cache/note.txt","data":"hello"}}}"#;
        assert!(matches!(
            parse_bridge_message(read),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(matches!(
            parse_bridge_message(write),
            Ok(BridgeMessage::Request(_))
        ));
    }

    #[test]
    fn rejects_generic_filesystem_shapes_for_plugin_storage() {
        // No absolute path, URL, channel, key, or generic invoke surface.
        let absolute = r#"{"version":1,"request_id":"ps-1","command":{"type":"plugin_storage_read","payload":{"plugin_id":"p","relative_path":"C:\\secret"}}}"#;
        // Shape-level: absolute-looking input passes the parser byte check
        // (strict traversal rejection lives in `plugin_storage`); unknown
        // members must fail here.
        assert!(parse_bridge_message(absolute).is_ok());
        let with_path = r#"{"version":1,"request_id":"ps-2","command":{"type":"plugin_storage_read","payload":{"plugin_id":"p","relative_path":"a.txt","path":"C:\\secret"}}}"#;
        let with_url = r#"{"version":1,"request_id":"ps-3","command":{"type":"plugin_storage_write","payload":{"plugin_id":"p","relative_path":"a.txt","data":"x","url":"https://attacker.invalid"}}}"#;
        let invoke = r#"{"version":1,"request_id":"ps-4","command":{"type":"invoke","payload":{"method":"fs.readFile"}}}"#;
        let fs_read = r#"{"version":1,"request_id":"ps-5","command":{"type":"fs_read_file","payload":{"path":"C:\\secret"}}}"#;
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_url).is_err());
        assert!(parse_bridge_message(invoke).is_err());
        assert!(parse_bridge_message(fs_read).is_err());
    }

    #[test]
    fn rejects_malformed_plugin_storage_payloads() {
        let empty_id = r#"{"version":1,"request_id":"ps-1","command":{"type":"plugin_storage_read","payload":{"plugin_id":"","relative_path":"a.txt"}}}"#;
        let empty_path = r#"{"version":1,"request_id":"ps-2","command":{"type":"plugin_storage_read","payload":{"plugin_id":"p","relative_path":""}}}}"#;
        let missing_data = r#"{"version":1,"request_id":"ps-3","command":{"type":"plugin_storage_write","payload":{"plugin_id":"p","relative_path":"a.txt"}}}"#;
        let numeric_data = r#"{"version":1,"request_id":"ps-4","command":{"type":"plugin_storage_write","payload":{"plugin_id":"p","relative_path":"a.txt","data":42}}}"#;
        let oversized_id = format!(
            r#"{{"version":1,"request_id":"ps-5","command":{{"type":"plugin_storage_read","payload":{{"plugin_id":"{}","relative_path":"a.txt"}}}}}}"#,
            "p".repeat(65)
        );
        assert!(parse_bridge_message(empty_id).is_err());
        assert!(parse_bridge_message(empty_path).is_err());
        assert!(parse_bridge_message(missing_data).is_err());
        assert!(parse_bridge_message(numeric_data).is_err());
        assert!(parse_bridge_message(&oversized_id).is_err());
    }

    #[test]
    fn serializes_typed_plugin_storage_responses_without_paths() {
        use super::{PluginStorageReadResponse, PluginStorageWriteResponse};
        let read = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "ps-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::PluginStorageRead(
                PluginStorageReadResponse {
                    data: "hello".to_owned(),
                    payload_bytes: 5,
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(read).unwrap();
        assert_eq!(value["payload"]["type"], "plugin_storage_read");
        assert_eq!(value["payload"]["payload"]["data"], "hello");
        assert!(value["payload"]["payload"].get("path").is_none());

        let write = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "ps-2".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::PluginStorageWrite(
                PluginStorageWriteResponse { payload_bytes: 5 },
            )),
            error: None,
        };
        let value = serde_json::to_value(write).unwrap();
        assert_eq!(value["payload"]["type"], "plugin_storage_write");
        assert_eq!(value["payload"]["payload"]["payload_bytes"], 5);
        assert!(value["payload"]["payload"].get("data").is_none());
        assert!(value["payload"]["payload"].get("path").is_none());
    }

    #[test]
    fn validates_plugin_storage_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"plugin_storage_probe","payload":{"ok":true,"stage":"write_read","payload_bytes":32,"round_trip":true,"attacks_blocked":3,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"plugin_storage_probe","payload":{"ok":false,"stage":"write_read","payload_bytes":0,"round_trip":false,"attacks_blocked":0,"error":"timeout"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"plugin_storage_probe","payload":{"ok":true,"stage":"deleted","payload_bytes":1,"round_trip":true,"attacks_blocked":3,"error":null}}}"#;
        let inconsistent = r#"{"version":1,"event":{"type":"plugin_storage_probe","payload":{"ok":true,"stage":"write_read","payload_bytes":1,"round_trip":true,"attacks_blocked":3,"error":"extra"}}}"#;
        let missing_attacks = r#"{"version":1,"event":{"type":"plugin_storage_probe","payload":{"ok":true,"stage":"write_read","payload_bytes":1,"round_trip":true,"attacks_blocked":2,"error":null}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
        assert!(parse_bridge_message(missing_attacks).is_err());
    }
}

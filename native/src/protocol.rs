use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::arrpc::RpcBlacklistEntry;
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
    SettingsImportElectron(SettingsImportElectronRequest),
    LocalizationGetLang(LocalizationGetLangRequest),
    AppGetVersion(AppGetVersionRequest),
    PluginStorageRead(PluginStorageReadRequest),
    PluginStorageWrite(PluginStorageWriteRequest),
    PluginsList(PluginsListRequest),
    PluginsSetEnabled(PluginsSetEnabledRequest),
    PluginsReload(PluginsReloadRequest),
    PluginsGetRuntimeEntries(PluginsGetRuntimeEntriesRequest),
    PluginsGetRuntimeScript(PluginsGetRuntimeScriptRequest),
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
    ClipboardCopyImage(ClipboardCopyImageRequest),
    DiagnosticsOpenWebrtcInternals(DiagnosticsOpenWebrtcInternalsRequest),
    DiagnosticsOpenGpuPage(DiagnosticsOpenGpuPageRequest),
    ModClearCache(ModClearCacheRequest),
    SpellcheckGetState(SpellcheckGetStateRequest),
    BackupSave(BackupSaveRequest),
    BackupInspect(BackupInspectRequest),
    BackupCommitRestore(BackupCommitRestoreRequest),
    BackupDiscardRestore(BackupDiscardRestoreRequest),
    GalleryList(GalleryListRequest),
    KeybindsList(KeybindsListRequest),
    KeybindsAdd(KeybindsAddRequest),
    KeybindsToggle(KeybindsToggleRequest),
    KeybindsRemove(KeybindsRemoveRequest),
    MenusGetTemplate(MenusGetTemplateRequest),
    GalleryAddUrl(GalleryAddUrlRequest),
    GalleryAddLink(GalleryAddLinkRequest),
    GalleryRemove(GalleryRemoveRequest),
    GalleryBulkRemove(GalleryBulkRemoveRequest),
    GalleryUpdate(GalleryUpdateRequest),
    GalleryCategories(GalleryCategoriesRequest),
    GalleryCategoryAdd(GalleryCategoryAddRequest),
    GalleryCategoryRename(GalleryCategoryRenameRequest),
    GalleryCategoryRemove(GalleryCategoryRemoveRequest),
    GalleryUsage(GalleryUsageRequest),
    GalleryFile(GalleryFileRequest),
    GalleryRoot(GalleryRootRequest),
    RpcGetProcessList(RpcGetProcessListRequest),
    RpcRefreshProcessList(RpcRefreshProcessListRequest),
    RpcGetDetectables(RpcGetDetectablesRequest),
    RpcAddDetectable(RpcAddDetectableRequest),
    RpcRemoveDetectable(RpcRemoveDetectableRequest),
    RpcGetBlacklist(RpcGetBlacklistRequest),
    RpcBlacklistGame(RpcBlacklistGameRequest),
    RpcUnblacklistGame(RpcUnblacklistGameRequest),
    DownloadsList(DownloadsListRequest),
    DownloadsOpenFolder(DownloadsOpenFolderRequest),
    DownloadsReveal(DownloadsRevealRequest),
    WindowShow(WindowShowRequest),
    WindowHide(WindowHideRequest),
    WindowMinimize(WindowMinimizeRequest),
    WindowMaximize(WindowMaximizeRequest),
    WindowUnmaximize(WindowUnmaximizeRequest),
    WindowDrag(WindowDragRequest),
    WindowToggleFullscreen(WindowToggleFullscreenRequest),
    WindowQuit(WindowQuitRequest),
    WindowGetState(WindowGetStateRequest),
    TrayGetState(TrayGetStateRequest),
    BadgeGetState(BadgeGetStateRequest),
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

/// Empty request for the `settings.import_electron` capability: copies the
/// validated Electron-profile preferences into the native profile. There
/// is deliberately NO source path, URL, key list, or value parameter: the
/// host builds the fixed `%APPDATA%\Remcord\storage\settings.json` source
/// itself and applies exactly its own allowlist. Any extra field is
/// rejected, so a renderer cannot select, narrow, or redirect the import.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsImportElectronRequest {}

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
            SettingsSetKey::ScreenshareQuality => SettingsSetValue::ScreenshareQuality(
                serde_json::from_value(raw.value).map_err(serde::de::Error::custom)?,
            ),
            SettingsSetKey::ScanInterval => SettingsSetValue::ScanInterval(
                serde_json::from_value(raw.value).map_err(serde::de::Error::custom)?,
            ),
            SettingsSetKey::SupportBannerDismissed
            | SettingsSetKey::ShowExperimentalPluginMenu
            | SettingsSetKey::ExtendedPluginAbilities
            | SettingsSetKey::QuickCss
            | SettingsSetKey::InviteWebsocket
            | SettingsSetKey::ProcessScanning
            | SettingsSetKey::WindowsLegacyScanning => SettingsSetValue::Bool(
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

/// Empty request for the read-only `plugins.list` inventory capability.
/// Returns one display entry per valid manifest under the host-owned
/// plugins root, in sorted directory order. There is deliberately NO path,
/// root, selector, URL, or filter: the host owns enumeration, manifest
/// reads, enablement (from the authoritative settings), compatibility, and
/// bounds. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginsListRequest {}

/// Flip request for the `plugins.set_enabled` capability: exactly one
/// logical plugin id plus the new boolean state. There is deliberately NO
/// path, manifest, entry, URL, or filesystem selector: the host resolves
/// the id against fresh discovery and persists the flip into the
/// `pluginStates` settings map. Charset is re-checked at dispatch; unknown
/// ids resolve `{ok:false}` (never an error), and enabling an incompatible
/// plugin resolves `{ok:false}` like Electron.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginsSetEnabledRequest {
    pub plugin_id: String,
    pub enabled: bool,
}

/// Re-validation request for the `plugins.reload` capability: exactly one
/// logical plugin id. There is deliberately NO path, manifest, entry, or
/// selector: the host re-discovers and re-validates that plugin's manifest
/// and files, reporting whether it is currently servable. Unknown or
/// incompatible ids resolve `{ok:false}` like Electron. The renderer loader
/// picks the result up on the next document; no live in-place re-execution
/// is manufactured.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginsReloadRequest {
    pub plugin_id: String,
}

/// Scoped runtime-entries request for the `plugins.get_runtime_entries`
/// capability: exactly one closed target enum (`preload` or `renderer`).
/// There is deliberately NO plugin id, path, URL, or selector: the host
/// returns every enabled+compatible plugin's logical `{id, name}` for that
/// target (Electron's absolute `path` member is dropped — the loader
/// requests sources by id+target and the host re-resolves them). Unknown
/// targets are rejected during deserialization, before any work.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginsGetRuntimeEntriesRequest {
    pub target: crate::plugins::PluginTarget,
}

/// Scoped runtime-source request for the `plugins.get_runtime_script`
/// capability: exactly one logical plugin id plus the closed target enum.
/// There is deliberately NO path, URL, filename, or entry selector: the
/// host re-resolves the manifest entry under its own plugins root. An
/// undeclared, unresolvable, missing, or empty entry resolves
/// `{source:null}` (the loader skips falsy sources, mirroring Electron's
/// `if (!source) continue`); over-limit, non-UTF-8, or unreadable entries
/// are typed errors. Charset is re-checked at dispatch.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginsGetRuntimeScriptRequest {
    pub plugin_id: String,
    pub target: crate::plugins::PluginTarget,
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

/// Empty request for the read-only `gallery.list` capability: the host
/// owns the gallery root, the index read, and the newest-first ordering.
/// There is deliberately NO path, root, selector, URL, or filter. Any
/// extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryListRequest {}

/// Empty request for the read-only `keybinds.list` capability: the host
/// owns the keybind inventory, validation, and bounds. There is
/// deliberately NO path, selector, URL, or filter. Any extra field is
/// rejected. Entries carry metadata only (id/accelerator/action/flags) —
/// never code: `runJavascript` entries are skipped host-side and a `js`
/// member can never parse.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeybindsListRequest {}

/// Save request for the `keybinds.add` capability: exactly one id, one
/// accelerator, one action, and two flags. There is deliberately NO `js`
/// member (`deny_unknown_fields` refuses it — stored code can never
/// cross), NO path, URL, or second entry: the host validates the id
/// shape, parses the accelerator, and accepts only the eight executable
/// actions (`runJavascript` fails validation at dispatch).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeybindsAddRequest {
    pub id: String,
    pub accelerator: String,
    pub action: String,
    pub global: bool,
    pub enabled: bool,
}

/// Flip request for the `keybinds.toggle` capability: exactly one
/// validated id. The host flips the stored flag and re-registers.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeybindsToggleRequest {
    pub id: String,
}

/// Removal request for the `keybinds.remove` capability: exactly one
/// validated id. The host removes the entry and re-registers.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeybindsRemoveRequest {
    pub id: String,
}

/// Empty request for the read-only `menus.get_template` capability (F15):
/// the host owns the Electron-mirrored template and builds the Keybind
/// submenu from the validated keybind inventory. There is deliberately
/// NO selector, id, label, or locale parameter — labels resolve
/// renderer-side through `getLang`. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenusGetTemplateRequest {}

/// Renderer-supplied save metadata for gallery additions. Every member is
/// optional and bounded; unknown members are rejected. The host slices
/// each string exactly like Electron's `addGalleryUrl`/`addGalleryLink`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryMetaRequest {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub guild_id: String,
    #[serde(default)]
    pub guild_name: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub channel_name: String,
    #[serde(default)]
    pub author_name: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub categories: Vec<String>,
}

/// Save request for the `gallery.add_url` capability: exactly one URL plus
/// save metadata. There is deliberately NO path, filename, id, kind, or
/// destination: the host validates the scheme, derives the kind and file
/// name, generates the id, and downloads image/video bytes itself. A
/// non-http(s) URL is refused before any network or filesystem work.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryAddUrlRequest {
    pub url: String,
    pub meta: GalleryMetaRequest,
}

/// Save request for the `gallery.add_link` capability: exactly one URL
/// plus save metadata. Metadata-only (no download); same shape rules as
/// `gallery.add_url`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryAddLinkRequest {
    pub url: String,
    pub meta: GalleryMetaRequest,
}

/// Removal request for the `gallery.remove` capability: exactly one
/// validated item id. There is deliberately NO path or filename: the host
/// resolves the cached file from its own index and enforces containment.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryRemoveRequest {
    pub id: String,
}

/// Bulk removal request for the `gallery.bulk_remove` capability: item
/// ids only (at most 2000 are honored, mirroring Electron's slice).
/// Invalid ids are skipped, never dispatched.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryBulkRemoveRequest {
    pub ids: Vec<String>,
}

/// Update request for the `gallery.update` capability: exactly one item
/// id plus an optional title and/or category list. There is deliberately
/// NO kind, URL, file, size, mime, timestamp, or path member: only the
/// title and the tag list are mutable, mirroring Electron's patch shape.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryUpdateRequest {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub categories: Option<Vec<String>>,
}

/// Empty request for the read-only `gallery.categories` capability.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryCategoriesRequest {}

/// Creation request for the `gallery.category_add` capability: exactly
/// one name. Normalization and uniqueness rules live in `gallery`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryCategoryAddRequest {
    pub name: String,
}

/// Rename request for the `gallery.category_rename` capability: exactly
/// the old and new names. Items are retagged by the host.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryCategoryRenameRequest {
    pub old_name: String,
    pub new_name: String,
}

/// Removal request for the `gallery.category_remove` capability: exactly
/// one name. Items are untagged by the host.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryCategoryRemoveRequest {
    pub name: String,
}

/// Empty request for the read-only `gallery.usage` capability: exact disk
/// usage measured from the filesystem by the host.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryUsageRequest {}

/// Preview request for the `gallery.file` capability: exactly one
/// validated item id. The host resolves, bounds (48 MiB transport cap),
/// and base64-encodes the cached bytes itself.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryFileRequest {
    pub id: String,
}

/// Empty request for the read-only `gallery.root` capability: reports the
/// fixed profile-relative root label. There is deliberately NO path
/// member in either direction — an absolute path never crosses.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryRootRequest {}

/// Empty request for the read-only `rpc.get_process_list` capability: the
/// host enumerates running processes live (ToolHelp32 snapshot, sorted by
/// pid) and returns arRPC `ProcessInfo` tuples (`[pid, exe_path, args]`;
/// args are always `[]` — unavailable from a snapshot, never invented).
/// There is deliberately NO pid, path, filter, or selector: the inventory
/// is whole and bounded. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcGetProcessListRequest {}

/// Empty request for the `rpc.refresh_process_list` capability: same live
/// enumeration as the read, resolving `{refreshed, count}` so callers can
/// confirm the scan ran (Electron's fire-and-forget `void` cannot confirm).
/// Parameter-free by protocol, like the read.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcRefreshProcessListRequest {}

/// Empty request for the read-only `rpc.get_detectables` capability: the
/// host-owned custom detectables inventory. There is deliberately NO path,
/// selector, URL, or filter. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcGetDetectablesRequest {}

/// Save request for the `rpc.add_detectable` capability: exactly one game
/// object in the `AddDetectableModal` shape. There is deliberately NO path,
/// URL, id override, or destination: the host validates the closed shape
/// (`arrpc::validate_detectable`) and appends the canonical rebuilt value.
/// Deep shape rules live in `arrpc` and are re-enforced at dispatch.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcAddDetectableRequest {
    pub detectable: serde_json::Value,
}

/// Removal request for the `rpc.remove_detectable` capability: exactly one
/// custom detectable id (string match, mirroring `removeDetectable`). There
/// is deliberately NO path or index: the host filters its own store.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcRemoveDetectableRequest {
    pub id: String,
}

/// Empty request for the read-only `rpc.get_blacklist` capability. The
/// facade serves it synchronously from its document-start-seeded cache
/// (Electron's preload-cache parity); the host read behind it is the
/// tolerant `<profile>/storage/blacklist.json` load. Parameter-free by
/// protocol: no path, selector, or filter. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcGetBlacklistRequest {}

/// Save request for the `rpc.blacklist_game` capability: exactly one game
/// name plus its numeric app id. There is deliberately NO path, URL, or
/// second entry: the host normalizes, dedupes by id, persists, and returns
/// the new list (Electron's handler returns it for the cache + broadcast).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcBlacklistGameRequest {
    pub name: String,
    pub id: i64,
}

/// Removal request for the `rpc.unblacklist_game` capability: exactly one
/// numeric app id. The host filters its own store and returns the new list.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcUnblacklistGameRequest {
    pub id: i64,
}
/// Empty request for the read-only `downloads.list` manager capability:
/// newest-first inventory of the host-owned downloads dir. There is
/// deliberately NO path, root, selector, URL, or filter: the host owns
/// the root, enumeration, ordering, and bounds. Any extra field is
/// rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadsListRequest {}

/// Empty request for the `downloads.open_folder` manager capability:
/// reveals the fixed host-owned downloads root (same reveal semantics as
/// `folders.open`). There is deliberately NO path, URL, directory name,
/// or selector. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadsOpenFolderRequest {}

/// Reveal request for the `downloads.reveal` manager capability: exactly
/// one already-sanitized file NAME. There is deliberately NO path, URL,
/// id, or destination: the host re-sanitizes the name with byte-exact
/// equality, joins it onto its own downloads root, and enforces
/// canonical containment. Charset and length are re-checked at dispatch.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadsRevealRequest {
    pub name: String,
}

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

/// Copy request for the `clipboard.copy_image` capability (F17): exactly
/// one `http`/`https` image URL. The host downloads and decodes the bytes
/// itself; the renderer never supplies bytes, paths, dimensions, or
/// selectors. Shape-checked here (presence + byte cap + scheme); the
/// content rules (raster MIME, size, decodability) live in `clipboard` and
/// are re-enforced at dispatch. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipboardCopyImageRequest {
    pub url: String,
}
/// Empty request for the `diagnostics.open_webrtc_internals` capability.
/// Opens (or focuses the live singleton of) the host-owned WebRTC internals
/// diagnostics window (`crate::diagnostics_windows`). There are no
/// renderer-supplied parameters by design: no page id, URL, size, or flag —
/// the page is named by the wire type itself. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsOpenWebrtcInternalsRequest {}

/// Empty request for the `diagnostics.open_gpu_page` capability. Opens (or
/// focuses the live singleton of) the host-owned GPU diagnostics window.
/// Same no-parameter contract as the WebRTC request above.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsOpenGpuPageRequest {}

/// Empty request for the `mod.clear_cache` capability. Deletes the five
/// host-managed downloaded bundle caches and resets the stored `modCache`
/// map (`crate::mod_cache`). There are no renderer-supplied parameters by
/// design: no mod id, path, filename, or selector. Any extra field is
/// rejected, so user-owned `custom.js`/`custom.css` can never be selected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModClearCacheRequest {}

/// Empty request for the read-only `spellcheck.get_state` capability (F18).
/// Returns the validated spellcheck state from `crate::spellcheck`
/// (enabled flag, normalized languages, primary language). There are no
/// renderer-supplied parameters by design: no language, flag, or selector.
/// Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpellcheckGetStateRequest {}

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

/// Empty request for the `window.get_state` capability (F16): read the
/// persisted window geometry plus the live maximized flag. The renderer
/// supplies no parameter of any kind — geometry is host-owned. Any extra
/// field is rejected. There is deliberately NO `window.set_state` /
/// `window.move` / `window.resize`: arbitrary renderer-driven placement
/// is not an Electron surface.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowGetStateRequest {}
/// Empty request for the `tray.get_state` capability: the read-only tray
/// state snapshot from `crate::tray` (resolved mode, availability, the
/// caller's `minimizeToTray`, and the resulting close policy). There are
/// no renderer-supplied parameters by design: no mode, flag, title, badge,
/// path, handle, or asset selector. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrayGetStateRequest {}
/// Empty request for the read-only `badge.get_state` capability: reports the
/// host's last-applied overlay state (kind, mention count, asset, whether
/// the taskbar call succeeded). There is deliberately NO title, window id,
/// handle, or asset parameter: the state derives from `badge_title` page
/// reports the host parsed itself. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BadgeGetStateRequest {}

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
    /// Shipped plugin-loader execution telemetry (log-only, no response).
    PluginResult(PluginResult),
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
    /// Shipped one-way event posted by the badge title observer when
    /// `document.title` changes. Carries only the bounded title text: the
    /// host re-parses it (`crate::badge::parse_title`), applies the overlay
    /// itself, and never reflects the text anywhere. Over-long titles are
    /// rejected before any parse or Win32 work; the stored state is left
    /// untouched.
    BadgeTitle(BadgeTitle),
    /// Shipped one-way event posted by the keybind matcher script when a
    /// `keydown` matches an enabled non-global keybind. Carries only the
    /// entry id: the host re-validates (known, enabled, non-global) and
    /// executes the stored action itself. No accelerator, action, code,
    /// path, or URL can cross this event.
    KeybindMatched(KeybindMatched),
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
pub struct BadgeTitle {
    pub title: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeybindMatched {
    pub id: String,
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

/// Shipped one-way execution report posted by the native plugin loader
/// (`native/web/plugins/loader.js`) after each plugin entry runs. Fire-and-
/// forget telemetry, not a gate: the host logs it (verifying the id against
/// fresh discovery so logs stay trustworthy) and sends no response. Carries
/// only the logical id, the closed target, the verdict, and a bounded
/// failure message — never source, path, or URL.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginResult {
    pub plugin_id: String,
    pub target: crate::plugins::PluginTarget,
    pub ok: bool,
    pub error: Option<String>,
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
    SettingsImported(SettingsImportedResponse),
    LocalizationString(LocalizationStringResponse),
    AppVersion(AppVersionResponse),
    PluginStorageRead(PluginStorageReadResponse),
    PluginStorageWrite(PluginStorageWriteResponse),
    PluginsList(PluginsListResponse),
    PluginsSetEnabled(PluginsSetEnabledResponse),
    PluginsReload(PluginsReloadResponse),
    PluginsRuntimeEntries(PluginsRuntimeEntriesResponse),
    PluginsRuntimeScript(PluginsRuntimeScriptResponse),
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
    ClipboardImageCopied(ClipboardImageCopiedResponse),
    ModCacheCleared(ModCacheClearedResponse),
    SpellcheckState(SpellcheckStateResponse),
    BackupSaved(BackupSavedResponse),
    BackupInspected(BackupInspectedResponse),
    BackupRestored(BackupRestoredResponse),
    BackupRestoreDiscarded(BackupRestoreDiscardedResponse),
    GalleryList(GalleryListResponse),
    GalleryAdd(GalleryAddResponse),
    GalleryRemove(GalleryRemoveResponse),
    GalleryBulkRemove(GalleryBulkRemoveResponse),
    GalleryUpdate(GalleryUpdateResponse),
    GalleryCategories(GalleryCategoriesResponse),
    GalleryCategoryAdd(GalleryCategoryAddResponse),
    GalleryCategoryRename(GalleryCategoryRenameResponse),
    GalleryCategoryRemove(GalleryCategoryRemoveResponse),
    GalleryUsage(GalleryUsageResponse),
    GalleryFile(GalleryFileResponse),
    GalleryRoot(GalleryRootResponse),
    DownloadsList(DownloadsListResponse),
    DownloadsFolderOpened(DownloadsFolderOpenedResponse),
    DownloadsRevealed(DownloadsRevealedResponse),
    KeybindsList(KeybindsListResponse),
    KeybindsAdd(KeybindsAddResponse),
    KeybindsToggle(KeybindsToggleResponse),
    KeybindsRemove(KeybindsRemoveResponse),
    RpcProcessList(RpcProcessListResponse),
    RpcProcessRefresh(RpcProcessRefreshResponse),
    RpcDetectables(RpcDetectablesResponse),
    RpcDetectableAdd(RpcDetectableAddResponse),
    RpcDetectableRemove(RpcDetectableRemoveResponse),
    RpcBlacklist(RpcBlacklistResponse),
    MenusTemplate(MenusTemplateResponse),
    BadgeState(BadgeStateResponse),
    DiagnosticsWindowOpened(DiagnosticsWindowOpenedResponse),
    WindowAck(WindowAckResponse),
    WindowState(WindowStateResponse),
    TrayState(TrayGetStateResponse),
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

/// Validated response for the copy-only `settings.import_electron`
/// capability: which allowlisted keys were applied and which were
/// present-but-invalid (skipped). Key names only — never values, never
/// paths: the UI re-reads display state through `settings.get_config`.
#[derive(Serialize)]
pub struct SettingsImportedResponse {
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
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

/// Read-only inventory response for the `plugins.list` capability: one
/// [`crate::plugins::PluginInfo`] per valid manifest, in sorted directory
/// order. Display metadata only — no paths, URLs, sources, or filesystem
/// error text.
#[derive(Debug, Serialize)]
pub struct PluginsListResponse {
    pub plugins: Vec<crate::plugins::PluginInfo>,
}

/// Flip response for the `plugins.set_enabled` capability. Electron parity:
/// `{ok}` only — `true` carries the validated id echo, the persisted state
/// (which equals the request, including the idempotent same-state no-op),
/// the persistence source (always `"file"` after a successful write), and
/// the persisted file size; `false` (unknown id, or enabling an
/// incompatible plugin) carries no echo because nothing was applied. No
/// path crosses the boundary.
#[derive(Debug, Serialize)]
pub struct PluginsSetEnabledResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_bytes: Option<usize>,
}

/// Re-validation response for the `plugins.reload` capability. Same `{ok}`
/// contract as the enable flip: `true` echoes the re-validated id, `false`
/// (unknown or incompatible id) echoes nothing.
#[derive(Debug, Serialize)]
pub struct PluginsReloadResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
}

/// Scoped runtime-entries response for the `plugins.get_runtime_entries`
/// capability: the closed target echo plus every enabled+compatible
/// plugin's logical `{id, name}` for that target. Electron's absolute
/// `path` member is deliberately dropped — the loader requests sources by
/// id+target and the host re-resolves them.
#[derive(Debug, Serialize)]
pub struct PluginsRuntimeEntriesResponse {
    pub target: crate::plugins::PluginTarget,
    pub entries: Vec<crate::plugins::PluginRuntimeEntry>,
}

/// Scoped runtime-source response for the `plugins.get_runtime_script`
/// capability: the id+target echo plus the bounded entry source, or
/// `source: null` when the entry is undeclared, unresolvable, missing, or
/// empty (the loader skips falsy sources, mirroring Electron's
/// `if (!source) continue`). No path crosses the boundary.
#[derive(Debug, Serialize)]
pub struct PluginsRuntimeScriptResponse {
    pub plugin_id: String,
    pub target: crate::plugins::PluginTarget,
    pub source: Option<String>,
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

/// Read-only inventory response for the `gallery.list` capability:
/// items newest-first plus categories, in Electron's `{items,
/// categories}` shape. Items carry the camelCase display keys the
/// Gallery page casts; `localFile` values are profile-relative
/// `files/<name>` references, never absolute paths.
#[derive(Serialize)]
pub struct GalleryListResponse {
    pub items: Vec<serde_json::Value>,
    pub categories: Vec<serde_json::Value>,
}

/// Read-only inventory response for the `keybinds.list` capability:
/// validated entries with metadata only (id/accelerator/action/flags).
/// `runJavascript` entries are skipped host-side; no `js` member, path,
/// URL, or code ever crosses.
#[derive(Serialize)]
pub struct KeybindsListResponse {
    pub keybinds: Vec<serde_json::Value>,
}

/// Save response for the `keybinds.add` capability, in Electron's
/// fire-and-forget spirit but with a typed echo: `{ok, id} | {ok,
/// error}`. Failures resolve (never reject) so the maker can toast.
#[derive(Serialize)]
pub struct KeybindsAddResponse {
    pub ok: bool,
    pub id: Option<String>,
    pub error: Option<String>,
}

/// Flip response for the `keybinds.toggle` capability: whether an entry
/// was flipped plus its new state. Unknown ids resolve `{ok: false}`.
#[derive(Serialize)]
pub struct KeybindsToggleResponse {
    pub ok: bool,
    pub enabled: Option<bool>,
}

/// Removal response for the `keybinds.remove` capability: whether an
/// entry was removed. Unknown ids resolve `{ok: false}`.
#[derive(Serialize)]
pub struct KeybindsRemoveResponse {
    pub ok: bool,
}

/// Read-only inventory response for the `rpc.get_process_list`
/// capability: live-enumerated arRPC `ProcessInfo` tuples
/// (`[pid, exe_path, args]` with args always `[]`). Pids and exe names
/// only — no command lines, window titles, or usernames.
#[derive(Serialize)]
pub struct RpcProcessListResponse {
    pub processes: Vec<serde_json::Value>,
}

/// Refresh response for the `rpc.refresh_process_list` capability: whether
/// the live scan ran plus the enumerated entry count.
#[derive(Serialize)]
pub struct RpcProcessRefreshResponse {
    pub refreshed: bool,
    pub count: usize,
}

/// Read-only inventory response for the `rpc.get_detectables` capability:
/// validated custom detectable game objects in stored order.
#[derive(Serialize)]
pub struct RpcDetectablesResponse {
    pub detectables: Vec<serde_json::Value>,
}

/// Save response for the `rpc.add_detectable` capability: whether the
/// validated entry was stored. Failures resolve `{ok:false, error}` (never
/// reject) so the UI can toast the reason.
#[derive(Serialize)]
pub struct RpcDetectableAddResponse {
    pub ok: bool,
    pub error: Option<String>,
}

/// Removal response for the `rpc.remove_detectable` capability: whether an
/// entry was removed. Unknown ids resolve `{ok: false}`.
#[derive(Serialize)]
pub struct RpcDetectableRemoveResponse {
    pub ok: bool,
}

/// Blacklist response for `rpc.get_blacklist` / `rpc.blacklist_game` /
/// `rpc.unblacklist_game`: the full validated list (Electron's handlers
/// return the list for the cache update + broadcast). Mutations converge
/// every document through the `remcord-native:rpc-blacklist-updated` host
/// event as well.
#[derive(Serialize)]
pub struct RpcBlacklistResponse {
    pub entries: Vec<RpcBlacklistEntry>,
}
/// Template response for the `menus.get_template` capability (F15): the
/// four static Electron-mirrored menus, the dynamic Keybind submenu
/// (enabled + non-global entries only, `{kind, id, labelId,
/// accelerator}`), and the fixed context-menu state (`"suppressed"` —
/// Electron parity). Labels are ids resolved renderer-side through
/// `getLang`; no localized string, path, URL, or code ever crosses.
#[derive(Serialize)]
pub struct MenusTemplateResponse {
    pub menus: Vec<serde_json::Value>,
    pub keybind_menu: Vec<serde_json::Value>,
    pub context_menu: String,
}
/// Read-only overlay-state response for the `badge.get_state` capability:
/// the parsed kind, the mention count (only for `mention`), the applied
/// asset file (only when a badge shows), and whether the last taskbar
/// overlay call reported success. No title text, no handle, no path.
#[derive(Serialize)]
pub struct BadgeStateResponse {
    pub kind: crate::badge::BadgeKind,
    pub pings: Option<u32>,
    pub asset: Option<&'static str>,
    pub overlay_applied: bool,
}

/// Save response for the `gallery.add_url` / `gallery.add_link`
/// capabilities, in Electron's `{ok, item} | {ok, error}` shape. Typed
/// failures resolve (never reject) so the page can toast the reason.
#[derive(Serialize)]
pub struct GalleryAddResponse {
    pub ok: bool,
    pub item: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Removal response for the `gallery.remove` capability: whether an item
/// was removed. Unknown ids resolve `ok: false` (Electron parity).
#[derive(Serialize)]
pub struct GalleryRemoveResponse {
    pub ok: bool,
}

/// Bulk removal response for the `gallery.bulk_remove` capability: how
/// many items were removed.
#[derive(Serialize)]
pub struct GalleryBulkRemoveResponse {
    pub removed: usize,
}

/// Update response for the `gallery.update` capability, in Electron's
/// `{ok, item} | {ok, error}` shape.
#[derive(Serialize)]
pub struct GalleryUpdateResponse {
    pub ok: bool,
    pub item: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Read-only inventory response for the `gallery.categories`
/// capability: categories alphabetically, display keys only.
#[derive(Serialize)]
pub struct GalleryCategoriesResponse {
    pub categories: Vec<serde_json::Value>,
}

/// Creation response for the `gallery.category_add` capability, in
/// Electron's `{ok, category} | {ok, error}` shape.
#[derive(Serialize)]
pub struct GalleryCategoryAddResponse {
    pub ok: bool,
    pub category: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Rename response for the `gallery.category_rename` capability, in
/// Electron's `{ok} | {ok, error}` shape.
#[derive(Serialize)]
pub struct GalleryCategoryRenameResponse {
    pub ok: bool,
    pub error: Option<String>,
}

/// Removal response for the `gallery.category_remove` capability:
/// whether a category was removed.
#[derive(Serialize)]
pub struct GalleryCategoryRemoveResponse {
    pub ok: bool,
}

/// Read-only usage response for the `gallery.usage` capability: exact
/// bytes, file count, and item count measured from the filesystem.
#[derive(Serialize)]
pub struct GalleryUsageResponse {
    pub bytes: u64,
    pub files: u64,
    pub items: usize,
}

/// Preview response for the `gallery.file` capability, in Electron's
/// `{ok, base64, mime, fileName} | {ok, error}` shape. The base64 text
/// is bounded by the 48 MiB transport cap enforced at dispatch.
#[derive(Serialize)]
pub struct GalleryFileResponse {
    pub ok: bool,
    pub base64: Option<String>,
    pub mime: Option<String>,
    pub file_name: Option<String>,
    pub error: Option<String>,
}

/// Read-only root response for the `gallery.root` capability: the fixed
/// profile-relative root label. Never an absolute path.
#[derive(Serialize)]
pub struct GalleryRootResponse {
    pub root: String,
}

/// Read-only inventory response for the `downloads.list` manager
/// capability: display metadata only (`name`, `bytes`, `modified_ms`),
/// newest-first. No paths, no URLs, no contents, no filesystem error
/// text.
#[derive(Serialize)]
pub struct DownloadsListResponse {
    pub files: Vec<crate::download_manager::DownloadEntry>,
}

/// Reveal response for the `downloads.open_folder` manager capability:
/// the fixed location-label echo, so the renderer can confirm which root
/// was revealed. No path crosses the boundary.
#[derive(Serialize)]
pub struct DownloadsFolderOpenedResponse {
    pub location: &'static str,
}

/// Reveal response for the `downloads.reveal` manager capability: the
/// validated name echo, so one file's reveal can never resolve another
/// file's call. No path crosses the boundary.
#[derive(Serialize)]
pub struct DownloadsRevealedResponse {
    pub name: String,
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

/// Copy outcome for the `clipboard.copy_image` capability (F17), in
/// Electron's `{ok, item} | {ok, error}` spirit with a typed echo: `{ok,
/// width, height} | {ok, error}`. Typed failures resolve (never reject)
/// so the page can toast the reason; `error` carries only the
/// fixed-vocabulary code. Dimensions echo the placed bitmap.
#[derive(Serialize)]
pub struct ClipboardImageCopiedResponse {
    pub ok: bool,
    pub width: u32,
    pub height: u32,
    pub error: Option<String>,
}

/// Outcome response for the `mod.clear_cache` capability: which
/// host-managed cache files were deleted plus whether a stored `modCache`
/// map existed and was reset. Fixed allowlisted names only — no paths.
#[derive(Serialize)]
pub struct ModCacheClearedResponse {
    pub cleared: Vec<String>,
    pub mod_cache_cleared: bool,
}

/// Validated state response for the `spellcheck.get_state` capability
/// (F18): the closed enabled flag, the normalized language list, and the
/// primary language. Fixed-vocabulary tags only — no paths, no raw
/// settings text, no renderer echo.
#[derive(Serialize)]
pub struct SpellcheckStateResponse {
    pub enabled: bool,
    pub languages: Vec<String>,
    pub primary_language: String,
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

/// Persisted window geometry for the `window.get_state` capability (F16).
/// Snake-case bridge shape (every other response payload is snake-case);
/// the on-disk `<profile>/storage/window.json` keeps Electron's camelCase
/// keys instead (see `window_state`). `width`/`height` are clamped to
/// `400x300..=16384`, origins to `±32768`; `display_id` is the
/// best-effort `available_monitors()` index hint (or absent when the
/// capture knew no monitor). No handle, path, or URL crosses here.
#[derive(Serialize)]
pub struct WindowStateResponse {
    pub width: i64,
    pub height: i64,
    pub x: i64,
    pub y: i64,
    pub is_maximized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_id: Option<i64>,
}
/// Read-only tray state response for the `tray.get_state` capability:
/// the canonical mode name (`disabled` / `dynamic` / `static` — never the
/// raw setting string), availability, the authoritative `minimizeToTray`,
/// and the resulting close policy. No title/badge text, no paths, no
/// handles, no asset bytes cross the boundary.
#[derive(Serialize)]
pub struct TrayGetStateResponse {
    pub mode: String,
    pub available: bool,
    pub minimize_to_tray: bool,
    pub should_hide: bool,
/// Acknowledgement for opening (or focusing) one diagnostics window
/// (F25: WebRTC internals / GPU page). `page` echoes the closed page id
/// (`webrtc-internals` / `gpu`) so one page's acknowledgement can never
/// resolve the other page's call; `opened` vs `focused_existing` reports
/// which singleton path was taken. No handle, URL, or geometry crosses.
#[derive(Serialize)]
pub struct DiagnosticsWindowOpenedResponse {
    pub page: &'static str,
    pub opened: bool,
    pub focused_existing: bool,
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

/// Parse-time shape check for gallery save envelopes: presence plus
/// byte caps that keep the envelope under the 8 KiB transport bound (the
/// host slices strings Electron-style at dispatch, so anything up to
/// these caps shrinks at dispatch instead of failing). Scheme, dedupe,
/// kind, and containment rules live in `gallery`.
fn gallery_save_shape_valid(url: &str, meta: &GalleryMetaRequest) -> bool {
    !(url.is_empty()
        || url.len() > crate::gallery::MAX_URL_BYTES
        || meta.title.len() > 1024
        || meta.guild_id.len() > 256
        || meta.guild_name.len() > 256
        || meta.channel_id.len() > 256
        || meta.channel_name.len() > 256
        || meta.author_name.len() > 256
        || meta.message_id.len() > 256
        || meta.categories.len() > 16
        || meta.categories.iter().any(|c| c.len() > 128))
}

/// Parse-time shape check for gallery item ids: presence plus byte cap.
/// Charset rules live in `gallery` and are re-enforced at dispatch.
fn gallery_id_shape_valid(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64
}

/// Parse-time shape check for gallery category names: presence plus byte
/// cap. Normalization and uniqueness live in `gallery`.
fn gallery_name_shape_valid(name: &str) -> bool {
    !name.is_empty() && name.len() <= 256
}

/// Parse-time shape check for keybind ids: presence plus byte cap.
/// Charset rules live in `keybinds` and are re-enforced at dispatch.
fn keybind_id_shape_valid(id: &str) -> bool {
    !id.is_empty() && id.len() <= crate::keybinds::MAX_KEYBIND_ID_BYTES
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
                // Parameter-free by protocol (`deny_unknown_fields` already
                // rejected any renderer-supplied member), so no shape check
                // is needed. The source path and the key allowlist live in
                // `settings_import` and are enforced at dispatch.
                BridgeCommand::SettingsImportElectron(_) => {}
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
                // is needed. Enumeration, manifest reads, enablement, and
                // bounds live in `plugins` and are enforced at dispatch.
                BridgeCommand::PluginsList(_) => {}
                BridgeCommand::PluginsSetEnabled(payload) => {
                    // Shape check only (id presence/cap; the boolean is
                    // typed by serde); id charset, discovery membership,
                    // and compatibility live in `plugins` (plus the
                    // `pluginStates` settings owner) and are re-enforced at
                    // dispatch.
                    if payload.plugin_id.is_empty()
                        || payload.plugin_id.len() > crate::plugins::MAX_PLUGIN_ID_BYTES
                    {
                        return Err("invalid plugin enable flip".to_owned());
                    }
                }
                BridgeCommand::PluginsReload(payload) => {
                    // Shape check only (id presence/cap); membership and
                    // compatibility are re-enforced at dispatch.
                    if payload.plugin_id.is_empty()
                        || payload.plugin_id.len() > crate::plugins::MAX_PLUGIN_ID_BYTES
                    {
                        return Err("invalid plugin reload".to_owned());
                    }
                }
                // The target is a closed enum validated by serde itself:
                // unknown variants (including `"main"`) never reach
                // dispatch. There is no id, path, or selector to
                // shape-check.
                BridgeCommand::PluginsGetRuntimeEntries(_) => {}
                BridgeCommand::PluginsGetRuntimeScript(payload) => {
                    // Shape check only (id presence/cap); id charset,
                    // membership, enablement, and the entry jail live in
                    // `plugins` and are re-enforced at dispatch.
                    if payload.plugin_id.is_empty()
                        || payload.plugin_id.len() > crate::plugins::MAX_PLUGIN_ID_BYTES
                    {
                        return Err("invalid plugin source request".to_owned());
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
                | BridgeCommand::ModClearCache(_)
                | BridgeCommand::TrayGetState(_)
                | BridgeCommand::SpellcheckGetState(_) => {}
                // Clipboard image copy (F17): shape check only (presence +
                // byte cap + http(s) scheme); the content rules (raster
                // MIME, byte/pixel caps, WIC decodability) live in
                // `clipboard` and are re-enforced at dispatch.
                BridgeCommand::ClipboardCopyImage(payload) => {
                    if payload.url.trim().is_empty()
                        || payload.url.len() > crate::clipboard::MAX_CLIPBOARD_IMAGE_URL_BYTES
                    {
                        return Err("invalid clipboard image url".to_owned());
                    }
                    let lower = payload.url.to_ascii_lowercase();
                    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                        return Err("invalid clipboard image url".to_owned());
                    }
                }
                | BridgeCommand::DiagnosticsOpenWebrtcInternals(_)
                | BridgeCommand::DiagnosticsOpenGpuPage(_)
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
                | BridgeCommand::WindowQuit(_)
                | BridgeCommand::WindowGetState(_) => {}
                BridgeCommand::WindowQuit(_) => {}
                // Badge state is parameter-free by protocol
                // (`deny_unknown_fields` already rejected any
                // renderer-supplied member: no title, window id, handle,
                // or asset can cross). The reported state derives from
                // `badge_title` page reports parsed host-side.
                BridgeCommand::BadgeGetState(_) => {}
                // Gallery: shape check only (presence + byte caps); URL
                // schemes, id charset, name rules, category bounds, and
                // containment live in `gallery` and are re-enforced at
                // dispatch. Meta strings are sliced host-side exactly like
                // Electron, so oversized members shrink instead of failing.
                BridgeCommand::GalleryList(_)
                | BridgeCommand::GalleryCategories(_)
                | BridgeCommand::GalleryUsage(_)
                | BridgeCommand::GalleryRoot(_)
                | BridgeCommand::DownloadsList(_)
                | BridgeCommand::DownloadsOpenFolder(_) => {}
                // Keybinds: shape check only (presence + byte caps); the
                // accelerator grammar, id charset, action allowlist, and
                // inventory bounds live in `keybinds` and are re-enforced
                // at dispatch. The `js` member can never parse
                // (`deny_unknown_fields`); `runJavascript` fails the
                // action check at dispatch.
                BridgeCommand::KeybindsList(_) => {}
                // Menus (F15): parameter-free by protocol
                // (`deny_unknown_fields` already rejected any
                // renderer-supplied member), so no shape check is needed.
                // The static template lives in `menus`; the Keybind
                // submenu is built from the `keybinds` inventory at
                // dispatch. The context menu stays suppressed (Electron
                // parity) — no kind, entry, or action crosses.
                BridgeCommand::MenusGetTemplate(_) => {}
                BridgeCommand::KeybindsAdd(payload) => {
                    if payload.id.is_empty()
                        || payload.id.len() > crate::keybinds::MAX_KEYBIND_ID_BYTES
                        || payload.accelerator.is_empty()
                        || payload.accelerator.len() > crate::keybinds::MAX_ACCELERATOR_BYTES
                        || payload.action.is_empty()
                        || payload.action.len() > 32
                    {
                        return Err("invalid keybind save".to_owned());
                    }
                }
                BridgeCommand::KeybindsToggle(payload) => {
                    if !keybind_id_shape_valid(&payload.id) {
                        return Err("invalid keybind id".to_owned());
                    }
                }
                BridgeCommand::KeybindsRemove(payload) => {
                    if !keybind_id_shape_valid(&payload.id) {
                        return Err("invalid keybind id".to_owned());
                    }
                }
                // arRPC: shape check only (presence + byte caps); id
                // charsets, the detectable game shape, blacklist
                // normalization, and inventory bounds live in `arrpc` and
                // are re-enforced at dispatch. Reads are parameter-free by
                // protocol (`deny_unknown_fields` already rejected any
                // renderer-supplied member).
                BridgeCommand::RpcGetProcessList(_)
                | BridgeCommand::RpcRefreshProcessList(_)
                | BridgeCommand::RpcGetDetectables(_)
                | BridgeCommand::RpcGetBlacklist(_) => {}
                BridgeCommand::RpcAddDetectable(payload) => {
                    // The game object is deep-validated in `arrpc` at
                    // dispatch; the transport bound is the only parse-time
                    // gate so hostile multi-megabyte objects fail early.
                    let serialized = serde_json::to_string(&payload.detectable)
                        .map_err(|_| "invalid detectable game".to_owned())?;
                    if serialized.len() > 16 * 1024 {
                        return Err("invalid detectable game".to_owned());
                    }
                }
                BridgeCommand::RpcRemoveDetectable(payload) => {
                    if payload.id.is_empty()
                        || payload.id.len() > crate::arrpc::MAX_DETECTABLE_ID_BYTES
                    {
                        return Err("invalid detectable id".to_owned());
                    }
                }
                BridgeCommand::RpcBlacklistGame(payload) => {
                    if payload.name.is_empty()
                        || payload.name.len() > crate::arrpc::MAX_BLACKLIST_NAME_BYTES
                    {
                        return Err("invalid blacklist entry".to_owned());
                    }
                }
                BridgeCommand::RpcUnblacklistGame(_) => {}
                BridgeCommand::GalleryAddUrl(payload) => {
                    if !gallery_save_shape_valid(&payload.url, &payload.meta) {
                        return Err("invalid gallery save".to_owned());
                    }
                }
                BridgeCommand::GalleryAddLink(payload) => {
                    if !gallery_save_shape_valid(&payload.url, &payload.meta) {
                        return Err("invalid gallery save".to_owned());
                    }
                }
                BridgeCommand::GalleryRemove(payload) => {
                    if !gallery_id_shape_valid(&payload.id) {
                        return Err("invalid gallery id".to_owned());
                    }
                }
                BridgeCommand::GalleryFile(payload) => {
                    if !gallery_id_shape_valid(&payload.id) {
                        return Err("invalid gallery id".to_owned());
                    }
                }
                BridgeCommand::DownloadsReveal(payload) => {
                    // Shape check only (presence + byte cap); the
                    // byte-exact sanitize round trip, existence, and
                    // containment rules live in `download_manager` and are
                    // re-enforced at dispatch.
                    if payload.name.is_empty()
                        || payload.name.len() > crate::download_manager::MAX_DOWNLOAD_NAME_BYTES
                    {
                        return Err("invalid download name".to_owned());
                    }
                }
                BridgeCommand::GalleryBulkRemove(payload) => {
                    if payload.ids.is_empty() || payload.ids.len() > 4096 {
                        return Err("invalid gallery bulk removal".to_owned());
                    }
                }
                BridgeCommand::GalleryUpdate(payload) => {
                    if payload.id.is_empty()
                        || payload.id.len() > 64
                        || payload.title.as_ref().is_some_and(|t| t.len() > 4096)
                        || payload.categories.as_ref().is_some_and(|cats| {
                            cats.len() > 64 || cats.iter().any(|c| c.len() > 256)
                        })
                    {
                        return Err("invalid gallery update".to_owned());
                    }
                }
                BridgeCommand::GalleryCategoryAdd(payload) => {
                    if !gallery_name_shape_valid(&payload.name) {
                        return Err("invalid gallery category name".to_owned());
                    }
                }
                BridgeCommand::GalleryCategoryRemove(payload) => {
                    if !gallery_name_shape_valid(&payload.name) {
                        return Err("invalid gallery category name".to_owned());
                    }
                }
                BridgeCommand::GalleryCategoryRename(payload) => {
                    if payload.old_name.is_empty()
                        || payload.old_name.len() > 256
                        || payload.new_name.is_empty()
                        || payload.new_name.len() > 256
                    {
                        return Err("invalid gallery category rename".to_owned());
                    }
                }
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
                BridgeEvent::PluginResult(payload) => {
                    // Shipped loader telemetry: the id shape is checked
                    // here (discovery membership is verified at dispatch,
                    // where a fresh scan is cheap); the target is a closed
                    // enum validated by serde; ok/error must agree like
                    // every other verdict event.
                    if crate::plugins::validate_plugin_id(&payload.plugin_id).is_err() {
                        return Err("invalid plugin id in plugin result".to_owned());
                    }
                    if payload
                        .error
                        .as_ref()
                        .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid plugin result".to_owned());
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
                BridgeEvent::KeybindMatched(payload) => {
                    // Shape check only (presence + byte cap); id charset,
                    // membership, enablement, and scope live in `keybinds`
                    // and are re-enforced at handling.
                    if !keybind_id_shape_valid(&payload.id) {
                        return Err("invalid keybind id".to_owned());
                    }
                }
                BridgeEvent::BadgeTitle(payload) => {
                    // Length only: the character bound lives in `badge`
                    // (`MAX_BADGE_TITLE_CHARS`) and is re-checked at
                    // handling; the title text is parsed host-side and
                    // never reflected anywhere.
                    if payload.title.chars().count() > crate::badge::MAX_BADGE_TITLE_CHARS {
                        return Err("invalid badge title".to_owned());
                    }
                }
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
    fn accepts_plugin_commands_and_rejects_source_selectors() {
        // All five plugin commands parse with their exact shapes; the
        // closed target enum accepts only preload/renderer (`main` has no
        // native runtime and fails closed at parse time).
        for message in [
            r#"{"version":1,"request_id":"p-1","command":{"type":"plugins_list","payload":{}}}"#,
            r#"{"version":1,"request_id":"p-2","command":{"type":"plugins_set_enabled","payload":{"plugin_id":"demo","enabled":true}}}"#,
            r#"{"version":1,"request_id":"p-3","command":{"type":"plugins_reload","payload":{"plugin_id":"demo"}}}"#,
            r#"{"version":1,"request_id":"p-4","command":{"type":"plugins_get_runtime_entries","payload":{"target":"preload"}}}"#,
            r#"{"version":1,"request_id":"p-5","command":{"type":"plugins_get_runtime_script","payload":{"plugin_id":"demo","target":"renderer"}}}"#,
        ] {
            assert!(
                matches!(parse_bridge_message(message), Ok(BridgeMessage::Request(_))),
                "must parse {message}"
            );
        }
        // A path, URL, filename, manifest, entry, or source member is never
        // a valid member: it cannot select what the host reads or serves.
        for hostile in [
            r#"{"version":1,"request_id":"p-6","command":{"type":"plugins_set_enabled","payload":{"plugin_id":"demo","enabled":true,"path":"C:\\secret"}}}"#,
            r#"{"version":1,"request_id":"p-7","command":{"type":"plugins_reload","payload":{"plugin_id":"demo","url":"https://attacker.invalid/x.js"}}}"#,
            r#"{"version":1,"request_id":"p-8","command":{"type":"plugins_get_runtime_entries","payload":{"target":"preload","plugin_id":"demo"}}}"#,
            r#"{"version":1,"request_id":"p-9","command":{"type":"plugins_get_runtime_script","payload":{"plugin_id":"demo","target":"renderer","source":"x=1"}}}"#,
            r#"{"version":1,"request_id":"p-10","command":{"type":"plugins_get_runtime_entries","payload":{"target":"main"}}}"#,
            r#"{"version":1,"request_id":"p-11","command":{"type":"plugins_get_runtime_entries","payload":{"target":"PRELOAD"}}}"#,
            r#"{"version":1,"request_id":"p-12","command":{"type":"plugins_get_runtime_script","payload":{"plugin_id":"demo"}}}"#,
            r#"{"version":1,"request_id":"p-13","command":{"type":"plugins_list","payload":{"filter":"demo"}}}"#,
        ] {
            assert!(
                parse_bridge_message(hostile).is_err(),
                "must reject {hostile}"
            );
        }
    }

    #[test]
    fn plugin_result_events_require_agreeing_verdicts() {
        // ok:true carries no error; ok:false carries a bounded message.
        // Anything else (including a `main` target) is rejected at parse.
        let ok = r#"{"version":1,"event":{"type":"plugin_result","payload":{"plugin_id":"demo","target":"preload","ok":true,"error":null}}}"#;
        assert!(matches!(
            parse_bridge_message(ok),
            Ok(BridgeMessage::Event(_))
        ));
        for hostile in [
            r#"{"version":1,"event":{"type":"plugin_result","payload":{"plugin_id":"demo","target":"preload","ok":true,"error":"x"}}}"#,
            r#"{"version":1,"event":{"type":"plugin_result","payload":{"plugin_id":"demo","target":"preload","ok":false,"error":null}}}"#,
            r#"{"version":1,"event":{"type":"plugin_result","payload":{"plugin_id":"demo","target":"main","ok":true,"error":null}}}"#,
            r#"{"version":1,"event":{"type":"plugin_result","payload":{"plugin_id":"../x","target":"preload","ok":true,"error":null}}}"#,
        ] {
            assert!(
                parse_bridge_message(hostile).is_err(),
                "must reject {hostile}"
            );
        }
    }

    #[test]
    fn plugin_responses_serialize_the_electron_display_shape() {
        use super::{PluginsListResponse, PluginsReloadResponse, PluginsSetEnabledResponse};
        use crate::plugins::{PluginInfo, PluginTarget};
        // The list entry mirrors manager.ts listPlugins() key for key
        // (camelCase on the wire); unset optionals are omitted exactly like
        // Electron's undefined members.
        let info = PluginInfo {
            id: "demo".to_owned(),
            name: "Demo".to_owned(),
            version: "1.0.0".to_owned(),
            description: None,
            author: Some("tester".to_owned()),
            enabled: true,
            compatible: false,
            compatibility_message: Some(
                "Incompatible with Remcord 0.0.0 (supports: 9.9.x)".to_owned(),
            ),
            compatible_versions: vec!["9.9.x".to_owned()],
            has_main: true,
            has_preload: true,
            has_renderer: false,
        };
        let listed = serde_json::to_value(PluginsListResponse {
            plugins: vec![info],
        })
        .expect("list response must serialize");
        let entry = &listed["plugins"][0];
        assert_eq!(entry["id"], "demo");
        assert_eq!(entry["compatibleVersions"], serde_json::json!(["9.9.x"]));
        assert_eq!(entry["hasMain"], true);
        assert_eq!(entry["hasRenderer"], false);
        assert!(entry.get("description").is_none());
        assert!(entry.get("path").is_none());
        assert!(entry.get("url").is_none());
        // Flip success echoes id+state+source; known failure echoes nothing.
        let flipped = serde_json::to_value(PluginsSetEnabledResponse {
            ok: true,
            plugin_id: Some("demo".to_owned()),
            enabled: Some(true),
            source: Some("file"),
            payload_bytes: Some(512),
        })
        .expect("flip response must serialize");
        assert_eq!(flipped["ok"], true);
        assert_eq!(flipped["plugin_id"], "demo");
        assert_eq!(flipped["source"], "file");
        let denied = serde_json::to_value(PluginsSetEnabledResponse {
            ok: false,
            plugin_id: None,
            enabled: None,
            source: None,
            payload_bytes: None,
        })
        .expect("denied flip must serialize");
        assert_eq!(denied, serde_json::json!({"ok": false}));
        let reloaded = serde_json::to_value(PluginsReloadResponse {
            ok: true,
            plugin_id: Some("demo".to_owned()),
        })
        .expect("reload response must serialize");
        assert_eq!(
            reloaded,
            serde_json::json!({"ok": true, "plugin_id": "demo"})
        );
        // The target enum serializes to Electron's lowercase wire values.
        assert_eq!(
            serde_json::to_value(PluginTarget::Preload).unwrap(),
            serde_json::json!("preload")
        );
        assert_eq!(
            serde_json::to_value(PluginTarget::Renderer).unwrap(),
            serde_json::json!("renderer")
        );
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
        // Charset-valid ids parse here; registry membership (registered ids
        // like `shelter`/`custom` vs file names or unknown mods) is enforced
        // at dispatch with UNKNOWN_MOD: exact registry match only, so a file
        // name or an unregistered mod can never become a source selector.
        for parseable in ["shelter", "vencord.js", "equicord", "custom", "Vencord"] {
            let message = format!(
                r#"{{"version":1,"request_id":"m-x","command":{{"type":"mod_get_bundle","payload":{{"mod_id":"{parseable}"}}}}}}"#
            );
            assert!(
                matches!(
                    parse_bridge_message(&message),
                    Ok(BridgeMessage::Request(BridgeRequest {
                        command: BridgeCommand::ModGetBundle(_),
                        ..
                    }))
                ),
                "{parseable} must parse; the host registry decides servability"
            );
            assert!(validate_mod_bundle_id(parseable).is_ok());
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
    fn accepts_all_nine_window_commands_without_parameters() {
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
            ("window_get_state", "win-9"),
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
                "window_get_state" => matches!(command, BridgeCommand::WindowGetState(_)),
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
                "window_get_state",
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
    fn window_get_state_carries_no_parameters() {
        // The geometry read is parameter-free: no bounds, handle, monitor
        // selector, or flag may be supplied.
        let request = r#"{"version":1,"request_id":"win-9","command":{"type":"window_get_state","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));
        for payload in [
            r#"{"width":835}"#,
            r#"{"x":10,"y":10}"#,
            r#"{"is_maximized":true}"#,
            r#"{"display_id":0}"#,
            r#"{"live":true}"#,
        ] {
            let message = format!(
                "{{\"version\":1,\"request_id\":\"win-x\",\"command\":{{\"type\":\"window_get_state\",\"payload\":{payload}}}}}"
            );
            assert!(
                parse_bridge_message(&message).is_err(),
                "window_get_state with {payload} must be rejected"
            );
        }
        // And no setter exists: placement is host-owned, never renderer-driven.
        for wire in [
            "window_set_state",
            "window_move",
            "window_resize",
            "window_set_bounds",
        ] {
            let message = format!(
                "{{\"version\":1,\"request_id\":\"win-y\",\"command\":{{\"type\":\"{wire}\",\"payload\":{{}}}}}}"
            );
            assert!(
                parse_bridge_message(&message).is_err(),
                "{wire} must be rejected"
            );
        }
    }

    #[test]
    fn serializes_a_typed_window_state_without_handles_or_paths() {
        use super::{BridgeResponsePayload, WindowStateResponse};
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "win-9".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::WindowState(WindowStateResponse {
                width: 835,
                height: 600,
                x: 100,
                y: 100,
                is_maximized: false,
                display_id: Some(0),
            })),
            error: None,
        };
        let serialized = serde_json::to_value(&response).unwrap();
        assert_eq!(
            serialized["payload"]["type"],
            serde_json::Value::String("window_state".to_owned())
        );
        let inner = &serialized["payload"]["payload"];
        assert_eq!(inner["width"], serde_json::Value::from(835));
        assert_eq!(inner["is_maximized"], serde_json::Value::from(false));
        assert_eq!(inner["display_id"], serde_json::Value::from(0));
        // Geometry plus flags only: no handle, path, URL, or error text.
        let mut keys: Vec<&str> = inner
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["display_id", "height", "is_maximized", "width", "x", "y"]
        );
        // The hint is optional: a capture without a known monitor omits it.
        let without_hint = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "win-10".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::WindowState(WindowStateResponse {
                width: 835,
                height: 600,
                x: 100,
                y: 100,
                is_maximized: true,
                display_id: None,
            })),
            error: None,
        };
        let serialized = serde_json::to_value(&without_hint).unwrap();
        assert!(serialized["payload"]["payload"].get("display_id").is_none());
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
    fn downloads_commands_parse_with_closed_shapes() {
        fn wire(wire_type: &str, payload: &str) -> String {
            format!("{{\"version\":1,\"request_id\":\"d\",\"command\":{{\"type\":\"{wire_type}\",\"payload\":{payload}}}}}")
        }
        // Parameter-free inventory and folder reveal.
        for wire_type in ["downloads_list", "downloads_open_folder"] {
            assert!(
                matches!(
                    parse_bridge_message(&wire(wire_type, "{}")),
                    Ok(BridgeMessage::Request(_))
                ),
                "{wire_type} must parse"
            );
            assert!(
                parse_bridge_message(&wire(wire_type, "{\"path\":\"C:\\\\x\"}")).is_err(),
                "{wire_type} must refuse paths"
            );
        }
        // Name-only reveal.
        assert!(matches!(
            parse_bridge_message(&wire("downloads_reveal", "{\"name\":\"report.txt\"}")),
            Ok(BridgeMessage::Request(_))
        ));
        let empty = wire("downloads_reveal", "{}");
        let oversized = wire(
            "downloads_reveal",
            &format!("{{\"name\":\"{}\"}}", "n".repeat(257)),
        );
        let with_path = wire(
            "downloads_reveal",
            "{\"name\":\"report.txt\",\"path\":\"C:\\\\x\"}",
        );
        assert!(parse_bridge_message(&empty).is_err());
        assert!(parse_bridge_message(&oversized).is_err());
        assert!(parse_bridge_message(&with_path).is_err());
    }

    #[test]
    fn serializes_typed_downloads_responses_without_paths() {
        use super::{
            DownloadsFolderOpenedResponse, DownloadsListResponse, DownloadsRevealedResponse,
        };
        let list = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "downloads-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::DownloadsList(
                DownloadsListResponse {
                    files: vec![crate::download_manager::DownloadEntry {
                        name: "report.txt".to_owned(),
                        bytes: 5,
                        modified_ms: 1,
                    }],
                },
            )),
            error: None,
        };
        let list_value = serde_json::to_value(list).unwrap();
        assert_eq!(list_value["payload"]["type"], "downloads_list");
        assert_eq!(
            list_value["payload"]["payload"]["files"][0]["name"],
            "report.txt"
        );
        assert!(list_value["payload"]["payload"]["files"][0]
            .get("path")
            .is_none());

        let opened = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "downloads-2".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::DownloadsFolderOpened(
                DownloadsFolderOpenedResponse {
                    location: crate::download_manager::DOWNLOADS_LOCATION_LABEL,
                },
            )),
            error: None,
        };
        let opened_value = serde_json::to_value(opened).unwrap();
        assert_eq!(opened_value["payload"]["type"], "downloads_folder_opened");
        assert_eq!(opened_value["payload"]["payload"]["location"], "downloads");
        assert!(opened_value["payload"]["payload"].get("path").is_none());

        let revealed = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "downloads-3".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::DownloadsRevealed(
                DownloadsRevealedResponse {
                    name: "report.txt".to_owned(),
                },
            )),
            error: None,
        };
        let revealed_value = serde_json::to_value(revealed).unwrap();
        assert_eq!(revealed_value["payload"]["type"], "downloads_revealed");
        assert_eq!(revealed_value["payload"]["payload"]["name"], "report.txt");
        assert!(revealed_value["payload"]["payload"].get("path").is_none());
    }

    #[test]
    fn gallery_commands_parse_with_closed_shapes() {
        fn wire(wire_type: &str, payload: &str) -> String {
            format!("{{\"version\":1,\"request_id\":\"g\",\"command\":{{\"type\":\"{wire_type}\",\"payload\":{payload}}}}}")
        }
        // Parameter-free inventory/usage/root.
        for wire_type in [
            "gallery_list",
            "gallery_categories",
            "gallery_usage",
            "gallery_root",
        ] {
            assert!(
                matches!(
                    parse_bridge_message(&wire(wire_type, "{}")),
                    Ok(BridgeMessage::Request(_))
                ),
                "{wire_type} must parse"
            );
            assert!(
                parse_bridge_message(&wire(wire_type, "{\"path\":\"C:\\\\x\"}")).is_err(),
                "{wire_type} must refuse paths"
            );
        }
        // Save pair.
        assert!(matches!(
            parse_bridge_message(&wire(
                "gallery_add_url",
                "{\"url\":\"https://x/y.png\",\"meta\":{\"title\":\"t\"}}"
            )),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(
            parse_bridge_message(&wire("gallery_add_link", "{\"url\":\"https://x/y\"}")).is_err(),
            "meta is required"
        );
        assert!(
            parse_bridge_message(&wire(
                "gallery_add_url",
                "{\"url\":\"https://x/y.png\",\"meta\":{},\"path\":\"C:\\\\x\"}"
            ))
            .is_err(),
            "paths are refused"
        );
        assert!(
            matches!(
                parse_bridge_message(&wire(
                    "gallery_add_url",
                    "{\"url\":\"file:///C:/x.png\",\"meta\":{}}"
                )),
                Ok(BridgeMessage::Request(_))
            ),
            "scheme enforcement lives at dispatch, not parse"
        );
        // Id pair.
        assert!(matches!(
            parse_bridge_message(&wire("gallery_remove", "{\"id\":\"abc-1\"}")),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(
            matches!(
                parse_bridge_message(&wire("gallery_file", "{\"id\":\"../x\"}")),
                Ok(BridgeMessage::Request(_))
            ),
            "charset enforcement lives at dispatch, not parse"
        );
        // Bulk + update + category trio.
        assert!(matches!(
            parse_bridge_message(&wire("gallery_bulk_remove", "{\"ids\":[\"a\",\"b\"]}")),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(
            parse_bridge_message(&wire("gallery_bulk_remove", "{\"ids\":[]}")).is_err(),
            "empty bulk is refused"
        );
        assert!(matches!(
            parse_bridge_message(&wire(
                "gallery_update",
                "{\"id\":\"a\",\"title\":\"t\",\"categories\":[\"c\"]}"
            )),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(
            parse_bridge_message(&wire(
                "gallery_update",
                "{\"id\":\"a\",\"kind\":\"image\",\"path\":\"C:\\\\x\"}"
            ))
            .is_err(),
            "kind/path are refused"
        );
        assert!(matches!(
            parse_bridge_message(&wire(
                "gallery_category_rename",
                "{\"old_name\":\"a\",\"new_name\":\"b\"}"
            )),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(
            parse_bridge_message(&wire("gallery_category_add", "{\"name\":\"\"}")).is_err(),
            "empty names are refused"
        );
        // Unknown command names are rejected.
        assert!(
            parse_bridge_message(&wire("gallery_import_dialog", "{}")).is_err(),
            "F5b dialog wires do not exist"
        );
    }

    #[test]
    fn rpc_commands_parse_with_closed_shapes() {
        fn wire(wire_type: &str, payload: &str) -> String {
            format!("{{\"version\":1,\"request_id\":\"r\",\"command\":{{\"type\":\"{wire_type}\",\"payload\":{payload}}}}}")
        }
        // Parameter-free reads/refresh.
        for wire_type in [
            "rpc_get_process_list",
            "rpc_refresh_process_list",
            "rpc_get_detectables",
            "rpc_get_blacklist",
        ] {
            assert!(
                matches!(
                    parse_bridge_message(&wire(wire_type, "{}")),
                    Ok(BridgeMessage::Request(_))
                ),
                "{wire_type} must parse"
            );
            assert!(
                parse_bridge_message(&wire(wire_type, "{\"path\":\"C:\\\\x\"}")).is_err(),
                "{wire_type} must refuse paths"
            );
        }
        // Detectable save: object required, oversized refused early.
        assert!(matches!(
            parse_bridge_message(&wire(
                "rpc_add_detectable",
                "{\"detectable\":{\"id\":\"1\",\"name\":\"G\",\"executables\":[]}}"
            )),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(
            parse_bridge_message(&wire("rpc_add_detectable", "{\"id\":\"1\"}")).is_err(),
            "detectable member is required"
        );
        assert!(
            parse_bridge_message(&wire(
                "rpc_add_detectable",
                &format!(
                    "{{\"detectable\":{{\"id\":\"1\",\"name\":\"{}\"}}}}",
                    "x".repeat(17 * 1024)
                )
            ))
            .is_err(),
            "oversized detectables fail early"
        );
        // Detectable remove: id presence + cap.
        assert!(matches!(
            parse_bridge_message(&wire("rpc_remove_detectable", "{\"id\":\"abc\"}")),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(parse_bridge_message(&wire("rpc_remove_detectable", "{\"id\":\"\"}")).is_err());
        // Blacklist save: name presence + cap; id is a typed integer.
        assert!(matches!(
            parse_bridge_message(&wire("rpc_blacklist_game", "{\"name\":\"G\",\"id\":7}")),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(
            parse_bridge_message(&wire("rpc_blacklist_game", "{\"name\":\"\",\"id\":7}")).is_err(),
            "empty names are refused"
        );
        assert!(
            parse_bridge_message(&wire("rpc_blacklist_game", "{\"name\":\"G\",\"id\":\"7\"}"))
                .is_err(),
            "string ids never parse"
        );
        assert!(
            parse_bridge_message(&wire(
                "rpc_blacklist_game",
                "{\"name\":\"G\",\"id\":7,\"path\":\"C:\\\\x\"}"
            ))
            .is_err(),
            "paths are refused"
        );
        assert!(matches!(
            parse_bridge_message(&wire("rpc_unblacklist_game", "{\"id\":7}")),
            Ok(BridgeMessage::Request(_))
        ));
    }

    #[test]
    fn settings_import_electron_parses_parameter_free() {
        let request = r#"{"version":1,"request_id":"import-1","command":{"type":"settings_import_electron","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));
        // No source path, URL, key list, or value may ride along: the host
        // owns the fixed source and the allowlist.
        for hostile in [
            r#"{"version":1,"request_id":"import-2","command":{"type":"settings_import_electron","payload":{"path":"C:\\x"}}}"#,
            r#"{"version":1,"request_id":"import-3","command":{"type":"settings_import_electron","payload":{"url":"https://attacker.invalid/s.json"}}}"#,
            r#"{"version":1,"request_id":"import-4","command":{"type":"settings_import_electron","payload":{"keys":["mods"]}}}"#,
            r#"{"version":1,"request_id":"import-5","command":{"type":"settings_import_electron","payload":{"values":{"mods":[]}}}}"#,
        ] {
            assert!(parse_bridge_message(hostile).is_err());
        }
    }

    #[test]
    fn keybind_commands_parse_with_closed_shapes() {
        fn wire(wire_type: &str, payload: &str) -> String {
            format!("{{\"version\":1,\"request_id\":\"k\",\"command\":{{\"type\":\"{wire_type}\",\"payload\":{payload}}}}}")
        }
        assert!(matches!(
            parse_bridge_message(&wire("keybinds_list", "{}")),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(parse_bridge_message(&wire("keybinds_list", "{\"path\":\"C:\\\\x\"}")).is_err());
        // Save: exact shape only. `js` can never parse; runJavascript is a
        // dispatch-time refusal, not a parse error (clearer typed code).
        assert!(matches!(
            parse_bridge_message(&wire(
                "keybinds_add",
                "{\"id\":\"a-1\",\"accelerator\":\"Ctrl+M\",\"action\":\"mute\",\"global\":true,\"enabled\":true}"
            )),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(parse_bridge_message(&wire(
            "keybinds_add",
            "{\"id\":\"a-1\",\"accelerator\":\"Ctrl+M\",\"action\":\"runJavascript\",\"global\":true,\"enabled\":true}"
        ))
        .is_ok(), "runJavascript parses; dispatch refuses it");
        assert!(
            parse_bridge_message(&wire(
                "keybinds_add",
                "{\"id\":\"a-1\",\"accelerator\":\"Ctrl+M\",\"action\":\"mute\",\"global\":true,\"enabled\":true,\"js\":\"alert(1)\"}"
            ))
            .is_err(),
            "stored code can never cross"
        );
        assert!(
            parse_bridge_message(&wire(
                "keybinds_add",
                "{\"id\":\"a-1\",\"accelerator\":\"Ctrl+M\",\"action\":\"mute\",\"global\":true}"
            ))
            .is_err(),
            "enabled is required"
        );
        // Toggle/remove: id only.
        assert!(matches!(
            parse_bridge_message(&wire("keybinds_toggle", "{\"id\":\"a-1\"}")),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(parse_bridge_message(&wire("keybinds_remove", "{\"id\":\"\"}")).is_err());
        assert!(
            parse_bridge_message(&wire(
                "keybinds_toggle",
                "{\"id\":\"a-1\",\"enabled\":true}"
            ))
            .is_err(),
            "no second member"
        );
        // Matched event: id only.
        let matched = r#"{"version":1,"event":{"type":"keybind_matched","payload":{"id":"a-1"}}}"#;
        assert!(matches!(
            parse_bridge_message(matched),
            Ok(BridgeMessage::Event(_))
        ));
        let matched_action = r#"{"version":1,"event":{"type":"keybind_matched","payload":{"id":"a-1","action":"mute"}}}"#;
        assert!(
            parse_bridge_message(matched_action).is_err(),
            "actions never cross events"
        );
    }

    #[test]
    fn serializes_a_typed_keybinds_list_response() {
        use super::KeybindsListResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "k-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::KeybindsList(KeybindsListResponse {
                keybinds: vec![
                    serde_json::json!({"id": "a-1", "accelerator": "Ctrl+M", "action": "mute", "global": true, "enabled": true}),
                ],
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "keybinds_list");
        assert_eq!(value["payload"]["payload"]["keybinds"][0]["id"], "a-1");
        assert!(value["payload"]["payload"]["keybinds"][0]
            .get("js")
            .is_none());
    }

    #[test]
    fn menus_get_template_parses_parameter_free() {
        // The F15 template read is host-owned: any renderer-supplied
        // member (selector, id, label, locale, kind) is rejected.
        let request = r#"{"version":1,"request_id":"menus-1","command":{"type":"menus_get_template","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(request))
                if matches!(request.command, BridgeCommand::MenusGetTemplate(_))
        ));
        for hostile in [
            r#"{"version":1,"request_id":"menus-2","command":{"type":"menus_get_template","payload":{"id":"remcord"}}}"#,
            r#"{"version":1,"request_id":"menus-3","command":{"type":"menus_get_template","payload":{"locale":"de"}}}"#,
            r#"{"version":1,"request_id":"menus-4","command":{"type":"menus_get_template","payload":{"kind":"image"}}}"#,
            r#"{"version":1,"request_id":"menus-5","command":{"type":"menus_get_template","payload":{"labels":["menu-quit"]}}}"#,
        ] {
            assert!(parse_bridge_message(hostile).is_err());
        }
    }

    #[test]
    fn serializes_a_typed_menus_template_response() {
        use super::MenusTemplateResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "menus-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::MenusTemplate(
                MenusTemplateResponse {
                    menus: vec![
                        serde_json::json!({"id": "remcord", "labelId": "menu-remcord", "items": []}),
                    ],
                    keybind_menu: vec![
                        serde_json::json!({"kind": "keybind", "id": "a-1", "labelId": "keybind-mute", "accelerator": "Ctrl+M"}),
                    ],
                    context_menu: "suppressed".to_owned(),
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "menus_template");
        assert_eq!(value["payload"]["payload"]["menus"][0]["id"], "remcord");
        assert_eq!(
            value["payload"]["payload"]["keybind_menu"][0]["labelId"],
            "keybind-mute"
        );
        assert_eq!(value["payload"]["payload"]["context_menu"], "suppressed");
        assert!(value["payload"]["payload"].get("labels").is_none());
        assert!(value["payload"]["payload"].get("path").is_none());
    }

    #[test]
    fn serializes_a_typed_settings_imported_response() {
        use super::SettingsImportedResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "import-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::SettingsImported(
                SettingsImportedResponse {
                    applied: vec!["mods".to_owned(), "csp".to_owned()],
                    skipped: vec!["channel".to_owned()],
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "settings_imported");
        assert_eq!(
            value["payload"]["payload"]["applied"],
            serde_json::json!(["mods", "csp"])
        );
        assert!(value["payload"]["payload"].get("values").is_none());
        assert!(value["payload"]["payload"].get("path").is_none());
    }

    #[test]
    fn serializes_a_typed_gallery_list_response() {
        use super::GalleryListResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "g-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::GalleryList(GalleryListResponse {
                items: vec![serde_json::json!({"id": "a-1", "kind": "link", "localFile": ""})],
                categories: vec![serde_json::json!({"name": "clips"})],
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "gallery_list");
        assert_eq!(value["payload"]["payload"]["items"][0]["id"], "a-1");
        assert!(value["payload"]["payload"]["items"][0]
            .get("path")
            .is_none());
    }

    #[test]
    fn serializes_typed_rpc_responses() {
        use super::{
            RpcBlacklistResponse, RpcDetectablesResponse, RpcProcessListResponse,
            RpcProcessRefreshResponse,
        };
        use crate::arrpc::RpcBlacklistEntry;
        let list = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "r-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::RpcProcessList(
                RpcProcessListResponse {
                    processes: vec![serde_json::json!([4, "C:\\a.exe", []])],
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(list).unwrap();
        assert_eq!(value["payload"]["type"], "rpc_process_list");
        assert_eq!(
            value["payload"]["payload"]["processes"][0],
            serde_json::json!([4, "C:\\a.exe", []])
        );
        let refresh = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "r-2".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::RpcProcessRefresh(
                RpcProcessRefreshResponse {
                    refreshed: true,
                    count: 3,
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(refresh).unwrap();
        assert_eq!(value["payload"]["type"], "rpc_process_refresh");
        assert_eq!(value["payload"]["payload"]["count"], 3);
        let detectables = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "r-3".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::RpcDetectables(
                RpcDetectablesResponse {
                    detectables: vec![serde_json::json!({"id": "1"})],
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(detectables).unwrap();
        assert_eq!(value["payload"]["type"], "rpc_detectables");
        let blacklist = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "r-4".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::RpcBlacklist(RpcBlacklistResponse {
                entries: vec![RpcBlacklistEntry {
                    name: "G".to_owned(),
                    id: 7,
                }],
            })),
            error: None,
        };
        let value = serde_json::to_value(blacklist).unwrap();
        assert_eq!(value["payload"]["type"], "rpc_blacklist");
        assert_eq!(value["payload"]["payload"]["entries"][0]["id"], 7);
        assert!(value["payload"]["payload"]["entries"][0]
            .get("path")
            .is_none());
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
    fn accepts_tray_get_state_without_parameters() {
        // F13 tray state read: a parameter-free host snapshot (resolved
        // mode, availability, minimizeToTray, close policy). Any
        // renderer-supplied member — mode, flag, title, badge, path,
        // handle, or asset selector — is rejected.
        let message = r#"{"version":1,"request_id":"tray-1","command":{"type":"tray_get_state","payload":{}}}"#;
        let parsed = parse_bridge_message(message);
        assert!(parsed.is_ok(), "tray_get_state must parse: {parsed:?}");
        let command = match parsed.unwrap() {
            BridgeMessage::Request(request) => request.command,
            BridgeMessage::Event(_) => panic!("tray_get_state must parse as a request"),
        };
        assert!(
            matches!(command, BridgeCommand::TrayGetState(_)),
            "tray_get_state must dispatch to its own typed variant"
        );
        for payload in [
            r#"{"mode":"dynamic"}"#,
            r#"{"minimize_to_tray":true}"#,
            r#"{"title":"(3) Discord"}"#,
            r#"{"badge":"badge-3.ico"}"#,
            r#"{"handle":"0x1234"}"#,
            r#"{"asset":"ac_plug_colored.png"}"#,
        ] {
            let message = format!(
                "{{\"version\":1,\"request_id\":\"x\",\"command\":{{\"type\":\"tray_get_state\",\"payload\":{payload}}}}}"
            );
            assert!(
                parse_bridge_message(&message).is_err(),
                "tray_get_state with {payload} must be rejected"
            );
        }
    }

    #[test]
    fn accepts_spellcheck_get_state_without_parameters() {
        // F18 read-only state: the empty payload parses to the typed
        // variant; any renderer-supplied member is rejected.
        let message = r#"{"version":1,"request_id":"spell-1","command":{"type":"spellcheck_get_state","payload":{}}}"#;
        let parsed = parse_bridge_message(message).expect("spellcheck_get_state must parse");
        assert!(matches!(
            parsed,
            BridgeMessage::Request(request)
                if matches!(request.command, BridgeCommand::SpellcheckGetState(_))
        ));
        for payload in [
            r#"{"languages":["en-US"]}"#,
            r#"{"enabled":true}"#,
            r#"{"path":"C:\\secret"}"#,
        ] {
            let hostile = format!(
                "{{\"version\":1,\"request_id\":\"x\",\"command\":{{\"type\":\"spellcheck_get_state\",\"payload\":{payload}}}}}"
            );
            assert!(
                parse_bridge_message(&hostile).is_err(),
                "spellcheck_get_state with {payload} must be rejected"
            );
        }
    }

    #[test]
    fn clipboard_copy_image_parses_one_http_url_and_rejects_the_rest() {
        // The F17 image copy takes exactly one http(s) URL: the host
        // downloads the bytes itself, so bytes, paths, dimensions, and
        // selectors can never parse.
        let request = r#"{"version":1,"request_id":"clip-1","command":{"type":"clipboard_copy_image","payload":{"url":"https://cdn.discordapp.com/attachments/a/b.png"}}}"#;
        let parsed = parse_bridge_message(request).expect("valid clipboard copy must parse");
        match parsed {
            BridgeMessage::Request(request) => match request.command {
                BridgeCommand::ClipboardCopyImage(payload) => {
                    assert_eq!(
                        payload.url,
                        "https://cdn.discordapp.com/attachments/a/b.png"
                    );
                }
                other => panic!("must dispatch to ClipboardCopyImage, got {other:?}"),
            },
            BridgeMessage::Event(_) => panic!("clipboard_copy_image must parse as a request"),
        }
        for hostile in [
            // Empty / blank.
            r#"{"version":1,"request_id":"clip-2","command":{"type":"clipboard_copy_image","payload":{"url":""}}}"#,
            r#"{"version":1,"request_id":"clip-3","command":{"type":"clipboard_copy_image","payload":{"url":"   "}}}"#,
            // Non-web schemes and bare hosts.
            r#"{"version":1,"request_id":"clip-4","command":{"type":"clipboard_copy_image","payload":{"url":"ftp://example.com/i.png"}}}"#,
            r#"{"version":1,"request_id":"clip-5","command":{"type":"clipboard_copy_image","payload":{"url":"file:///C:/pic.png"}}}"#,
            r#"{"version":1,"request_id":"clip-6","command":{"type":"clipboard_copy_image","payload":{"url":"javascript:alert(1)"}}}"#,
            r#"{"version":1,"request_id":"clip-7","command":{"type":"clipboard_copy_image","payload":{"url":"data:image/png;base64,iVBOR"}}}"#,
            r#"{"version":1,"request_id":"clip-8","command":{"type":"clipboard_copy_image","payload":{"url":"example.com/i.png"}}}"#,
            // Renderer-supplied bytes/paths/dimensions can never parse.
            r#"{"version":1,"request_id":"clip-9","command":{"type":"clipboard_copy_image","payload":{"url":"https://example.com/i.png","bytes":"aGVsbG8="}}}"#,
            r#"{"version":1,"request_id":"clip-10","command":{"type":"clipboard_copy_image","payload":{"url":"https://example.com/i.png","path":"C:\\pic.png"}}}"#,
            r#"{"version":1,"request_id":"clip-11","command":{"type":"clipboard_copy_image","payload":{"url":"https://example.com/i.png","width":64}}}"#,
        ] {
            assert!(
                parse_bridge_message(hostile).is_err(),
                "{hostile} must be rejected"
            );
        }
        // Overlong URLs are rejected, never truncated.
        let long = format!(
            "{{\"version\":1,\"request_id\":\"clip-12\",\"command\":{{\"type\":\"clipboard_copy_image\",\"payload\":{{\"url\":\"https://example.com/{}.png\"}}}}}}",
            "a".repeat(2500)
        );
        assert!(parse_bridge_message(&long).is_err());
    }

    #[test]
    fn serializes_a_typed_clipboard_image_copied_response() {
        use super::ClipboardImageCopiedResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "clip-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::ClipboardImageCopied(
                ClipboardImageCopiedResponse {
                    ok: true,
                    width: 64,
                    height: 48,
                    error: None,
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "clipboard_image_copied");
        assert_eq!(value["payload"]["payload"]["ok"], true);
        assert_eq!(value["payload"]["payload"]["width"], 64);
        assert_eq!(value["payload"]["payload"]["height"], 48);
        assert!(
            value["payload"]["payload"].get("error").is_none()
                || value["payload"]["payload"]["error"].is_null()
        );
        assert!(value["payload"]["payload"].get("bytes").is_none());
        assert!(value["payload"]["payload"].get("path").is_none());
        assert!(value["payload"]["payload"].get("url").is_none());
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

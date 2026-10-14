mod app;
mod backup;
mod browser_recovery;
mod bundles;
mod channel;
mod child_recovery;
mod close_behavior;
mod css_watch;
mod custom_bundle;
mod diagnostics;
mod downloads;
mod environment_observer;
mod folders;
mod gallery;
mod graphics;
mod keybinds;
mod localization;
mod main_webview;
mod mod_cache;
mod mod_settings;
mod mod_substrate;
mod navigation_completed;
mod navigation_gate;
mod new_window;
mod permissions;
mod plugin_storage;
mod plugins;
mod process_failure;
mod profile;
mod profile_migration;
mod protocol;
mod quick_css;
mod recovery;
mod remcord_protocol;
mod screenshare_media;
mod screenshare_picker;
mod screenshare_quality;
mod settings;
mod settings_boot;
mod settings_import;
mod settings_plugin;
mod shelter_injector;
mod spellcheck;
mod theme_import;
mod themes;
mod titlebar_plugin;
mod user_agent;
mod webview_runtime;

use std::{
    collections::HashMap,
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use mod_substrate::{
    load_mod_bundle, managed_css_order_script, managed_css_remove_script, managed_css_script,
    mod_css_script, mod_execution_context_script, mod_marker_script, stage as mod_stage,
    validate_descriptor, validate_mod_bundle, DocumentModLifecycle, ModArtifactIdentity, ModBundle,
    ModCache, ModCssSource, ModDescriptor, ModEnabling, ModHostMetrics, PageGlobalReadiness,
    PendingModBootstrap,
};

use app::{current_app_version, validate_version};
use backup::{
    build_backup_manifest, build_backup_zip, collect_backup_entries, commit_staged_restore,
    created_at_now, discard_staged_restore, read_archive_file, show_open_picker, show_save_picker,
    stage_restore_from_bytes, validate_backup_request, BackupFailure, BackupSaveSummary,
    OpenPickerOutcome, RestorePreview, RestoreSummary, SavePickerOutcome, StagedRestore,
};
use bundles::{describe_bundle_files, BUNDLE_FILES};
use css_watch::{
    diff_theme_snapshots, quick_css_transition, start_live_css_watchers, LiveCssCounters,
    LiveCssFsSignal, QuickCssTransition, ResourceDebouncer, CSS_DEBOUNCE_MS,
};
use folders::{
    ensure_folder_root, open_folder, resolve_theme_folder, reveal_folder, FolderLocation,
};
use localization::{lookup as lookup_lang_string, validate_key as validate_lang_key};
use navigation_completed::NavigationCompletedEvent;
use plugin_storage::{read_plugin_file, write_plugin_file};
use process_failure::ProcessFailedEvent;
use protocol::{
    parse_bridge_message, validate_mod_bundle_id, validate_vencord_css_mod_id,
    window_state_event_json, AppVersionResponse, BackupInspectedResponse,
    BackupRestoreDiscardedResponse, BackupRestoredResponse, BackupSavedResponse, BridgeCommand,
    BridgeError, BridgeEvent, BridgeEventMessage, BridgeMessage, BridgeRequest, BridgeResponse,
    BridgeResponsePayload, BundleFileEntry, BundlesListResponse, DiagnosticsInfoResponse,
    FoldersOpenedResponse, GalleryAddResponse, GalleryBulkRemoveResponse,
    GalleryCategoriesResponse, GalleryCategoryAddResponse, GalleryCategoryRemoveResponse,
    GalleryCategoryRenameResponse, GalleryFileResponse, GalleryListResponse, GalleryRemoveResponse,
    GalleryRootResponse, GalleryUpdateResponse, GalleryUsageResponse, KeybindsAddResponse,
    KeybindsListResponse, KeybindsRemoveResponse, KeybindsToggleResponse,
    LocalizationStringResponse, ModBundleResponse, ModCacheClearedResponse, PingResponse,
    PluginStorageReadResponse, PluginStorageWriteResponse, PluginsListResponse,
    PluginsReloadResponse, PluginsRuntimeEntriesResponse, PluginsRuntimeScriptResponse,
    PluginsSetEnabledResponse, QuickCssReadResponse, QuickCssWrittenResponse, RequestTracker,
    SettingsConfigResponse, SettingsImportedResponse, SettingsMutationResponse,
    SettingsSetResponse, SpellcheckStateResponse, ThemeEnabledResponse, ThemeFolderOpenedResponse,
    ThemeImportedResponse, ThemeInstalledResponse, ThemeWrittenResponse, ThemesListResponse,
    WindowAckResponse, PROTOCOL_VERSION,
};
use quick_css::{load_quick_css, read_quick_css_text, write_quick_css_text, QUICK_CSS_STYLE_ID};
use recovery::{MainRendererRecovery, MAIN_INSTANCE_ID};
use remcord_protocol::scheme::create_environment as create_remcord_webview_environment;
use remcord_protocol::{build_namespace_roots, register_remcord_protocol};
use serde::Serialize;
use settings::{
    apply_block_power_saving, apply_plugin_state, apply_settings_set, load_settings, SettingsSetKey,
};
use tao::{
    dpi::{LogicalSize, Size},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};
use theme_import::{
    import_from_paths, show_import_picker, FileImportOutcome, PickerOutcome, ThemeImportFailure,
    ThemeImportSummary,
};
use themes::{
    install_theme_from_code, list_themes, load_enabled_themes, set_theme_enabled, write_theme_css,
    LoadedTheme,
};
#[cfg(target_os = "windows")]
use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2, ICoreWebView2Environment};
use wry::{
    http::{Request, Uri},
    NewWindowResponse, WebContext, WebView, WebViewBuilder,
};
#[cfg(target_os = "windows")]
use wry::{WebViewBuilderExtWindows, WebViewExtWindows};

/// Vencord as the first consumer of the shared mod substrate. Every bundle
/// identity value lives in this descriptor; the substrate mechanics in
/// `mod_substrate.rs` never name Vencord. Registering Shelter or Equicord
/// means adding a second descriptor plus a thin renderer adapter.
const VENCORD_MOD: ModDescriptor = ModDescriptor {
    id: "vencord",
    capability: "mod.get_bundle",
    js_url: "https://github.com/Vendicated/Vencord/releases/download/devbuild/browser.js",
    js_file: "vencord.js",
    css: Some(ModCssSource {
        url: "https://github.com/Vendicated/Vencord/releases/download/devbuild/browser.css",
        file: "vencord.css",
        max_bytes: 8 * 1024 * 1024,
        style_element_id: "remcord-native-vencord-css",
        result_event: "vencord_css_result",
    }),
    allowed_hosts: &["github.com", "release-assets.githubusercontent.com"],
    max_js_bytes: 32 * 1024 * 1024,
    identity: ModArtifactIdentity::BuildHeader {
        prefix: "// Vencord ",
    },
    page_global: "Vencord",
    page_global_requires: None,
    page_global_readiness: PageGlobalReadiness::Synchronous,
    enabling: ModEnabling::SettingsMods,
    success_marker: "vencord-bootstrap-executed",
    settings_key: Some("VencordSettings"),
    bootstrap_dom_event: "remcord-native:vencord-bootstrap",
    storage_context_global: "__remcordNativeVencordStorageAtExecution",
    log_tag: "Mod:Vencord",
};

/// Shelter as the second consumer of the shared mod substrate. Traced from
/// `src/discord/preload/mods/shelter.mts` and `ipc.ts:109`, not inferred:
///
/// - Source: the fixed `uwu/shelter-builds` `shelter.js` in `src/common/consts.ts`.
/// - Retrieval: `getShelterBundle` returns ONLY `js`; there is no `shelter.css`
///   anywhere in the repo, so the descriptor declares no stylesheet.
/// - Identity: the published build carries NO banner (the file begins directly
///   with its IIFE), so identity is proven with upstream's own unique
///   initialization sentinels plus a host-computed content digest.
/// - Execution: `webFrame.executeJavaScript` from the preload -> page world,
///   document start, same as every other client mod. Electron additionally
///   wraps the source in an IIFE that declares `SHELTER_INJECTOR_PLUGINS`
///   with remote (`remcord://` URL) entries for all five bundled plugins.
///   Natively the host prepends an equivalent declaration in LOCAL form
///   (see `shelter_injector.rs`): pinned `{js, manifest, update: false}`
///   objects for the servable builtins only, so Shelter's own registrar
///   loads them with no fetch and no update checks. Shelter still skips
///   the injector entirely when the global is absent.
/// - Initialization: Shelter publishes `window.shelter` from an inner async
///   IIFE only after Discord's `#app-mount` exists and has children, so the
///   readiness is asynchronous rather than synchronous.
/// - Settings: Shelter keeps settings in IndexedDB, never `localStorage`, so no
///   settings key is invented for the storage diagnostic.
/// - Enablement: Electron hardcodes `enabled: true`, so Shelter is not gated on
///   `settings.mods`.
const SHELTER_MOD: ModDescriptor = ModDescriptor {
    id: "shelter",
    capability: "mod.get_bundle",
    js_url: "https://raw.githubusercontent.com/uwu/shelter-builds/main/shelter.js",
    js_file: "shelter.js",
    css: None,
    allowed_hosts: &["raw.githubusercontent.com"],
    max_js_bytes: 32 * 1024 * 1024,
    identity: ModArtifactIdentity::SourceSentinels {
        // Verbatim upstream literals; none appears in Vencord's or Equicord's
        // bundle, so another mod's artifact can never pass as Shelter's.
        sentinels: &[
            "shelter is initializing...",
            "shelter is initialized. took: ",
        ],
    },
    page_global: "shelter",
    page_global_requires: Some("plugins"),
    page_global_readiness: PageGlobalReadiness::Awaited {
        ready_global: "__remcordNativeShelterPageReady",
        // Shelter waits for `#app-mount` to render, so the global appears well
        // after bundle evaluation. The pre-bundle trap resolves the normal case
        // with no polling; the 250 ms poll is only a bounded fallback.
        timeout_ms: 120_000,
        poll_ms: 250,
    },
    enabling: ModEnabling::AlwaysEnabled,
    success_marker: "shelter-bootstrap-executed",
    settings_key: None,
    bootstrap_dom_event: "remcord-native:shelter-bootstrap",
    storage_context_global: "__remcordNativeShelterStorageAtExecution",
    log_tag: "Mod:Shelter",
};

/// Equicord as the third consumer of the same substrate. Traced from
/// `src/discord/preload/mods/equicord.mts` and `ipc.ts:122`:
///
/// - Source: the fixed Equicord `latest` release `browser.js`/`browser.css`.
/// - CSS: required — the Electron preload calls `webFrame.insertCSS(bundle.css!)`
///   with an explicit "Equicord requires CSS" note, so the descriptor declares
///   a full stylesheet contract.
/// - Identity: the artifact opens with `// Equicord <sha>`, so a build banner
///   proves identity.
/// - Execution: page world, document start. Equicord is a Vencord fork and
///   publishes the `Vencord` global, which is exactly what its own bundle does.
/// - Enablement: gated on `settings.mods`, matching
///   `getConfig("mods").includes("equicord")`.
const EQUICORD_MOD: ModDescriptor = ModDescriptor {
    id: "equicord",
    capability: "mod.get_bundle",
    js_url: "https://github.com/Equicord/Equicord/releases/download/latest/browser.js",
    js_file: "equicord.js",
    css: Some(ModCssSource {
        url: "https://github.com/Equicord/Equicord/releases/download/latest/browser.css",
        file: "equicord.css",
        max_bytes: 8 * 1024 * 1024,
        style_element_id: "remcord-native-equicord-css",
        // The mod-agnostic CSS result event, with the host-supplied mod id
        // echoed back so the host can refuse a result claimed for another mod.
        result_event: "mod_css_result",
    }),
    allowed_hosts: &["github.com", "release-assets.githubusercontent.com"],
    max_js_bytes: 32 * 1024 * 1024,
    identity: ModArtifactIdentity::BuildHeader {
        prefix: "// Equicord ",
    },
    page_global: "Vencord",
    page_global_requires: None,
    page_global_readiness: PageGlobalReadiness::Synchronous,
    enabling: ModEnabling::SettingsMods,
    success_marker: "equicord-bootstrap-executed",
    settings_key: Some("EquicordSettings"),
    bootstrap_dom_event: "remcord-native:equicord-bootstrap",
    storage_context_global: "__remcordNativeEquicordStorageAtExecution",
    log_tag: "Mod:Equicord",
};

/// Custom client mods as the fourth consumer of the same substrate. Traced
/// from `src/discord/preload/mods/custom.mts` and `ipc.ts:130-137`:
///
/// - Source: the user's own `<profile>/custom.js` + `<profile>/custom.css`
///   (host-built fixed paths; the renderer supplies only the logical id
///   `"custom"`, never a path or URL). There is deliberately NO download:
///   `js_url` / `css.url` below are non-resolving `example.invalid`
///   placeholders that satisfy structural validation only — retrieval is
///   local-file only (`custom_bundle.rs`) and `load_mod_bundle` refuses a
///   `UserOwned` descriptor outright, so the URLs can never be fetched even
///   by a mistaken call.
/// - Identity: user-owned bytes carry no upstream banner or sentinel, so
///   the `UserOwned` identity accepts any UTF-8 bytes within the
///   descriptor's size bounds and versions them by content digest.
/// - CSS: optional — the Electron preload calls `insertCSS(bundle.css)`
///   only when CSS is present, so a missing `custom.css` skips the CSS
///   stage instead of failing (unlike Equicord, whose CSS is required).
/// - Verification: Electron verifies nothing after execution, and no
///   custom bundle can be required to publish a page-world global. The
///   marker therefore checks the host's own storage-context probe object
///   (defined by the pre-execution script immediately before the user JS),
///   which proves "host context plus user JS evaluated without delivery
///   failing" — the exact Electron success shape.
/// - Enablement: gated on `settings.mods`, matching
///   `getConfig("mods").includes("custom")`.
const CUSTOM_MOD: ModDescriptor = ModDescriptor {
    id: "custom",
    capability: "mod.get_bundle",
    js_url: "https://example.invalid/custom.js",
    js_file: "custom.js",
    css: Some(ModCssSource {
        url: "https://example.invalid/custom.css",
        file: "custom.css",
        max_bytes: custom_bundle::MAX_CUSTOM_CSS_BYTES,
        style_element_id: "remcord-native-custom-css",
        // The mod-agnostic CSS result event, with the host-supplied mod id
        // echoed back so the host can refuse a result claimed for another mod.
        result_event: "mod_css_result",
    }),
    allowed_hosts: &["example.invalid"],
    max_js_bytes: custom_bundle::MAX_CUSTOM_JS_BYTES,
    identity: ModArtifactIdentity::UserOwned,
    page_global: "__remcordNativeCustomStorageAtExecution",
    page_global_requires: None,
    page_global_readiness: PageGlobalReadiness::Synchronous,
    enabling: ModEnabling::SettingsMods,
    success_marker: "custom-bootstrap-executed",
    settings_key: None,
    bootstrap_dom_event: "remcord-native:custom-bootstrap",
    storage_context_global: "__remcordNativeCustomStorageAtExecution",
    log_tag: "Mod:Custom",
};

fn lookup_mod_descriptor(mod_id: &str) -> Option<ModDescriptor> {
    KNOWN_MODS
        .iter()
        .find(|descriptor| descriptor.id == mod_id)
        .copied()
}

/// Borrowed registry lookup, for callers that need a reference to the
/// descriptor itself rather than a copy.
fn find_mod_descriptor(mod_id: &str) -> Option<&'static ModDescriptor> {
    KNOWN_MODS.iter().find(|descriptor| descriptor.id == mod_id)
}

/// Loads a user-owned custom bundle from host-built profile-root files.
///
/// The renderer's logical id got us here; both file paths below are fixed
/// host constants (`custom_bundle.rs`), so no renderer path, URL, or
/// filename input can reach the filesystem. Envelope validation (UTF-8,
/// size caps, at-least-one-half) stays owned by the shared substrate via
/// the `UserOwned` identity: this function only moves raw bytes.
fn load_custom_bundle(
    descriptor: &ModDescriptor,
    profile: std::path::PathBuf,
) -> Result<ModBundle, String> {
    debug_assert!(matches!(
        descriptor.identity,
        ModArtifactIdentity::UserOwned
    ));
    let raw = custom_bundle::load_raw_custom_bundle(&profile)
        .map_err(|failure| format!("[{}:{}] {}", descriptor.id, failure.code, failure.message))?;
    validate_mod_bundle(descriptor, raw.js.unwrap_or_default(), raw.css)
}

/// Stateless plugin inventory for `plugins.list`: fresh discovery plus
/// enablement from the authoritative settings. A missing plugins root
/// lists empty; an unreadable settings file fails closed (enablement is
/// unknowable, so guessing would misreport). No path crosses in either
/// direction.
fn plugins_list_compute(
    profile: &Path,
) -> Result<Vec<plugins::PluginInfo>, plugins::PluginFailure> {
    let discovered = plugins::discover_plugins(profile, current_app_version());
    let settings = load_settings(profile).map_err(|message| plugins::PluginFailure {
        code: "PLUGINS_UNAVAILABLE",
        message,
    })?;
    Ok(plugins::plugin_infos(
        &discovered,
        &settings.snapshot.plugin_states,
    ))
}

/// Enablement flip for `plugins.set_enabled`. Unknown ids and
/// incompatible enables resolve `Ok(ok:false)` without writing (Electron
/// parity); malformed ids and persistence failures are typed errors.
fn plugins_set_enabled_compute(
    profile: &Path,
    plugin_id: &str,
    enabled: bool,
) -> Result<PluginsSetEnabledResponse, plugins::PluginFailure> {
    if plugins::validate_plugin_id(plugin_id).is_err() {
        return Err(plugins::PluginFailure {
            code: "INVALID_PLUGIN_ID",
            message: "plugin id is invalid".to_owned(),
        });
    }
    let discovered = plugins::discover_plugins(profile, current_app_version());
    let Some(record) = plugins::find_plugin(&discovered, plugin_id) else {
        return Ok(PluginsSetEnabledResponse {
            ok: false,
            plugin_id: None,
            enabled: None,
            source: None,
            payload_bytes: None,
        });
    };
    if enabled && !record.compatible {
        return Ok(PluginsSetEnabledResponse {
            ok: false,
            plugin_id: None,
            enabled: None,
            source: None,
            payload_bytes: None,
        });
    }
    let mutation = apply_plugin_state(profile, plugin_id, enabled).map_err(|message| {
        plugins::PluginFailure {
            code: "PLUGINS_UNAVAILABLE",
            message,
        }
    })?;
    Ok(PluginsSetEnabledResponse {
        ok: true,
        plugin_id: Some(mutation.plugin_id),
        enabled: Some(mutation.enabled),
        source: Some(mutation.source.as_str()),
        payload_bytes: Some(mutation.payload_bytes),
    })
}

/// Re-validation for `plugins.reload`: unknown or incompatible ids resolve
/// `Ok(ok:false)` (Electron parity — reload never enables anything). The
/// re-discovery itself IS the reload in the stateless design; the renderer
/// loader picks the result up on the next document.
fn plugins_reload_compute(
    profile: &Path,
    plugin_id: &str,
) -> Result<PluginsReloadResponse, plugins::PluginFailure> {
    if plugins::validate_plugin_id(plugin_id).is_err() {
        return Err(plugins::PluginFailure {
            code: "INVALID_PLUGIN_ID",
            message: "plugin id is invalid".to_owned(),
        });
    }
    let discovered = plugins::discover_plugins(profile, current_app_version());
    let Some(record) = plugins::find_plugin(&discovered, plugin_id) else {
        return Ok(PluginsReloadResponse {
            ok: false,
            plugin_id: None,
        });
    };
    if !record.compatible {
        return Ok(PluginsReloadResponse {
            ok: false,
            plugin_id: None,
        });
    }
    Ok(PluginsReloadResponse {
        ok: true,
        plugin_id: Some(plugin_id.to_owned()),
    })
}

/// Runtime entries for `plugins.get_runtime_entries`: every
/// enabled+compatible plugin's logical `{id, name}` for the target.
/// Enablement comes from the authoritative settings; unresolvable entries
/// are skipped (a plugin whose file vanished mid-run loads nothing).
fn plugins_entries_compute(
    profile: &Path,
    target: plugins::PluginTarget,
) -> Result<Vec<plugins::PluginRuntimeEntry>, plugins::PluginFailure> {
    let discovered = plugins::discover_plugins(profile, current_app_version());
    let settings = load_settings(profile).map_err(|message| plugins::PluginFailure {
        code: "PLUGINS_UNAVAILABLE",
        message,
    })?;
    let root = plugins::plugins_dir(profile);
    let mut out = Vec::new();
    for plugin in &discovered {
        let enabled = settings
            .snapshot
            .plugin_states
            .get(&plugin.manifest.id)
            .copied()
            .unwrap_or(false);
        if !enabled || !plugin.compatible {
            continue;
        }
        if plugins::resolve_plugin_entry(&root, plugin, target).is_none() {
            continue;
        }
        out.push(plugins::PluginRuntimeEntry {
            id: plugin.manifest.id.clone(),
            name: plugin.manifest.name.clone(),
        });
    }
    Ok(out)
}

/// Runtime source for `plugins.get_runtime_script`: `None` when the id is
/// unknown, disabled, incompatible, or the entry is undeclared,
/// unresolvable, missing, or empty (the loader skips falsy sources,
/// mirroring Electron). Hard failures (over-limit, non-UTF-8, unreadable)
/// are typed errors.
fn plugins_script_compute(
    profile: &Path,
    plugin_id: &str,
    target: plugins::PluginTarget,
) -> Result<Option<String>, plugins::PluginFailure> {
    if plugins::validate_plugin_id(plugin_id).is_err() {
        return Err(plugins::PluginFailure {
            code: "INVALID_PLUGIN_ID",
            message: "plugin id is invalid".to_owned(),
        });
    }
    let discovered = plugins::discover_plugins(profile, current_app_version());
    let Some(record) = plugins::find_plugin(&discovered, plugin_id) else {
        return Ok(None);
    };
    let settings = load_settings(profile).map_err(|message| plugins::PluginFailure {
        code: "PLUGINS_UNAVAILABLE",
        message,
    })?;
    let enabled = settings
        .snapshot
        .plugin_states
        .get(&record.manifest.id)
        .copied()
        .unwrap_or(false);
    if !enabled || !record.compatible {
        return Ok(None);
    }
    let root = plugins::plugins_dir(profile);
    plugins::read_plugin_source(&root, record, target)
}

/// Builds the page-world execution script for one retrieved bundle.
///
/// Every mod evaluates the substrate storage-context probe immediately
/// before its own bytes. Shelter additionally evaluates the host-built
/// `SHELTER_INJECTOR_PLUGINS` declaration FIRST (Electron's wrapper order),
/// so the global is in place before Shelter's boot reads it. The injector
/// declaration is static per launch (embedded artifacts); other mods pass
/// no declaration and keep the exact previous shape.
fn execution_script_for(
    descriptor: &ModDescriptor,
    bundle_js: &str,
    shelter_injector: &str,
) -> String {
    let context = mod_execution_context_script(descriptor);
    if descriptor.id == SHELTER_MOD.id {
        format!("{shelter_injector}\n{context}\n{bundle_js}")
    } else {
        format!("{context}\n{bundle_js}")
    }
}

/// Every mod the native host serves. Each entry is a thin typed descriptor plus
/// a thin renderer adapter under `native/web/mods/<id>/`; the substrate
/// mechanics in `mod_substrate.rs` are identical for all of them. Custom is
/// the user-owned member: same capability, same pipeline, but local-file
/// retrieval (`custom_bundle.rs`) instead of a fixed upstream download.
const KNOWN_MODS: &[ModDescriptor] = &[VENCORD_MOD, SHELTER_MOD, EQUICORD_MOD, CUSTOM_MOD];

/// DEV diagnostic: force a mod's adapter OFF regardless of `settings.mods`, so
/// a "native without <mod>" control run needs no profile edit.
const MOD_ADAPTER_OVERRIDE_ENV: &str = "REMCORD_NATIVE_DIAGNOSTIC_MODS";
/// DEV-only: enable the post-load mod runtime observation.
const MOD_SMOKE_ENV: &str = "REMCORD_NATIVE_MOD_SMOKE";
const SETTINGS_SMOKE_ENV: &str = "REMCORD_NATIVE_SETTINGS_SMOKE";
const SETTINGS_MUTATION_SMOKE_ENV: &str = "REMCORD_NATIVE_SETTINGS_MUTATION_SMOKE";
const SETTINGS_SET_SMOKE_ENV: &str = "REMCORD_NATIVE_SETTINGS_SET_SMOKE";
const GETLANG_SMOKE_ENV: &str = "REMCORD_NATIVE_GETLANG_SMOKE";
const APP_VERSION_SMOKE_ENV: &str = "REMCORD_NATIVE_APP_VERSION_SMOKE";
const GRAPHICS_SMOKE_ENV: &str = "REMCORD_NATIVE_GRAPHICS_SMOKE";
const PLUGIN_STORAGE_SMOKE_ENV: &str = "REMCORD_NATIVE_PLUGIN_STORAGE_SMOKE";
const THEME_INSTALL_SMOKE_ENV: &str = "REMCORD_NATIVE_THEME_INSTALL_SMOKE";
const THEME_WRITE_SMOKE_ENV: &str = "REMCORD_NATIVE_THEME_WRITE_SMOKE";
const THEME_IMPORT_SMOKE_ENV: &str = "REMCORD_NATIVE_THEME_IMPORT_SMOKE";
const THEME_SET_ENABLED_SMOKE_ENV: &str = "REMCORD_NATIVE_THEME_SET_ENABLED_SMOKE";
const THEME_LIST_SMOKE_ENV: &str = "REMCORD_NATIVE_THEME_LIST_SMOKE";
const BACKUP_SAVE_SMOKE_ENV: &str = "REMCORD_NATIVE_BACKUP_SAVE_SMOKE";
const BACKUP_RESTORE_SMOKE_ENV: &str = "REMCORD_NATIVE_BACKUP_RESTORE_SMOKE";
const SHELTER_LOADER_PROBE_ENV: &str = "REMCORD_NATIVE_LOADER_PROBE";
/// DEV-only: enable the post-marker Shelter injector observation.
const INJECTOR_PROBE_ENV: &str = "REMCORD_NATIVE_INJECTOR_PROBE";
const SETTINGS_BOOT_SMOKE_ENV: &str = "REMCORD_NATIVE_SETTINGS_BOOT_SMOKE";
const SETTINGS_UI_SMOKE_ENV: &str = "REMCORD_NATIVE_SETTINGS_UI_SMOKE";
const FOLDERS_SMOKE_ENV: &str = "REMCORD_NATIVE_FOLDERS_SMOKE";
const REMCORD_PROTOCOL_SMOKE_ENV: &str = "REMCORD_NATIVE_PROTOCOL_SMOKE";
const PERMISSIONS_SMOKE_ENV: &str = "REMCORD_NATIVE_PERMISSIONS_SMOKE";
const THEMES_SMOKE_ENV: &str = "REMCORD_NATIVE_THEMES_SMOKE";
const QUICKCSS_SMOKE_ENV: &str = "REMCORD_NATIVE_QUICKCSS_SMOKE";
const BARE_PAGE_ENV: &str = "REMCORD_NATIVE_DIAGNOSTIC_BARE_PAGE";
const INTERACTION_DIAGNOSTIC_ENV: &str = "REMCORD_NATIVE_DIAGNOSTIC_INTERACTION";
const BOOT_SNAPSHOT: BootSnapshot = BootSnapshot {
    runtime: "webview2",
    platform: "win32",
    protocol_version: PROTOCOL_VERSION,
    window: BootWindowState {
        // The window is created hidden and restored; no stored-bounds
        // restore exists in this milestone, so neither flag can be true.
        focused: false,
        maximized: false,
    },
    capabilities: &[
        "bridge.ping",
        "mod.get_bundle",
        "settings.get_config",
        "settings.set_block_power_saving_in_voice_chat",
        "settings.set",
        "settings.import_electron",
        "localization.get_lang",
        "app.get_version",
        "plugin_storage.read",
        "plugin_storage.write",
        "plugins.list",
        "plugins.set_enabled",
        "plugins.reload",
        "plugins.get_runtime_entries",
        "plugins.get_runtime_script",
        "backup.list_bundles",
        "backup.save",
        "backup.inspect",
        "backup.commit_restore",
        "backup.discard_restore",
        "folders.open",
        "folders.open_theme",
        "keybinds.list",
        "keybinds.add",
        "keybinds.toggle",
        "keybinds.remove",
        "gallery.list",
        "gallery.add_url",
        "gallery.add_link",
        "gallery.remove",
        "gallery.bulk_remove",
        "gallery.update",
        "gallery.categories",
        "gallery.category_add",
        "gallery.category_rename",
        "gallery.category_remove",
        "gallery.usage",
        "gallery.file",
        "gallery.root",
        "themes.install_code",
        "themes.write_css",
        "themes.import_files",
        "themes.set_enabled",
        "themes.list",
        "quickcss.read",
        "quickcss.write",
        "diagnostics.get_info",
        "graphics.get_info",
        "mod.clear_cache",
        "spellcheck.get_state",
        "window.show",
        "window.hide",
        "window.minimize",
        "window.maximize",
        "window.unmaximize",
        "window.drag",
        "window.toggle_fullscreen",
        "window.quit",
    ],
};
const COMPATIBILITY_SCRIPT: &str = include_str!("../web/compatibility.js");
const MOD_SUBSTRATE_SCRIPT: &str = include_str!("../web/mod-substrate/mod-bootstrap.js");
const VENCORD_ADAPTER_SCRIPT: &str = include_str!("../web/mods/vencord/adapter.js");
const SHELTER_ADAPTER_SCRIPT: &str = include_str!("../web/mods/shelter/adapter.js");
const EQUICORD_ADAPTER_SCRIPT: &str = include_str!("../web/mods/equicord/adapter.js");
const CUSTOM_ADAPTER_SCRIPT: &str = include_str!("../web/mods/custom/adapter.js");
/// Runtime plugin loader: the native counterpart of Electron's
/// `src/discord/preload/plugins.mts` (sequential preload-then-renderer
/// execution through the `plugins.get_runtime_entries` /
/// `plugins.get_runtime_script` capabilities with one uniform page-world
/// api). Always injected (see [`initialization_script`]).
const PLUGIN_LOADER_SCRIPT: &str = include_str!("../web/plugins/loader.js");
/// Non-global keybind matcher: the native counterpart of Electron's
/// menu-accelerator path for focused-app-only keybinds (F8 core). Always
/// injected (see [`initialization_script`]).
const KEYBIND_MATCHER_SCRIPT: &str = include_str!("../web/keybinds/matcher.js");

/// The renderer adapter for one registered mod, or `None` for a descriptor
/// with no adapter file (a registry entry must always carry one).
fn mod_adapter_script(descriptor: &ModDescriptor) -> Option<&'static str> {
    match descriptor.id {
        "vencord" => Some(VENCORD_ADAPTER_SCRIPT),
        "shelter" => Some(SHELTER_ADAPTER_SCRIPT),
        "equicord" => Some(EQUICORD_ADAPTER_SCRIPT),
        "custom" => Some(CUSTOM_ADAPTER_SCRIPT),
        _ => None,
    }
}

/// The `stateName` a mod's adapter publishes, used by the DEV mod observation
/// to read the same per-mod state the host already reports.
///
/// The value is read out of the adapter's own configuration at compile time,
/// so the observation can never drift from what the adapter actually sets. It
/// adds no capability and no renderer surface.
fn adapter_state_name(descriptor: &ModDescriptor) -> Option<&'static str> {
    const STATE_NAMES: &[(&str, &str)] = &[
        ("vencord", "__remcordNativeVencord"),
        ("shelter", "__remcordNativeShelter"),
        ("equicord", "__remcordNativeEquicord"),
        ("custom", "__remcordNativeCustom"),
    ];
    STATE_NAMES
        .iter()
        .find(|(id, _)| *id == descriptor.id)
        .map(|(_, name)| *name)
        .filter(|_| mod_adapter_script(descriptor).is_some())
}

/// Resolves which registered mods get their renderer adapter injected.
///
/// Two host-controlled inputs decide this, and the renderer supplies neither:
/// 1. Each descriptor's own [`ModEnabling`] policy. `SettingsMods` reproduces
///    Electron's `getConfig("mods").includes(<id>)` gate exactly; the authoritative
///    settings snapshot is read from the same place the settings capability
///    reads it, so corrupt settings fail closed to "no extra mods".
/// 2. An optional DEV diagnostic allowlist (`REMCORD_NATIVE_DIAGNOSTIC_MODS`),
///    used to run "native with X" against "native without X" control runs
///    without editing the DEV profile.
fn resolve_active_mods(
    settings_mods: &[String],
    override_list: Option<&str>,
) -> Vec<&'static ModDescriptor> {
    let allowed: Option<Vec<&str>> = override_list.map(|raw| {
        raw.split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect()
    });
    KNOWN_MODS
        .iter()
        .filter(|descriptor| {
            if let Some(allowed) = &allowed {
                if !allowed.contains(&descriptor.id) {
                    return false;
                }
            }
            match descriptor.enabling {
                ModEnabling::AlwaysEnabled => true,
                ModEnabling::SettingsMods => {
                    settings_mods.iter().any(|entry| entry == descriptor.id)
                }
            }
        })
        .collect()
}

const STORAGE_START_DIAGNOSTIC_SCRIPT: &str =
    include_str!("../web/mod-substrate/storage-preservation.js");
const CSS_INJECTION_SCRIPT: &str = r#"
(() => {
  const id = "remcord-native-prototype-css";
  let style = document.getElementById(id);
  if (!style) {
    style = document.createElement("style");
    style.id = id;
    (document.head || document.documentElement).appendChild(style);
  }
  style.textContent = ":root { --remcord-native-prototype-css: ready; }";
  window.remcord.native.reportCss(
    getComputedStyle(document.documentElement)
      .getPropertyValue("--remcord-native-prototype-css")
      .trim()
  );
})();
"#;

const BRIDGE_PROBE_SCRIPT: &str = r#"
(() => {
  const api = window.remcord;
  if (
    !api ||
    api.boot.runtime !== "webview2" ||
    api.boot.protocol_version !== 1 ||
    api.native.supports("bridge.ping") !== true
  ) {
    console.error("[Remcord native DEV] compatibility facade or boot snapshot is unavailable");
    return;
  }

  try {
    api.native.requireCapability("filesystem");
    console.error("[Remcord native DEV] unsupported filesystem capability was accepted");
    return;
  } catch (error) {
    if (error.code !== "UNSUPPORTED_CAPABILITY") {
      console.error("[Remcord native DEV] unsupported capability failed unexpectedly", error);
      return;
    }
  }

  document.documentElement.dataset.remcordNativeCompatibility = "ready";
  api.native.ping("discord-load")
    .then((result) => {
      document.documentElement.dataset.remcordNativeBridge = result.message;
      console.info("[Remcord native DEV] bridge round-trip succeeded");
    })
    .catch((error) => console.error("[Remcord native DEV] bridge round-trip failed", error));
})();
"#;

/// Renders the DEV-only mod runtime observation.
///
/// The observation reads only host-managed state (each descriptor's own
/// page-world global, managed style id, and adapter state property) and reports
/// what it found. It never requests, installs, or executes anything, so it
/// cannot be mistaken for part of the mod path.
///
/// `specs` is embedded as a JSON **string literal** whose contents are the spec
/// JSON: the script parses it at runtime, so no host value can ever be spliced
/// into the observation as code. The whole body is wrapped in a try/catch that
/// always returns a JSON string, so a settling document reports an error field
/// instead of failing the callback.
fn mod_smoke_observation_script(specs: &str) -> Result<String, serde_json::Error> {
    let encoded = serde_json::to_string(specs)?;
    Ok(format!(
        r#"(() => {{
  try {{
    const mods = JSON.parse({encoded});
    const out = {{}};
    for (const [id, spec] of Object.entries(mods)) {{
      let pageGlobalPresent = false;
      let pageGlobalMembers = null;
      let stylePresent = false;
      let styleBytes = 0;
      try {{
        const value = globalThis[spec.page_global];
        pageGlobalPresent = Boolean(value) && typeof value === "object";
        if (pageGlobalPresent && spec.requires) {{
          pageGlobalMembers = typeof value[spec.requires];
        }}
      }} catch (error) {{
        pageGlobalMembers = "unreadable";
      }}
      try {{
        if (spec.style_id) {{
          const style = document.getElementById(spec.style_id);
          if (style) {{
            stylePresent = true;
            styleBytes = (style.textContent || "").length;
          }}
        }}
      }} catch (error) {{
        styleBytes = -1;
      }}
      let state = null;
      try {{
        state = window[spec.state_name] || null;
      }} catch (error) {{
        state = null;
      }}
      out[id] = {{
        pageGlobalPresent,
        pageGlobalMembers,
        stylePresent,
        styleBytes,
        adapterStatus: state ? state.status : null,
        marker: state ? state.marker : null,
        executionCount: state ? state.executionCount : null,
        bundleVersion: state ? state.bundleVersion : null,
        bundleBytes: state ? state.bundleBytes : null,
        cssBytes: state ? state.cssBytes : null,
        retrievalMs: state ? state.retrievalMs : null,
        requestLatencyMs: state ? state.requestLatencyMs : null,
        localStorageApi: state ? state.localStorageApi : null,
        settingsState: state ? state.settingsState : null,
        error: state ? state.error : null
      }};
    }}
    return JSON.stringify(out);
  }} catch (error) {{
    return JSON.stringify({{ observationError: String((error && error.message) || error) }});
  }}
}})()"#
    ))
}

const RENDERER_HEALTH_SCRIPT: &str = r#"
(() => {
  let localStorageApi;
  try {
    localStorageApi = typeof window.localStorage?.getItem;
  } catch (error) {
    localStorageApi = `blocked:${error?.name || "unknown"}`;
  }
  return JSON.stringify({
    origin: location.origin,
    path: location.pathname,
    readyState: document.readyState,
    localStorageApi,
    localStorageWindowDescriptor: Object.getOwnPropertyDescriptor(window, "localStorage")
      ? "present"
      : "missing",
    localStoragePrototypeDescriptor: Object.getOwnPropertyDescriptor(
      Object.getPrototypeOf(window),
      "localStorage"
    )
      ? "present"
      : "missing",
    storageConstructor: typeof Storage,
    documentStartStorage: window.__remcordNativeStorageAtDocumentStart || null,
    hasBody: Boolean(document.body),
    hasAppMount: Boolean(document.getElementById("app-mount")),
    hasLoginForm: Boolean(document.querySelector('input[type="email"], input[name="email"], input[type="password"]')),
    editableCount: document.querySelectorAll('[contenteditable="true"]').length,
    messageComposerCount: document.querySelectorAll('[contenteditable="true"][role="textbox"]').length,
    buttonCount: document.querySelectorAll("button").length
  });
})()
"#;

const INTERACTION_PROBE_SCRIPT: &str = r#"
(() => {
  if (Object.hasOwn(window, "__remcordNativeInteractionProbe")) return;
  Object.defineProperty(window, "__remcordNativeInteractionProbe", { value: true });
  const report = (phase) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "interaction_probe", payload: { phase } } })
  );
  const findComposer = () => {
    return [...document.querySelectorAll('[contenteditable="true"]')].find((el) => {
      const role = el.getAttribute("role");
      const label = el.getAttribute("aria-label") || "";
      const multiline = el.getAttribute("aria-multiline");
      return role === "textbox" && (multiline === "true" || label.startsWith("Message "));
    }) || null;
  };
  const isComposerEmpty = (composer) => {
    return composer.textContent.replace(/[\uFEFF\n\r\s]/g, "") === "";
  };
  const focusComposer = () => {
    const composer = findComposer();
    if (!composer) {
      report("composer_missing");
      return;
    }
    if (!isComposerEmpty(composer)) {
      report("composer_not_empty");
      return;
    }
    composer.focus({ preventScroll: true });
    if (document.activeElement !== composer) {
      report("focus_failed");
      return;
    }
    report("composer_focused");
    composer.addEventListener("input", () => {
      report("input_observed");
      setTimeout(() => {
        document.execCommand("delete");
        if (isComposerEmpty(composer)) {
          report("character_erased");
        }
      }, 100);
    }, { once: true });
    setTimeout(() => {
      document.execCommand("insertText", false, "a");
    }, 200);
  };
  const findChannel = () => {
    return [...document.querySelectorAll('a[href^="/channels/"]')].find((link) => {
      const path = new URL(link.href).pathname.split("/");
      return link.getClientRects().length > 0 && path.length >= 4;
    });
  };
  let attempts = 0;
  const inspectPage = () => {
    if (findComposer()) {
      focusComposer();
      return;
    }
    const channel = findChannel();
    if (channel) {
      channel.click();
      report("channel_navigation_requested");
      let composerAttempts = 0;
      const tryComposer = () => {
        if (findComposer()) {
          focusComposer();
        } else if (++composerAttempts < 20) {
          setTimeout(tryComposer, 500);
        } else {
          report("composer_missing");
        }
      };
      setTimeout(tryComposer, 1000);
      return;
    }
    if (++attempts < 20) {
      setTimeout(inspectPage, 500);
    } else {
      report("channel_missing");
    }
  };
  setTimeout(inspectPage, 2000);
})();
"#;

/// DEV-only localization smoke probe (`REMCORD_NATIVE_GETLANG_SMOKE=1`).
/// Exercises the exact lookup the invite back-button caller performs:
/// `window.remcord.getLang("invite-goBackToApp")`, plus a missing key to
/// verify Electron's empty-string behavior. Reported as one typed
/// `get_lang_probe` event per document.
const GETLANG_SMOKE_SCRIPT: &str = r#"
(() => {
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "get_lang_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, lang: "", value: "", missing_is_empty: false,
    error: String(error?.message ?? error).slice(0, 256),
  });
  try {
    const getLang = window.remcord?.getLang;
    if (typeof getLang !== "function") {
      fail("localization facade missing");
      return;
    }
    const valid = getLang.call(window.remcord, "invite-goBackToApp");
    const missing = getLang.call(window.remcord, "remcord-native-missing-key-probe");
    if (!(valid instanceof Promise) || !(missing instanceof Promise)) {
      fail("localization facade is not asynchronous");
      return;
    }
    Promise.all([valid, missing]).then(
      ([value, missingValue]) => probe({
        ok: true,
        lang: "unknown",
        value: String(value).slice(0, 256),
        missing_is_empty: missingValue === "",
        error: null,
      }),
      fail,
    );
  } catch (error) {
    fail(error);
  }
})();
"#;

/// DEV-only app-version smoke probe (`REMCORD_NATIVE_APP_VERSION_SMOKE=1`).
/// Exercises the exact read the migrated update-checker caller performs:
/// `window.remcord.native.getVersion()` — the async replacement for the
/// synchronous `window.remcord.version` Electron read. Reported as one typed
/// `app_version_probe` event per document.
const APP_VERSION_SMOKE_SCRIPT: &str = r#"
(() => {
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "app_version_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, version: "",
    error: String(error?.message ?? error).slice(0, 256),
  });
  try {
    const getVersion = window.remcord?.native?.getVersion;
    if (typeof getVersion !== "function") {
      fail("app version facade missing");
      return;
    }
    const pending = getVersion.call(window.remcord.native);
    if (!(pending instanceof Promise)) {
      fail("app version facade is not asynchronous");
      return;
    }
    pending.then(
      (version) => probe({
        ok: typeof version === "string" && version.length > 0,
        version: String(version).slice(0, 64),
        error: null,
      }),
      fail,
    );
  } catch (error) {
    fail(error);
  }
})();
"#;

/// DEV-only graphics smoke probe (`REMCORD_NATIVE_GRAPHICS_SMOKE=1`).
/// Exercises the real `graphics.get_info` capability from the live document:
/// the two fixed CDP probes run through the host async assembly and the
/// bounded measured-or-explicitly-missing report resolves the facade call.
/// Reported as one typed `graphics_probe` event per document with
/// `stage: "report"`. `ok` is true exactly when the facade call resolved
/// with a shape-valid report (sections may still be `unsupported` or
/// `unavailable` — that is the measured runtime behavior, not a probe
/// failure). Device/feature summaries carry parsed GPU facts only — never
/// the raw command line, paths, or file contents.
const GRAPHICS_SMOKE_SCRIPT: &str = r#"
(() => {
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "graphics_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, stage: "report",
    browser_status: "unavailable", system_status: "unavailable",
    device_count: 0, feature_count: 0,
    device_summary: "", feature_summary: "",
    webgl_status: "unavailable", webgl_summary: "",
    error: String(error?.message ?? error).slice(0, 256),
  });
  const validStatus = (status) =>
    status === "measured" || status === "unsupported" || status === "unavailable";
  const bounded = (value, max) => String(value ?? "").slice(0, max);
  const observeWebGL = () => {
    try {
      const canvas = document.createElement("canvas");
      const gl = canvas.getContext("webgl2") || canvas.getContext("webgl");
      if (!gl) return { status: "unavailable", summary: "" };
      const debug = gl.getExtension("WEBGL_debug_renderer_info");
      const value =
        (debug && gl.getParameter(debug.UNMASKED_RENDERER_WEBGL)) || gl.getParameter(gl.RENDERER);
      if (typeof value !== "string" || value.length === 0) {
        return { status: "unavailable", summary: "" };
      }
      return { status: "measured", summary: value.slice(0, 512) };
    } catch {
      return { status: "unavailable", summary: "" };
    }
  };
  (async () => {
    try {
      const graphics = window.remcord?.graphics;
      if (typeof graphics?.getInfo !== "function") {
        fail("graphics facade missing");
        return;
      }
      const report = await graphics.getInfo();
      if (
        !report || typeof report !== "object" ||
        !validStatus(report.browser?.status) || !validStatus(report.system?.status) ||
        !Array.isArray(report.system?.devices) || !Array.isArray(report.system?.feature_status)
      ) {
        fail("graphics report shape is invalid");
        return;
      }
      const devices = report.system.devices
        .map((device) => device?.device_string ?? device?.vendor_string ?? "(unnamed)")
        .join(" | ");
      const features = report.system.feature_status
        .slice(0, 8)
        .map((entry) => `${entry?.name}=${entry?.status}`)
        .join(", ");
      const webgl = observeWebGL();
      probe({
        ok: true, stage: "report",
        browser_status: report.browser.status,
        system_status: report.system.status,
        device_count: report.system.devices.length,
        feature_count: report.system.feature_status.length,
        device_summary: bounded(devices, 1024),
        feature_summary: bounded(features, 1024),
        webgl_status: webgl.status,
        webgl_summary: webgl.summary,
        error: null,
      });
    } catch (error) {
      fail(error);
    }
  })();
})();
"#;

/// DEV-only plugin-storage smoke probe
/// (`REMCORD_NATIVE_PLUGIN_STORAGE_SMOKE=1`). Exercises the exact public
/// third-party contract (`window.remcord.pluginStorage.writeFile/readFile`,
/// the native replacement for Electron's `window.remcord.fs` over
/// `pluginWriteFile`/`pluginReadFile`): write a fixture under a temporary
/// logical plugin id, read it back, verify exact bytes, then block three
/// attack paths (traversal, absolute path, Windows drive path). Reported as
/// one typed `plugin_storage_probe` event per document with
/// `stage: "write_read"`. Never touches real user plugin namespaces.
const PLUGIN_STORAGE_SMOKE_SCRIPT: &str = r#"
(() => {
  const PLUGIN_ID = "remcordNativeProbe";
  const REL_PATH = "smoke/note.txt";
  const CONTENT = "remcord-native-plugin-storage-smoke-v1";
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "plugin_storage_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, stage: "write_read", payload_bytes: 0,
    round_trip: false, attacks_blocked: 0,
    error: String(error?.message ?? error).slice(0, 256),
  });
  const expectReject = async (pending) => {
    try {
      await pending;
      return false;
    } catch {
      return true;
    }
  };
  (async () => {
    try {
      const storage = window.remcord?.pluginStorage;
      if (typeof storage?.writeFile !== "function" || typeof storage?.readFile !== "function") {
        fail("plugin storage facade missing");
        return;
      }
      if (window.remcord.fs !== undefined) {
        fail("generic fs facade must not exist");
        return;
      }
      const bytes = await storage.writeFile(PLUGIN_ID, REL_PATH, CONTENT);
      const data = await storage.readFile(PLUGIN_ID, REL_PATH);
      const blockedTraversal = await expectReject(storage.readFile(PLUGIN_ID, "../escape.txt"));
      const blockedAbsolute = await expectReject(storage.readFile(PLUGIN_ID, "/absolute/path.txt"));
      const blockedDrive = await expectReject(storage.readFile(PLUGIN_ID, "C:\\secret.txt"));
      const attacksBlocked = [blockedTraversal, blockedAbsolute, blockedDrive].filter(Boolean).length;
      const roundTrip = data === CONTENT;
      probe({
        ok: roundTrip && attacksBlocked === 3 && bytes === CONTENT.length,
        stage: "write_read",
        payload_bytes: data.length,
        round_trip: roundTrip,
        attacks_blocked: attacksBlocked,
        error: null,
      });
    } catch (error) {
      fail(error);
    }
  })();
})();
"#;

/// DEV-only theme install smoke probe (`REMCORD_NATIVE_THEME_INSTALL_SMOKE=1`).
/// Installs one disposable fixture (`RemcordSmoke` →
/// `RemcordSmoke-BD`, disabled by Electron parity so the loader skips
/// it and nothing visual changes) through the real `themes.install_code`
/// capability, verifies the id/enabled/bytes contract, then blocks three
/// attack shapes (missing marker, traversal `@name`, oversized CSS).
/// Reported as one typed `theme_install_probe` event per document with
/// `stage: "install"`. Separately observes (never alters) whether a real
/// theme-host fetch survives the Discord CSP, reported as `fetch_observed`.
/// The disposable fixture directory is removed by hand after validation
/// (uninstall is a deferred slice).
const THEME_INSTALL_SMOKE_SCRIPT: &str = r#"
(() => {
  const FIXTURE_CSS = "/**\n * @name RemcordSmoke\n * @author Remcord Probe\n * @version 0.0.0\n */\n:root {\n  --remcord-native-install-probe: 1;\n}\n";
  const FETCH_URL = "https://raw.githubusercontent.com/BetterDiscord/themes/main/README.md";
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "theme_install_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, stage: "install", id: "", css_bytes: 0, enabled: false,
    attacks_blocked: 0, fetch_observed: "error",
    error: String(error?.message ?? error).slice(0, 256),
  });
  const expectReject = async (pending) => {
    try {
      await pending;
      return false;
    } catch {
      return true;
    }
  };
  const observeFetch = async () => {
    try {
      await fetch(FETCH_URL, { method: "GET" });
      return "allowed";
    } catch (error) {
      return String(error?.message ?? error).includes("Failed to fetch") ? "blocked" : "error";
    }
  };
  (async () => {
    try {
      const themes = window.remcord?.themes;
      if (typeof themes?.installFromCode !== "function") {
        fail("theme install facade missing");
        return;
      }
      if (typeof themes.install === "function") {
        fail("URL-taking theme install must not exist");
        return;
      }
      const installed = await themes.installFromCode(FIXTURE_CSS);
      const blockedMarker = await expectReject(themes.installFromCode(":root {}\n"));
      const blockedTraversal = await expectReject(
        themes.installFromCode("/**\n * @name ../escape\n */\n:root {}\n")
      );
      const blockedOversized = await expectReject(
        themes.installFromCode("/**\n * @name Big\n */\n" + "x".repeat(1048577))
      );
      const attacksBlocked = [blockedMarker, blockedTraversal, blockedOversized].filter(Boolean).length;
      const fetchObserved = await observeFetch();
      const contract = installed
        && installed.id === "RemcordSmoke-BD"
        && installed.enabled === false
        && installed.css_bytes === FIXTURE_CSS.length;
      probe({
        ok: Boolean(contract) && attacksBlocked === 3,
        stage: "install",
        id: installed && typeof installed.id === "string" ? installed.id : "",
        css_bytes: installed && typeof installed.css_bytes === "number" ? installed.css_bytes : 0,
        enabled: Boolean(installed && installed.enabled),
        attacks_blocked: attacksBlocked,
        fetch_observed: fetchObserved,
        error: null,
      });
    } catch (error) {
      fail(error);
    }
  })();
})();
"#;

/// DEV-only theme write smoke probe (`REMCORD_NATIVE_THEME_WRITE_SMOKE=1`).
/// Replaces the CSS of a manually pre-placed ENABLED fixture
/// (`RemcordWriteProbe`, whose `manifest.theme` is deliberately NOT
/// `src.css`) through the real `themes.write_css` capability, then proves
/// the new value is reflected via `getComputedStyle` with exactly one
/// managed style node. Writes a second value to prove repeated atomic
/// replacement converges, then blocks three attack shapes (traversal id,
/// unknown theme, oversized CSS). Reported as one typed
/// `theme_write_probe` event per document with `stage: "write"`. The
/// fixture is removed by hand after validation (uninstall is deferred).
const THEME_WRITE_SMOKE_SCRIPT: &str = r#"
(() => {
  const THEME_ID = "RemcordWriteProbe";
  const FIRST_CSS = ":root{\n  --remcord-native-write-probe: first;\n}\n";
  const SECOND_CSS = ":root{\n  --remcord-native-write-probe: second;\n}\n";
  const STYLE_ID = "remcord-native-theme-RemcordWriteProbe";
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "theme_write_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, stage: "write", id: "", css_bytes: 0, enabled: false,
    attacks_blocked: 0, reflected: false,
    error: String(error?.message ?? error).slice(0, 256),
  });
  const expectReject = async (pending) => {
    try {
      await pending;
      return false;
    } catch {
      return true;
    }
  };
  const probeVar = () => getComputedStyle(document.documentElement)
    .getPropertyValue("--remcord-native-write-probe").trim();
  const styleCount = () => document.querySelectorAll('style[id="' + STYLE_ID + '"]').length;
  const waitFor = async (condition) => {
    for (let attempt = 0; attempt < 50; attempt++) {
      if (condition()) return true;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return false;
  };
  (async () => {
    try {
      const themes = window.remcord?.themes;
      if (typeof themes?.writeCss !== "function") {
        fail("theme write facade missing");
        return;
      }
      if (typeof themes.edit === "function") {
        fail("editor-launch surface must not exist");
        return;
      }
      const first = await themes.writeCss(THEME_ID, FIRST_CSS);
      const reflectedFirst = await waitFor(() => probeVar() === "first");
      const singleFirst = styleCount() === 1;
      const second = await themes.writeCss(THEME_ID, SECOND_CSS);
      const reflectedSecond = await waitFor(() => probeVar() === "second");
      const singleSecond = styleCount() === 1;
      const blockedTraversal = await expectReject(themes.writeCss("../escape", FIRST_CSS));
      const blockedUnknown = await expectReject(themes.writeCss("remcord-native-no-such-theme", FIRST_CSS));
      const blockedOversized = await expectReject(themes.writeCss(THEME_ID, "x".repeat(1048577)));
      const attacksBlocked = [blockedTraversal, blockedUnknown, blockedOversized].filter(Boolean).length;
      const contract = first && first.id === THEME_ID && first.enabled === true
        && second && second.id === THEME_ID && second.css_bytes === SECOND_CSS.length;
      const reflected = reflectedFirst && singleFirst && reflectedSecond && singleSecond;
      probe({
        ok: Boolean(contract) && reflected && attacksBlocked === 3,
        stage: "write",
        id: second && typeof second.id === "string" ? second.id : "",
        css_bytes: second && typeof second.css_bytes === "number" ? second.css_bytes : 0,
        enabled: Boolean(second && second.enabled),
        attacks_blocked: attacksBlocked,
        reflected: reflected,
        error: null,
      });
    } catch (error) {
      fail(error);
    }
  })();
})();
"#;

/// DEV-only theme import smoke probe (`REMCORD_NATIVE_THEME_IMPORT_SMOKE=1`).
/// Opens the REAL host picker through the `themes.import_files` capability
/// and reports the bounded summary as one typed `theme_import_probe` event
/// per document with `stage: "import"`. Completion installs through the
/// shared installer (the existing watcher observes the files); dismissal
/// resolves `{ canceled: true }` and the probe reports the silent path.
/// Unlike the install/write probes there is no `attacks_blocked`: the facade
/// takes no input, so there is no client-side attack shape to block. Any
/// installed fixture directory is removed by hand after validation
/// (uninstall is a deferred slice).
const THEME_IMPORT_SMOKE_SCRIPT: &str = r#"
(() => {
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "theme_import_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, stage: "import", canceled: false, imported: 0, failed: 0,
    error: String(error?.message ?? error).slice(0, 256),
  });
  (async () => {
    try {
      const themes = window.remcord?.themes;
      if (typeof themes?.importFiles !== "function") {
        fail("theme import facade missing");
        return;
      }
      // The picker is host-owned: no picker-send, path, URL, filter, or
      // dialog-option surface may exist alongside the capability.
      if (typeof themes.openImportPicker === "function") {
        fail("picker send must not exist");
        return;
      }
      // Opens the real host picker; user completion or dismissal resolves.
      // Cancellation is control flow ({ canceled: true }), never a reject.
      const summary = await themes.importFiles();
      const contract = summary
        && typeof summary.canceled === "boolean"
        && Number.isSafeInteger(summary.imported) && summary.imported >= 0
        && Number.isSafeInteger(summary.failed) && summary.failed >= 0;
      probe({
        ok: Boolean(contract),
        stage: "import",
        canceled: Boolean(summary && summary.canceled),
        imported: summary && Number.isSafeInteger(summary.imported) ? summary.imported : 0,
        failed: summary && Number.isSafeInteger(summary.failed) ? summary.failed : 0,
        error: null,
      });
    } catch (error) {
      fail(error);
    }
  })();
})();
"#;

/// DEV-only theme enable smoke probe (`REMCORD_NATIVE_THEME_SET_ENABLED_SMOKE=1`).
/// Flips a disposable theme's flag through the real `themes.set_enabled`
/// capability, proves the managed style node appears carrying the exact
/// installed CSS bytes, proves a same-state flip changes nothing, installs
/// and enables a second disposable theme to prove coexistence (no mutual
/// exclusion), unflips and proves removal, then blocks three attack shapes
/// (traversal id, unknown theme, non-boolean flag). Reported as one typed
/// `theme_set_enabled_probe` event per document with `stage: "set_enabled"`.
/// Both disposable fixture directories are removed by hand after validation
/// (uninstall is a deferred slice).
const THEME_SET_ENABLED_SMOKE_SCRIPT: &str = r#"
(() => {
  const THEME_ID = "RemcordEnableProbe-BD";
  const THEME_CSS = "/**\n * @name RemcordEnableProbe\n * @version 0.0.0\n */\n:root {\n  --remcord-native-enable-probe: 1;\n}\n";
  const SECOND_ID = "RemcordSecondProbe-BD";
  const SECOND_CSS = "/**\n * @name RemcordSecondProbe\n * @version 0.0.0\n */\n:root {\n  --remcord-native-second-probe: 2;\n}\n";
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "theme_set_enabled_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, stage: "set_enabled", id: "", enabled: false,
    attacks_blocked: 0, reflected: false, multi: false, clearvision: "failed",
    error: String(error?.message ?? error).slice(0, 256),
  });
  const expectReject = async (pending) => {
    try {
      await pending;
      return false;
    } catch {
      return true;
    }
  };
  const styleId = (id) => "remcord-native-theme-" + id;
  const nodeBytes = (id) => {
    const node = document.getElementById(styleId(id));
    return node ? (node.textContent || "").length : -1;
  };
  const probeVar = () => getComputedStyle(document.documentElement)
    .getPropertyValue("--remcord-native-enable-probe").trim();
  const singleNode = (id) => document.querySelectorAll('style[id="' + styleId(id) + '"]').length === 1;
  const waitFor = async (condition) => {
    for (let attempt = 0; attempt < 50; attempt++) {
      if (condition()) return true;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return false;
  };
  (async () => {
    try {
      const themes = window.remcord?.themes;
      if (typeof themes?.setEnabled !== "function") {
        fail("theme enable facade missing");
        return;
      }
      // No generic set surface may exist alongside the typed flip.
      if (typeof themes.set === "function") {
        fail("untyped theme set must not exist");
        return;
      }
      // The observed-problem fixture: enable the real imported theme when
      // present, prove its managed node appears, then restore the found
      // state. Absent (UNKNOWN_THEME) skips without failing.
      const CLEARVISION_ID = "ClearVision-BD";
      let clearvision = "absent";
      try {
        const cvOn = await themes.setEnabled(CLEARVISION_ID, true);
        const cvSeen = await waitFor(() => nodeBytes(CLEARVISION_ID) > 0)
          && singleNode(CLEARVISION_ID);
        const cvOff = await themes.setEnabled(CLEARVISION_ID, false);
        const cvGone = await waitFor(() => nodeBytes(CLEARVISION_ID) === -1);
        clearvision = (cvOn.enabled && cvSeen && !cvOff.enabled && cvGone)
          ? "reflected"
          : "failed";
      } catch (error) {
        clearvision = /unknown/i.test(String(error?.message ?? error)) ? "absent" : "failed";
      }
      // Fixtures install disabled (Electron parity); the flips below are
      // the only state changes. Both directories are removed by hand after
      // validation (uninstall is deferred).
      await themes.installFromCode(THEME_CSS);
      const turnedOn = await themes.setEnabled(THEME_ID, true);
      // Semantic proof: the custom property is actually reflected in the
      // rendered document, carried by exactly one managed style node.
      const reflected = await waitFor(() => probeVar() === "1")
        && singleNode(THEME_ID) && nodeBytes(THEME_ID) > 0;
      // Same-state flip: resolves the state without side effects.
      const repeat = await themes.setEnabled(THEME_ID, true);
      const idempotent = repeat && repeat.id === THEME_ID && repeat.enabled === true;
      // A second enabled theme coexists: no mutual exclusion.
      await themes.installFromCode(SECOND_CSS);
      const secondOn = await themes.setEnabled(SECOND_ID, true);
      const multi = await waitFor(() => nodeBytes(SECOND_ID) > 0)
        && probeVar() === "1" && singleNode(THEME_ID)
        && secondOn && secondOn.enabled === true;
      const turnedOff = await themes.setEnabled(THEME_ID, false);
      const removed = await waitFor(() => probeVar() === "" && nodeBytes(THEME_ID) === -1)
        && turnedOff && turnedOff.enabled === false;
      // Second fixture stays enabled (its directory is removed by hand
      // after validation, like every disposable fixture).
      const blockedTraversal = await expectReject(themes.setEnabled("../escape", true));
      const blockedUnknown = await expectReject(themes.setEnabled("remcord-native-no-such-theme", true));
      const blockedFlag = await expectReject(themes.setEnabled(THEME_ID, "yes"));
      const attacksBlocked = [blockedTraversal, blockedUnknown, blockedFlag].filter(Boolean).length;
      const contract = turnedOn && turnedOn.id === THEME_ID && turnedOn.enabled === true;
      probe({
        ok: Boolean(contract) && reflected && idempotent && multi && removed
          && clearvision !== "failed" && attacksBlocked === 3,
        stage: "set_enabled",
        id: turnedOff && typeof turnedOff.id === "string" ? turnedOff.id : "",
        enabled: Boolean(turnedOff && turnedOff.enabled),
        attacks_blocked: attacksBlocked,
        reflected: reflected,
        multi: multi,
        clearvision: clearvision,
        error: null,
      });
    } catch (error) {
      fail(error);
    }
  })();
})();
"#;

/// DEV-only theme list smoke probe (`REMCORD_NATIVE_THEME_LIST_SMOKE=1`).
/// Inventories the host themes root through the real `themes.list`
/// capability and verifies the bounded display shape (array, entry keys,
/// id pattern, string bounds, boolean state), then reports the count plus
/// the observed-problem fixture state (`present-enabled`/`present-disabled`/
/// `absent`) as one typed `theme_list_probe` event per document with
/// `stage: "list"`. The probe never carries CSS contents or host paths.
const THEME_LIST_SMOKE_SCRIPT: &str = r#"
(() => {
  const CLEARVISION_ID = "ClearVision-BD";
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "theme_list_probe", payload } })
  );
  const fail = (clearvision, error) => probe({
    ok: false, stage: "list", count: 0, clearvision: clearvision,
    error: String(error?.message ?? error).slice(0, 256),
  });
  (async () => {
    try {
      const themes = window.remcord?.themes;
      if (typeof themes?.listThemes !== "function") {
        fail("failed", "theme list facade missing");
        return;
      }
      // The inventory is metadata-only: no list/getter surface for CSS,
      // paths, or the filesystem may exist alongside the capability.
      if (typeof themes.getThemes === "function" || typeof themes.getCSS === "function") {
        fail("failed", "metadata-only surface exceeded");
        return;
      }
      const listed = await themes.listThemes();
      if (!Array.isArray(listed)) {
        fail("failed", "inventory is not an array");
        return;
      }
      const entry = listed.find((item) => item && item.id === CLEARVISION_ID);
      const clearvision = !entry
        ? "absent"
        : entry.enabled === true
          ? "present-enabled"
          : entry.enabled === false
            ? "present-disabled"
            : "failed";
      probe({
        ok: clearvision !== "failed",
        stage: "list",
        count: listed.length,
        clearvision: clearvision,
        error: null,
      });
    } catch (error) {
      fail("failed", error);
    }
  })();
})();
"#;

/// DEV-only backup save probe (`REMCORD_NATIVE_BACKUP_SAVE_SMOKE=1`).
/// Runs the REAL `backup.save` capability with a fixed all-includes payload
/// and a small synthetic clientMods envelope through the host dialog; user
/// completion or dismissal resolves the bounded outcome as one typed
/// `backup_save_probe` event per document with `stage: "save"`. Like the
/// theme import probe there is no `attacks_blocked`: the payload is fixed,
/// never user data. The probe never carries settings, themes, paths, URLs,
/// or client-mod contents — only stage strings, flags, and byte counts.
const BACKUP_SAVE_SMOKE_SCRIPT: &str = r#"
(() => {
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "backup_save_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, stage: "save", canceled: false, bytes: 0,
    error: String(error?.message ?? error).slice(0, 256),
  });
  (async () => {
    try {
      const backup = window.remcord?.backup;
      if (typeof backup?.save !== "function") {
        fail("backup save facade missing");
        return;
      }
      // Fixed probe payload: every section on, small synthetic envelope.
      // Cancellation is control flow ({ ok: false, error: "CANCELLED" }),
      // never a rejection.
      const payload = JSON.stringify({
        includes: {
          remcordConfig: true,
          remcordThemesAndQuickCss: true,
          remcordExtensionPlugins: true,
          vencordModData: true,
          equicordModData: true,
          shelterModData: true,
          modBundles: true,
        },
        clientMods: {
          vencordLocalStorage: "{\"probe\":1}",
          equicordLocalStorage: null,
          shelter: { plugins: {}, enabledPlugins: [] },
        },
      });
      const result = await backup.save(payload);
      if (!result || typeof result.ok !== "boolean") {
        fail("backup save result shape is invalid");
        return;
      }
      if (result.ok === true) {
        probe({ ok: true, stage: "save", canceled: false, bytes: 0, error: null });
      } else if (result.error === "CANCELLED") {
        probe({ ok: true, stage: "save", canceled: true, bytes: 0, error: null });
      } else {
        fail(result.error || "backup save failed");
      }
    } catch (error) {
      fail(error);
    }
  })();
})();
"#;

/// DEV-only backup restore probe (`REMCORD_NATIVE_BACKUP_RESTORE_SMOKE=1`).
/// Opens the REAL host picker through the `backup.inspect` capability,
/// previews, then commits through `backup.commit_restore`, and reports the
/// bounded outcome as one typed `backup_restore_probe` event per document
/// with `stage: "restore"`. Completion validates, stages, previews,
/// commits, and reports the clientMods byte size (the host never applies
/// clientMods — no localStorage write, no remote-plugin fetch, no script
/// execution); dismissal resolves `{ canceled: true }` and the probe
/// reports the silent path without committing anything. Like the save probe
/// there is no `attacks_blocked`: the facade takes no input, so there is no
/// client-side attack shape to block. The probe never carries settings,
/// themes, paths, URLs, or client-mod contents — only stage strings, flags,
/// and byte counts.
const BACKUP_RESTORE_SMOKE_SCRIPT: &str = r#"
(() => {
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "backup_restore_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, stage: "restore", canceled: false, client_mods_bytes: 0,
    error: String(error?.message ?? error).slice(0, 256),
  });
  (async () => {
    try {
      const backup = window.remcord?.backup;
      if (typeof backup?.inspect !== "function" || typeof backup?.commitRestore !== "function") {
        fail("backup inspect/commit facade missing");
        return;
      }
      // Parameter-free by protocol: no archive path, filename, URL,
      // filter, or dialog option may exist alongside the capabilities.
      if (backup.inspect.length !== 0 || backup.commitRestore.length !== 0) {
        fail("inspect/commit facades must take no arguments");
        return;
      }
      // Opens the real host picker and stages; user completion or
      // dismissal resolves. Cancellation is control flow, never a
      // rejection, and commits nothing.
      const inspected = await backup.inspect();
      if (!inspected || typeof inspected.canceled !== "boolean") {
        fail("backup inspect result shape is invalid");
        return;
      }
      if (inspected.canceled === true) {
        probe({ ok: true, stage: "restore", canceled: true, client_mods_bytes: 0, error: null });
        return;
      }
      // Preview-then-commit: the commit applies exactly the held staged
      // bytes the preview described. The host never applies clientMods:
      // the probe reports the envelope size only.
      const result = await backup.commitRestore();
      if (!result || typeof result.ok !== "boolean") {
        fail("backup commit result shape is invalid");
        return;
      }
      if (result.ok === true) {
        const envelope = JSON.stringify(result.clientMods ?? null);
        probe({ ok: true, stage: "restore", canceled: false, client_mods_bytes: envelope.length, error: null });
      } else if (result.error === "CANCELLED") {
        probe({ ok: true, stage: "restore", canceled: true, client_mods_bytes: 0, error: null });
      } else {
        fail(result.error || "backup restore failed");
      }
    } catch (error) {
      fail(error);
    }
  })();
})();
"#;

/// DEV-only Shelter plugin-loader probe (`REMCORD_NATIVE_LOADER_PROBE=1`).
/// Verifies the exact upstream `shelter.plugins` entry points a future
/// native settings-plugin loader needs — `addLocalPlugin`, `startPlugin`,
/// `stopPlugin`, `removePlugin` — and asserts the URL-based
/// `ensureLoaderPlugin` is NOT exposed on the public plugins object (it is
/// omitted upstream, so no URL-selected loading surface is reachable).
/// It then round-trips one synthetic plugin with host-fixed id and source
/// (`remcord-loader-probe`, `({onLoad(){}})`) through add/start/stop/remove
/// and proves removal behaviorally (a clean registry rejects a fresh start).
/// The throw-chain reads no reactive store: upstream throws on taken,
/// missing, or already-running ids, so a zero-throw run verifies the cycle
/// against live state without relying on the synchronous freshness of
/// Shelter's deferred store getters. No settings code executes, no renderer
/// input is accepted (id, source, and manifest are literals in this fixed
/// script), nothing is fetched, and the payload carries only stage strings
/// and booleans — never plugin source, paths, or URLs. The single eval in
/// the chain is upstream Shelter's own plugin executor over these fixed
/// bytes. One typed `shelter_loader_probe` event per evaluation: the probe runs
/// at each trusted page load and again right after Shelter's own bootstrap
/// marker (Shelter publishes its global asynchronously, so the page-load run
/// may report presence-false while the post-marker run is authoritative).
/// Both runs are fully synchronous and idempotent (a leftover probe id is
/// removed first), so they can neither overlap nor duplicate the synthetic
/// plugin.
const SHELTER_LOADER_PROBE_SCRIPT: &str = r#"
(() => {
  const PROBE_ID = "remcord-loader-probe";
  const PROBE_JS = "({onLoad(){}})";
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "shelter_loader_probe", payload } })
  );
  // The throw-chain below is self-verifying and reads no reactive store:
  // upstream throws when an id is taken (add), missing (start/stop/remove),
  // or already running (start), so a zero-throw run proves the full cycle
  // against live state. Store snapshots are deliberately NOT read: Shelter
  // serves them through deferred reactive getters whose synchronous
  // freshness inside one evaluation cannot be relied on.
  const fail = (stage, flags, error) => probe({
    ok: false, stage,
    shelter: !!flags.shelter, plugins: !!flags.plugins,
    add_local_plugin: !!flags.addLocalPlugin, start_plugin: !!flags.startPlugin,
    stop_plugin: !!flags.stopPlugin, remove_plugin: !!flags.removePlugin,
    url_loader_absent: !!flags.urlLoaderAbsent, synthetic: "skipped",
    error: String(error?.message ?? error).slice(0, 256),
  });
  try {
    const shelter = window.shelter;
    const plugins = shelter && typeof shelter === "object" ? shelter.plugins : undefined;
    const flags = {
      shelter: !!shelter,
      plugins: !!plugins,
      addLocalPlugin: typeof plugins?.addLocalPlugin === "function",
      startPlugin: typeof plugins?.startPlugin === "function",
      stopPlugin: typeof plugins?.stopPlugin === "function",
      removePlugin: typeof plugins?.removePlugin === "function",
      urlLoaderAbsent: typeof plugins?.ensureLoaderPlugin === "undefined",
    };
    if (!flags.shelter || !flags.plugins || !flags.addLocalPlugin || !flags.startPlugin
        || !flags.stopPlugin || !flags.removePlugin || !flags.urlLoaderAbsent) {
      fail("presence", flags, "loader surface unavailable");
      return;
    }
    try {
      // Clear any leftover id first: remove throws only when absent.
      try {
        plugins.removePlugin(PROBE_ID);
      } catch {
        // Not present: nothing to clear.
      }
      plugins.addLocalPlugin(PROBE_ID, {
        js: PROBE_JS, manifest: { name: "Remcord Loader Probe" }, update: false,
      });
      plugins.startPlugin(PROBE_ID);
      plugins.stopPlugin(PROBE_ID);
      plugins.removePlugin(PROBE_ID);
      // Prove removal behaviorally: a truly clean registry rejects a fresh
      // start, while a resident entry would restart cleanly (restored right
      // after, best effort).
      let residue = false;
      try {
        plugins.startPlugin(PROBE_ID);
        residue = true;
        try {
          plugins.stopPlugin(PROBE_ID);
        } catch {
          // Best effort: leave no running instance behind.
        }
        try {
          plugins.removePlugin(PROBE_ID);
        } catch {
          // Best effort: leave no registry entry behind.
        }
      } catch {
        residue = false;
      }
      if (residue) throw new Error("synthetic plugin residue remains");
    } catch (error) {
      fail("synthetic", flags, error);
      return;
    }
    probe({
      ok: true, stage: "cleanup",
      shelter: true, plugins: true,
      add_local_plugin: true, start_plugin: true, stop_plugin: true, remove_plugin: true,
      url_loader_absent: true, synthetic: "verified", error: null,
    });
  } catch (error) {
    fail("presence", {}, error);
  }
})();
"#;

/// DEV-only Shelter injector observation (`REMCORD_NATIVE_INJECTOR_PROBE=1`).
/// Reads back the two facts the injector path guarantees, after Shelter's
/// own bootstrap marker proves the global exists: whether the host
/// declaration was present ahead of the bundle, and whether the rpc plugin
/// installed its page-world marker (`window.remcordRPC` in `onLoad`).
/// Read-only (no registration, no start, no mutation); the returned string
/// is logged by the host, never bridged. The power plugin has no
/// page-world marker of its own — its registration is proven by the
/// declaration unit tests plus the absence of injector load errors (a
/// malformed map aborts Shelter's boot loop, failing the marker itself).
const INJECTOR_PROBE_SCRIPT: &str = r#"(()=>{ try {
  const declared = typeof SHELTER_INJECTOR_PLUGINS !== "undefined";
  const rpc = typeof window.remcordRPC !== "undefined" && window.remcordRPC !== null;
  return "injector-probe:declared-" + declared + " rpc-started-" + rpc;
} catch (error) { return "injector-probe:error " + String(error && error.message || error).slice(0, 128); } })()"#;

/// DEV-only settings boot snapshot probe (`REMCORD_NATIVE_SETTINGS_BOOT_SMOKE=1`).
/// Observes the live `window.remcord.settingsBoot` surface in the document:
/// exactly the seven approved members, the `settingsSource` enum consistent
/// with a null/non-null config, array/object shapes, both narrow functions
/// present, and deep-frozen state. One typed `settings_boot_probe` event per
/// evaluation; counts and byte lengths only, never settings values, theme
/// manifests, translations, paths, or URLs.
///
/// This probe runs at page load, so it proves presence and shape in the live
/// document — not the before-plugin ordering. Ordering is proven by
/// construction (the snapshot is spliced into the same document-start script
/// BEFORE the mod adapters; see `initialization_script` and its ordering
/// unit test): anything the closed loader could execute already runs after
/// the snapshot exists.
const SETTINGS_BOOT_SMOKE_SCRIPT: &str = r#"
(() => {
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "settings_boot_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, settings_source: "unavailable", settings_bytes: 0,
    themes_count: 0, i18n_keys: 0, lang: "", frozen: false,
    keys_exact: false, snapshot_bytes: 0,
    error: String(error?.message ?? error).slice(0, 256),
  });
  try {
    const boot = window.remcord?.settingsBoot;
    if (!boot || typeof boot !== "object") {
      fail("settingsBoot surface missing");
      return;
    }
    const keys_exact = Object.keys(boot).sort().join(",")
      === "i18n,lang,restart,setLang,settings,settingsSource,themes";
    const settings_ok = (boot.settingsSource === "unavailable" && boot.settings === null)
      || ((boot.settingsSource === "file" || boot.settingsSource === "defaults")
        && boot.settings !== null && typeof boot.settings === "object");
    const themes_ok = Array.isArray(boot.themes);
    const i18n_ok = boot.i18n !== null && typeof boot.i18n === "object";
    const fns_ok = typeof boot.setLang === "function" && typeof boot.restart === "function";
    const lang_ok = typeof boot.lang === "string" && boot.lang.length > 0;
    if (!keys_exact || !settings_ok || !themes_ok || !i18n_ok || !fns_ok || !lang_ok) {
      fail("settingsBoot shape mismatch");
      return;
    }
    const frozen = Object.isFrozen(boot)
      && (boot.settings === null || Object.isFrozen(boot.settings))
      && Object.isFrozen(boot.themes) && Object.isFrozen(boot.i18n);
    if (!frozen) {
      fail("settingsBoot is not frozen");
      return;
    }
    const settings_text = JSON.stringify(boot.settings) || "";
    const snapshot_text = JSON.stringify({ s: boot.settings, t: boot.themes, i: boot.i18n }) || "";
    probe({
      ok: true, settings_source: boot.settingsSource, settings_bytes: settings_text.length,
      themes_count: boot.themes.length, i18n_keys: Object.keys(boot.i18n).length,
      lang: String(boot.lang).slice(0, 32), frozen: true, keys_exact: true,
      snapshot_bytes: snapshot_text.length, error: null,
    });
  } catch (error) {
    fail(error);
  }
})();
"#;

/// DEV-only settings UI workflow probe (`REMCORD_NATIVE_SETTINGS_UI_SMOKE=1`).
/// Drives the real executing settings UI end to end in the page: opens Discord
/// user settings via the gear button, asserts the Remcord section registered
/// (proving the plugin did not unload on missing sync APIs), opens the Themes
/// page, enables ClearVision (asserting the persisted disabled baseline
/// first), asserts the managed style node appears, enables a second fixture
/// theme simultaneously, disables ClearVision (asserting removal), clicks the
/// import picker (open-only: the host modal dialog cannot be driven
/// headlessly) and the per-card folder action (the host log proves the open),
/// then reloads and verifies persistence before restoring the DEV profile to
/// its original disabled state.
///
/// Staged `settings_ui_probe` events carry step/counts/flags only — never
/// settings values, manifests, translations, paths, or URLs. Bounded retries
/// per step (this is DEV instrumentation, like the substrate's own bounded
/// poll — shipped code has no polling). A stage failure stops the workflow
/// with the exact step named; it never builds more bridge surface to proceed.
const SETTINGS_UI_PROBE_TEMPLATE: &str = r#"
(() => {
  const STAGE_KEY = "remcord-settings-ui-stage";
  const NONCE_KEY = "remcord-settings-ui-nonce";
  const RUN_NONCE = "__REMCORD_SETTINGS_UI_RUN_NONCE__";
  const probe = (stage, ok, detail, error) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "settings_ui_probe", payload: { stage, ok, detail: String(detail || "").slice(0, 512), error: error ? String((error && error.message) || error).slice(0, 256) : null } } })
  );
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const visible = (el) => !!el && el.getClientRects().length > 0;
  const themeStyleCount = () => document.querySelectorAll('style[id^="remcord-native-theme-"]').length;
  const themeStylePresent = (id) => !!document.getElementById("remcord-native-theme-" + id);
  function findGear() {
    // Exact label only: anything fuzzier grabs Server/Channel Settings
    // lookalikes whose clicks never open the User Settings modal.
    const direct = document.querySelector('button[aria-label="User Settings"]');
    return visible(direct) ? direct : null;
  }
  function findText(scope, text) {
    const all = (scope || document).querySelectorAll("div, span, button, h1, h2, h3, header");
    for (const el of all) {
      if (!visible(el)) continue;
      const own = [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent.trim()).join(" ");
      if (own === text) return el;
    }
    return null;
  }
  function cardFor(name) {
    // Nearest ancestor holding the theme's switch AND its action buttons
    // (the full card). A narrower scope (e.g. the title row, which holds the
    // switch but no buttons) works for toggles yet starves the folder step,
    // so the climb prefers the scope with both and falls back to switch-only.
    const label = findText(document, name);
    if (!label) return null;
    let el = label;
    let fallback = null;
    for (let depth = 0; depth < 10 && el && el !== document.body; depth++) {
      el = el.parentElement;
      if (!el) break;
      const hasSwitch = !!el.querySelector('input[type="checkbox"], [role="switch"]');
      const hasButtons = el.querySelectorAll("button").length > 0;
      if (!fallback && hasSwitch) fallback = el;
      if (hasSwitch && hasButtons) return el;
    }
    return fallback;
  }
  function describeCard(name) {
    try {
      const matches = findAllText(document, name);
      const card = cardFor(name);
      if (!card) return "matches=" + matches.length + " card=none";
      const html = (card.innerHTML || "").slice(0, 200).replace(/\s+/g, " ");
      return "matches=" + matches.length + " tag=" + card.tagName + " class=" + (card.className || "").toString().slice(0, 80)
        + " inputs=" + card.querySelectorAll("input").length
        + " buttons=" + card.querySelectorAll("button").length
        + " html=" + html;
    } catch (error) { return "describe-failed"; }
  }
  function switchOf(card) {
    return card.querySelector('input[type="checkbox"], [role="switch"]');
  }
  function switchChecked(sw) {
    if (!sw) return null;
    if (typeof sw.checked === "boolean") return sw.checked;
    const aria = sw.getAttribute("aria-checked");
    return aria === "true" ? true : aria === "false" ? false : null;
  }
  async function waitFor(fn, tries, gapMs) {
    for (let i = 0; i < tries; i++) {
      const value = fn();
      if (value) return value;
      await sleep(gapMs);
    }
    return null;
  }
  function countText(scope, text) {
    return findAllText(scope, text).length;
  }
  function findAllText(scope, text) {
    const found = [];
    const all = (scope || document).querySelectorAll("div, span, button, h1, h2, h3, header");
    for (const el of all) {
      if (!visible(el)) continue;
      const own = [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent.trim()).join(" ");
      if (own === text) found.push(el);
    }
    return found;
  }
  // Open the Themes page with a fresh nav lookup per attempt: a previously
  // found node can detach on a Solid re-render, and re-clicking a detached
  // node is a silent no-op. Several visible "Themes" texts can coexist (nav
  // item, headers, Discord's own labels), and only the nav item navigates,
  // so every candidate is tried in document order until the page header
  // appears.
  async function openThemesPage() {
    for (let round = 0; round < 3; round++) {
      const candidates = findAllText(document, "Themes");
      for (const item of candidates) {
        item.click();
        const header = await waitFor(() => findText(document, "Installed Themes"), 3, 500);
        if (header) return true;
      }
      await sleep(500);
    }
    return false;
  }
  // State-aware toggle: click only when the switch is not already in the
  // wanted state, so re-runs and mid-run states converge instead of
  // inverting. Returns whether the wanted state holds afterwards.
  async function ensureSwitch(card, want) {
    for (let attempt = 0; attempt < 3; attempt++) {
      const current = switchChecked(switchOf(card));
      if (current === want) return true;
      if (current === null || current === undefined) return false;
      switchOf(card).click();
      const settled = await waitFor(
        () => (switchChecked(switchOf(card)) === want ? true : null), 20, 500);
      if (settled) return true;
    }
    return switchChecked(switchOf(card)) === want;
  }
  async function runWorkflow() {
    probe("begin", true, "nodes=" + themeStyleCount());
    const shelter = await waitFor(() => (window.shelter && window.shelter.plugins ? true : null), 60, 500);
    if (!shelter) { probe("shelter", false, "", "shelter plugins unavailable"); return; }
    // Recon: compact button diagnostics plus the shelter settings keys, so
    // the activation step can be diagnosed without guessing upstream APIs.
    // The gear lookup below requires the EXACT "User Settings" label: a fuzzy
    // /settings/i match grabs lookalikes (Server/Channel Settings) whose
    // clicks never open the User Settings modal.
    const recon = (() => {
      try {
        const labels = [...document.querySelectorAll("[aria-label]")]
          .map((el) => el.getAttribute("aria-label") || "")
          .filter((label) => /settings/i.test(label))
          .slice(0, 5)
          .join("|");
        const exact = document.querySelector('button[aria-label="User Settings"]');
        const settingsKeys = window.shelter && window.shelter.settings
          ? Object.keys(window.shelter.settings).join(",")
          : "none";
        return "labels=" + (labels || "none")
          + " exact=" + (!!exact && exact.getClientRects().length > 0)
          + " href=" + location.pathname
          + " state=" + document.readyState
          + " shelterSettings=" + settingsKeys.slice(0, 120);
      } catch (error) { return "recon-failed"; }
    })();
    const gear = await waitFor(findGear, 40, 500);
    if (!gear) { probe("gear", false, recon, "settings gear missing"); return; }
    probe("gear", true, recon + " tag=" + gear.tagName + " role=" + gear.getAttribute("role"));
    // Activation attempts in order: direct click, full mouse sequence, then
    // keyboard activation. Each attempt re-queries the button: a previously
    // found node can detach on a Discord re-render, and clicking a detached
    // node is a silent no-op. Report which method (if any) opens the modal.
    // The modal is detected by its dialog layer (Discord restores the last
    // settings section, so the default "My Account" header is NOT always
    // showing): open = a visible dialog containing "My Account" or "Remcord".
    function settingsDialog() {
      const dialogs = [...document.querySelectorAll('[role="dialog"], [role="alertdialog"]')].filter(visible);
      return dialogs[dialogs.length - 1] || null;
    }
    const modalOpen = () => {
      const dlg = settingsDialog();
      if (!dlg) return false;
      return !!findText(dlg, "My Account") || !!findText(dlg, "Remcord");
    };
    let openedBy = null;
    let disabledSeen = false;
    const tryClick = async () => {
      const g = findGear();
      if (!g) return false;
      if (g.disabled) disabledSeen = true;
      g.click();
      return await waitFor(modalOpen, 6, 500);
    };
    if (await tryClick()) openedBy = "click";
    if (!openedBy) {
      const g = findGear();
      if (g) {
        if (g.disabled) disabledSeen = true;
        const rect = g.getClientRects()[0];
        const at = rect ? { clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 } : {};
        for (const type of ["pointerdown", "mousedown", "pointerup", "mouseup"]) {
          g.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, ...at }));
        }
        g.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, ...at }));
        if (await waitFor(modalOpen, 6, 500)) openedBy = "sequence";
      }
    }
    if (!openedBy) {
      const g = findGear();
      if (g) {
        if (g.disabled) disabledSeen = true;
        g.focus();
        g.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", bubbles: true, cancelable: true }));
        g.dispatchEvent(new KeyboardEvent("keyup", { key: "Enter", code: "Enter", bubbles: true, cancelable: true }));
        if (await waitFor(modalOpen, 6, 500)) openedBy = "keyboard";
      }
    }
    // Verify the modal genuinely opened: "Remcord" alone could match an
    // unrelated surface (toast, chat text). Discord's default settings page
    // header "My Account" proves the User Settings modal is showing.
    if (!openedBy) {
      // Forensics: is there a modal layer at all (opened elsewhere?), how
      // many account texts, total buttons - distinguishes dead clicks from a
      // renamed/moved settings surface.
      const forensics = (() => {
        try {
          const dialogs = [...document.querySelectorAll('[role="dialog"], [role="alertdialog"]')]
            .filter(visible).length;
          const layers = [...document.querySelectorAll("div")]
            .filter((el) => visible(el) && /layer|modal|backdrop/i.test(el.className || "")).length;
          return " dialogs=" + dialogs + " layers=" + layers
            + " myAccount=" + countText(document, "My Account")
            + " buttons=" + [...document.querySelectorAll("button")].filter(visible).length;
        } catch (error) { return " forensics-failed"; }
      })();
      probe("open", false, "disabledSeen=" + disabledSeen + " remcord=" + !!findText(document, "Remcord") + forensics, "settings modal did not open");
      return;
    }
    probe("open", true, "settings-ui-open via=" + openedBy);
    const remcord = await waitFor(() => findText(document, "Remcord"), 10, 500);
    if (!remcord) { probe("open", false, "via=" + openedBy, "Remcord section missing"); return; }
    const opened = await openThemesPage();
    if (!opened) {
      probe("themes", false, "navThemes=" + countText(document, "Themes") + " remcord=" + !!findText(document, "Remcord"), "Installed Themes missing");
      return;
    }
    probe("themes", true, "themes-page-open");
    const card = await waitFor(() => cardFor("ClearVision"), 20, 500);
    const card2 = cardFor("ProbeSecond");
    probe("cards", !!(card && card2), "clearvision=" + !!card + " second=" + !!card2 + " " + describeCard("ClearVision"),
      card && card2 ? null : "theme cards missing");
    if (!card || !card2) return;
    const before = switchChecked(switchOf(card));
    const on = await ensureSwitch(card, true);
    const node = await waitFor(() => (themeStylePresent("ClearVision-BD") ? true : null), 20, 500);
    probe("enable", !!(on && node), "was=" + before + " checked=true nodes=" + themeStyleCount(),
      on && node ? null : "enable failed");
    if (!on || !node) return;
    const on2 = await ensureSwitch(card2, true);
    const nodes2 = await waitFor(() => (themeStyleCount() >= 2 ? true : null), 20, 500);
    probe("second", !!(on2 && nodes2), "nodes=" + themeStyleCount(), on2 && nodes2 ? null : "second enable failed");
    if (!on2 || !nodes2) return;
    const off = await ensureSwitch(card, false);
    const gone = await waitFor(
      () => (!themeStylePresent("ClearVision-BD") && themeStylePresent("ProbeSecond-BD") ? true : null), 20, 500);
    probe("disable", !!(off && gone), "nodes=" + themeStyleCount(), off && gone ? null : "disable failed");
    if (!off || !gone) return;
    const importBtn = findText(document, "Import from file");
    if (importBtn && typeof importBtn.click === "function") {
      importBtn.click();
      await sleep(3000);
    }
    probe("import", true, "import-clicked nodes=" + themeStyleCount() + " alive=true");
    // Re-query the card fresh: earlier toggles can replace the captured node
    // with a re-rendered one, and querying a detached subtree finds nothing.
    const folderCard = await waitFor(() => cardFor("ClearVision"), 10, 500);
    if (!folderCard) { probe("folder", false, describeCard("ClearVision"), "ClearVision card gone at folder time"); return; }
    const folderButtons = [...folderCard.querySelectorAll("button[title]")];
    const allButtons = [...folderCard.querySelectorAll("button")];
    if (folderButtons.length === 0 && allButtons.length === 0) {
      probe("folder", false, describeCard("ClearVision"), "no card buttons at all");
      return;
    }
    // In native mode only the folder action renders (delete/edit/update are
    // hidden), so the last button is the folder action even if its title
    // attribute did not survive rendering.
    const folderBtn = folderButtons.length > 0
      ? folderButtons[folderButtons.length - 1]
      : allButtons[allButtons.length - 1];
    // Click the wired button (best effort), then confirm the capability
    // end to end directly: the host log shows one Metrics line per call, so
    // a working button plus this direct call produce two lines.
    folderBtn.click();
    await sleep(2000);
    let folderOk = false;
    try {
      const echoed = await window.remcord.folders.openTheme("ClearVision-BD");
      folderOk = echoed === "ClearVision-BD";
    } catch (error) {
      probe("folder", false, "", "folder capability failed: " + String((error && error.message) || error).slice(0, 128));
      return;
    }
    probe("folder", folderOk, "folder-clicked direct=" + folderOk);
    await ensureSwitch(card, true);
    try { localStorage.setItem(STAGE_KEY, "verify"); } catch (ignored) {}
    probe("reload", true, "reloading nodes=" + themeStyleCount());
    await sleep(1000);
    window.location.reload();
  }
  async function runVerify() {
    await sleep(4000);
    const count = themeStyleCount();
    const cv = themeStylePresent("ClearVision-BD");
    const p2 = themeStylePresent("ProbeSecond-BD");
    const ok = count === 2 && cv && p2;
    probe("verify", ok, "nodes=" + count + " clearvision=" + cv + " second=" + p2,
      ok ? null : "reload persistence mismatch");
    if (!ok) return;
    const gear = await waitFor(findGear, 40, 500);
    if (!gear) { probe("restore", false, "", "gear missing on restore"); return; }
    gear.click();
    const openedRestore = await openThemesPage();
    if (!openedRestore) { probe("restore", false, "", "Themes page missing on restore"); return; }
    for (const name of ["ClearVision", "ProbeSecond"]) {
      const card = await waitFor(() => cardFor(name), 20, 500);
      if (!card) { probe("restore", false, "", name + " card missing on restore"); return; }
      const sw = switchOf(card);
      if (switchChecked(sw) === true) {
        sw.click();
        const off = await waitFor(() => (switchChecked(switchOf(card)) === false ? true : null), 20, 500);
        if (!off) { probe("restore", false, "", name + " would not switch off"); return; }
      }
    }
    const clean = await waitFor(() => (themeStyleCount() === 0 ? true : null), 20, 500);
    try { localStorage.setItem(STAGE_KEY, "done"); } catch (ignored) {}
    probe("restore", !!clean, "nodes=" + themeStyleCount(), clean ? null : "cleanup mismatch");
    if (clean) probe("done", true, "workflow complete, DEV profile restored");
  }
  try {
    // Per-launch nonce: a new app launch always starts a fresh workflow even
    // though the stage keys live in persistent localStorage (sessionStorage
    // proved unreliable across the probe's own reload in WebView2). Same-run
    // reloads keep the nonce and continue the machine; a repeat run gets a
    // new nonce and starts over instead of wedging on stale keys.
    let stage = null;
    let nonce = null;
    try {
      stage = localStorage.getItem(STAGE_KEY);
      nonce = localStorage.getItem(NONCE_KEY);
    } catch (ignored) {}
    if (nonce !== RUN_NONCE) {
      try {
        localStorage.setItem(NONCE_KEY, RUN_NONCE);
        localStorage.setItem(STAGE_KEY, "workflow");
      } catch (ignored) {}
      runWorkflow();
      return;
    }
    if (stage === "verify") {
      runVerify();
    } else if (stage === "done") {
      probe("done", true, "already complete");
    } else {
      try { localStorage.setItem(STAGE_KEY, "workflow"); } catch (ignored) {}
      runWorkflow();
    }
  } catch (error) {
    probe("fatal", false, "", error);
  }
})();
"#;

/// Builds the per-launch settings UI probe script by splicing a random run
/// nonce into the template. Fails fast on an unspliced placeholder.
fn settings_ui_probe_script(nonce: u64) -> Result<String, String> {
    let script =
        SETTINGS_UI_PROBE_TEMPLATE.replace("__REMCORD_SETTINGS_UI_RUN_NONCE__", &nonce.to_string());
    if script.contains("__REMCORD_SETTINGS_UI_RUN_NONCE__") {
        return Err("settings UI probe script has an unspliced nonce".to_owned());
    }
    Ok(script)
}
/// DEV-only remcord:// protocol smoke probe
/// (`REMCORD_NATIVE_PROTOCOL_SMOKE=1`). Loads three real bundle
/// resources that exist on disk through real WebView2 subresource/fetch
/// paths and records the outcome in document dataset flags the host reads
/// back:
///
/// - a stylesheet (`remcord://assets/css/discord.css` via `<link>`,
///   exercises style-src CSP + 200 + text/css)
/// - an image (`remcord://assets/desktop.png` via `<img>`, exercises
///   img-src CSP + 200 + image/png)
/// - a plugin resource (`remcord://plugins/settings/plugin.js` via
///   `fetch`, observed only — CORS headers are deliberately NOT added, so
///   a TypeError here proves the handler served while reporting the CORS
///   posture honestly)
///
/// The host logs one `[RemcordProtocol][Smoke]` line per document; no bridge
/// event, no new capability, no script execution of the fetched bytes.
const REMCORD_PROTOCOL_SMOKE_SCRIPT: &str = r#"
(() => {
  try {
    const root = document.documentElement;
    if (root.dataset.remcordProtocolSmoke === "started") return;
    root.dataset.remcordProtocolSmoke = "started";
    root.dataset.remcordProtocolCss = "pending";
    root.dataset.remcordProtocolImg = "pending";
    root.dataset.remcordProtocolPlugin = "pending";
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = "remcord://assets/css/discord.css";
    link.onload = () => { root.dataset.remcordProtocolCss = "loaded"; };
    link.onerror = () => { root.dataset.remcordProtocolCss = "error"; };
    (document.head || document.documentElement).appendChild(link);
    const img = new Image();
    img.onload = () => { root.dataset.remcordProtocolImg = "loaded"; };
    img.onerror = () => { root.dataset.remcordProtocolImg = "error"; };
    img.src = "remcord://assets/desktop.png";
    // Distinguish CSP blocks from scheme/network failures: a
    // securitypolicyviolation naming a remcord:// URI proves the scheme
    // resolved and CSP refused it; silence + error proves the request never
    // reached the handler.
    root.dataset.remcordProtocolCsp = "none";
    document.addEventListener("securitypolicyviolation", (event) => {
      try {
        const blocked = String(event.blockedURI || "");
        if (blocked.indexOf("remcord://") === 0) {
          root.dataset.remcordProtocolCsp =
            String(event.violatedDirective || "unknown").slice(0, 48)
            + "|" + blocked.slice(0, 80);
        }
      } catch (ignored) {}
    });
    // CORS posture observation: a no-cors fetch that resolves opaque proves
    // the handler served bytes while a cors-mode rejection stands.
    root.dataset.remcordProtocolNoCors = "pending";
    fetch("remcord://plugins/settings/plugin.js", { method: "GET", mode: "no-cors" }).then(
      () => { root.dataset.remcordProtocolNoCors = "resolved"; },
      (error) => {
        root.dataset.remcordProtocolNoCors =
          "rejected:" + String((error && error.name) || error).slice(0, 32);
      },
    );
    fetch("remcord://plugins/settings/plugin.js", { method: "GET" }).then(
      (response) => {
        root.dataset.remcordProtocolPlugin =
          "http-" + response.status + "-bytes-" + (response.headers.get("content-type") || "no-mime");
      },
      (error) => {
        root.dataset.remcordProtocolPlugin =
          "fetch-rejected:" + String((error && error.name) || error).slice(0, 64);
      },
    );
  } catch (error) {
    document.documentElement.dataset.remcordProtocolSmoke =
      "probe-failed:" + String((error && error.message) || error).slice(0, 64);
  }
})();
"#;

/// Reads back the DEV-only remcord:// probe dataset flags. Returns JSON
/// through the evaluate callback (like the renderer health probe), never
/// through the typed bridge.
const REMCORD_PROTOCOL_SMOKE_READBACK_SCRIPT: &str = r#"
(() => {
  const root = document.documentElement;
  return JSON.stringify({
    smoke: root.dataset.remcordProtocolSmoke || "absent",
    css: root.dataset.remcordProtocolCss || "absent",
    img: root.dataset.remcordProtocolImg || "absent",
    plugin: root.dataset.remcordProtocolPlugin || "absent",
    csp: root.dataset.remcordProtocolCsp || "absent",
    nocors: root.dataset.remcordProtocolNoCors || "absent",
  });
})()
"#;

/// DEV-only permissions smoke probe (`REMCORD_NATIVE_PERMISSIONS_SMOKE=1`).
/// Triggers one real request per exercised kind from the trusted document
/// and records the renderer-observed outcome in dataset flags the host
/// reads back; the host log carries the authoritative kind/origin/decision
/// per request. Microphone/camera tracks are stopped immediately in the
/// same expression (no hot mic, no lingering capture). Never screenshare:
/// no `getDisplayMedia`, no picker, no audio graph. No bridge event, no new
/// capability, no renderer input accepted — every target is a literal.
const PERMISSIONS_SMOKE_SCRIPT: &str = r#"
(() => {
  try {
    const root = document.documentElement;
    if (root.dataset.remcordPermissionsSmoke === "started") return;
    root.dataset.remcordPermissionsSmoke = "started";
    root.dataset.remcordPermissionsNotification = "pending";
    root.dataset.remcordPermissionsMic = "pending";
    root.dataset.remcordPermissionsCamera = "pending";
    root.dataset.remcordPermissionsGeo = "pending";
    try {
      const pending = Notification.requestPermission();
      if (pending && typeof pending.then === "function") {
        pending.then(
          (value) => { root.dataset.remcordPermissionsNotification = String(value); },
          (error) => { root.dataset.remcordPermissionsNotification = "error"; },
        );
      } else {
        root.dataset.remcordPermissionsNotification = String(Notification.permission);
      }
    } catch (error) {
      root.dataset.remcordPermissionsNotification = "error";
    }
    try {
      navigator.mediaDevices.getUserMedia({ audio: true }).then(
        (stream) => {
          try { stream.getTracks().forEach((track) => track.stop()); } catch (ignored) {}
          root.dataset.remcordPermissionsMic = "live-stopped";
        },
        (error) => { root.dataset.remcordPermissionsMic = "denied:" + String((error && error.name) || error).slice(0, 32); },
      );
    } catch (error) {
      root.dataset.remcordPermissionsMic = "error";
    }
    try {
      navigator.mediaDevices.getUserMedia({ video: true }).then(
        (stream) => {
          try { stream.getTracks().forEach((track) => track.stop()); } catch (ignored) {}
          root.dataset.remcordPermissionsCamera = "live-stopped";
        },
        (error) => { root.dataset.remcordPermissionsCamera = "denied:" + String((error && error.name) || error).slice(0, 32); },
      );
    } catch (error) {
      root.dataset.remcordPermissionsCamera = "error";
    }
    try {
      navigator.geolocation.getCurrentPosition(
        () => { root.dataset.remcordPermissionsGeo = "unexpected-allow"; },
        (error) => { root.dataset.remcordPermissionsGeo = "denied:" + (error && error.code); },
        { timeout: 8000 },
      );
    } catch (error) {
      root.dataset.remcordPermissionsGeo = "error";
    }
  } catch (error) {
    document.documentElement.dataset.remcordPermissionsSmoke =
      "probe-failed:" + String((error && error.message) || error).slice(0, 64);
  }
})();
"#;

/// Reads back the DEV-only permissions probe dataset flags. Returns JSON
/// through the evaluate callback, never through the typed bridge.
const PERMISSIONS_SMOKE_READBACK_SCRIPT: &str = r#"
(() => {
  const root = document.documentElement;
  return JSON.stringify({
    smoke: root.dataset.remcordPermissionsSmoke || "absent",
    notification: root.dataset.remcordPermissionsNotification || "absent",
    mic: root.dataset.remcordPermissionsMic || "absent",
    camera: root.dataset.remcordPermissionsCamera || "absent",
    geo: root.dataset.remcordPermissionsGeo || "absent",
  });
})()
"#;
/// DEV-only themes observation (`REMCORD_NATIVE_THEMES_SMOKE=1`,
/// `REMCORD_NATIVE_QUICKCSS_SMOKE=1`). Theme and Quick CSS injection are host
/// pushes: the renderer never requests their CSS, so this probe only observes
/// the managed `<style>` nodes the host injected at document load. It returns
/// JSON through the evaluate callback (like the renderer health probe), never
/// through the typed bridge. `order_probe` reports the computed value of the
/// harmless `--remcord-native-order-probe` custom property the DEV fixtures
/// set in both Quick CSS and themes, proving cascade order (later stylesheets
/// win on equal specificity).
const THEMES_SMOKE_SCRIPT: &str = r#"
(() => {
  const head = document.head;
  const headOrder = head ? [...head.querySelectorAll("style")].map((node) => node.id) : [];
  const nodes = [...document.querySelectorAll('style[id^="remcord-native-theme-"]')]
    .map((node) => ({ id: node.id, bytes: (node.textContent || "").length }));
  const quickCssNode = document.getElementById("remcord-quick-css");
  return JSON.stringify({
    count: nodes.length,
    themes: nodes,
    head_style_order: headOrder,
    vencord_present: headOrder.includes("remcord-native-vencord-css"),
    quickcss_present: Boolean(quickCssNode),
    quickcss_bytes: quickCssNode ? (quickCssNode.textContent || "").length : 0,
    order_probe: getComputedStyle(document.documentElement)
      .getPropertyValue("--remcord-native-order-probe")
      .trim(),
  });
})()
"#;

#[derive(Serialize)]
struct BootSnapshot {
    runtime: &'static str,
    platform: &'static str,
    protocol_version: u8,
    window: BootWindowState,
    capabilities: &'static [&'static str],
}

/// Synchronous initial window state for the renderer facade. The window is
/// created hidden and restored, so both flags are false at boot; the typed
/// `window_state` host event converges them afterwards. This keeps the
/// existing synchronous `maximized()`/`isNormal()` titlebar checks total
/// without synchronous IPC.
#[derive(Serialize)]
struct BootWindowState {
    focused: bool,
    maximized: bool,
}

/// Closed registry id for the real Remcord settings plugin. It matches the
/// `"remcord-settings"` key Electron pins in the Shelter injector map
/// (`src/discord/preload/mods/shelter.mts`), so host logs and the Electron
/// contract name the same plugin. The renderer never supplies this id: the
/// host splices it into the fixed start script below.
const SETTINGS_PLUGIN_ID: &str = "remcord-settings";

/// Fixed manifest for the real Remcord settings plugin registration. Only the
/// name is carried (the loader-probe precedent); the renderer supplies no
/// manifest, id, source, or update flag.
const SETTINGS_PLUGIN_MANIFEST_JSON: &str = r#"{"name":"Remcord Settings"}"#;

/// Host-built start script for the real Remcord settings plugin, evaluated
/// once per document after Shelter's own bootstrap marker proves
/// `window.shelter.plugins` exists. It registers the embedded artifact
/// through the same closed add/start lifecycle the loader probe proved
/// (remove-first for idempotency, host-fixed id/source/manifest, then start),
/// and returns a plain status string the host logs — never plugin source,
/// paths, or URLs.
///
/// `__REMCORD_SETTINGS_PLUGIN_JS__` is spliced as a JSON-encoded string
/// literal (never interpolated raw); `__REMCORD_SETTINGS_PLUGIN_MANIFEST__`
/// is spliced as fixed JSON.
const SETTINGS_PLUGIN_START_TEMPLATE: &str = r#"
(() => {
  const ID = "remcord-settings";
  try {
    const plugins = window.shelter && window.shelter.plugins;
    if (!plugins) return "settings-plugin-failed: shelter plugins unavailable";
    for (const name of ["addLocalPlugin", "startPlugin", "removePlugin"]) {
      if (typeof plugins[name] !== "function") return "settings-plugin-failed: missing " + name;
    }
    try {
      plugins.removePlugin(ID);
    } catch (ignored) {}
    plugins.addLocalPlugin(ID, {
      js: __REMCORD_SETTINGS_PLUGIN_JS__,
      manifest: __REMCORD_SETTINGS_PLUGIN_MANIFEST__,
      update: false,
    });
    plugins.startPlugin(ID);
    return "settings-plugin-started";
  } catch (error) {
    return "settings-plugin-failed: " + String((error && error.message) || error).slice(0, 256);
  }
})()
"#;

/// Builds the per-launch settings plugin start script by splicing the
/// embedded artifact and the fixed manifest into the closed template.
/// Fails fast on an unspliced placeholder or an unserializable artifact.
fn settings_plugin_start_script() -> Result<String, String> {
    let js = serde_json::to_string(settings_plugin::artifact().js)
        .map_err(|error| format!("settings plugin artifact cannot be serialized: {error}"))?;
    let script = SETTINGS_PLUGIN_START_TEMPLATE
        .replace("__REMCORD_SETTINGS_PLUGIN_JS__", &js)
        .replace(
            "__REMCORD_SETTINGS_PLUGIN_MANIFEST__",
            SETTINGS_PLUGIN_MANIFEST_JSON,
        );
    if script.contains("__REMCORD_SETTINGS_PLUGIN_") {
        return Err("settings plugin start script has an unspliced placeholder".to_owned());
    }
    if !script.contains(SETTINGS_PLUGIN_ID) {
        return Err("settings plugin start script lost the closed plugin id".to_owned());
    }
    Ok(script)
}

/// Per-document guard for one real Remcord Shelter plugin start (settings,
/// titlebar, …). The plugin may start at most once per document generation:
/// the guard compares against the live generation, so reloads start fresh
/// and replays are refused. Threaded as one bundle to keep the bootstrap
/// handler readable.
struct ShelterPluginStart<'a> {
    /// Host-built evaluation script (closed id + embedded artifact).
    script: &'a str,
    /// Generation that already started the plugin (`None` = not yet started).
    started_generation: &'a mut Option<u64>,
    /// Count of documents the plugin was started in (observability).
    start_count: &'a mut u64,
}

/// Decides whether the settings plugin may start in `current_generation`.
/// Pure so the reload-fresh / replay-refuse semantics are unit-testable
/// without a WebView.
fn settings_plugin_start_due(started_generation: Option<u64>, current_generation: u64) -> bool {
    started_generation != Some(current_generation)
}

/// Closed registry id for the real Remcord titlebar plugin. It matches the
/// `"remcord-titlebar"` key Electron pins in the Shelter injector map
/// (`src/discord/preload/mods/shelter.mts`), so host logs and the Electron
/// contract name the same plugin. The renderer never supplies this id: the
/// host splices it into the fixed start script below.
const TITLEBAR_PLUGIN_ID: &str = "remcord-titlebar";

/// Fixed manifest for the real Remcord titlebar plugin registration. Only the
/// name is carried (the loader-probe precedent); the renderer supplies no
/// manifest, id, source, or update flag.
const TITLEBAR_PLUGIN_MANIFEST_JSON: &str = r#"{"name":"Remcord Titlebar Controller"}"#;

/// Host-built start script for the real Remcord titlebar plugin, evaluated
/// once per document after Shelter's own bootstrap marker proves
/// `window.shelter.plugins` exists (same trigger as the settings plugin).
/// It registers the embedded artifact through the same closed add/start
/// lifecycle the loader probe proved (remove-first for idempotency,
/// host-fixed id/source/manifest, then start), and returns a plain status
/// string the host logs — never plugin source, paths, or URLs.
///
/// The same script also runs as the trusted-load settle pass, which races
/// Shelter's asynchronous publish: when `window.shelter.plugins` is absent
/// the script reports `titlebar-pending` (the marker-time start covers the
/// real start once the marker lands), never `titlebar-failed`. A pending
/// settle is therefore routine timing noise, not a defect; every other
/// branch keeps the `titlebar-failed` prefix so genuine start failures stay
/// loud.
///
/// Before registration it reproduces `src/discord/preload/newTitlebar.mts`
/// for the overlay-chrome styles: the host platform comes from the
/// facade-owned `window.remcord.platform` (never renderer input), the body
/// attributes and the build-embedded per-OS stylesheets are host-chosen and
/// injected as inline `<style>` nodes (no subresource fetch, so reloads
/// cannot silently unstyle the controls). All existing window actions flow through the typed
/// `window.remcord.window.*` operations the plugin already calls; no new
/// RPC is added.
///
/// `__REMCORD_TITLEBAR_PLUGIN_JS__` is spliced as a JSON-encoded string
/// literal (never interpolated raw); `__REMCORD_TITLEBAR_PLUGIN_MANIFEST__`
/// is spliced as fixed JSON; `__REMCORD_TITLEBAR_CSS__` is spliced as a
/// JSON object of build-embedded stylesheet texts keyed `base`/`win`/
/// `darwin`/`linux` (never paths or URLs).
const TITLEBAR_PLUGIN_START_TEMPLATE: &str = r#"
(() => {
  const ID = "remcord-titlebar";
  try {
    const plugins = window.shelter && window.shelter.plugins;
    if (!plugins) return "titlebar-pending: shelter plugins unavailable";
    for (const name of ["addLocalPlugin", "startPlugin", "removePlugin"]) {
      if (typeof plugins[name] !== "function") return "titlebar-failed: missing " + name;
    }
    const remcord = window.remcord;
    if (!remcord || typeof remcord.platform !== "string") return "titlebar-failed: facade unavailable";
    const body = document.body;
    if (!body) return "titlebar-failed: no document body";
    body.setAttribute("remcord-platform", remcord.platform);
    const cssText = __REMCORD_TITLEBAR_CSS__;
    const wanted = ["base"];
    if (remcord.platform === "win32") {
      // Never overwrite the body class attribute: Discord owns it, and a
      // full overwrite would drop Discord classes and force a style recalc.
      body.classList.add("platform-win");
      wanted.push("win");
    } else if (remcord.platform === "darwin") {
      wanted.push("darwin");
    } else {
      body.classList.add("platform-linux");
      wanted.push("linux");
    }
    // Inline <style> nodes from build-embedded text (no subresource fetch:
    // fetched sheets stop reaching the host after a reload, which silently
    // unstyles link-based sheets). Stale link sheets are removed; style nodes are deduped and refreshed per
    // document, so reloads converge instead of stacking.
    for (const stale of document.querySelectorAll('link[data-remcord-titlebar-css]')) {
      stale.remove();
    }
    for (const key of wanted) {
      const text = cssText && cssText[key];
      if (typeof text !== "string" || text.length === 0) return "titlebar-failed: missing " + key + " css";
      let node = document.querySelector('style[data-remcord-titlebar-css="' + key + '"]');
      if (!node) {
        node = document.createElement("style");
        node.setAttribute("data-remcord-titlebar-css", key);
        document.head.appendChild(node);
      }
      if (node.textContent !== text) node.textContent = text;
    }
    // Shelter auto-starts persisted plugins at its own boot, before this
    // marker-time script runs — and the plugin's remove/unload path never
    // removes injected DOM. Drop any stale controls first so exactly one set
    // exists no matter who started the plugin earlier in this document.
    for (const id of ["remcordNavControls", "remcordTitlebar"]) {
      const stale = document.getElementById(id);
      if (stale) stale.remove();
    }
    try {
      plugins.removePlugin(ID);
    } catch (ignored) {}
    plugins.addLocalPlugin(ID, {
      js: __REMCORD_TITLEBAR_PLUGIN_JS__,
      manifest: __REMCORD_TITLEBAR_PLUGIN_MANIFEST__,
      update: false,
    });
    plugins.startPlugin(ID);
    // Structural census (DOM + inline style nodes + platform + readyState).
    // Styles are build-embedded text injected above (no fetch), so the node
    // count is steady-state truthful here, unlike link load timing. The host
    // re-runs this same idempotent script at document finish (settle pass).
    const controls = document.querySelectorAll('#remcordNavControls').length;
    const styles = Array.from(document.querySelectorAll('style[data-remcord-titlebar-css]'));
    const platform = document.body.getAttribute("remcord-platform") || "none";
    const ready = document.readyState || "unknown";
    if (controls < 1) {
      return "titlebar-started:controls-missing";
    }
    return "titlebar-started:controls-" + controls + "-styles-" + styles.length
      + "-platform-" + platform + "-ready-" + ready;
  } catch (error) {
    return "titlebar-failed: " + String((error && error.message) || error).slice(0, 256);
  }
})()
"#;

/// Builds the per-launch titlebar plugin start script by splicing the
/// embedded artifact, the fixed manifest, and the embedded stylesheets into
/// the closed template. Fails fast on an unspliced placeholder or an
/// unserializable artifact.
fn titlebar_plugin_start_script() -> Result<String, String> {
    let js = serde_json::to_string(titlebar_plugin::artifact().js)
        .map_err(|error| format!("titlebar plugin artifact cannot be serialized: {error}"))?;
    let css = titlebar_plugin::TITLEBAR_CSS_KEYS
        .iter()
        .map(|key| {
            titlebar_plugin::css_for(key)
                .map(|text| (*key, text))
                .ok_or_else(|| format!("titlebar css key {key:?} is missing"))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    let css_json = serde_json::to_string(&css)
        .map_err(|error| format!("titlebar stylesheets cannot be serialized: {error}"))?;
    let script = TITLEBAR_PLUGIN_START_TEMPLATE
        .replace("__REMCORD_TITLEBAR_PLUGIN_JS__", &js)
        .replace(
            "__REMCORD_TITLEBAR_PLUGIN_MANIFEST__",
            TITLEBAR_PLUGIN_MANIFEST_JSON,
        )
        .replace("__REMCORD_TITLEBAR_CSS__", &css_json);
    if script.contains("__REMCORD_TITLEBAR_PLUGIN_") || script.contains("__REMCORD_TITLEBAR_CSS__")
    {
        return Err("titlebar plugin start script has an unspliced placeholder".to_owned());
    }
    if !script.contains(TITLEBAR_PLUGIN_ID) {
        return Err("titlebar plugin start script lost the closed plugin id".to_owned());
    }
    Ok(script)
}

/// DEV-only settings smoke probe (`REMCORD_NATIVE_SETTINGS_SMOKE=1`).
/// Exercises the exact read the migrated Shelter power caller performs:
/// `window.remcord.settings.getConfig()` → `blockPowerSavingInVoiceChat`.
/// The result is reported through the typed `settings_probe` event so the
/// host log shows request/response behavior per document.
const SETTINGS_SMOKE_SCRIPT: &str = r#"
(() => {
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "settings_probe", payload } })
  );
  try {
    const getConfig = window.remcord?.settings?.getConfig;
    if (typeof getConfig !== "function") {
      probe({ ok: false, source: "unavailable", payload_bytes: 0, block_power_saving: false, error: "settings facade missing" });
      return;
    }
    const result = getConfig.call(window.remcord.settings);
    // Dual contract: the closed settings shim returns the snapshot value
    // synchronously while staying awaitable for the live async capability.
    if (!result || (typeof result.then !== "function" && !(result instanceof Promise))) {
      probe({ ok: false, source: "unavailable", payload_bytes: 0, block_power_saving: false, error: "settings facade is not awaitable" });
      return;
    }
    result.then(
      (settings) => probe({
        ok: true,
        source: "unknown",
        payload_bytes: JSON.stringify(settings).length,
        block_power_saving: settings.blockPowerSavingInVoiceChat === true,
        error: null,
      }),
      (error) => probe({
        ok: false,
        source: "unavailable",
        payload_bytes: 0,
        block_power_saving: false,
        error: String(error?.message ?? error).slice(0, 256),
      }),
    );
  } catch (error) {
    probe({ ok: false, source: "unavailable", payload_bytes: 0, block_power_saving: false, error: String(error).slice(0, 256) });
  }
})();
"#;

/// DEV-only settings mutation smoke probe
/// (`REMCORD_NATIVE_SETTINGS_MUTATION_SMOKE=1`). Exercises the exact
/// write the migrated Shelter settings caller performs: read the current
/// `blockPowerSavingInVoiceChat` via `getConfig`, flip it via the
/// allowlisted `setBlockPowerSavingInVoiceChat`, then re-read via
/// `getConfig` and verify. Reports one typed `settings_mutation_probe`
/// event with `stage: "mutated"`. The host then reloads the document so the
/// restore stage can verify the mutation survived the reload.
const SETTINGS_MUTATION_SMOKE_SCRIPT: &str = r#"
(() => {
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "settings_mutation_probe", payload } })
  );
  const fail = (before, error) => probe({
    ok: false, stage: "mutated", before, value: before, reread_matches: false,
    error: String(error?.message ?? error).slice(0, 256),
  });
  try {
    const settings = window.remcord?.settings;
    if (typeof settings?.getConfig !== "function" || typeof settings?.setBlockPowerSavingInVoiceChat !== "function") {
      fail(false, "settings mutation facade missing");
      return;
    }
    settings.getConfig().then(
      (snapshot) => {
        const before = snapshot.blockPowerSavingInVoiceChat === true;
        const pending = settings.setBlockPowerSavingInVoiceChat(!before);
        if (!(pending instanceof Promise)) {
          fail(before, "settings mutation facade is not asynchronous");
          return;
        }
        pending.then(
          (applied) => {
            settings.getConfig().then(
              (reread) => probe({
                ok: reread.blockPowerSavingInVoiceChat === applied && applied === !before,
                stage: "mutated",
                before,
                value: applied,
                reread_matches: reread.blockPowerSavingInVoiceChat === applied,
                error: null,
              }),
              (error) => fail(before, error),
            );
          },
          (error) => fail(before, error),
        );
      },
      (error) => fail(false, error),
    );
  } catch (error) {
    fail(false, error);
  }
})();
"#;

/// DEV-only restore stage of the mutation smoke probe. The host formats
/// `__REMCORD_MUTATION_RESTORE_VALUE__` with the pre-mutation value before
/// evaluating: read the post-reload value (proving the mutation survived),
/// write the original back, re-read, and report `stage: "restored"`.
const SETTINGS_MUTATION_RESTORE_SCRIPT_TEMPLATE: &str = r#"
(() => {
  const RESTORE_TO = __REMCORD_MUTATION_RESTORE_VALUE__;
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "settings_mutation_probe", payload } })
  );
  const fail = (before, error) => probe({
    ok: false, stage: "restored", before, value: before, reread_matches: false,
    error: String(error?.message ?? error).slice(0, 256),
  });
  try {
    const settings = window.remcord?.settings;
    if (typeof settings?.getConfig !== "function" || typeof settings?.setBlockPowerSavingInVoiceChat !== "function") {
      fail(false, "settings mutation facade missing");
      return;
    }
    settings.getConfig().then(
      (snapshot) => {
        const before = snapshot.blockPowerSavingInVoiceChat === true;
        settings.setBlockPowerSavingInVoiceChat(RESTORE_TO).then(
          (applied) => {
            settings.getConfig().then(
              (reread) => probe({
                ok: applied === RESTORE_TO && reread.blockPowerSavingInVoiceChat === RESTORE_TO,
                stage: "restored",
                before,
                value: applied,
                reread_matches: reread.blockPowerSavingInVoiceChat === applied,
                error: null,
              }),
              (error) => fail(before, error),
            );
          },
          (error) => fail(before, error),
        );
      },
      (error) => fail(false, error),
    );
  } catch (error) {
    fail(false, error);
  }
})();
"#;

/// DEV-only settings.set smoke probe (`REMCORD_NATIVE_SETTINGS_SET_SMOKE=1`).
/// Exercises the closed `settings.set` allowlist through the side-effect-free
/// `support_banner_dismissed` flag: block three attack shapes (unknown key,
/// wrong value type, key↔variant mismatch), flip the flag, re-read via
/// `getConfig`, and verify. Reports one typed `settings_set_probe` event
/// with `stage: "mutated"`. The host then reloads the document so the restore
/// stage can verify the mutation survived the reload.
const SETTINGS_SET_SMOKE_SCRIPT: &str = r#"
(() => {
  const KEY = "support_banner_dismissed";
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "settings_set_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, stage: "mutated", key: KEY, before: false, reread_matches: false,
    attacks_blocked: 0, error: String(error?.message ?? error).slice(0, 256),
  });
  try {
    const settings = window.remcord?.settings;
    if (typeof settings?.getConfig !== "function" || typeof settings?.set !== "function") {
      fail("settings set facade missing");
      return;
    }
    // Attack shapes the facade must reject before dispatch: an unknown key,
    // a wrong value type for a bool key, and a key/variant mismatch. Each
    // must reject; a resolution means the allowlist leaked.
    let attacks = 0;
    const expectReject = (pending) => pending.then(
      () => { throw new Error("attack shape was not blocked"); },
      () => { attacks += 1; },
    );
    Promise.all([
      expectReject(settings.set("tray", true)),
      expectReject(settings.set("support_banner_dismissed", "yes")),
      expectReject(settings.set("mods", true)),
    ]).then(
      () => {
        if (attacks !== 3) {
          fail(`attack shapes not blocked: ${attacks}`);
          return;
        }
        settings.getConfig().then(
          (snapshot) => {
            const before = snapshot.supportBannerDismissed === true;
            const pending = settings.set(KEY, !before);
            if (!(pending instanceof Promise)) {
              fail("settings set facade is not asynchronous");
              return;
            }
            pending.then(
              (applied) => {
                settings.getConfig().then(
                  (reread) => probe({
                    ok: applied.key === KEY && reread.supportBannerDismissed === applied.value
                      && applied.value === !before,
                    stage: "mutated",
                    key: KEY,
                    before,
                    reread_matches: reread.supportBannerDismissed === applied.value,
                    attacks_blocked: attacks,
                    error: null,
                  }),
                  (error) => fail(error),
                );
              },
              (error) => fail(error),
            );
          },
          (error) => fail(error),
        );
      },
      (error) => fail(error),
    );
  } catch (error) {
    fail(error);
  }
})();
"#;

/// DEV-only restore stage of the settings.set smoke probe. The host formats
/// `__REMCORD_SETTINGS_SET_RESTORE_VALUE__` with the pre-mutation value
/// before evaluating: read the post-reload value (proving the mutation
/// survived), write the original back, re-read, and report `stage:
/// "restored"`. The three attack shapes were proven blocked in the mutated
/// stage, so this stage reports the same count.
const SETTINGS_SET_RESTORE_SCRIPT_TEMPLATE: &str = r#"
(() => {
  const RESTORE_TO = __REMCORD_SETTINGS_SET_RESTORE_VALUE__;
  const KEY = "support_banner_dismissed";
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "settings_set_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, stage: "restored", key: KEY, before: false, reread_matches: false,
    attacks_blocked: 3, error: String(error?.message ?? error).slice(0, 256),
  });
  try {
    const settings = window.remcord?.settings;
    if (typeof settings?.getConfig !== "function" || typeof settings?.set !== "function") {
      fail("settings set facade missing");
      return;
    }
    settings.getConfig().then(
      (snapshot) => {
        const postReload = snapshot.supportBannerDismissed === true;
        settings.set(KEY, RESTORE_TO).then(
          (applied) => {
            settings.getConfig().then(
              (reread) => probe({
                // postReload proves the mutated value survived the reload:
                // it must still be the flipped value (!RESTORE_TO) before
                // this stage writes the original back.
                ok: postReload === !RESTORE_TO && applied.value === RESTORE_TO
                  && reread.supportBannerDismissed === RESTORE_TO,
                stage: "restored",
                key: KEY,
                before: postReload,
                reread_matches: reread.supportBannerDismissed === applied.value,
                attacks_blocked: 3,
                error: null,
              }),
              (error) => fail(error),
            );
          },
          (error) => fail(error),
        );
      },
      (error) => fail(error),
    );
  } catch (error) {
    fail(error);
  }
})();
"#;

enum NativeEvent {
    PageLoadStarted,
    PageLoadFinished(String),
    BridgeRequest(BridgeRequest, Instant),
    BridgeEvent(BridgeEventMessage),
    /// Raw filesystem signal from a native watcher thread. Never carries
    /// paths or content: the event loop debounces, then re-reads
    /// authoritative host state.
    LiveCssSignal(LiveCssFsSignal),
    /// Debounced fire for one logical resource. The epoch + document
    /// generation gate stale callbacks (superseded bursts, reloads).
    LiveCssFire {
        resource: LiveCssFsSignal,
        epoch: u64,
        generation: u64,
    },
    ModBundleReady {
        mod_id: &'static str,
        generation: u64,
        request_id: String,
        received_at: Instant,
        retrieval_ms: f64,
        result: Result<mod_substrate::ModBundle, String>,
    },
    /// One fixed CDP graphics probe completion (`System.getInfo` or
    /// `Browser.getVersion`). The one-shot COM handler owns only the bounded
    /// answer (or the bounded failure reason) plus the request id and probe
    /// kind — never COM objects. The event loop records the part in the
    /// matching `PendingGraphics` and answers the bridge call only when both
    /// parts arrived; unknown ids are late renderer timeouts, logged and
    /// dropped.
    GraphicsCdpPart {
        request_id: String,
        method: graphics::CdpMethod,
        outcome: Result<String, String>,
    },
    /// Completed theme file import from the dedicated STA picker worker.
    /// The worker owns the modal dialog plus the sequential bounded
    /// ingestion, so the WebView/UI thread never blocks; the response is
    /// always delivered here, on the event loop.
    ThemeImportReady {
        request_id: String,
        received_at: Instant,
        result: Result<ThemeImportSummary, ThemeImportFailure>,
    },
    /// Completed backup save from the dedicated STA save worker. The worker
    /// owns collection, ZIP creation, the modal dialog, and the final write,
    /// so the WebView/UI thread never blocks; the bounded outcome
    /// (cancellation plus byte count — never a path) is delivered here.
    BackupSaveReady {
        request_id: String,
        received_at: Instant,
        result: Result<BackupSaveSummary, BackupFailure>,
    },
    /// Completed backup restore COMMIT from the dedicated STA restore
    /// worker. The worker owns the commit with rollback of the previously
    /// inspected held staging, so the WebView/UI thread never blocks; the
    /// bounded outcome (the clientMods envelope — never a path) is
    /// delivered here.
    BackupRestoreReady {
        request_id: String,
        received_at: Instant,
        result: Result<RestoreSummary, BackupFailure>,
    },
    /// Completed backup INSPECT from the dedicated STA restore worker. The
    /// worker owns the modal open dialog, the bounded archive read, full
    /// validation, and staging; the staged bytes are HELD worker-side for
    /// one explicit commit or discard, and only the preview (counts, flags,
    /// bounded display listing — never contents or paths) is delivered
    /// here.
    BackupInspectReady {
        request_id: String,
        received_at: Instant,
        result: Result<Option<RestorePreview>, BackupFailure>,
    },
    /// Completed backup DISCARD from the dedicated STA restore worker. The
    /// held staged restore (if any) was dropped without touching the
    /// profile; `discarded` tells whether anything was actually held.
    BackupDiscardReady {
        request_id: String,
        received_at: Instant,
        discarded: bool,
    },
    /// One global-hotkey firing from the keybind worker thread. Carries
    /// only the keybind id: the event loop re-validates (known, enabled,
    /// global) and executes the stored action itself.
    KeybindFired {
        keybind_id: String,
    },
    /// One DEV-only window chrome smoke step (`REMCORD_NATIVE_WINDOW_SMOKE=1`).
    /// Drives the shipped renderer facade through the real typed bridge and
    /// logs the resulting native state. Never present in normal runs.
    #[allow(dead_code)]
    WindowSmokeStep(u8),
    /// Approved popup request from `NewWindowRequested` (P0.4). The WebView2
    /// popup itself is always denied at the layer above; the event loop
    /// creates the single managed child (or focuses the live one). Carries
    /// only the validated URL plus its kind — never renderer options.
    PopupRequested {
        url: String,
        kind: new_window::PopupKind,
    },
    /// Sanitized WebView2 process-failure record (Phase 1 observer, Phase
    /// 2A bounded main-renderer recovery). The COM callback owns only
    /// primitives; this payload carries the role, per-instance id, kind,
    /// reason, and exit code — never URLs, page data, or interface objects.
    /// Main `RenderProcessExited` records may approve one host-side reload
    /// through the recovery budget; child `RenderProcessExited` records
    /// close the managed child through the Phase 2H policy; every other
    /// record stays observe-only.
    ProcessFailed(ProcessFailedEvent),
    /// Sanitized WebView2 navigation-completion record (Phase 2B observer;
    /// Phase 2C gating). The COM callback owns only primitives; this
    /// payload carries the role, per-instance id, success flag, web error,
    /// and HTTP status — never URLs, page data, or interface objects.
    /// Counted per live instance and logged; the main-WebView success flag
    /// additionally resolves the trusted-load gate (run the pipeline on
    /// success, skip it on failure). Still never a reload, retry, or
    /// recovery-budget input: failures only skip, and the manual F5 path
    /// recovers through a fresh gated navigation.
    NavigationCompleted(NavigationCompletedEvent),
    /// Sanitized WebView2 environment-level browser-process exit record
    /// (Phase 2G). The COM callback owns only primitives; this payload
    /// carries the browser PID and exit kind (Normal/Failed) — never URLs,
    /// page data, or interface objects. A `Failed` exit may approve one
    /// browser-process recreation through the separate browser-recovery budget.
    BrowserProcessExited(browser_recovery::BrowserProcessExitedEvent),
    /// DEV-only delayed readback for the remcord:// protocol smoke probe.
    /// Fired once per document ~5 s after the probe started, so async
    /// subresource loads have settled before the host reads the flags.
    RemcordProtocolSmokeReadback,
    /// DEV-only delayed readback for the permissions smoke probe. Fired
    /// once per document ~8 s after the probe started, so the async media
    /// and geolocation settlements have landed before the host reads back.
    PermissionsSmokeReadback,
}

fn is_discord_uri(uri: &Uri) -> bool {
    uri.scheme_str() == Some("https")
        && uri.authority().is_some_and(|authority| {
            authority.port_u16().is_none()
                && (authority.host().eq_ignore_ascii_case("discord.com")
                    || authority
                        .host()
                        .to_ascii_lowercase()
                        .ends_with(".discord.com"))
        })
}

fn is_trusted_bridge_origin(uri: &Uri) -> bool {
    // The three first-party entry hosts (stable/canary/ptb): the channel
    // setting selects which one the host navigates to, and the bridge must
    // initialize on any of them. Exact hosts only — navigation still permits
    // the wider `*.discord.com` surface, but subdomains, lookalikes, ports,
    // and userinfo never carry a trusted bridge (see the entry-host tests).
    is_discord_uri(uri)
        && uri.authority().is_some_and(|authority| {
            // No explicit port (even :443) and no userinfo: both are
            // rejected shapes, matching the permission origin gate.
            authority.port_u16().is_none()
                && !authority.as_str().contains('@')
                && crate::channel::is_entry_host(authority.host())
        })
}

fn is_allowed_navigation(url: &str) -> bool {
    // `about:blank` is the two-phase boot document only (see the builder):
    // it carries no content and commits before any handler exists. Popups
    // stay denied through `NewWindowRequested` regardless of this.
    url == "about:blank" || url.parse::<Uri>().is_ok_and(|uri| is_discord_uri(&uri))
}

/// Native startup window visibility (minimum port of Electron
/// `applyStartupWindowVisibility` in `src/common/windowVisibility.ts`).
/// The Tao window is built hidden and shown exactly once, host-owned, when
/// the first real (non-`about:blank`) allowed document finishes loading —
/// late enough that the WebView has content to paint (no blank flash, no
/// externally-forced bounds), with no timers, no polling, and no renderer
/// involvement. No new show/hide RPC: the existing P0.1 `window.show`
/// capability is untouched.
///
/// Electron differences (documented, minimal): CLI flags
/// (`--start-in-tray`/`--start-minimized`) have no native CLI parser, so
/// the settings-file mode alone decides; `skipTaskbar` is not toggled (no
/// native tray icon yet — tray mode only stays hidden, like Electron's
/// disabled-tray warning path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupVisibility {
    Show,
    ShowMinimized,
    HiddenTray,
}

/// Maps the `startMinimized` setting to its mode. Mirrors Electron's
/// `START_MINIMIZED_MODES` exactly: only `"minimized"` and `"tray"` are
/// special; anything else (including unknown/empty) fails safe to `Show`,
/// like Electron's `"off"` fallback.
fn startup_visibility(start_minimized: &str) -> StartupVisibility {
    match start_minimized {
        "minimized" => StartupVisibility::ShowMinimized,
        "tray" => StartupVisibility::HiddenTray,
        _ => StartupVisibility::Show,
    }
}

fn apply_startup_visibility(window: &tao::window::Window, mode: StartupVisibility, tray: &str) {
    match mode {
        StartupVisibility::Show => window.set_visible(true),
        StartupVisibility::ShowMinimized => {
            window.set_visible(true);
            window.set_minimized(true);
        }
        StartupVisibility::HiddenTray => {
            if tray == "disabled" {
                eprintln!(
                    "[Window] startMinimized is \"tray\" but the tray icon is disabled; the window will stay hidden with no tray."
                );
            }
            // Stay hidden; there is no native tray icon yet to reveal from.
        }
    }
}

/// Builds the document-start script for one run.
///
/// The always-present prefix (storage preservation, the frozen facade, the
/// shared adapter factory) is identical for every combination. Only the
/// per-mod adapter files are appended, and only for the mods
/// [`resolve_active_mods`] selected — so "native without <mod>" is a genuinely
/// absent adapter, not a disabled one. The runtime plugin loader
/// (`PLUGIN_LOADER_SCRIPT`) is appended unconditionally after the adapters,
/// mirroring Electron's `plugins.mts` which runs on every document: it
/// no-ops when no enabled+compatible plugin declares an entry, and all
/// enablement lives in settings + manifests, never in this script's shape.
fn initialization_script(
    active_mods: &[&'static ModDescriptor],
    settings_boot_json: &str,
    host_platform_json: &str,
) -> Result<String, serde_json::Error> {
    let snapshot = serde_json::to_string(&BOOT_SNAPSHOT)?;
    // Fail fast on malformed host data: either would otherwise ship a broken
    // boot surface to every document of the run.
    let _: serde_json::Value = serde_json::from_str(settings_boot_json)?;
    let _: serde_json::Value = serde_json::from_str(host_platform_json)?;
    let compatibility = COMPATIBILITY_SCRIPT
        .replace("__REMCORD_BOOT_SNAPSHOT__", &snapshot)
        .replace("__REMCORD_SETTINGS_BOOT_SNAPSHOT__", settings_boot_json)
        .replace("__REMCORD_HOST_PLATFORM__", host_platform_json);
    if compatibility.contains("__REMCORD_BOOT_SNAPSHOT__")
        || compatibility.contains("__REMCORD_SETTINGS_BOOT_SNAPSHOT__")
        || compatibility.contains("__REMCORD_HOST_PLATFORM__")
    {
        return Err(serde::de::Error::custom(
            "document-start facade is missing a boot snapshot splice",
        ));
    }
    let base =
        format!("{STORAGE_START_DIAGNOSTIC_SCRIPT}\n{compatibility}\n{MOD_SUBSTRATE_SCRIPT}");
    let mut script = base;
    for descriptor in active_mods {
        let Some(adapter) = mod_adapter_script(descriptor) else {
            return Err(serde::de::Error::custom(format!(
                "registered mod '{}' has no renderer adapter script",
                descriptor.id
            )));
        };
        script.push('\n');
        script.push_str(adapter);
    }
    // Runtime plugin execution (F2) runs after the client-mod adapters,
    // matching Electron's preload order (mods, then plugins). The keybind
    // matcher (F8) runs last: it only observes keydown and reports matches,
    // so it can never perturb mod bootstrap, plugin loading, or settings.
    script.push('\n');
    script.push_str(PLUGIN_LOADER_SCRIPT);
    script.push('\n');
    script.push_str(KEYBIND_MATCHER_SCRIPT);
    Ok(script)
}

/// Full document-start script: the media-hardening layer
/// ([`screenshare_media`], F9 core) FIRST so the SDP/camera wrappers exist
/// before any page script can bind `RTCPeerConnection` or call
/// `getUserMedia`, then the base [`initialization_script`] (facade, boot
/// snapshot, mod adapters, plugin loader, keybind matcher), then the
/// spellcheck enforcement layer ([`spellcheck`], F18) so editable roots
/// carry the host language before the user types, then the
/// host-selected screenshare capture-quality script, appended AFTER the mod
/// adapters so capture constraining is never before (and can never perturb)
/// the settings boot snapshot or mod bootstrap — plus the Remcord stream
/// setup popup ([`screenshare_picker`]), appended last so it wraps outermost
/// and its modal is the first thing a `getDisplayMedia` call reaches.
/// `raw_mode` is normalized by the quality module (unknown values fail safe
/// to `"default"`, which preserves existing behavior exactly); the
/// spellcheck pair is normalized by the spellcheck module (F18: Electron
/// `webPreferences.spellcheck` plus the session language-list parity — a
/// hostile/empty language falls back to `"en-US"`, never an empty tag).
fn document_start_script(
    active_mods: &[&'static ModDescriptor],
    settings_boot_json: &str,
    host_platform_json: &str,
    raw_mode: &str,
    prefer_hw_h264: bool,
    spellcheck_enabled: bool,
    spellcheck_lang: &str,
) -> Result<String, serde_json::Error> {
    let media = screenshare_media::screenshare_media_script(prefer_hw_h264)
        .map_err(serde::de::Error::custom)?;
    let mut script = String::with_capacity(media.len() + 4096);
    script.push_str(&media);
    script.push('\n');
    script.push_str(&initialization_script(
        active_mods,
        settings_boot_json,
        host_platform_json,
    )?);
    let spell = spellcheck::spellcheck_script(spellcheck_enabled, spellcheck_lang)
        .map_err(serde::de::Error::custom)?;
    script.push('\n');
    script.push_str(&spell);
    let quality = screenshare_quality::screenshare_quality_script(raw_mode)
        .map_err(serde::de::Error::custom)?;
    script.push('\n');
    script.push_str(&quality);
    script.push('\n');
    script.push_str(&screenshare_picker::screenshare_picker_script());
    Ok(script)
}

fn queue_event<T>(proxy: &EventLoopProxy<T>, event: T)
where
    T: 'static,
{
    if let Err(error) = proxy.send_event(event) {
        eprintln!("[Native] Could not queue event: {error}");
    }
}

/// Global-hotkey worker thread (F8 core). Owns the Win32 hotkey table
/// plus its own message loop: each sync replaces the whole table
/// (unregister-all + re-register, Electron's refresh model), and every
/// `WM_HOTKEY` firing crosses to the event loop as `KeybindFired`
/// carrying only the keybind id. Ids never reach Win32 (worker-local
/// table indices do). Polls the sync channel every 50 ms so management
/// mutations apply promptly without blocking hotkey delivery. No hooks,
/// no SendInput, no synthetic key events.
fn run_hotkey_worker(
    rx: std::sync::mpsc::Receiver<keybinds::HotkeySync>,
    proxy: &EventLoopProxy<NativeEvent>,
) {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
        };
        use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, PM_REMOVE, WM_HOTKEY};
        let mut current: Vec<(i32, String)> = Vec::new();
        let mut next_id: i32 = 1;
        let unregister_all = |current: &mut Vec<(i32, String)>| {
            for (id, _) in current.drain(..) {
                unsafe {
                    let _ = UnregisterHotKey(None, id);
                }
            }
        };
        loop {
            // Drain hotkey firings without blocking.
            loop {
                let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
                let has =
                    unsafe { PeekMessageW(&mut message, None, WM_HOTKEY, WM_HOTKEY, PM_REMOVE) };
                if !has.as_bool() {
                    break;
                }
                let fired = message.wParam.0 as i32;
                if let Some((_, keybind_id)) = current.iter().find(|(id, _)| *id == fired) {
                    let keybind_id = keybind_id.clone();
                    queue_event(proxy, NativeEvent::KeybindFired { keybind_id });
                }
            }
            match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(keybinds::HotkeySync(bindings)) => {
                    unregister_all(&mut current);
                    for binding in &bindings {
                        let id = next_id;
                        next_id = next_id.wrapping_add(1);
                        let registered = unsafe {
                            RegisterHotKey(
                                None,
                                id,
                                HOT_KEY_MODIFIERS(binding.modifiers),
                                binding.vk,
                            )
                        };
                        if registered.is_ok() {
                            current.push((id, binding.id.clone()));
                        } else {
                            // Conflict (system/Discord/another app holds the
                            // chord): loud per-keybind skip, the rest stand.
                            eprintln!(
                                "[Keybinds][conflict] id={} modifiers={} vk={} is already registered; skipping",
                                binding.id, binding.modifiers, binding.vk
                            );
                        }
                    }
                    println!("[Keybinds][Hotkeys] registered={}", current.len());
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    unregister_all(&mut current);
                    return;
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        while rx.recv().is_ok() {}
    }
}

/// User-facing missing-Runtime diagnostic (release blocker 4). stderr for
/// console launches plus a message box for shortcut launches with no
/// console. Names the exact component and the remedy; no profile, browser,
/// or configuration is touched, and nothing is retried.
#[cfg(target_os = "windows")]
fn report_missing_runtime() {
    const MESSAGE: &str = "Remcord needs the Microsoft Edge WebView2 Evergreen Runtime to run, and none was found on this PC.\n\nInstall the Evergreen Runtime from Microsoft's official WebView2 page, then start Remcord again. Your Remcord data was not changed.";
    eprintln!("[Runtime] {MESSAGE}");
    use windows::{
        core::{HSTRING, PCWSTR},
        Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK},
    };
    let text = HSTRING::from(MESSAGE);
    let caption = HSTRING::from("Remcord");
    unsafe {
        MessageBoxW(
            None,
            PCWSTR::from_raw(text.as_ptr()),
            PCWSTR::from_raw(caption.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// Non-Windows fallback: stderr only (no WebView2 shell exists here).
#[cfg(not(target_os = "windows"))]
fn report_missing_runtime() {
    eprintln!(
        "[Runtime] WebView2 Evergreen Runtime is required on Windows; this platform is unsupported"
    );
}

/// Attaches the P0.5 download policy to one WebView builder (main and P0.4
/// child share this helper — no policy fork).
///
/// Start: take ONLY the file name from WebView2's suggested path (its
/// directory is discarded), sanitize, resolve a unique host-owned
/// destination; invalid/exhausted denies. Synchronous by wry design — no
/// picker, no shell, no JS, no bridge traffic. Completion only logs the
/// redacted URL, the bare file name, and success (paths stay host-side).
fn apply_download_handlers(
    builder: WebViewBuilder<'_>,
    downloads_dir: PathBuf,
) -> WebViewBuilder<'_> {
    let start_dir = downloads_dir.clone();
    let done_dir = downloads_dir;
    builder
        .with_download_started_handler(move |url, path| {
            let redacted = downloads::redact_url(&url);
            match downloads::decide_download(&start_dir, path) {
                downloads::DownloadDecision::Allow(destination) => {
                    let name = destination
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "?".to_owned());
                    println!("[Downloads] start file={name} source={redacted}");
                    *path = destination;
                    true
                }
                downloads::DownloadDecision::Deny => {
                    println!("[Downloads] denied source={redacted}");
                    false
                }
            }
        })
        .with_download_completed_handler(move |url, result_path, success| {
            let redacted = downloads::redact_url(&url);
            // Defense in depth: the completed path must still sit directly
            // beneath the host dir (we set it, but never trust it back).
            let name = result_path
                .as_ref()
                .filter(|path| path.parent() == Some(done_dir.as_path()))
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned());
            match (success, name) {
                (true, Some(name)) => {
                    println!("[Downloads] completed file={name} source={redacted}")
                }
                (true, None) => println!("[Downloads] completed-unverified source={redacted}"),
                (false, _) => println!("[Downloads] interrupted source={redacted}"),
            }
        })
}
/// DEV-only smoke/probe switches plus the two mutation-smoke stage
/// machines, copied per trusted-load run. Every field is a plain value:
/// the pipeline never flips a flag, so a failed document cannot perturb a
/// later run through shared switch state.
#[derive(Debug, Clone, Copy)]
struct TrustedLoadFlags {
    bare_page: bool,
    themes_smoke: bool,
    quickcss_smoke: bool,
    mod_smoke: bool,
    interaction: bool,
    settings_smoke: bool,
    settings_mutation_smoke: bool,
    settings_set_smoke: bool,
    getlang_smoke: bool,
    app_version_smoke: bool,
    graphics_smoke: bool,
    plugin_storage_smoke: bool,
    theme_install_smoke: bool,
    theme_write_smoke: bool,
    theme_import_smoke: bool,
    theme_set_enabled_smoke: bool,
    theme_list_smoke: bool,
    backup_save_smoke: bool,
    backup_restore_smoke: bool,
    shelter_loader_probe: bool,
    settings_boot_smoke: bool,
    remcord_protocol_smoke: bool,
    settings_ui_smoke: bool,
    permissions_smoke: bool,
    mutation_stage: u8,
    mutation_before: bool,
    settings_set_stage: u8,
    settings_set_before: bool,
}

/// Borrowed context for one deferred trusted-load run (Phase 2C). The
/// event loop builds this when a finished Discord document is proven
/// successful — either immediately (outcome already known or no
/// observer) or later, when the observer outcome resolves a stashed
/// finish. All document state is reborrowed per run; nothing is retained
/// between runs, so a failed document leaves no residue for the next one.
struct TrustedLoadContext<'a> {
    webview: &'a WebView,
    profile_path: &'a PathBuf,
    proxy: &'a EventLoopProxy<NativeEvent>,
    generation: u64,
    titlebar_plugin_start_script: &'a str,
    mod_smoke_script: &'a str,
    settings_ui_probe_script: &'a str,
    titlebar_settled_generation: &'a mut Option<u64>,
    last_quickcss_injected: &'a mut bool,
    last_quickcss_css: &'a mut Option<String>,
    last_theme_style_ids: &'a mut Vec<String>,
    last_themes: &'a mut Vec<LoadedTheme>,
    flags: TrustedLoadFlags,
}

/// Re-pushes the host-owned Quick CSS into the live document from the
/// authoritative settings plus file.
///
/// The trusted-load pipeline and the post-mutation side effects (the
/// `quickCss` settings toggle, the `quickcss.write` editor save) share this
/// helper, so a toggle or a save applies to the running document without a
/// reload. It updates the live watcher's last-known state exactly like the
/// pipeline always has, so live diffing never mistakes the push for an
/// external file change.
fn repush_quick_css(
    webview: &WebView,
    profile_path: &std::path::Path,
    last_injected: &mut bool,
    last_css: &mut Option<String>,
) {
    // Host-pushed Quick CSS injection (Phase D): the renderer
    // never requests a push, so there is no bridge
    // round trip here. The host reads its own fixed
    // quickCss.css when the authoritative settings flag
    // is enabled and pushes it through the shared
    // managed-CSS lifecycle BEFORE themes, preserving
    // Electron's Vencord -> Quick CSS -> themes cascade.
    let quickcss_report = load_quick_css(profile_path);
    let mut quickcss_injected = false;
    match &quickcss_report.css {
        Some(css) => match managed_css_script(QUICK_CSS_STYLE_ID, css) {
            Ok(script) => {
                if let Err(error) = webview.evaluate_script(&script) {
                    eprintln!("[QuickCss][css_injection] {error}");
                } else {
                    quickcss_injected = true;
                }
            }
            Err(error) => {
                eprintln!("[QuickCss][css_injection] {error}");
            }
        },
        None => {
            // Disabled (or unreadable) after a previous
            // injection: remove the managed style so no
            // stale Quick CSS survives the reset. On a
            // fresh document this is a cheap no-op.
            if *last_injected {
                match managed_css_remove_script(QUICK_CSS_STYLE_ID) {
                    Ok(script) => {
                        if let Err(error) = webview.evaluate_script(&script) {
                            eprintln!("[QuickCss][css_removal] {error}");
                        }
                    }
                    Err(error) => {
                        eprintln!("[QuickCss][css_removal] {error}");
                    }
                }
            }
        }
    }
    *last_injected = quickcss_injected;
    // Content snapshot for live diffing: `Some` exactly
    // when the managed style is present in this document.
    *last_css = if quickcss_injected {
        quickcss_report.css.clone()
    } else {
        None
    };
    println!(
        "[QuickCss][Metrics] enabled={} state={} injected={} css_bytes={} read_ms={:.3}",
        quickcss_report.enabled,
        quickcss_report.state,
        quickcss_injected,
        quickcss_report.css_bytes,
        quickcss_report.read_ms,
    );
}

/// The trusted Discord document pipeline: everything that assumes live
/// Discord content is present (titlebar settle, health/CSS/bridge probes,
/// host-pushed Quick CSS and themes, DEV smoke observations).
///
/// Phase 2C runs this ONLY for a successfully completed navigation —
/// never for a failed one, never twice for one document, never for a
/// superseded document. The caller owns that decision via
/// [`navigation_gate::NavigationGate`]; this function only executes it.
/// It performs no navigation, no reload, no retry, and no trust decision
/// of its own.
fn run_trusted_load_pipeline(ctx: TrustedLoadContext<'_>) {
    // Titlebar settle pass: the marker-time start can land
    // before Discord finishes mounting (warm reloads), and a
    // mount can discard body-appended nodes afterwards. This
    // one-shot-per-generation re-run of the same idempotent
    // script heals that wipe; the marker path still covers a
    // Shelter marker that arrives after finish, in either
    // order, with at most two evaluations per document and
    // never a retry loop.
    if *ctx.titlebar_settled_generation != Some(ctx.generation) {
        *ctx.titlebar_settled_generation = Some(ctx.generation);
        let generation = ctx.generation;
        if let Err(error) = ctx.webview.evaluate_script_with_callback(
            ctx.titlebar_plugin_start_script,
            move |result| {
                println!("[Titlebar] settle generation={generation} result={result}");
            },
        ) {
            eprintln!("[Titlebar] settle evaluation failed: {error}");
        }
    }
    if let Err(error) = ctx
        .webview
        .evaluate_script_with_callback(RENDERER_HEALTH_SCRIPT, |result| {
            println!("[Renderer][Health] {result}")
        })
    {
        eprintln!("[Renderer][Health] probe failed: {error}");
    }
    if !ctx.flags.bare_page {
        if let Err(error) = ctx.webview.evaluate_script(CSS_INJECTION_SCRIPT) {
            eprintln!("[Injection] CSS script failed: {error}");
        }
        if let Err(error) = ctx.webview.evaluate_script(BRIDGE_PROBE_SCRIPT) {
            eprintln!("[Injection] Bridge probe failed: {error}");
        }
        // Host-pushed Quick CSS injection (Phase D): shared with the
        // post-mutation side effects so toggles and editor saves apply to
        // the running document without a reload.
        repush_quick_css(
            ctx.webview,
            ctx.profile_path,
            ctx.last_quickcss_injected,
            ctx.last_quickcss_css,
        );
        // Host-pushed theme injection (Phase D): the renderer
        // never requests theme CSS, so there is no bridge
        // round trip here. The host loads enabled themes from
        // its own root and pushes each through the shared
        // managed-CSS lifecycle in sorted-id order.
        let theme_report = load_enabled_themes(ctx.profile_path);
        // Reset semantics: drop managed styles for themes that
        // were injected before but are no longer enabled, so
        // no stale CSS survives a document reset. On a fresh
        // document this is a cheap no-op (the nodes do not
        // exist yet); the removal script is idempotent.
        let current_theme_ids: Vec<String> = theme_report
            .injected
            .iter()
            .map(|theme| theme.style_id.clone())
            .collect();
        for stale_id in ctx
            .last_theme_style_ids
            .iter()
            .filter(|id| !current_theme_ids.contains(id))
        {
            match managed_css_remove_script(stale_id) {
                Ok(script) => {
                    if let Err(error) = ctx.webview.evaluate_script(&script) {
                        eprintln!("[Themes][css_removal] {stale_id}: {error}");
                    }
                }
                Err(error) => {
                    eprintln!("[Themes][css_removal] {stale_id}: {error}");
                }
            }
        }
        *ctx.last_theme_style_ids = current_theme_ids;
        let mut theme_injected = 0_usize;
        let mut injected_now: Vec<LoadedTheme> = Vec::new();
        for theme in &theme_report.injected {
            match managed_css_script(&theme.style_id, &theme.css) {
                Ok(script) => {
                    if let Err(error) = ctx.webview.evaluate_script(&script) {
                        eprintln!("[Themes][css_injection] {}: {error}", theme.id);
                    } else {
                        theme_injected += 1;
                        injected_now.push(theme.clone());
                    }
                }
                Err(error) => {
                    eprintln!("[Themes][css_injection] {}: {error}", theme.id);
                }
            }
        }
        // Content snapshot for live diffing: only successfully
        // injected themes, so a failed push is retried rather
        // than mistaken for present.
        *ctx.last_themes = injected_now;
        if theme_report.discovered > 0 || ctx.flags.themes_smoke {
            println!(
                "[Themes][Metrics] discovered={} enabled={} injected={} skipped={} css_bytes={} read_ms={:.3}",
                theme_report.discovered,
                theme_report.enabled,
                theme_injected,
                theme_report.skipped.len(),
                theme_report.css_bytes,
                theme_report.read_ms,
            );
            for skipped in &theme_report.skipped {
                println!("[Themes][Skip] id={} reason={}", skipped.id, skipped.reason);
            }
        }
        if ctx.flags.themes_smoke || ctx.flags.quickcss_smoke {
            // Copied for the 'static observation callback.
            let themes_smoke = ctx.flags.themes_smoke;
            let quickcss_smoke = ctx.flags.quickcss_smoke;
            if let Err(error) =
                ctx.webview
                    .evaluate_script_with_callback(THEMES_SMOKE_SCRIPT, move |result| {
                        if themes_smoke {
                            println!("[Themes][Smoke] managed_styles={result}");
                        }
                        if quickcss_smoke {
                            println!("[QuickCss][Smoke] managed_styles={result}");
                        }
                    })
            {
                eprintln!("[Themes][Smoke] observation failed: {error}");
            }
        }
        // DEV-only read-only observation of every active mod's
        // real page-world global, managed style, and adapter
        // state. Observation only: no request, no injection.
        if ctx.flags.mod_smoke {
            if let Err(error) = ctx
                .webview
                .evaluate_script_with_callback(ctx.mod_smoke_script, |result| {
                    println!("[Mods][Smoke] {result}")
                })
            {
                eprintln!("[Mods][Smoke] observation failed: {error}");
            }
        }
        if ctx.flags.interaction {
            if let Err(error) = ctx.webview.evaluate_script(INTERACTION_PROBE_SCRIPT) {
                eprintln!("[Renderer][Interaction] probe failed: {error}");
            }
        }
        if ctx.flags.settings_smoke {
            if let Err(error) = ctx.webview.evaluate_script(SETTINGS_SMOKE_SCRIPT) {
                eprintln!("[Settings][Smoke] probe failed: {error}");
            }
        }
        if ctx.flags.settings_mutation_smoke {
            if ctx.flags.mutation_stage == 0 {
                if let Err(error) = ctx.webview.evaluate_script(SETTINGS_MUTATION_SMOKE_SCRIPT) {
                    eprintln!("[Settings][MutationSmoke] probe failed: {error}");
                }
            } else if ctx.flags.mutation_stage == 1 {
                let restore = SETTINGS_MUTATION_RESTORE_SCRIPT_TEMPLATE.replace(
                    "__REMCORD_MUTATION_RESTORE_VALUE__",
                    if ctx.flags.mutation_before {
                        "true"
                    } else {
                        "false"
                    },
                );
                if let Err(error) = ctx.webview.evaluate_script(&restore) {
                    eprintln!("[Settings][MutationSmoke] restore failed: {error}");
                }
            }
        }
        if ctx.flags.settings_set_smoke {
            if ctx.flags.settings_set_stage == 0 {
                if let Err(error) = ctx.webview.evaluate_script(SETTINGS_SET_SMOKE_SCRIPT) {
                    eprintln!("[Settings][SetSmoke] probe failed: {error}");
                }
            } else if ctx.flags.settings_set_stage == 1 {
                let restore = SETTINGS_SET_RESTORE_SCRIPT_TEMPLATE.replace(
                    "__REMCORD_SETTINGS_SET_RESTORE_VALUE__",
                    if ctx.flags.settings_set_before {
                        "true"
                    } else {
                        "false"
                    },
                );
                if let Err(error) = ctx.webview.evaluate_script(&restore) {
                    eprintln!("[Settings][SetSmoke] restore failed: {error}");
                }
            }
        }
        if ctx.flags.getlang_smoke {
            if let Err(error) = ctx.webview.evaluate_script(GETLANG_SMOKE_SCRIPT) {
                eprintln!("[Localization][Smoke] probe failed: {error}");
            }
        }
        if ctx.flags.app_version_smoke {
            if let Err(error) = ctx.webview.evaluate_script(APP_VERSION_SMOKE_SCRIPT) {
                eprintln!("[App][Smoke] probe failed: {error}");
            }
        }
        if ctx.flags.graphics_smoke {
            if let Err(error) = ctx.webview.evaluate_script(GRAPHICS_SMOKE_SCRIPT) {
                eprintln!("[Graphics][Smoke] probe failed: {error}");
            }
        }
        if ctx.flags.plugin_storage_smoke {
            if let Err(error) = ctx.webview.evaluate_script(PLUGIN_STORAGE_SMOKE_SCRIPT) {
                eprintln!("[PluginStorage][Smoke] probe failed: {error}");
            }
        }
        if ctx.flags.theme_install_smoke {
            if let Err(error) = ctx.webview.evaluate_script(THEME_INSTALL_SMOKE_SCRIPT) {
                eprintln!("[Themes][InstallSmoke] probe failed: {error}");
            }
        }
        if ctx.flags.theme_write_smoke {
            if let Err(error) = ctx.webview.evaluate_script(THEME_WRITE_SMOKE_SCRIPT) {
                eprintln!("[Themes][WriteSmoke] probe failed: {error}");
            }
        }
        if ctx.flags.theme_import_smoke {
            if let Err(error) = ctx.webview.evaluate_script(THEME_IMPORT_SMOKE_SCRIPT) {
                eprintln!("[Themes][ImportSmoke] probe failed: {error}");
            }
        }
        if ctx.flags.theme_set_enabled_smoke {
            if let Err(error) = ctx.webview.evaluate_script(THEME_SET_ENABLED_SMOKE_SCRIPT) {
                eprintln!("[Themes][SetEnabledSmoke] probe failed: {error}");
            }
        }
        if ctx.flags.theme_list_smoke {
            if let Err(error) = ctx.webview.evaluate_script(THEME_LIST_SMOKE_SCRIPT) {
                eprintln!("[Themes][ListSmoke] probe failed: {error}");
            }
        }
        if ctx.flags.backup_save_smoke {
            if let Err(error) = ctx.webview.evaluate_script(BACKUP_SAVE_SMOKE_SCRIPT) {
                eprintln!("[Backup][SaveSmoke] probe failed: {error}");
            }
        }
        if ctx.flags.backup_restore_smoke {
            if let Err(error) = ctx.webview.evaluate_script(BACKUP_RESTORE_SMOKE_SCRIPT) {
                eprintln!("[Backup][RestoreSmoke] probe failed: {error}");
            }
        }
        if ctx.flags.shelter_loader_probe {
            if let Err(error) = ctx.webview.evaluate_script(SHELTER_LOADER_PROBE_SCRIPT) {
                eprintln!("[ShelterLoader][Probe] probe failed: {error}");
            }
        }
        if ctx.flags.settings_boot_smoke {
            if let Err(error) = ctx.webview.evaluate_script(SETTINGS_BOOT_SMOKE_SCRIPT) {
                eprintln!("[SettingsBoot][Smoke] probe failed: {error}");
            }
        }
        if ctx.flags.remcord_protocol_smoke {
            if let Err(error) = ctx.webview.evaluate_script(REMCORD_PROTOCOL_SMOKE_SCRIPT) {
                eprintln!("[RemcordProtocol][Smoke] probe failed: {error}");
            } else {
                // One-shot delayed readback: subresource loads
                // settle asynchronously, so the flags are read
                // ~5 s later on the event loop (DEV only).
                let readback_proxy = ctx.proxy.clone();
                if let Err(error) = thread::Builder::new()
                    .name("remcord-protocol-smoke-readback".to_owned())
                    .spawn(move || {
                        thread::sleep(Duration::from_millis(5000));
                        queue_event(&readback_proxy, NativeEvent::RemcordProtocolSmokeReadback);
                    })
                {
                    eprintln!("[RemcordProtocol][Smoke] readback timer failed: {error}");
                }
            }
        }
        if ctx.flags.settings_ui_smoke {
            if let Err(error) = ctx.webview.evaluate_script(ctx.settings_ui_probe_script) {
                eprintln!("[SettingsUi][Smoke] probe failed: {error}");
            }
        }
        if ctx.flags.permissions_smoke {
            if let Err(error) = ctx.webview.evaluate_script(PERMISSIONS_SMOKE_SCRIPT) {
                eprintln!("[Permissions][Smoke] probe failed: {error}");
            } else {
                // One-shot delayed readback: media and
                // geolocation settle asynchronously, so the
                // flags are read ~8 s later (DEV only).
                let readback_proxy = ctx.proxy.clone();
                if let Err(error) = thread::Builder::new()
                    .name("remcord-permissions-smoke-readback".to_owned())
                    .spawn(move || {
                        thread::sleep(Duration::from_millis(8000));
                        queue_event(&readback_proxy, NativeEvent::PermissionsSmokeReadback);
                    })
                {
                    eprintln!("[Permissions][Smoke] readback timer failed: {error}");
                }
            }
        }
    }
}
/// The single managed popup child (P0.4). At most one lives at a time:
/// the approved Discord popout / `about:blank` / hidden ICS surface.
/// The `WebView` field is never read — it exists to keep the child alive
/// exactly as long as the slot; dropping the slot destroys the child.
struct PopupChild {
    window: Arc<tao::window::Window>,
    #[allow(dead_code)]
    webview: WebView,
    /// Slot generation assigned at creation; scopes the child observer's
    /// per-instance id and counters. Never reused across children.
    instance_id: u64,
    /// Phase 1 process-failure subscription: the CoreWebView2 clone plus
    /// its registration token, removed before the slot is dropped.
    #[cfg(target_os = "windows")]
    process_failed: Option<(ICoreWebView2, i64)>,
    /// Phase 2B navigation-completed subscription: the CoreWebView2 clone
    /// plus its registration token, removed before the slot is dropped.
    /// A second, additive `NavigationCompleted` handler — wry's own
    /// page-load subscription is untouched.
    #[cfg(target_os = "windows")]
    navigation_completed: Option<(ICoreWebView2, i64)>,
}

/// Builds the single managed child for an approved popup (P0.4).
///
/// Verified mechanism (A): wry's own `NewWindowResponse::Create` path needs
/// a caller-built `ICoreWebView2` plus an `EventLoopWindowTarget` that the
/// `NewWindowRequested` closure does not have, so approved popups are
/// denied at that layer and re-created here on the event loop instead.
/// Every child option is host-owned: fixed 800x600 size, default
/// decorations, `alwaysOnTop` only from the host `popoutPiP` setting and
/// only for the Discord popout. Renderer window features are never read.
/// The child shares the main profile/session (same `WebContext` plus the
/// same adopted `ICoreWebView2Environment`), the same document-start
/// facade, the same bridge origin gate, the same `remcord://` registration,
/// and the same permission policy; nested popups deny unconditionally and
/// navigation stays clamped to Discord (plus the ICS blob shape for hidden
/// ICS children only).
#[allow(clippy::too_many_arguments)]
fn create_popup_child(
    target: &tao::event_loop::EventLoopWindowTarget<NativeEvent>,
    kind: new_window::PopupKind,
    url: &str,
    web_context: &mut WebContext,
    #[cfg(target_os = "windows")] environment: ICoreWebView2Environment,
    initialization_script: &str,
    ipc_proxy: &EventLoopProxy<NativeEvent>,
    roots: &crate::remcord_protocol::NamespaceRoots,
    downloads_dir: &std::path::Path,
    always_on_top: bool,
    instance_id: u64,
) -> Result<PopupChild, String> {
    let (visible, initial_url) = match kind {
        new_window::PopupKind::AboutBlank => (true, "about:blank"),
        // Hidden transient (Electron `show:false` parity). No download
        // manager exists yet: the blob load is attempted, never shown.
        new_window::PopupKind::Ics => (false, url),
        new_window::PopupKind::DiscordPopout => (true, url),
    };
    let child_window = WindowBuilder::new()
        .with_title("Remcord")
        .with_inner_size(Size::Logical(LogicalSize::new(800.0, 600.0)))
        .with_visible(false)
        .with_always_on_top(always_on_top && kind == new_window::PopupKind::DiscordPopout)
        .build(target)
        .map_err(|error| format!("popup window failed: {error}"))?;
    let child_window = Arc::new(child_window);
    let allow_blob = kind == new_window::PopupKind::Ics;
    let child_ipc_proxy = ipc_proxy.clone();
    // The builder IPC closure below takes `child_ipc_proxy`; the observer
    // keeps its own clone (same event loop).
    let child_observer_proxy = child_ipc_proxy.clone();
    let mut builder = WebViewBuilder::new_with_web_context(web_context);
    #[cfg(target_os = "windows")]
    {
        builder = builder.with_environment(environment);
    }
    let builder = apply_download_handlers(builder, downloads_dir.to_path_buf());
    let builder = builder
        .with_url(initial_url)
        .with_initialization_script_for_main_only(initialization_script, true)
        .with_navigation_handler(move |navigation_url| {
            if is_allowed_navigation(&navigation_url) {
                return true;
            }
            if allow_blob && new_window::is_discord_ics_blob_url(&navigation_url) {
                return true;
            }
            println!("[Popup][Navigation] block {navigation_url}");
            false
        })
        .with_new_window_req_handler(|nested_url, _features| {
            println!("[Popup][Navigation] blocked nested popup: {nested_url}");
            NewWindowResponse::Deny
        })
        .with_ipc_handler(move |request: Request<String>| {
            if !is_trusted_bridge_origin(request.uri()) {
                eprintln!(
                    "[Popup][Bridge] rejected message from untrusted origin: {}",
                    request.uri()
                );
                return;
            }
            match parse_bridge_message(request.body()) {
                Ok(BridgeMessage::Request(request)) => queue_event(
                    &child_ipc_proxy,
                    NativeEvent::BridgeRequest(request, Instant::now()),
                ),
                Ok(BridgeMessage::Event(event)) => {
                    queue_event(&child_ipc_proxy, NativeEvent::BridgeEvent(event));
                }
                Err(error) => eprintln!("[Popup][Bridge] rejected invalid message: {error}"),
            }
        });
    let child_webview = builder
        .build(&child_window)
        .map_err(|error| format!("popup webview failed: {error}"))?;
    #[cfg(target_os = "windows")]
    {
        let core_webview2: ICoreWebView2 = child_webview.webview();
        if let Err(error) = register_remcord_protocol(&core_webview2, roots.clone()) {
            eprintln!("[Popup][RemcordProtocol] registration failed: {error}");
        }
        if let Err(error) = permissions::register(&core_webview2) {
            eprintln!("[Popup][Permissions] registration failed: {error}");
        }
    }
    // Phase 1 process-failure observer (observability only). A
    // registration failure is logged loudly but must not block the
    // child: the pre-observer behavior (no visibility) is the fallback.
    #[cfg(target_os = "windows")]
    let process_failed = {
        let core_webview2: ICoreWebView2 = child_webview.webview();
        match process_failure::register(
            &core_webview2,
            process_failure::WebViewRole::Child,
            instance_id,
            &child_observer_proxy,
        ) {
            Ok(token) => {
                println!(
                    "[WebView][ProcessFailed] observer registered role=child instance={instance_id}"
                );
                Some((core_webview2, token))
            }
            Err(error) => {
                eprintln!(
                    "[WebView][ProcessFailed] observer registration failed role=child: {error}"
                );
                None
            }
        }
    };
    // Phase 2B navigation-completed observer (observability only). A
    // SECOND `NavigationCompleted` subscription: wry's own page-load
    // handler stays registered and keeps firing `PageLoadFinished`
    // unconditionally. A registration failure is logged loudly but must
    // not block the child: the pre-observer behavior (wry page-load
    // events only) is the fallback.
    #[cfg(target_os = "windows")]
    let navigation_completed = {
        let core_webview2: ICoreWebView2 = child_webview.webview();
        match navigation_completed::register(
            &core_webview2,
            process_failure::WebViewRole::Child,
            instance_id,
            &child_observer_proxy,
        ) {
            Ok(token) => {
                println!(
                    "[WebView][NavigationCompleted] observer registered role=child instance={instance_id}"
                );
                Some((core_webview2, token))
            }
            Err(error) => {
                eprintln!(
                    "[WebView][NavigationCompleted] observer registration failed role=child: {error}"
                );
                None
            }
        }
    };
    if visible {
        child_window.set_visible(true);
    }
    Ok(PopupChild {
        window: child_window,
        webview: child_webview,
        instance_id,
        #[cfg(target_os = "windows")]
        process_failed,
        #[cfg(target_os = "windows")]
        navigation_completed,
    })
}

/// Name of the dedicated STA worker thread that owns the theme import
/// picker dialog plus the sequential bounded ingestion. The modal dialog
/// must never run on the WebView/UI thread: COM is initialized exactly once
/// for this thread (STA, as the Common Item Dialog requires — never once per
/// request), and picker jobs are processed sequentially for determinism and
/// bounded memory.
const THEME_IMPORT_WORKER_NAME: &str = "remcord-theme-import-sta";

/// One theme-import picker job for the dedicated STA worker thread. The
/// renderer supplies nothing: the picker configuration is host-owned and the
/// profile path is host-built.
struct ThemeImportJob {
    request_id: String,
    received_at: Instant,
    profile: PathBuf,
    proxy: EventLoopProxy<NativeEvent>,
}

/// Name of the dedicated STA worker thread that owns the backup save dialog
/// plus collection, ZIP creation, and the final write. The modal dialog must
/// never run on the WebView/UI thread: COM is initialized exactly once for
/// this thread (STA, as the Common Item Dialog requires — never once per
/// request), and save jobs are processed sequentially for determinism and
/// bounded memory.
const BACKUP_SAVE_WORKER_NAME: &str = "remcord-backup-save-sta";

/// One backup-save job for the dedicated STA worker thread. The renderer
/// supplies only the validated include flags plus the bounded clientMods
/// envelope: collection roots, the manifest, ZIP creation, the dialog, and
/// the output path are all host-owned.
struct BackupSaveJob {
    request_id: String,
    received_at: Instant,
    profile: PathBuf,
    includes: backup::BackupIncludes,
    client_mods_manifest: serde_json::Value,
    proxy: EventLoopProxy<NativeEvent>,
}

/// Run one backup-save job on the STA worker: synthesize the manifest,
/// collect bounded entries, build the ZIP, show the host-owned save dialog,
/// and persist through a sibling temp file plus rename. Timings are logged
/// by stage (collection, ZIP, dialog wait, write) so dialog wait never
/// masquerades as host work.
fn run_backup_save_job(job: &BackupSaveJob) -> Result<BackupSaveSummary, BackupFailure> {
    let request = backup::ValidatedBackupRequest {
        includes: job.includes,
        client_mods_manifest: job.client_mods_manifest.clone(),
    };
    let collect_started = Instant::now();
    let manifest = build_backup_manifest(&request, current_app_version(), &created_at_now());
    let entries = collect_backup_entries(&job.profile, &job.includes).inspect_err(|failure| {
        eprintln!("[Backup][Save:{}] {}", failure.code, failure.message);
    })?;
    let collect_ms = collect_started.elapsed().as_secs_f64() * 1000.0;
    let uncompressed: usize = entries.iter().map(|entry| entry.data.len()).sum();
    let zip_started = Instant::now();
    let zip_bytes = build_backup_zip(&manifest, &entries)?;
    let zip_ms = zip_started.elapsed().as_secs_f64() * 1000.0;
    let dialog_started = Instant::now();
    let target = match show_save_picker(&job.profile) {
        SavePickerOutcome::Canceled => {
            println!("[Backup][Save] picker dismissed by the user");
            return Ok(BackupSaveSummary {
                canceled: true,
                bytes: 0,
            });
        }
        SavePickerOutcome::Selected(path) => path,
        SavePickerOutcome::Failed(failure) => {
            eprintln!("[Backup][Save:{}] {}", failure.code, failure.message);
            return Err(failure);
        }
    };
    let dialog_ms = dialog_started.elapsed().as_secs_f64() * 1000.0;
    let write_started = Instant::now();
    let persisted = backup::write_backup_file(&target, &zip_bytes).inspect_err(|failure| {
        eprintln!("[Backup][Save:{}] {}", failure.code, failure.message);
    })?;
    let write_ms = write_started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "[Backup][Save] files={} uncompressed={} bytes={} collect_ms={:.3} zip_ms={:.3} dialog_wait_ms={:.3} write_ms={:.3}",
        entries.len(),
        uncompressed,
        persisted,
        collect_ms,
        zip_ms,
        dialog_ms,
        write_ms,
    );
    Ok(BackupSaveSummary {
        canceled: false,
        bytes: persisted,
    })
}

/// Name of the dedicated STA worker thread that owns the backup restore
/// dialog plus the bounded read, full validation, staging, and commit with
/// rollback. The modal dialog must never run on the WebView/UI thread: COM
/// is initialized exactly once for this thread (STA, as the Common Item
/// Dialog requires — never once per request), and restore jobs are processed
/// sequentially for determinism and bounded memory. A separate worker from
/// the save path keeps the two lifecycles (write-new-archive vs
/// overwrite-profile) isolated: a stuck save dialog can never block a
/// restore and vice versa.
const BACKUP_RESTORE_WORKER_NAME: &str = "remcord-backup-restore-sta";

/// One backup-restore job for the dedicated STA worker thread. The renderer
/// supplies nothing: the dialog, the picked path, validation, staging,
/// commit, and discard are all host-owned. Inspect/commit/discard share the
/// worker (and its one held staged restore) so the three steps serialize:
/// a second inspect replaces the previous hold, and a commit applies
/// exactly the held staged bytes the preview described.
struct BackupRestoreJob {
    kind: BackupRestoreKind,
    request_id: String,
    received_at: Instant,
    profile: PathBuf,
    proxy: EventLoopProxy<NativeEvent>,
}

/// Which restore step one worker job performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackupRestoreKind {
    /// Show the host-owned open dialog; on selection, read bounded,
    /// validate the entire archive, and stage outside the profile. The
    /// staged bytes are HELD worker-side; only the preview crosses back.
    /// Cancellation is normal control flow (`None`, no hold).
    Inspect,
    /// Commit the held staged restore with rollback. With nothing held this
    /// is a typed error, never a silent no-op.
    Commit,
    /// Drop the held staged restore without touching the profile.
    Discard,
}

/// Shared send path for the three restore steps: queue the job on the
/// dedicated STA restore worker, or fail loud and inline when the worker is
/// gone (the renderer must never hang waiting for a dead worker).
#[allow(clippy::too_many_arguments)]
fn send_backup_restore_job(
    restore_tx: &std::sync::mpsc::Sender<BackupRestoreJob>,
    webview: &WebView,
    request_tracker: &mut RequestTracker,
    response_count: &mut u64,
    kind: BackupRestoreKind,
    request_id: String,
    received_at: Instant,
    profile_path: &std::path::Path,
    proxy: &EventLoopProxy<NativeEvent>,
    step: &str,
) {
    let job = BackupRestoreJob {
        kind,
        request_id: request_id.clone(),
        received_at,
        profile: profile_path.to_path_buf(),
        proxy: proxy.clone(),
    };
    match restore_tx.send(job) {
        Ok(()) => {
            println!("[Backup][Restore] {step} request accepted");
        }
        Err(_) => {
            eprintln!("[Backup][Restore:BACKUP_UNAVAILABLE] restore worker is unavailable");
            let response = BridgeResponse {
                version: PROTOCOL_VERSION,
                request_id: request_id.clone(),
                ok: false,
                payload: None,
                error: Some(BridgeError {
                    code: "BACKUP_UNAVAILABLE",
                    message: "backup restore is unavailable".to_owned(),
                }),
            };
            send_bridge_response(webview, &response);
            if let Err(error) = request_tracker.complete(&request_id) {
                eprintln!("[Bridge] correlation failure after response: {error}");
            }
            *response_count += 1;
        }
    }
}

/// Outcome of one backup-restore worker job, delivered to the event loop.
#[derive(Debug)]
enum BackupRestoreOutcome {
    /// Inspect finished: `None` when the user dismissed the picker (no
    /// hold), otherwise the preview of the newly held staged restore.
    Inspected(Result<Option<RestorePreview>, BackupFailure>),
    /// Commit finished: the restore summary, or the failure (including
    /// committing with nothing held).
    Committed(Result<RestoreSummary, BackupFailure>),
    /// Discard finished: whether a held staged restore was actually
    /// dropped.
    Discarded(bool),
}

/// Run one backup-restore job on the STA worker. `held` is the worker-local
/// single staged-restore slot, threaded through every job so inspect holds,
/// commit consumes, and discard drops.
///
/// Inspect timings are logged by stage (dialog wait, validation + staging)
/// so dialog wait never masquerades as host work.
fn run_backup_restore_job(
    job: &BackupRestoreJob,
    held: &mut Option<StagedRestore>,
) -> BackupRestoreOutcome {
    match job.kind {
        BackupRestoreKind::Inspect => {
            let dialog_started = Instant::now();
            let picked = match show_open_picker(&job.profile) {
                OpenPickerOutcome::Canceled => {
                    println!("[Backup][Inspect] picker dismissed by the user");
                    return BackupRestoreOutcome::Inspected(Ok(None));
                }
                OpenPickerOutcome::Selected(path) => path,
                OpenPickerOutcome::Failed(failure) => {
                    eprintln!("[Backup][Inspect:{}] {}", failure.code, failure.message);
                    return BackupRestoreOutcome::Inspected(Err(failure));
                }
            };
            let dialog_ms = dialog_started.elapsed().as_secs_f64() * 1000.0;
            let stage_started = Instant::now();
            let staged = (|| {
                let bytes = read_archive_file(&picked).inspect_err(|failure| {
                    eprintln!("[Backup][Inspect:{}] {}", failure.code, failure.message);
                })?;
                stage_restore_from_bytes(&bytes).inspect_err(|failure| {
                    eprintln!("[Backup][Inspect:{}] {}", failure.code, failure.message);
                })
            })();
            let stage_ms = stage_started.elapsed().as_secs_f64() * 1000.0;
            match staged {
                Ok(staged) => {
                    // A second inspect replaces the previous hold: the old
                    // staging is discarded, never committed behind the new
                    // preview's back.
                    if let Some(previous) = held.replace(staged) {
                        discard_staged_restore(previous);
                        println!("[Backup][Inspect] replaced a previous held restore");
                    }
                    let preview = held.as_ref().map(StagedRestore::preview);
                    println!(
                        "[Backup][Inspect] held entries={} dialog_wait_ms={:.3} stage_ms={:.3}",
                        preview
                            .as_ref()
                            .map(|preview| preview.entry_count)
                            .unwrap_or(0),
                        dialog_ms,
                        stage_ms,
                    );
                    BackupRestoreOutcome::Inspected(Ok(preview))
                }
                Err(failure) => BackupRestoreOutcome::Inspected(Err(failure)),
            }
        }
        BackupRestoreKind::Commit => {
            let Some(staged) = held.take() else {
                eprintln!("[Backup][Commit:NO_STAGED_RESTORE] no staged restore is held");
                return BackupRestoreOutcome::Committed(Err(BackupFailure {
                    code: "NO_STAGED_RESTORE",
                    message: "no staged restore is held; inspect a backup first".to_owned(),
                }));
            };
            let summary =
                commit_staged_restore(&job.profile, staged, None).inspect_err(|failure| {
                    eprintln!("[Backup][Commit:{}] {}", failure.code, failure.message);
                });
            if let Ok(summary) = &summary {
                println!(
                    "[Backup][Commit] entries={} uncompressed={} archive={} client_mods={}",
                    summary.entry_count,
                    summary.uncompressed_bytes,
                    summary.archive_bytes,
                    summary.client_mods_bytes,
                );
            }
            BackupRestoreOutcome::Committed(summary)
        }
        BackupRestoreKind::Discard => {
            let discarded = match held.take() {
                Some(staged) => {
                    discard_staged_restore(staged);
                    true
                }
                None => false,
            };
            println!("[Backup][Discard] discarded={discarded}");
            BackupRestoreOutcome::Discarded(discarded)
        }
    }
}

fn send_bridge_response(webview: &WebView, response: &BridgeResponse) {
    match serde_json::to_string(response) {
        Ok(serialized) => {
            let serialized =
                serde_json::to_string(&serialized).expect("a JSON string must serialize to JSON");
            let script = format!(
                "window.dispatchEvent(new CustomEvent('remcord-native:response', {{ detail: JSON.parse({serialized}) }}));"
            );
            if let Err(error) = webview.evaluate_script(&script) {
                eprintln!("[Bridge] Could not deliver response to renderer: {error}");
            }
        }
        Err(error) => eprintln!("[Bridge] Could not serialize response: {error}"),
    }
}

/// Sends one gallery success response and completes request correlation.
/// Shared by every `gallery.*` dispatch arm so the thirteen operations
/// cannot drift in envelope shape, correlation, or metrics logging.
fn respond_gallery_ok(
    webview: &WebView,
    request_tracker: &mut RequestTracker,
    response_count: &mut u64,
    request_id: String,
    payload: BridgeResponsePayload,
    metric: &str,
    latency_ms: f64,
) {
    let response = BridgeResponse {
        version: PROTOCOL_VERSION,
        request_id: request_id.clone(),
        ok: true,
        payload: Some(payload),
        error: None,
    };
    send_bridge_response(webview, &response);
    if let Err(error) = request_tracker.complete(&request_id) {
        eprintln!("[Bridge] correlation failure after response: {error}");
    }
    *response_count += 1;
    println!("[Gallery][Metrics] {metric} request_latency_ms={latency_ms:.3}");
}

/// Sends one gallery typed failure: path-free code plus message, same
/// envelope discipline as every other capability.
fn respond_gallery_err(
    webview: &WebView,
    request_tracker: &mut RequestTracker,
    response_count: &mut u64,
    request_id: String,
    code: &'static str,
    message: String,
    latency_ms: f64,
) {
    eprintln!("[Gallery][{code}] {message}");
    let response = BridgeResponse {
        version: PROTOCOL_VERSION,
        request_id: request_id.clone(),
        ok: false,
        payload: None,
        error: Some(BridgeError { code, message }),
    };
    send_bridge_response(webview, &response);
    if let Err(error) = request_tracker.complete(&request_id) {
        eprintln!("[Bridge] correlation failure after response: {error}");
    }
    *response_count += 1;
    println!("[Gallery][Metrics] op=error code={code} request_latency_ms={latency_ms:.3}");
}

/// Sends one keybind success response and completes request correlation.
/// Shared by every `keybinds.*` dispatch arm so the four operations
/// cannot drift in envelope shape, correlation, or metrics logging.
fn respond_keybinds_ok(
    webview: &WebView,
    request_tracker: &mut RequestTracker,
    response_count: &mut u64,
    request_id: String,
    payload: BridgeResponsePayload,
    metric: &str,
    latency_ms: f64,
) {
    let response = BridgeResponse {
        version: PROTOCOL_VERSION,
        request_id: request_id.clone(),
        ok: true,
        payload: Some(payload),
        error: None,
    };
    send_bridge_response(webview, &response);
    if let Err(error) = request_tracker.complete(&request_id) {
        eprintln!("[Bridge] correlation failure after response: {error}");
    }
    *response_count += 1;
    println!("[Keybinds][Metrics] {metric} request_latency_ms={latency_ms:.3}");
}

/// Sends one keybind typed failure: path-free code plus message, same
/// envelope discipline as every other capability.
fn respond_keybinds_err(
    webview: &WebView,
    request_tracker: &mut RequestTracker,
    response_count: &mut u64,
    request_id: String,
    code: &'static str,
    message: String,
    latency_ms: f64,
) {
    eprintln!("[Keybinds][{code}] {message}");
    let response = BridgeResponse {
        version: PROTOCOL_VERSION,
        request_id: request_id.clone(),
        ok: false,
        payload: None,
        error: Some(BridgeError { code, message }),
    };
    send_bridge_response(webview, &response);
    if let Err(error) = request_tracker.complete(&request_id) {
        eprintln!("[Bridge] correlation failure after response: {error}");
    }
    *response_count += 1;
    println!("[Keybinds][Metrics] op=error code={code} request_latency_ms={latency_ms:.3}");
}

/// Re-validates one keybind firing and executes its stored action.
/// `from_hotkey` selects the expected scope: worker firings must name
/// global entries, matcher reports must name non-global ones — a cross
/// claim is logged and dropped. Unknown or disabled ids are logged and
/// dropped (a stale table or a crafted event can never execute).
fn fire_keybind(
    webview: &WebView,
    profile_path: &std::path::Path,
    keybind_id: &str,
    from_hotkey: bool,
) {
    let entries = match keybinds::list_keybinds(profile_path) {
        Ok(entries) => entries,
        Err(failure) => {
            eprintln!("[Keybinds][{}] {}", failure.code, failure.message);
            return;
        }
    };
    let Some(entry) = entries.iter().find(|entry| entry.id == keybind_id) else {
        eprintln!("[Keybinds][unknown] firing for an unknown keybind id; dropping");
        return;
    };
    if !entry.enabled {
        eprintln!("[Keybinds][disabled] firing for a disabled keybind; dropping");
        return;
    }
    if entry.global != from_hotkey {
        eprintln!("[Keybinds][scope] firing claimed the wrong scope; dropping");
        return;
    }
    execute_keybind_action(webview, entry.action);
}

/// Executes one validated keybind action in the main document. The action
/// selects a fixed page script (flux dispatch, history calls, the F4
/// QuickCSS handoff, settings pushState); nothing is interpolated and no
/// response is ever sent (fire-and-forget, like Electron's `runAction`).
fn execute_keybind_action(webview: &WebView, action: keybinds::KeybindAction) {
    let script = keybinds::keybind_action_script(action);
    if let Err(error) = webview.evaluate_script(&script) {
        eprintln!(
            "[Keybinds][{}] action script failed: {error}",
            action.as_str()
        );
    } else {
        println!("[Keybinds][Action] action={}", action.as_str());
    }
}

/// Re-registers the worker hotkey table from authoritative settings
/// (Electron's `refreshGlobalKeybinds` model). Runs after startup and
/// after every successful management mutation. A broken worker channel
/// only logs: management still works, hotkeys go inert until restart.
fn sync_hotkey_worker(
    sender: &Option<std::sync::mpsc::Sender<keybinds::HotkeySync>>,
    profile_path: &std::path::Path,
) {
    let Some(sender) = sender else {
        return;
    };
    let bindings = match keybinds::list_keybinds(profile_path) {
        Ok(entries) => keybinds::global_hotkey_bindings(&entries),
        Err(failure) => {
            eprintln!("[Keybinds][{}] {}", failure.code, failure.message);
            return;
        }
    };
    if sender.send(keybinds::HotkeySync(bindings)).is_err() {
        eprintln!(
            "[Keybinds] hotkey worker is unavailable; global hotkeys are inert until restart"
        );
    }
}

/// Converts a renderer-supplied save-metadata envelope into the bounded
/// store form. Strings are sliced exactly like Electron's save paths;
/// unknown members never parse (protocol `deny_unknown_fields`).
fn gallery_meta_from_request(meta: &protocol::GalleryMetaRequest) -> gallery::GalleryMeta {
    gallery::GalleryMeta {
        title: meta.title.clone(),
        guild_id: meta.guild_id.clone(),
        guild_name: meta.guild_name.clone(),
        channel_id: meta.channel_id.clone(),
        channel_name: meta.channel_name.clone(),
        author_name: meta.author_name.clone(),
        message_id: meta.message_id.clone(),
        categories: meta.categories.clone(),
    }
}

/// Host-owned authoritative window state. `focused` converges only from Tao
/// `Focused` events; `maximized` converges from `Resized` queries plus the
/// maximize/unmaximize commands. Nothing here polls: every update is pushed
/// by a window event or a renderer command.
#[derive(Default)]
struct NativeWindowState {
    focused: bool,
    maximized: bool,
}

/// Delivers one typed `window_state` host event to the renderer. Callers must
/// invoke this only when the effective state actually changed.
fn send_window_state_event(webview: &WebView, focused: bool, maximized: bool) {
    let detail = window_state_event_json(focused, maximized);
    match serde_json::to_string(&detail) {
        Ok(serialized) => {
            let script = format!(
                "window.dispatchEvent(new CustomEvent('remcord-native:window-state', {{ detail: JSON.parse({serialized}) }}));"
            );
            if let Err(error) = webview.evaluate_script(&script) {
                eprintln!("[Window] Could not deliver window state to renderer: {error}");
            }
        }
        Err(error) => eprintln!("[Window] Could not serialize window state: {error}"),
    }
}

/// Re-queries the native maximized flag and emits a `window_state` event only
/// when it differs from the tracked state. Used after maximize/unmaximize
/// commands and `Resized` events. Focused state is never queried here: it
/// converges exclusively from `Focused` events.
fn sync_native_maximized_state(
    webview: &WebView,
    window: &Arc<tao::window::Window>,
    state: &mut NativeWindowState,
    reason: &str,
) {
    let maximized = window.is_maximized();
    if state.maximized != maximized {
        state.maximized = maximized;
        send_window_state_event(webview, state.focused, state.maximized);
        println!("[Window] maximized={maximized} ({reason})");
    }
}

/// Builds the application relaunch command for the settings-restart path.
///
/// Mod adapters are resolved once at startup and baked into the WebView's
/// document-start script, so a document reload can never activate a newly
/// enabled mod: restart must be a real relaunch. The command re-executes the
/// currently running binary with no arguments (the native shell takes none;
/// profile and diagnostics travel via the inherited environment) and, on
/// Windows, detached so it survives the parent's orderly shutdown. Pure
/// construction — the caller spawns, then shuts down; a spawn failure keeps
/// the running app alive instead of stranding the user.
fn relaunch_command() -> Result<std::process::Command, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("relaunch cannot locate the running executable: {error}"))?;
    let mut command = std::process::Command::new(&exe);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS: the child keeps running after this process exits.
        command.creation_flags(0x00000008);
    }
    Ok(command)
}

/// The orderly renderer-initiated shutdown shared by `window_quit` and a
/// `window_hide` that cannot safely hide (release blocker 1: no native tray
/// icon exists, so hiding would strand the app with no reopen path).
///
/// Prints the final failure counters, removes every observer subscription
/// best-effort (already-dead COM objects only log), and exits the event
/// loop — the same path as main-window `CloseRequested`, minus its extra
/// bridge/mod/CSS metrics. Idempotent: every slot `take()`s to `None`, so a
/// repeated close after shutdown started detaches nothing twice. The caller
/// sends the wire-type ack matching the command it received.
#[allow(clippy::too_many_arguments)]
fn perform_orderly_shutdown(
    control_flow: &mut ControlFlow,
    shutdown_started: &mut bool,
    main_process_failed_counters: &process_failure::ProcessFailedCounters,
    child_process_failed_counters: &process_failure::ProcessFailedCounters,
    main_navigation_counters: &navigation_completed::NavigationCompletedCounters,
    child_navigation_counters: &navigation_completed::NavigationCompletedCounters,
    #[cfg(target_os = "windows")] main_process_failed: &mut main_webview::ObserverSubscription,
    #[cfg(target_os = "windows")]
    main_navigation_completed: &mut main_webview::ObserverSubscription,
    #[cfg(target_os = "windows")] popup_child: &mut Option<PopupChild>,
    #[cfg(target_os = "windows")] env_browser_exited_token: &mut Option<i64>,
    #[cfg(target_os = "windows")] webview_env: &ICoreWebView2Environment,
) {
    *shutdown_started = true;
    println!(
        "[WebView][ProcessFailed] final main_total={} child_total={}",
        main_process_failed_counters.total, child_process_failed_counters.total,
    );
    println!(
        "[WebView][NavigationCompleted] final main_total={} main_ok={} main_failed={} child_total={} child_ok={} child_failed={}",
        main_navigation_counters.total,
        main_navigation_counters.succeeded,
        main_navigation_counters.failed,
        child_navigation_counters.total,
        child_navigation_counters.succeeded,
        child_navigation_counters.failed,
    );
    #[cfg(target_os = "windows")]
    {
        main_webview::detach_main_observers(
            main_process_failed.take(),
            main_navigation_completed.take(),
        );
        if let Some(child) = popup_child.take() {
            if let Some((core_webview2, token)) = child.process_failed {
                process_failure::unregister(&core_webview2, token);
            }
            if let Some((core_webview2, token)) = child.navigation_completed {
                navigation_completed::unregister(&core_webview2, token);
            }
        }
        // Remove environment-level browser-process exit observer.
        if let Some(token) = env_browser_exited_token.take() {
            environment_observer::unregister(webview_env, token);
        }
    }
    *control_flow = ControlFlow::Exit;
}

/// Acknowledges one typed window chrome command with the exact wire-type echo
/// the facade correlates against its pending request.
fn respond_window_ack(
    webview: &WebView,
    request_tracker: &mut RequestTracker,
    request_id: &str,
    command: &'static str,
    received_at: &Instant,
) {
    let response = BridgeResponse {
        version: PROTOCOL_VERSION,
        request_id: request_id.to_owned(),
        ok: true,
        payload: Some(BridgeResponsePayload::WindowAck(WindowAckResponse {
            command,
        })),
        error: None,
    };
    send_bridge_response(webview, &response);
    if let Err(error) = request_tracker.complete(request_id) {
        eprintln!("[Bridge] correlation failure after response: {error}");
    }
    println!(
        "[Window][Metrics] command={command} request_latency_ms={:.3}",
        received_at.elapsed().as_secs_f64() * 1000.0
    );
}

/// Push one managed style create/update to the live document. Returns true
/// only when the renderer accepted the script. Used by the live CSS fire
/// handler; startup injection keeps its own inline path with the same
/// builders.
fn apply_managed_css_update(webview: &WebView, style_id: &str, css: &str, tag: &str) -> bool {
    match managed_css_script(style_id, css) {
        Ok(script) => {
            if let Err(error) = webview.evaluate_script(&script) {
                eprintln!("[{tag}][css_injection] {style_id}: {error}");
                false
            } else {
                true
            }
        }
        Err(error) => {
            eprintln!("[{tag}][css_injection] {style_id}: {error}");
            false
        }
    }
}

/// Remove one managed style from the live document. A missing node is a
/// cheap no-op. Returns true unless script delivery itself failed.
fn apply_managed_css_removal(webview: &WebView, style_id: &str, tag: &str) -> bool {
    match managed_css_remove_script(style_id) {
        Ok(script) => {
            if let Err(error) = webview.evaluate_script(&script) {
                eprintln!("[{tag}][css_removal] {style_id}: {error}");
                false
            } else {
                true
            }
        }
        Err(error) => {
            eprintln!("[{tag}][css_removal] {style_id}: {error}");
            false
        }
    }
}

/// Re-assert cascade order after a structural change (Quick CSS recreated,
/// theme added/removed). `ordered_ids` is Quick CSS first (when present)
/// then sorted theme style ids. Content-only updates never call this.
fn apply_managed_css_order(webview: &WebView, ordered_ids: &[String], tag: &str) -> bool {
    let refs: Vec<&str> = ordered_ids.iter().map(String::as_str).collect();
    match managed_css_order_script(&refs) {
        Ok(script) => {
            if let Err(error) = webview.evaluate_script(&script) {
                eprintln!("[{tag}][css_order] {error}");
                false
            } else {
                true
            }
        }
        Err(error) => {
            eprintln!("[{tag}][css_order] {error}");
            false
        }
    }
}

fn send_mod_failure(
    descriptor: &ModDescriptor,
    webview: &WebView,
    request_tracker: &mut RequestTracker,
    request_id: String,
    code: &'static str,
    message: String,
) {
    eprintln!("[{}][{code}] {message}", descriptor.log_tag);
    let response = BridgeResponse {
        version: PROTOCOL_VERSION,
        request_id: request_id.clone(),
        ok: false,
        payload: None,
        error: Some(BridgeError { code, message }),
    };
    send_bridge_response(webview, &response);
    if let Err(error) = request_tracker.complete(&request_id) {
        eprintln!("[Bridge] Vencord request correlation failure: {error}");
    }
}

/// Shared CSS-result handler. Both the mod-agnostic `mod_css_result` event and
/// the historically Vencord-named `vencord_css_result` route here, so there is
/// exactly one implementation of the CSS gate for every registered mod.
#[allow(clippy::too_many_arguments)]
fn handle_mod_css_result(
    descriptor: &ModDescriptor,
    request_id: String,
    ok: bool,
    error: Option<String>,
    webview: &WebView,
    pending_mods: &mut HashMap<String, PendingModBootstrap>,
    document_lifecycle: &mut DocumentModLifecycle,
    request_tracker: &mut RequestTracker,
    response_count: &mut u64,
) {
    let Some(pending) = pending_mods.get_mut(&request_id) else {
        eprintln!(
            "[{}][css_injection] unmatched CSS result",
            descriptor.log_tag
        );
        return;
    };
    // Cross-mod guard: the descriptor handling this event must own the request.
    if pending.mod_id != descriptor.id {
        eprintln!(
            "[{}][css_injection] CSS result claims a different mod than the request",
            descriptor.log_tag
        );
        return;
    }
    let Some(css_source) = descriptor.css else {
        eprintln!(
            "[{}][css_injection] CSS result for a descriptor that declares no stylesheet",
            descriptor.log_tag
        );
        return;
    };
    let _ = css_source;
    if pending.generation != document_lifecycle.generation() {
        pending_mods.remove(&request_id);
        return;
    }
    if !ok {
        let error = error.unwrap_or_else(|| "CSS injection failed without details".to_owned());
        pending_mods.remove(&request_id);
        send_mod_failure(
            descriptor,
            webview,
            request_tracker,
            request_id,
            mod_stage::CSS_INJECTION,
            error,
        );
        *response_count += 1;
        return;
    }
    let execution_ms = pending.execution_ms;
    pending.css_injection_ms = Some(pending.css_started_at.elapsed().as_secs_f64() * 1000.0);
    let marker_script = match mod_marker_script(descriptor, &request_id, execution_ms) {
        Ok(script) => script,
        Err(error) => {
            pending_mods.remove(&request_id);
            send_mod_failure(
                descriptor,
                webview,
                request_tracker,
                request_id,
                mod_stage::ADAPTER_INITIALIZATION,
                format!(
                    "Could not prepare the {} execution marker: {error}",
                    descriptor.id
                ),
            );
            *response_count += 1;
            return;
        }
    };
    if let Err(error) = webview.evaluate_script(&marker_script) {
        pending_mods.remove(&request_id);
        send_mod_failure(
            descriptor,
            webview,
            request_tracker,
            request_id,
            mod_stage::ADAPTER_INITIALIZATION,
            format!(
                "WebView2 could not verify {} bootstrap execution: {error}",
                descriptor.id
            ),
        );
        *response_count += 1;
    }
}

/// Shared bootstrap-verdict handler for every registered mod.
#[allow(clippy::too_many_arguments)]
fn handle_mod_bootstrap_result(
    descriptor: &ModDescriptor,
    request_id: String,
    ok: bool,
    marker: String,
    error: Option<String>,
    local_storage_api: String,
    settings_state: String,
    webview: &WebView,
    pending_mods: &mut HashMap<String, PendingModBootstrap>,
    document_lifecycle: &mut DocumentModLifecycle,
    mod_metrics: &mut ModHostMetrics,
    request_tracker: &mut RequestTracker,
    smoke_reloads_remaining: &mut u32,
    smoke_reload_request_id: &mut Option<String>,
    response_count: &mut u64,
    observe_after_marker: Option<&str>,
    loader_probe_after_marker: Option<&str>,
    injector_probe_after_marker: Option<&str>,
    settings_plugin_start: Option<ShelterPluginStart<'_>>,
    titlebar_plugin_start: Option<ShelterPluginStart<'_>>,
) {
    let Some(pending) = pending_mods.remove(&request_id) else {
        eprintln!(
            "[{}][script_execution] unmatched bootstrap result",
            descriptor.log_tag
        );
        return;
    };
    if pending.mod_id != descriptor.id {
        eprintln!(
            "[{}][script_execution] bootstrap result claims a different mod than the request",
            descriptor.log_tag
        );
        return;
    }
    if pending.generation != document_lifecycle.generation() {
        return;
    }
    if !ok || marker != descriptor.success_marker {
        send_mod_failure(
            descriptor,
            webview,
            request_tracker,
            request_id,
            mod_stage::SCRIPT_EXECUTION,
            error.unwrap_or_else(|| format!("{} execution marker did not match", descriptor.id)),
        );
        *response_count += 1;
        return;
    }
    if !document_lifecycle.record_mod_execution(descriptor.id) {
        mod_metrics.record_duplicate(descriptor.id);
        send_mod_failure(
            descriptor,
            webview,
            request_tracker,
            request_id,
            mod_stage::DUPLICATE_BOOTSTRAP,
            format!("{} executed more than once in one document", descriptor.id),
        );
        *response_count += 1;
        return;
    }

    mod_metrics.record_bootstrap(descriptor.id);
    let request_latency_ms = pending.received_at.elapsed().as_secs_f64() * 1000.0;
    let css_injection_ms = pending
        .css_injection_ms
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "none".to_owned());
    println!(
        "[{}][Metrics] marker={marker} execution_count={} request_latency_ms={request_latency_ms:.3} css_injection_ms={css_injection_ms}",
        descriptor.log_tag,
        document_lifecycle.mod_execution_count(descriptor.id),
    );
    println!(
        "[{}][Context] localStorage_getItem_at_execution={local_storage_api} settings_state={settings_state}",
        descriptor.log_tag,
    );
    let response = BridgeResponse {
        version: PROTOCOL_VERSION,
        request_id: request_id.clone(),
        ok: true,
        payload: Some(BridgeResponsePayload::ModBundle(ModBundleResponse {
            mod_id: descriptor.id,
            bundle_version: pending.bundle_version,
            bundle_bytes: pending.bundle_bytes,
            css_bytes: pending.css_bytes,
            request_latency_ms,
            retrieval_ms: pending.retrieval_ms,
            execution_ms: pending.execution_ms,
            css_injection_ms: pending.css_injection_ms,
            marker: descriptor.success_marker,
            document_execution_count: document_lifecycle.mod_execution_count(descriptor.id),
        })),
        error: None,
    };
    send_bridge_response(webview, &response);
    // A mod whose page-world global is published asynchronously (Shelter) can
    // reach its marker AFTER page load, so the page-load observation may have
    // run too early to see the global. Re-observe now that the verdict is in,
    // which is the point at which the global is guaranteed present.
    if let Some(script) = observe_after_marker {
        let descriptor_id = descriptor.id;
        if let Err(error) = webview.evaluate_script_with_callback(script, move |result| {
            println!("[Mods][Smoke][after-marker] mod='{descriptor_id}' {result}");
        }) {
            eprintln!("[Mods][Smoke] post-marker observation failed: {error}");
        }
    }
    // Shelter publishes its page-world global asynchronously (after
    // `#app-mount` has children), so a page-load probe runs too early to see
    // `window.shelter.plugins`. Re-run the loader probe once Shelter's own
    // marker proves the global exists. Gated to Shelter: other mods' markers
    // must not duplicate the probe. The script is fully synchronous and
    // removes a leftover probe id first, so a page-load run and this run can
    // never overlap or duplicate the synthetic plugin.
    if let Some(script) = loader_probe_after_marker {
        if descriptor.id == SHELTER_MOD.id {
            if let Err(error) = webview.evaluate_script(script) {
                eprintln!("[ShelterLoader][Probe] post-marker probe failed: {error}");
            }
        }
    }
    // Shelter injector observation (F3): once Shelter's own marker proves
    // the global exists, read back whether the host declaration was present
    // ahead of the bundle and whether the rpc plugin installed its
    // page-world marker. Read-only and DEV-gated; gated to Shelter like the
    // loader probe so other mods' markers never duplicate it.
    if let Some(script) = injector_probe_after_marker {
        if descriptor.id == SHELTER_MOD.id {
            if let Err(error) = webview.evaluate_script_with_callback(script, move |result| {
                println!("[Injector][Probe] post-marker observation {result}");
            }) {
                eprintln!("[Injector][Probe] post-marker observation failed: {error}");
            }
        }
    }
    // Real Remcord settings plugin (3d-2): Shelter's own marker proves
    // `window.shelter.plugins` exists, so the host now registers the
    // embedded settings artifact through the same closed add/start lifecycle
    // the loader probe proved. Gated to Shelter, once per document
    // generation: the closed shim (document-start, before the adapters) is
    // therefore always in place before the plugin evaluates, and reloads
    // start fresh while replays are refused with a loud duplicate log.
    if descriptor.id == SHELTER_MOD.id {
        if let Some(start) = settings_plugin_start {
            let generation = document_lifecycle.generation();
            if !settings_plugin_start_due(*start.started_generation, generation) {
                eprintln!(
                    "[SettingsPlugin][duplicate] settings plugin already started in document generation {generation}"
                );
            } else {
                *start.started_generation = Some(generation);
                *start.start_count += 1;
                let count = *start.start_count;
                if let Err(error) =
                    webview.evaluate_script_with_callback(start.script, move |result| {
                        if result == "\"settings-plugin-started\"" {
                            println!(
                                "[SettingsPlugin] start ok=true count={count} generation={generation}"
                            );
                        } else {
                            eprintln!("[SettingsPlugin] start ok=false result={result}");
                        }
                    })
                {
                    eprintln!("[SettingsPlugin] start evaluation failed: {error}");
                }
            }
        }
        // Real Remcord titlebar plugin: same trigger, same closed lifecycle,
        // own generation guard so a settings replay can never start (or skip)
        // the titlebar. Failure is loud but never blocks the document.
        if let Some(start) = titlebar_plugin_start {
            let generation = document_lifecycle.generation();
            if !settings_plugin_start_due(*start.started_generation, generation) {
                eprintln!(
                    "[Titlebar][duplicate] titlebar plugin already started in document generation {generation}"
                );
            } else {
                *start.started_generation = Some(generation);
                *start.start_count += 1;
                let count = *start.start_count;
                if let Err(error) =
                    webview.evaluate_script_with_callback(start.script, move |result| {
                        if result.starts_with("\"titlebar-started:controls-") {
                            println!(
                                "[Titlebar] start ok=true count={count} generation={generation} result={result}"
                            );
                        } else {
                            eprintln!("[Titlebar] start ok=false result={result}");
                        }
                    })
                {
                    eprintln!("[Titlebar] start evaluation failed: {error}");
                }
            }
        }
    }
    if *smoke_reloads_remaining > 0 {
        *smoke_reload_request_id = Some(request_id.clone());
    }
    if let Err(error) = request_tracker.complete(&request_id) {
        eprintln!(
            "[Bridge] {} response correlation failure: {error}",
            descriptor.id
        );
    }
    *response_count += 1;
}

/// Shared acknowledgement handler that drives the DEV reload sequence.
fn handle_mod_response_ack(
    descriptor: Option<&'static ModDescriptor>,
    request_id: String,
    webview: &WebView,
    smoke_reloads_remaining: &mut u32,
    smoke_reload_request_id: &mut Option<String>,
) {
    // An acknowledgement claiming an unregistered mod is ignored entirely: it
    // must not be able to drive the DEV reload sequence.
    let Some(descriptor) = descriptor else {
        return;
    };
    if smoke_reload_request_id.as_deref() == Some(request_id.as_str()) {
        println!(
            "[{}] renderer acknowledged execution marker response (request_id={request_id})",
            descriptor.log_tag
        );
    }
    if *smoke_reloads_remaining > 0
        && smoke_reload_request_id.as_deref() == Some(request_id.as_str())
    {
        *smoke_reloads_remaining -= 1;
        *smoke_reload_request_id = None;
        println!(
            "[Navigation] requesting mod DEV smoke reload (remaining={smoke_reloads_remaining})"
        );
        if let Err(error) = webview.evaluate_script("window.location.reload();") {
            eprintln!("[Navigation] mod smoke reload failed: {error}");
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoopBuilder::<NativeEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("Remcord")
        .with_inner_size(Size::Logical(LogicalSize::new(835.0, 600.0)))
        .with_min_inner_size(Size::Logical(LogicalSize::new(400.0, 300.0)))
        .with_decorations(false)
        .with_visible(false)
        .build(&event_loop)?;
    // Keep a reference to the Tao window for native window chrome
    // operations (visible, minimize, maximize, focus, state queries).
    // The WebView is built on this window, but the WebView API does not
    // expose the Tao window directly, so we hold an Arc.
    let window = Arc::new(window);
    let window_for_webview = Arc::clone(&window);

    // Release blocker 2: the profile root is resolved per build, never
    // hardcoded. Debug takes the isolated DEV profile; release takes
    // `%LOCALAPPDATA%\Remcord`; an explicit absolute
    // `REMCORD_NATIVE_PROFILE_DIR` wins in both. Any failure fails startup
    // — never a silent fallback to another product's data.
    let override_raw = std::env::var(profile::PROFILE_OVERRIDE_ENV).ok();
    let resolved_profile = profile::resolve_for_build(
        cfg!(debug_assertions),
        override_raw.as_deref(),
        std::env::var("LOCALAPPDATA").ok().as_deref(),
    )
    .map_err(|error| format!("profile resolution failed: {error}"))?;
    // User-mediated import of a previous-generation profile (see
    // `profile_migration`): the user selects the source once via
    // `REMCORD_IMPORT_FROM`. Unset means no import. Any import failure fails
    // startup — never a silent fallback anywhere.
    let is_dev = cfg!(debug_assertions);
    let expected_leaf = if is_dev {
        profile::DEV_PROFILE_DIR
            .rsplit('\\')
            .next()
            .unwrap_or("Remcord.webview-dev-user-data")
    } else {
        profile::RELEASE_PROFILE_LEAF
    };
    let import_source = profile_migration::read_import_source();
    match profile_migration::import_previous_profile(
        import_source.as_deref(),
        &resolved_profile.path,
        expected_leaf,
    ) {
        Ok(report) => {
            if report.files_copied > 0 {
                println!(
                        "[Profile][Import] previous profile imported files={} bytes={} already_imported=false",
                        report.files_copied, report.bytes_copied
                    );
            } else if report.already_imported {
                println!("[Profile][Import] already imported; nothing to copy");
            }
        }
        Err(error) => {
            return Err(format!("profile import failed: {error}").into());
        }
    }
    let profile_path = resolved_profile.path;
    profile::ensure_profile_dir(&profile_path)
        .map_err(|error| format!("profile resolution failed: {error}"))?;
    // P0.5 host-owned downloads directory. Fail fast: every download
    // decision needs this root, and a broken profile dir is fatal anyway.
    let downloads_dir = downloads::ensure_downloads_dir(&profile_path)?;
    // Fail fast if the embedded package version ever loses its shape: the
    // capability serves this constant on every request.
    if let Err(error) = validate_version(current_app_version()) {
        return Err(format!("embedded app version is invalid: {error}").into());
    }
    // Fail fast on any descriptor the substrate could not serve safely: a
    // missing capability, an id the request validator would reject, a duplicate
    // id, or a value that could not be spliced into a generated script.
    let mut seen_ids: Vec<&str> = Vec::new();
    for descriptor in KNOWN_MODS {
        if !BOOT_SNAPSHOT.capabilities.contains(&descriptor.capability) {
            return Err(format!(
                "mod '{}' requires capability '{}' missing from the boot snapshot",
                descriptor.id, descriptor.capability
            )
            .into());
        }
        if validate_mod_bundle_id(descriptor.id).is_err() {
            return Err(format!("mod id '{}' is not servable", descriptor.id).into());
        }
        if seen_ids.contains(&descriptor.id) {
            return Err(format!("mod id '{}' is registered twice", descriptor.id).into());
        }
        if mod_adapter_script(descriptor).is_none() {
            return Err(format!("mod '{}' has no renderer adapter", descriptor.id).into());
        }
        validate_descriptor(descriptor)
            .map_err(|error| format!("mod descriptor is invalid: {error}"))?;
        seen_ids.push(descriptor.id);
    }
    // F4: in-mod plugin settings pages (Vencord/Equicord/Shelter) render
    // page-side from the executed bundles; the host adds no settings-page
    // capability. Fail fast if a descriptor drifts from the asserted storage
    // shape or a settings-page capability ever appears.
    mod_settings::validate_mod_settings_contract(
        VENCORD_MOD.settings_key,
        EQUICORD_MOD.settings_key,
        SHELTER_MOD.settings_key,
        CUSTOM_MOD.settings_key,
        BOOT_SNAPSHOT.capabilities,
    )
    .map_err(|error| format!("mod settings contract is invalid: {error}"))?;
    let mut web_context = WebContext::new(Some(profile_path.clone()));
    // Release blocker 4: Evergreen WebView2 Runtime prerequisite check,
    // before any WebView initialization. Supported loader lookup (default
    // per-machine + per-user registrations, no admin, no profile); the
    // result is logged and reused below to classify an environment failure
    // as a missing Runtime vs an unrelated error. Never a recovery input.
    let runtime_check = webview_runtime::check();
    match &runtime_check {
        webview_runtime::RuntimeCheck::Present { version } => {
            println!("[Runtime] WebView2 Evergreen Runtime version={version}");
        }
        webview_runtime::RuntimeCheck::Absent => {
            eprintln!(
                "[Runtime] WebView2 Evergreen Runtime not detected; environment creation will fail without it"
            );
        }
    }
    // Owned copy for the read-only diagnostics readout: the loader is
    // queried once at boot, never per request.
    let runtime_version: Option<String> = match &runtime_check {
        webview_runtime::RuntimeCheck::Present { version } => Some(version.clone()),
        webview_runtime::RuntimeCheck::Absent => None,
    };
    // WebView2 environment with the remcord custom scheme registered (P0.2).
    // Created here — before the environment exists and before navigation —
    // and adopted by wry below. Fail-fast: without registration, remcord://
    // subresources fail as an unknown scheme and no handler ever fires.
    #[cfg(target_os = "windows")]
    let webview_env = {
        match create_remcord_webview_environment(&profile_path) {
            Ok(env) => {
                println!(
                    "[RemcordProtocol] scheme registered: remcord (authority, secure, https://discord.com)"
                );
                env
            }
            Err(error) => {
                // Absent Runtime: stop cleanly with an actionable
                // diagnostic (stderr for consoles, a message box for
                // shortcut launches with no console). Nothing falls back
                // anywhere — no other profile, browser, or configuration.
                // A present Runtime means an unrelated init error: propagate
                // it unchanged for the existing fail-fast behavior.
                if runtime_check == webview_runtime::RuntimeCheck::Absent {
                    report_missing_runtime();
                    return Err("WebView2 Evergreen Runtime is not installed; install it from Microsoft and start Remcord again".into());
                }
                return Err(error);
            }
        }
    };
    // Phase 2G: register the environment-level browser-process exit observer
    // exactly once for the shared environment. This provides the authoritative
    // exit kind (Normal/Failed) and browser PID — information the per-WebView
    // ProcessFailed observer cannot provide. Unavailable interface degrades
    // gracefully: the WebView-level observer remains as the fallback signal.
    #[cfg(target_os = "windows")]
    let mut env_browser_exited_token = {
        let observer_proxy = proxy.clone(); // same event loop
        match environment_observer::register(&webview_env, &observer_proxy) {
            Ok(token) => {
                println!("[Environment][BrowserProcessExited] observer registered");
                Some(token)
            }
            Err(error) => {
                eprintln!("[Environment][BrowserProcessExited] registration failed (fallback to WebView-level): {error}");
                None
            }
        }
    };
    // Log whether the environment observer interface is available for diagnostics.
    #[cfg(target_os = "windows")]
    {
        let available = environment_observer::is_available(&webview_env);
        println!("[Environment][BrowserProcessExited] interface available={available}");
    }
    // Shared with the single P0.4 popup child so it runs in the same
    // WebView2 environment (same profile/session). Cloned before the main
    // builder consumes the original; COM `Clone` is a cheap `AddRef`.
    #[cfg(target_os = "windows")]
    let popup_environment = webview_env.clone();
    let bundle_cache = Arc::new(Mutex::new(ModCache::default()));
    let ipc_proxy = proxy.clone();
    let load_proxy = proxy.clone();
    // P0.4 popup path: the `NewWindowRequested` closure classifies
    // synchronously and queues approved popups here; the shell open for
    // external URLs also runs from that closure (both proxies are
    // thread-safe like the existing IPC path).
    let popup_proxy = proxy.clone();
    let popup_ipc_proxy = proxy.clone();
    // Dedicated STA worker for the theme import picker. The Common Item
    // Dialog is modal, so it must never run on the WebView/UI thread; COM is
    // initialized exactly once for this thread (STA, as the dialog
    // requires), and picker jobs are processed sequentially for
    // determinism and bounded memory.
    let (import_tx, import_rx) = std::sync::mpsc::channel::<ThemeImportJob>();
    if let Err(error) = thread::Builder::new()
        .name(THEME_IMPORT_WORKER_NAME.to_owned())
        .spawn(move || {
            #[cfg(target_os = "windows")]
            {
                let com_init = unsafe {
                    windows::Win32::System::Com::CoInitializeEx(
                        None,
                        windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
                    )
                };
                if com_init.is_err() {
                    eprintln!("[Themes][Import] STA COM initialization failed; picker jobs will fail closed");
                }
                let com_owned = com_init == windows::core::HRESULT(0);
                while let Ok(job) = import_rx.recv() {
                    let result = match show_import_picker(&job.profile) {
                        PickerOutcome::Canceled => {
                            println!("[Themes][Import] picker dismissed by the user");
                            Ok(ThemeImportSummary {
                                canceled: true,
                                imported: 0,
                                failed: 0,
                            })
                        }
                        PickerOutcome::Selected(paths) => {
                            let selected = paths.len();
                            let (summary, outcomes) = import_from_paths(&job.profile, &paths);
                            // Codes only, never paths: the selection is
                            // host-side and stays host-side.
                            let codes: Vec<&str> = outcomes
                                .iter()
                                .filter_map(|outcome| match outcome {
                                    FileImportOutcome::Failed(code) => Some(*code),
                                    FileImportOutcome::Imported(_) => None,
                                })
                                .collect();
                            println!(
                                "[Themes][Import] selected={selected} imported={} failed={} codes={codes:?}",
                                summary.imported, summary.failed
                            );
                            Ok(summary)
                        }
                        PickerOutcome::Failed(failure) => {
                            eprintln!("[Themes][Import:{}] {}", failure.code, failure.message);
                            Err(failure)
                        }
                    };
                    queue_event(
                        &job.proxy,
                        NativeEvent::ThemeImportReady {
                            request_id: job.request_id,
                            received_at: job.received_at,
                            result,
                        },
                    );
                }
                if com_owned {
                    unsafe {
                        windows::Win32::System::Com::CoUninitialize();
                    }
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                while let Ok(job) = import_rx.recv() {
                    queue_event(
                        &job.proxy,
                        NativeEvent::ThemeImportReady {
                            request_id: job.request_id,
                            received_at: job.received_at,
                            result: Err(ThemeImportFailure {
                                code: "IMPORT_UNSUPPORTED_PLATFORM",
                                message: "theme import is not supported on this platform".to_owned(),
                            }),
                        },
                    );
                }
            }
        })
    {
        eprintln!("[Themes][Import] could not start the picker worker: {error}");
    }
    // Dedicated STA worker for backup saves: collection, ZIP creation, the
    // modal save dialog, and the final write run sequentially here so the
    // WebView/UI thread never blocks. COM is initialized exactly once for
    // this thread (STA, as the Common Item Dialog requires); the typed
    // outcome comes back as BackupSaveReady on the event loop.
    let (backup_tx, backup_rx) = std::sync::mpsc::channel::<BackupSaveJob>();
    if let Err(error) = thread::Builder::new()
        .name(BACKUP_SAVE_WORKER_NAME.to_owned())
        .spawn(move || {
            #[cfg(target_os = "windows")]
            {
                let com_init = unsafe {
                    windows::Win32::System::Com::CoInitializeEx(
                        None,
                        windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
                    )
                };
                if com_init.is_err() {
                    eprintln!(
                        "[Backup][Save] STA COM initialization failed; save jobs will fail closed"
                    );
                }
                let com_owned = com_init == windows::core::HRESULT(0);
                while let Ok(job) = backup_rx.recv() {
                    let result = run_backup_save_job(&job);
                    queue_event(
                        &job.proxy,
                        NativeEvent::BackupSaveReady {
                            request_id: job.request_id,
                            received_at: job.received_at,
                            result,
                        },
                    );
                }
                if com_owned {
                    unsafe {
                        windows::Win32::System::Com::CoUninitialize();
                    }
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                while let Ok(job) = backup_rx.recv() {
                    queue_event(
                        &job.proxy,
                        NativeEvent::BackupSaveReady {
                            request_id: job.request_id,
                            received_at: job.received_at,
                            result: Err(BackupFailure {
                                code: "BACKUP_UNSUPPORTED_PLATFORM",
                                message: "backup save is not supported on this platform".to_owned(),
                            }),
                        },
                    );
                }
            }
        })
    {
        eprintln!("[Backup][Save] could not start the save worker: {error}");
    }
    // Dedicated STA worker for backup restores: the modal open dialog, the
    // bounded archive read, full validation, staging, and the commit with
    // rollback run sequentially here so the WebView/UI thread never blocks.
    // COM is initialized exactly once for this thread (STA, as the Common
    // Item Dialog requires); the typed outcome comes back as
    // BackupRestoreReady on the event loop.
    let (restore_tx, restore_rx) = std::sync::mpsc::channel::<BackupRestoreJob>();
    if let Err(error) = thread::Builder::new()
        .name(BACKUP_RESTORE_WORKER_NAME.to_owned())
        .spawn(move || {
            #[cfg(target_os = "windows")]
            {
                let com_init = unsafe {
                    windows::Win32::System::Com::CoInitializeEx(
                        None,
                        windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
                    )
                };
                if com_init.is_err() {
                    eprintln!(
                        "[Backup][Restore] STA COM initialization failed; restore jobs will fail closed"
                    );
                }
                let com_owned = com_init == windows::core::HRESULT(0);
                // Worker-local single staged-restore slot: inspect holds,
                // commit consumes, discard drops. It never leaves this
                // thread except through commit (profile) or discard
                // (deleted); only previews and summaries cross back.
                let mut held: Option<StagedRestore> = None;
                while let Ok(job) = restore_rx.recv() {
                    let request_id = job.request_id.clone();
                    let received_at = job.received_at;
                    let proxy = job.proxy.clone();
                    match run_backup_restore_job(&job, &mut held) {
                        BackupRestoreOutcome::Inspected(result) => {
                            queue_event(
                                &proxy,
                                NativeEvent::BackupInspectReady {
                                    request_id,
                                    received_at,
                                    result,
                                },
                            );
                        }
                        BackupRestoreOutcome::Committed(result) => {
                            queue_event(
                                &proxy,
                                NativeEvent::BackupRestoreReady {
                                    request_id,
                                    received_at,
                                    result,
                                },
                            );
                        }
                        BackupRestoreOutcome::Discarded(discarded) => {
                            queue_event(
                                &proxy,
                                NativeEvent::BackupDiscardReady {
                                    request_id,
                                    received_at,
                                    discarded,
                                },
                            );
                        }
                    }
                }
                if com_owned {
                    unsafe {
                        windows::Win32::System::Com::CoUninitialize();
                    }
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                while let Ok(job) = restore_rx.recv() {
                    queue_event(
                        &job.proxy,
                        NativeEvent::BackupRestoreReady {
                            request_id: job.request_id,
                            received_at: job.received_at,
                            result: Err(BackupFailure {
                                code: "BACKUP_UNSUPPORTED_PLATFORM",
                                message: "backup restore is not supported on this platform".to_owned(),
                            }),
                        },
                    );
                }
            }
        })
    {
        eprintln!("[Backup][Restore] could not start the restore worker: {error}");
    }
    // Dedicated global-hotkey worker for keybinds (F8 core): Win32
    // `RegisterHotKey` plus its own message loop live here so the
    // WebView/UI thread never blocks; firings cross back as `KeybindFired`
    // on the event loop. The table re-syncs from authoritative settings
    // at startup and after every management mutation (Electron's
    // refreshGlobalKeybinds model). Registration conflicts fail
    // per-keybind and loud, never aborting the set. No hooks, no
    // SendInput, no synthetic key events.
    const KEYBIND_HOTKEY_WORKER_NAME: &str = "remcord-keybind-hotkeys";
    let (hotkey_tx, hotkey_rx) = std::sync::mpsc::channel::<keybinds::HotkeySync>();
    let hotkey_proxy = proxy.clone();
    let mut keybind_hotkey_tx: Option<std::sync::mpsc::Sender<keybinds::HotkeySync>> = None;
    if let Err(error) = thread::Builder::new()
        .name(KEYBIND_HOTKEY_WORKER_NAME.to_owned())
        .spawn(move || {
            run_hotkey_worker(hotkey_rx, &hotkey_proxy);
        })
    {
        eprintln!("[Keybinds] could not start the hotkey worker: {error}");
    } else {
        keybind_hotkey_tx = Some(hotkey_tx);
    }
    sync_hotkey_worker(&keybind_hotkey_tx, &profile_path);
    // Adapter selection is entirely host-controlled: the authoritative settings
    // snapshot (the same file the settings capability reads) plus an optional
    // DEV diagnostic allowlist. The renderer never participates.
    let (
        settings_mods,
        discord_channel,
        screenshare_quality,
        prefer_hw_h264,
        spellcheck_enabled,
        spellcheck_languages,
    ) = match load_settings(&profile_path) {
        Ok(loaded) => (
            loaded.snapshot.mods,
            loaded.snapshot.channel,
            loaded.snapshot.screenshare_quality,
            loaded.snapshot.sdp_h264_baseline_rewrite,
            loaded.snapshot.spellcheck,
            loaded.snapshot.spellcheck_language,
        ),
        Err(error) => {
            // Corrupt/unreadable settings fail closed to "no extra mods", the
            // same parity the Quick CSS enablement already uses.
            eprintln!(
                    "[Mod:Registry] mod enablement could not be resolved from settings; treating as none ({error})"
                );
            (
                Vec::new(),
                "stable".to_owned(),
                "default".to_owned(),
                true,
                true,
                vec!["en-US".to_owned()],
            )
        }
    };
    // Release-channel entry URL, resolved once per process from the same
    // authoritative snapshot (restart-required model, redirect.html parity).
    // Hand-edited or forward-compat values fail safe to stable.
    let discord_url = channel::resolve_channel_url(&discord_channel);
    println!("[Channel] channel={discord_channel} url={discord_url}");
    let mod_override = std::env::var(MOD_ADAPTER_OVERRIDE_ENV).ok();
    let active_mods = resolve_active_mods(&settings_mods, mod_override.as_deref());
    for descriptor in KNOWN_MODS {
        let enabled = active_mods.iter().any(|active| active.id == descriptor.id);
        println!(
            "[Mod:Registry] mod='{}' adapter_enabled={} enabling={:?}",
            descriptor.id, enabled, descriptor.enabling
        );
    }
    let settings_plugin = settings_plugin::artifact();
    // Defense in depth: the build already validated the artifact, but the
    // running host re-checks the invariant so a corrupted embed can never be
    // served silently. Startup fails fast on drift, like the mod registry.
    settings_plugin::validate_artifact(settings_plugin.js, settings_plugin.name)
        .expect("embedded settings plugin artifact must validate at startup");
    settings_plugin::validate_sha256(settings_plugin.sha256)
        .expect("embedded settings plugin identity must be well-formed");
    println!(
        "[SettingsPlugin] artifact name='{}' bytes={} sha256={}",
        settings_plugin.name, settings_plugin.bytes, settings_plugin.sha256
    );
    let titlebar_plugin = titlebar_plugin::artifact();
    // Same defense in depth as the settings artifact: the build validated,
    // the running host re-checks, startup fails fast on drift.
    titlebar_plugin::validate_artifact(titlebar_plugin.js, titlebar_plugin.name)
        .expect("embedded titlebar plugin artifact must validate at startup");
    titlebar_plugin::validate_sha256(titlebar_plugin.sha256)
        .expect("embedded titlebar plugin identity must be well-formed");
    println!(
        "[Titlebar] artifact name='{}' bytes={} sha256={}",
        titlebar_plugin.name, titlebar_plugin.bytes, titlebar_plugin.sha256
    );
    // Shelter injector surface (F3): validate both embedded injector
    // artifacts and build the static declaration once per launch.
    // Defense in depth like the settings/titlebar artifacts: the build
    // validated, the running host re-checks, startup fails fast on drift.
    for (artifact, expected_name) in shelter_injector::artifacts()
        .iter()
        .zip(shelter_injector::INJECTOR_PLUGIN_EXPECTED_NAMES.iter())
    {
        shelter_injector::validate_artifact(artifact.js, artifact.name, expected_name)
            .unwrap_or_else(|error| {
                panic!(
                    "embedded injector plugin '{}' must validate at startup: {error}",
                    artifact.id
                )
            });
        shelter_injector::validate_sha256(artifact.sha256).unwrap_or_else(|error| {
            panic!(
                "embedded injector plugin '{}' identity must be well-formed: {error}",
                artifact.id
            )
        });
        println!(
            "[Injector] artifact id='{}' name='{}' bytes={} sha256={}",
            artifact.id, artifact.name, artifact.bytes, artifact.sha256
        );
    }
    let shelter_injector_decl = shelter_injector::injector_declaration()
        .expect("shelter injector declaration must build at startup");
    println!(
        "[Injector] declaration bytes={} plugins={:?}",
        shelter_injector_decl.len(),
        shelter_injector::INJECTOR_PLUGIN_IDS
    );
    let diagnostic_bare_page = std::env::var(BARE_PAGE_ENV).is_ok_and(|value| value == "1");
    let diagnostic_interaction =
        std::env::var(INTERACTION_DIAGNOSTIC_ENV).is_ok_and(|value| value == "1");
    let diagnostic_settings_smoke =
        std::env::var(SETTINGS_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_settings_smoke {
        println!("[Settings][Smoke] settings capability smoke probe enabled");
    }
    let diagnostic_settings_mutation_smoke =
        std::env::var(SETTINGS_MUTATION_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_settings_mutation_smoke {
        println!("[Settings][MutationSmoke] settings mutation smoke probe enabled");
    }
    let diagnostic_settings_set_smoke =
        std::env::var(SETTINGS_SET_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_settings_set_smoke {
        println!("[Settings][SetSmoke] settings set smoke probe enabled");
    }
    let diagnostic_getlang_smoke = std::env::var(GETLANG_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_getlang_smoke {
        println!("[Localization][Smoke] localization capability smoke probe enabled");
    }
    let diagnostic_app_version_smoke =
        std::env::var(APP_VERSION_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_app_version_smoke {
        println!("[App][Smoke] app version capability smoke probe enabled");
    }
    let diagnostic_graphics_smoke =
        std::env::var(GRAPHICS_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_graphics_smoke {
        println!("[Graphics][Smoke] graphics capability smoke probe enabled");
    }
    let diagnostic_plugin_storage_smoke =
        std::env::var(PLUGIN_STORAGE_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_plugin_storage_smoke {
        println!("[PluginStorage][Smoke] plugin storage capability smoke probe enabled");
    }
    let diagnostic_theme_install_smoke =
        std::env::var(THEME_INSTALL_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_theme_install_smoke {
        println!("[Themes][InstallSmoke] theme install smoke probe enabled");
    }
    let diagnostic_theme_write_smoke =
        std::env::var(THEME_WRITE_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_theme_write_smoke {
        println!("[Themes][WriteSmoke] theme write smoke probe enabled");
    }
    let diagnostic_theme_import_smoke =
        std::env::var(THEME_IMPORT_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_theme_import_smoke {
        println!("[Themes][ImportSmoke] theme import smoke probe enabled");
    }
    let diagnostic_theme_set_enabled_smoke =
        std::env::var(THEME_SET_ENABLED_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_theme_set_enabled_smoke {
        println!("[Themes][SetEnabledSmoke] theme enable smoke probe enabled");
    }
    let diagnostic_theme_list_smoke =
        std::env::var(THEME_LIST_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_theme_list_smoke {
        println!("[Themes][ListSmoke] theme list smoke probe enabled");
    }
    let diagnostic_backup_save_smoke =
        std::env::var(BACKUP_SAVE_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_backup_save_smoke {
        println!("[Backup][SaveSmoke] backup save smoke probe enabled");
    }
    let diagnostic_backup_restore_smoke =
        std::env::var(BACKUP_RESTORE_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_backup_restore_smoke {
        println!("[Backup][RestoreSmoke] backup restore smoke probe enabled");
    }
    let diagnostic_shelter_loader_probe =
        std::env::var(SHELTER_LOADER_PROBE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_shelter_loader_probe {
        println!("[ShelterLoader][Probe] shelter loader probe enabled");
    }
    let diagnostic_injector_probe =
        std::env::var(INJECTOR_PROBE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_injector_probe {
        println!("[Injector][Probe] shelter injector observation enabled");
    }
    let diagnostic_settings_boot_smoke =
        std::env::var(SETTINGS_BOOT_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_settings_boot_smoke {
        println!("[SettingsBoot][Smoke] settings boot snapshot smoke probe enabled");
    }
    let diagnostic_settings_ui_smoke =
        std::env::var(SETTINGS_UI_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_settings_ui_smoke {
        println!("[SettingsUi][Smoke] settings UI workflow probe enabled");
    }
    // Per-launch nonce so repeat runs start a fresh workflow machine even
    // though its stage keys persist in profile localStorage.
    let settings_ui_run_nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64 ^ std::process::id() as u64)
        .unwrap_or(0x5eed_u64);
    let settings_ui_probe_script = settings_ui_probe_script(settings_ui_run_nonce)
        .expect("settings UI probe script must build");
    if diagnostic_settings_ui_smoke {
        println!("[SettingsUi][Smoke] run nonce={settings_ui_run_nonce}");
    }
    let diagnostic_folders_smoke = std::env::var(FOLDERS_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_folders_smoke {
        println!("[Folders][Smoke] folder opening smoke probe enabled");
        // Host-side only: no renderer surface, no bridge traffic. Reveals
        // the real themes root once (a genuine OS shell operation) and
        // exercises the rejection paths, which must have no OS effect.
        match ensure_folder_root(&profile_path, FolderLocation::Themes)
            .and_then(|dir| reveal_folder(&dir))
        {
            Ok(()) => println!("[Folders][Smoke] reveal target=themes ok=true"),
            Err(failure) => {
                eprintln!(
                    "[Folders][Smoke] reveal target=themes ok=false code={}",
                    failure.code
                )
            }
        }
        match resolve_theme_folder(&profile_path, "../escape") {
            Err(failure) => println!(
                "[Folders][Smoke] traversal rejected code={} (no OS effect)",
                failure.code
            ),
            Ok(_) => eprintln!("[Folders][Smoke] traversal unexpectedly resolved"),
        }
        match resolve_theme_folder(&profile_path, "remcord-native-no-such-theme") {
            Err(failure) => println!(
                "[Folders][Smoke] unknown theme rejected code={} (no OS effect)",
                failure.code
            ),
            Ok(_) => eprintln!("[Folders][Smoke] unknown theme unexpectedly resolved"),
        }
    }
    let diagnostic_themes_smoke = std::env::var(THEMES_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_themes_smoke {
        println!("[Themes][Smoke] native theme injection smoke observation enabled");
    }
    let diagnostic_quickcss_smoke =
        std::env::var(QUICKCSS_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_quickcss_smoke {
        println!("[QuickCss][Smoke] native Quick CSS smoke observation enabled");
    }
    let diagnostic_remcord_protocol_smoke =
        std::env::var(REMCORD_PROTOCOL_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_remcord_protocol_smoke {
        println!("[RemcordProtocol][Smoke] remcord:// protocol smoke probe enabled");
    }
    let diagnostic_permissions_smoke =
        std::env::var(PERMISSIONS_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_permissions_smoke {
        println!("[Permissions][Smoke] permission smoke probe enabled");
    }
    let diagnostic_mod_smoke = std::env::var(MOD_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_mod_smoke {
        println!("[Mods][Smoke] native mod runtime observation enabled");
    }
    // The observation spec is built from the SAME descriptors the pipeline
    // uses, so it can only ever read host-managed identities.
    let mod_smoke_specs = serde_json::to_string(
        &active_mods
            .iter()
            .filter_map(|descriptor| {
                let state_name = adapter_state_name(descriptor)?;
                Some((
                    descriptor.id.to_owned(),
                    serde_json::json!({
                        "page_global": descriptor.page_global,
                        "requires": descriptor.page_global_requires,
                        "style_id": descriptor
                            .css
                            .map(|source| source.style_element_id)
                            .unwrap_or(""),
                        "state_name": state_name,
                    }),
                ))
            })
            .collect::<std::collections::BTreeMap<_, _>>(),
    )?;
    let mod_smoke_script = mod_smoke_observation_script(&mod_smoke_specs)?;
    // Synchronous settings boot snapshot (3d-2 prerequisite B): built once
    // per launch from the authoritative owners and spliced into the
    // document-start script BEFORE the mod adapters, so it exists before the
    // settings plugin could execute. Every document receives its own frozen
    // instance; values reflect launch-time state (see settings_boot.rs).
    let settings_boot_json = settings_boot::build_settings_boot_json(&profile_path);
    println!("[SettingsBoot] snapshot bytes={}", settings_boot_json.len());
    // Closed shim platform identity (3d-2): static launch-time strings with
    // Electron parity (process.platform / os.release()). The platform is the
    // same constant the boot snapshot already declares; the release is
    // queried from the real host once per launch, never invented.
    let os_release = app::os_version_string();
    app::validate_os_release(&os_release).expect("host OS release must validate");
    let host_platform_json = serde_json::json!({
        "platform": BOOT_SNAPSHOT.platform,
        "osRelease": os_release,
    })
    .to_string();
    println!(
        "[HostPlatform] platform={} osRelease={}",
        BOOT_SNAPSHOT.platform, os_release
    );
    let initialization_script = if diagnostic_bare_page {
        println!("[Native] Bare Discord page diagnostic enabled; bridge/mod scripts are omitted");
        STORAGE_START_DIAGNOSTIC_SCRIPT.to_owned()
    } else {
        // The capture-quality mode and the SDP H.264 rewrite are host-resolved
        // once per process from the same authoritative snapshot
        // (restart-required model, like `channel`): unknown quality values
        // fail safe to `"default"` inside the module.
        println!(
            "[ScreenshareQuality] mode={}",
            screenshare_quality::normalize_mode(&screenshare_quality)
        );
        println!("[ScreenshareMedia] prefer_hw_h264={prefer_hw_h264}");
        let spellcheck_state = spellcheck::resolve_state(spellcheck_enabled, &spellcheck_languages);
        println!(
            "[Spellcheck] enabled={} lang={}",
            spellcheck_state.enabled, spellcheck_state.primary_language
        );
        document_start_script(
            &active_mods,
            &settings_boot_json,
            &host_platform_json,
            &screenshare_quality,
            prefer_hw_h264,
            spellcheck_state.enabled,
            &spellcheck_state.primary_language,
        )?
    };
    println!(
        "[Native] Document-start script: {} bytes",
        initialization_script.len()
    );
    // Phase 2F: exactly one main WebView construction through the
    // host-owned helper. Per-WebView wry wiring lives in
    // `main_webview::main_webview_builder`; process-lifetime setup
    // (event loop, WebContext, environment, watchers, workers, caches)
    // is untouched. The instance id is threaded through so a future
    // replacement can take a fresh id; the boot value stays 0.
    let mut main_lifecycle = main_webview::MainWebviewLifecycle::new();
    let mut main_instance_id = main_lifecycle
        .record_initial()
        .expect("initial main WebView builds exactly once");
    debug_assert_eq!(main_instance_id, MAIN_INSTANCE_ID);
    debug_assert_eq!(main_instance_id, main_webview::INITIAL_MAIN_INSTANCE_ID);
    // The lifecycle owns the single-boot invariant: exactly one
    // construction, the boot id live, and a reserved fresh id a future
    // replacement (later milestone) will take. `is_stale` is exercised in
    // tests; the event loop keeps using the free-function id check on the
    // copied `main_instance_id` below so no shared state crosses threads.
    debug_assert_eq!(main_lifecycle.construction_count(), 1);
    debug_assert_eq!(main_lifecycle.active_instance(), Some(main_instance_id));
    debug_assert_ne!(
        main_lifecycle.next_instance_id(),
        main_webview::INITIAL_MAIN_INSTANCE_ID
    );
    debug_assert!(!main_lifecycle.is_stale(main_instance_id));
    let mut webview = {
        // Clone process-lifetime resources before moving them into the builder
        // so they remain available for browser-process recreation.
        #[cfg(target_os = "windows")]
        let webview_env_for_builder = webview_env.clone();
        let ipc_proxy_for_builder = ipc_proxy.clone();
        let load_proxy_for_builder = load_proxy.clone();
        let popup_proxy_for_builder = popup_proxy.clone();
        let builder = main_webview::main_webview_builder(
            &mut web_context,
            #[cfg(target_os = "windows")]
            webview_env_for_builder,
            &initialization_script,
            downloads_dir.clone(),
            ipc_proxy_for_builder,
            load_proxy_for_builder,
            popup_proxy_for_builder,
            main_instance_id,
        );
        builder.build(&window_for_webview)?
    };
    println!("[Native] WebView2 prototype started");
    println!(
        "[Native] profile kind={} dir={}",
        resolved_profile.kind.as_str(),
        profile_path.display()
    );

    // Per-WebView content interceptions (Phase 2F helper): the
    // `remcord://` registration, the permission policy, and the CSP
    // request filter. Runs after `build()` and before the initial Discord
    // navigation, so the first document is intercepted like every later
    // one. Returns the namespace roots for the P0.4 popup child
    // (registration is per-WebView, not per-environment).
    #[cfg(target_os = "windows")]
    let popup_roots =
        main_webview::attach_main_content_filters(&webview, &profile_path, &profile_path);
    // Second boot phase (P0.2): navigate to Discord only after both
    // WebResourceRequested filters exist, so the first Discord document is
    // intercepted like every later one. `about:blank` has no CSP of its own
    // and the navigation handler allows only it plus HTTPS Discord hosts.
    if let Err(error) = main_webview::navigate_to_discord(&webview, discord_url) {
        return Err(error.into());
    }
    println!("[Native] Initial navigation: {discord_url}");
    let mut smoke_reloads_remaining: u32 = std::env::var("REMCORD_NATIVE_SMOKE_RELOAD")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    let mut smoke_reload_request_id = None::<String>;
    let mut request_tracker = RequestTracker::default();
    let mut document_lifecycle = DocumentModLifecycle::default();
    let mut pending_mods = HashMap::<String, PendingModBootstrap>::new();
    // In-flight CDP graphics probes by bridge request id. Answered (and
    // removed) when both probe parts arrive; cleared with the bridge
    // tracker on document/WebView resets so no stale probe can answer a
    // later document.
    let mut pending_graphics = HashMap::<String, graphics::PendingGraphics>::new();
    // Whether the Quick CSS managed style was injected into the latest
    // trusted document, so the next document reset can remove it when the
    // flag flips to disabled. The DOM itself is fresh per document; this
    // only names what to clean on a state change.
    let mut last_quickcss_injected = false;
    // Exact bytes last pushed for Quick CSS (None when absent). Live updates
    // compare the debounced re-read against this so duplicate events and
    // identical rescans issue zero renderer work.
    let mut last_quickcss_css: Option<String> = None;
    // Managed theme style ids injected into the latest trusted document, so
    // the next document reset can remove styles for themes that are no
    // longer enabled. The DOM itself is fresh per document; this list only
    // names what to clean when the enabled set shrinks.
    let mut last_theme_style_ids: Vec<String> = Vec::new();
    // Full last-pushed theme snapshot for live diffing: only changed themes
    // are re-pushed, only removed ones are removed.
    let mut last_themes: Vec<themes::LoadedTheme> = Vec::new();
    // Live CSS lifecycle (Phase D): exactly one debouncer per logical
    // resource plus shared counters. The native watcher set is created once
    // below and moved into the event-loop closure; document reload never
    // creates watchers.
    let live_counters = Arc::new(Mutex::new(LiveCssCounters::default()));
    let quickcss_debounce = Arc::new(Mutex::new(ResourceDebouncer::default()));
    let themes_debounce = Arc::new(Mutex::new(ResourceDebouncer::default()));
    // The single application-owned watcher set. Creation failure only
    // disables live watching (startup injection still runs); the host never
    // crashes on a watcher error.
    let live_watchers = {
        let live_proxy = proxy.clone();
        match start_live_css_watchers(&profile_path, Arc::clone(&live_counters), move |signal| {
            queue_event(&live_proxy, NativeEvent::LiveCssSignal(signal));
        }) {
            Ok(watchers) => {
                println!(
                    "[LiveCss][Watch] started: watchers={} debounce_ms={}",
                    css_watch::LiveCssWatchers::watcher_count(),
                    CSS_DEBOUNCE_MS,
                );
                Some(watchers)
            }
            Err(error) => {
                eprintln!("[LiveCss][watch_error] live watching disabled: {error}");
                None
            }
        }
    };
    let mut mod_metrics = ModHostMetrics::default();
    // Mutation smoke stages: 0 = mutate on the first trusted document,
    // 1 = mutated, reload requested, restore pending on the next document,
    // 2 = restored (sequence complete, DEV value back to original).
    let mut mutation_smoke_stage = 0_u8;
    let mut mutation_smoke_before = false;
    let mut mutation_smoke_value = false;
    // Settings.set smoke stages: same mutate → reload → restore machine as
    // the mutation smoke above, but exercising the closed `settings.set`
    // allowlist through `support_banner_dismissed` (a side-effect-free flag).
    let mut settings_set_smoke_stage = 0_u8;
    let mut settings_set_smoke_before = false;
    let mut page_load_started_at = None;
    let mut incoming_messages = 0_u64;
    let mut request_count = 0_u64;
    let mut event_count = 0_u64;
    let mut response_count = 0_u64;
    // Real Remcord settings plugin (3d-2): host-built start script plus the
    // per-document guard, both moved into the event-loop closure below.
    let settings_plugin_start_script =
        settings_plugin_start_script().expect("settings plugin start script must build");
    println!(
        "[SettingsPlugin] start script bytes={}",
        settings_plugin_start_script.len()
    );
    let mut settings_plugin_started_generation = None::<u64>;
    let mut settings_plugin_start_count = 0_u64;
    // Real Remcord titlebar plugin: same host-built script plus per-document
    // guard as the settings plugin, moved into the event-loop closure below.
    // Started from the same Shelter bootstrap trigger, right after settings.
    let titlebar_plugin_start_script =
        titlebar_plugin_start_script().expect("titlebar plugin start script must build");
    println!(
        "[Titlebar] start script bytes={}",
        titlebar_plugin_start_script.len()
    );
    let mut titlebar_started_generation = None::<u64>;
    let mut titlebar_start_count = 0_u64;
    // Settle generation for the finish-time titlebar re-run (see the
    // PageLoadFinished arm): heals marker-time injections discarded by a
    // later mount, once per document, with no retry loop.
    let mut titlebar_settled_generation = None::<u64>;
    // Authoritative window state tracked from native events. Seeded from
    // boot snapshot (both false at startup since window is created hidden).
    let mut native_window_state = NativeWindowState::default();
    // Startup visibility fires exactly once per process (first real
    // document); reloads and later documents never re-apply it, and
    // shutdown drops the owned window handle with no stale state.
    let mut startup_visibility_applied = false;
    // P0.4 single popup slot: at most one managed child (Discord popout /
    // about:blank / hidden ICS). Repeated requests focus the live child;
    // child close clears the slot; parent shutdown drops it via RAII.
    let mut popup_child: Option<PopupChild> = None;
    // True once the main shutdown path has started (main-window
    // `CloseRequested` or renderer-initiated `window.quit`). Late
    // `ProcessFailed` records must not tear anything down afterwards.
    let mut shutdown_started = false;
    // Slot generation for the managed child: each created child takes the
    // next id, so observer records and counters never alias across
    // children sharing the slot over a process lifetime.
    let mut popup_generation: u64 = 0;
    // Phase 1 + 2B per-instance observers for the main WebView
    // (observability only), via the host-owned helper. Each is one raw-COM
    // subscription whose callback forwards sanitized primitives; the
    // CoreWebView2 clones plus tokens are removed on shutdown.
    // Registration failure only loses visibility; boot continues exactly
    // as before. The builder moved `ipc_proxy` into its IPC closure, so
    // the sibling popup proxy (same event loop) forwards observer records.
    #[cfg(target_os = "windows")]
    let (main_process_failed_opt, main_navigation_completed_opt) = {
        let observer_proxy = popup_ipc_proxy.clone();
        main_webview::attach_main_observers(&webview, main_instance_id, &observer_proxy)
    };
    #[cfg(target_os = "windows")]
    let mut main_process_failed = main_process_failed_opt;
    // Per-instance failure counters: main lives for the process, child
    // resets with every new child.
    let mut main_process_failed_counters = process_failure::ProcessFailedCounters::default();
    let mut child_process_failed_counters = process_failure::ProcessFailedCounters::default();
    #[cfg(target_os = "windows")]
    let mut main_navigation_completed = main_navigation_completed_opt;
    // Per-instance navigation counters: main lives for the process, child
    // resets with every new child. Observe-only: no budget, no guard.
    let mut main_navigation_counters = navigation_completed::NavigationCompletedCounters::default();
    let mut child_navigation_counters =
        navigation_completed::NavigationCompletedCounters::default();
    // Phase 2C trusted-load gate for the main WebView: a finished Discord
    // document enters the initialization pipeline only on a proven
    // successful navigation. The gate degrades to the pre-gating behavior
    // (every finish runs the pipeline) only when the observer
    // registration above failed.
    #[cfg(target_os = "windows")]
    let main_nav_observer_active = main_navigation_completed.is_some();
    #[cfg(not(target_os = "windows"))]
    let main_nav_observer_active = false;
    let mut main_trusted_gate =
        navigation_gate::NavigationGate::new(main_instance_id, main_nav_observer_active);
    // Phase 2A bounded main-renderer recovery: per-main-WebView budget and
    // single-flight guard (see `recovery.rs`). Scoped to the main
    // instance; child records never reach it.
    let mut main_recovery = MainRendererRecovery::new(main_instance_id);
    // Phase 2G bounded browser-process recovery: separate budget and
    // single-flight guard (see `browser_recovery.rs`). Scoped to the main
    // WebView slot (process-lifetime); independent of the renderer-recovery
    // budget. The environment observer provides the authoritative trigger.
    let mut browser_recovery = browser_recovery::BrowserRecovery::new();

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
                ..
            } => {
                // Child close clears the single slot; only the main window
                // shuts the application down (dropping the slot with it).
                if popup_child
                    .as_ref()
                    .is_some_and(|child| child.window.id() == window_id)
                {
                    // Child teardown: remove the Phase 1 observer first so
                    // no callback can target the slot being dropped, then
                    // drop the slot (destroying the child WebView).
                    if let Some(child) = popup_child.take() {
                        #[cfg(target_os = "windows")]
                        if let Some((core_webview2, token)) = child.process_failed {
                            process_failure::unregister(&core_webview2, token);
                        }
                        // Remove the navigation observer before the slot is
                        // dropped so no late callback can target it.
                        #[cfg(target_os = "windows")]
                        if let Some((core_webview2, token)) = child.navigation_completed {
                            navigation_completed::unregister(&core_webview2, token);
                        }
                        println!(
                            "[Popup] child closed instance={}; clearing single slot",
                            child.instance_id
                        );
                    }
                    return;
                }
                println!("[Native] Close requested; shutting down WebView");
                shutdown_started = true;
                println!(
                    "[Bridge][Metrics] messages_in={incoming_messages} requests={request_count} events={event_count} responses={response_count}"
                );
                for descriptor in KNOWN_MODS {
                    if let Some(snapshot) = mod_metrics.snapshot(descriptor.id) {
                        println!(
                            "[{}][Metrics] capability_requests={} successful_document_bootstraps={} duplicate_bootstraps={}",
                            descriptor.log_tag,
                            snapshot.capability_requests,
                            snapshot.successful_bootstraps,
                            snapshot.duplicate_bootstraps,
                        );
                    }
                }
                if let Ok(guard) = live_counters.lock() {
                    let snapshot = guard.snapshot();
                    println!(
                        "[LiveCss][Metrics] final watchers={} replacements={} raw_fs_events={} debounced_reloads={} css_updates={} css_removes={} stale_discarded={} order_repairs={}",
                        snapshot.watcher_count,
                        snapshot.watcher_replacements,
                        snapshot.raw_fs_events,
                        snapshot.debounced_reloads,
                        snapshot.css_updates,
                        snapshot.css_removes,
                        snapshot.stale_discarded,
                        snapshot.order_repairs,
                    );
                }
                println!(
                    "[WebView][ProcessFailed] final main_total={} child_total={}",
                    main_process_failed_counters.total, child_process_failed_counters.total,
                );
                println!(
                    "[WebView][NavigationCompleted] final main_total={} main_ok={} main_failed={} child_total={} child_ok={} child_failed={}",
                    main_navigation_counters.total,
                    main_navigation_counters.succeeded,
                    main_navigation_counters.failed,
                    child_navigation_counters.total,
                    child_navigation_counters.succeeded,
                    child_navigation_counters.failed,
                );
                println!(
                    "[WebView][Recovery] final attempts_in_window={} in_flight={}",
                    main_recovery.attempts_in_window(Instant::now()),
                    main_recovery.is_in_flight(),
                );
                // Application shutdown: remove observer subscriptions before
                // the loop exits so no callback can target a dead loop.
                // Remaining windows/WebViews drop via RAII afterwards.
                #[cfg(target_os = "windows")]
                {
                    main_webview::detach_main_observers(
                        main_process_failed.take(),
                        main_navigation_completed.take(),
                    );
                    if let Some(child) = popup_child.take() {
                        if let Some((core_webview2, token)) = child.process_failed {
                            process_failure::unregister(&core_webview2, token);
                        }
                        if let Some((core_webview2, token)) = child.navigation_completed {
                            navigation_completed::unregister(&core_webview2, token);
                        }
                    }
                    // Remove environment-level browser-process exit observer.
                    #[cfg(target_os = "windows")]
                    if let Some(token) = env_browser_exited_token.take() {
                        environment_observer::unregister(&webview_env, token);
                    }
                }
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent {
                event: WindowEvent::Focused(focused),
                ..
            } => {
                if native_window_state.focused != focused {
                    native_window_state.focused = focused;
                    send_window_state_event(&webview, native_window_state.focused, native_window_state.maximized);
                    println!("[Window] focused={focused}");
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => {
                // Maximize/unmaximize state changes via the native OS are
                // reported through Resized. Query the authoritative flag and
                // emit only when it actually changed.
                sync_native_maximized_state(&webview, &window, &mut native_window_state, "Resized");
            }
            Event::UserEvent(NativeEvent::PopupRequested { url, kind }) => {
                if let Some(child) = popup_child.as_ref() {
                    // Single slot: never stack unbounded windows. The live
                    // child is focused instead of creating another.
                    child.window.set_focus();
                    println!("[Popup] slot occupied; focused live child for {url}");
                    return;
                }
                // `alwaysOnTop` is host-owned (existing `popoutPiP`
                // setting, read fresh per request) and applies to the
                // Discord popout only; every other child option is fixed.
                let always_on_top = load_settings(&profile_path)
                    .map(|loaded| loaded.snapshot.popout_pip)
                    .unwrap_or(false);
                #[cfg(target_os = "windows")]
                let environment = popup_environment.clone();
                popup_generation += 1;
                let child_instance = popup_generation;
                match create_popup_child(
                    target,
                    kind,
                    &url,
                    &mut web_context,
                    #[cfg(target_os = "windows")]
                    environment,
                    &initialization_script,
                    &popup_ipc_proxy,
                    #[cfg(target_os = "windows")]
                    &popup_roots,
                    &downloads_dir,
                    always_on_top,
                    child_instance,
                ) {
                    Ok(child) => {
                        println!(
                            "[Popup] child opened {kind:?} instance={}: {url}",
                            child.instance_id
                        );
                        // Fresh slot generation, fresh counters: records
                        // from a previous child must never alias this one.
                        child_process_failed_counters =
                            process_failure::ProcessFailedCounters::default();
                        child_navigation_counters =
                            navigation_completed::NavigationCompletedCounters::default();
                        popup_child = Some(child);
                    }
                    Err(error) => eprintln!("[Popup] child failed: {error} url={url}"),
                }
            }
            Event::UserEvent(NativeEvent::ProcessFailed(event)) => {
                // Phase 1 observer: count per instance and log the
                // sanitized record for both roles. Phase 2A adds bounded
                // recovery for main renderer exits only: one host-side
                // reload per decision, inside a per-WebView budget. Phase
                // 2H closes the managed child on its own renderer exit;
                // unresponsive, frame-exit, browser-process, and navigation
                // failures never reload or close here.
                if event.role == process_failure::WebViewRole::Child {
                    let live_child_instance =
                        popup_child.as_ref().map(|child| child.instance_id);
                    // Phase 2H policy first: stale records (an older child
                    // instance, or no live child at all) are dropped before
                    // counting so they can never alias the live slot's
                    // totals or close it. This mirrors the
                    // `NavigationCompleted` child arm below.
                    let decision =
                        child_recovery::decide(&event, live_child_instance, shutdown_started);
                    match &decision {
                        child_recovery::ChildExitDecision::Ignore {
                            reason:
                                child_recovery::ChildExitIgnoreReason::NoLiveChild,
                        } => {
                            println!(
                                "{} stale no_live_child",
                                process_failure::format_log(&event)
                            );
                            return;
                        }
                        child_recovery::ChildExitDecision::Ignore {
                            reason: child_recovery::ChildExitIgnoreReason::Stale { .. },
                        } => {
                            println!("{}", child_recovery::format_log(&decision, &event));
                            return;
                        }
                        _ => {}
                    }
                    child_process_failed_counters.record(event.kind);
                    println!("{}", process_failure::format_log(&event));
                    // A confirmed child renderer exit closes and drops that
                    // child on this thread. Nothing else here closes,
                    // reloads, recreates, spends either recovery budget, or
                    // retries a download.
                    match decision {
                        child_recovery::ChildExitDecision::Close { instance_id } => {
                            println!("{}", child_recovery::format_log(&decision, &event));
                            // Mark closing by clearing the slot first, and
                            // only when it still refers to that exact
                            // instance: a duplicate collapses into a no-op
                            // and a late record can never clear a newer
                            // replacement (including across browser
                            // recovery, which drops the previous child).
                            let still_live = popup_child
                                .as_ref()
                                .is_some_and(|child| child.instance_id == instance_id);
                            if !still_live {
                                // Unreachable on the event-loop thread (the
                                // slot cannot change mid-arm), but fail
                                // closed: never clear a slot that no longer
                                // refers to this instance.
                                println!(
                                    "[Popup][ChildExit] ignore role=child instance={instance_id} kind=RenderProcessExited reason=stale"
                                );
                                return;
                            }
                            if let Some(child) = popup_child.take() {
                                // Invalidate the child's callbacks first so
                                // no late callback can target the slot being
                                // dropped. Best-effort: already-dead COM
                                // objects only log, never propagate.
                                #[cfg(target_os = "windows")]
                                if let Some((core_webview2, token)) = child.process_failed {
                                    process_failure::unregister(&core_webview2, token);
                                }
                                #[cfg(target_os = "windows")]
                                if let Some((core_webview2, token)) = child.navigation_completed {
                                    navigation_completed::unregister(&core_webview2, token);
                                }
                                // Dropping the slot destroys the child
                                // WebView/controller and closes its native
                                // window; its download handlers drop with
                                // it, so a pending hidden-ICS download is
                                // dropped without ever claiming completion.
                                println!(
                                    "[Popup] child closed after renderer exit instance={instance_id}; clearing single slot"
                                );
                                // `child` (WebView/controller + native
                                // window) drops here, at scope end.
                            }
                        }
                        decision @ child_recovery::ChildExitDecision::Ignore { .. } => {
                            println!("{}", child_recovery::format_log(&decision, &event));
                        }
                    }
                    return;
                }
                main_process_failed_counters.record(event.kind);
                println!("{}", process_failure::format_log(&event));
                if event.role == process_failure::WebViewRole::Main {
                    let decision = main_recovery.decide(&event, Instant::now());
                    println!("{}", recovery::format_log(&decision, &event));
                    if matches!(
                        decision,
                        recovery::RecoveryDecision::ReloadRequested { .. }
                    ) {
                        match webview.reload() {
                            Ok(()) => {
                                println!("{}", recovery::format_reload_result(true, None));
                            }
                            Err(error) => {
                                main_recovery.on_reload_error();
                                println!(
                                    "{}",
                                    recovery::format_reload_result(
                                        false,
                                        Some(&error.to_string())
                                    )
                                );
                            }
                        }
                    }
                }
            }
            Event::UserEvent(NativeEvent::BrowserProcessExited(event)) => {
                // Phase 2G: environment-level browser-process exit observer.
                // Provides authoritative exit kind (Normal/Failed) and browser PID.
                // Only Failed exits are eligible for browser-process recreation.
                // The per-WebView ProcessFailed observer (above) remains as a
                // corroborating signal but does NOT drive browser recreation.
                let decision = browser_recovery.decide(&event, Instant::now());
                println!("{}", browser_recovery::format_browser_recovery_log(&decision, &event));
                if matches!(
                    decision,
                    browser_recovery::BrowserRecoveryDecision::RecreationRequested { .. }
                ) {
                    println!("[BrowserRecovery] Starting browser-process recreation (instance={} -> {})",
                        main_instance_id, main_lifecycle.next_instance_id());

                    // 1. Confirm shutdown has not started (event loop is running).
                    // 2. Enter single-flight recovery state (already done by decide()).

                    // 3. Prevent callbacks from old WebView by detaching observers first.
                    #[cfg(target_os = "windows")]
                    {
                        main_webview::detach_main_observers(
                            main_process_failed.take(),
                            main_navigation_completed.take(),
                        );
                    }

                    // 4. Drop the managed popup child and its subscriptions because
                    // it shares the dead browser process.
                    if let Some(child) = popup_child.take() {
                        #[cfg(target_os = "windows")]
                        if let Some((core_webview2, token)) = child.process_failed {
                            process_failure::unregister(&core_webview2, token);
                        }
                        #[cfg(target_os = "windows")]
                        if let Some((core_webview2, token)) = child.navigation_completed {
                            navigation_completed::unregister(&core_webview2, token);
                        }
                        println!("[BrowserRecovery] Dropped popup child instance={}", child.instance_id);
                    }
                    // Reset child counters since the child is gone.
                    child_process_failed_counters = process_failure::ProcessFailedCounters::default();
                    child_navigation_counters = navigation_completed::NavigationCompletedCounters::default();

                    // 5. The old WebView is dropped by replacing it with a new one.
                    // The old CoreWebView2 is released when the WebView is dropped.

                    // 6. Allocate a fresh, monotonically increasing main-WebView instance ID.
                    let new_instance_id = main_lifecycle.record_replacement();
                    main_instance_id = new_instance_id;
                    println!("[BrowserRecovery] New main WebView instance id={}", main_instance_id);

                    // 7. Reset per-instance state for the new WebView.
                    document_lifecycle = DocumentModLifecycle::default();
                    request_tracker.clear();
                    pending_mods.clear();
                    pending_graphics.clear();
                    main_process_failed_counters = process_failure::ProcessFailedCounters::default();
                    main_navigation_counters = navigation_completed::NavigationCompletedCounters::default();
                    main_recovery = MainRendererRecovery::new(main_instance_id);
                    main_trusted_gate = navigation_gate::NavigationGate::new(main_instance_id, true);
                    // Reset trusted-load pipeline state.
                    startup_visibility_applied = false;
                    page_load_started_at = None;
                    titlebar_settled_generation = None;
                    last_quickcss_injected = false;
                    last_quickcss_css = None;
                    last_theme_style_ids.clear();
                    last_themes.clear();

                    // 8. Build a fresh main WebView using the Phase 2F helpers.
                    let builder = main_webview::main_webview_builder(
                        &mut web_context,
                        #[cfg(target_os = "windows")]
                        webview_env.clone(),
                        &initialization_script,
                        downloads_dir.clone(),
                        ipc_proxy.clone(),
                        load_proxy.clone(),
                        popup_proxy.clone(),
                        main_instance_id,
                    );
                    webview = builder.build(&window_for_webview).expect("failed to build replacement WebView");
                    println!("[BrowserRecovery] Replacement WebView built");

                    // 9. Re-attach per-WebView content filters (remcord://, permissions, CSP).
                    #[cfg(target_os = "windows")]
                    {
                        let _ = main_webview::attach_main_content_filters(&webview, &profile_path, &profile_path);
                    }

                    // 10. Re-attach per-WebView observers.
                    #[cfg(target_os = "windows")]
                    {
                        let observer_proxy = popup_ipc_proxy.clone();
                        let (pf_opt, nc_opt) = main_webview::attach_main_observers(&webview, main_instance_id, &observer_proxy);
                        main_process_failed = pf_opt;
                        main_navigation_completed = nc_opt;
                    }

                    // 11. Re-run the initial Discord navigation (two-phase: about:blank -> Discord).
                    // The builder already navigated to about:blank; now navigate to Discord.
                    if let Err(error) = main_webview::navigate_to_discord(&webview, discord_url) {
                        eprintln!("[BrowserRecovery] Initial Discord navigation failed: {error}");
                        browser_recovery.on_recreation_error();
                        println!("[BrowserRecovery] Recreation failed; parked. Manual restart required.");
                    } else {
                        println!("[BrowserRecovery] Initial Discord navigation started for replacement");
                    }
                }
            }
            Event::UserEvent(NativeEvent::NavigationCompleted(event)) => {
                // Phase 2B observer: count per live instance and log the
                // sanitized record. Phase 2C additionally resolves a
                // stashed main-document finish with the authoritative
                // `IsSuccess` (HTTP/error metadata never decide). Still no
                // recovery state change and no reload: a failure only
                // skips the trusted pipeline, and the manual F5 path still
                // recovers through a fresh gated navigation. Child records
                // stay observe-only: the child builder registers no
                // page-load handler, so no finish-driven pipeline exists
                // for the child to gate (per-message origin checks are
                // unaffected by load state).
                match event.role {
                    process_failure::WebViewRole::Main => {
                        if navigation_completed::is_stale(&event, main_instance_id) {
                            println!(
                                "{}",
                                navigation_completed::format_stale_log(&event, main_instance_id)
                            );
                            return;
                        }
                        main_navigation_counters.record(&event);
                    }
                    process_failure::WebViewRole::Child => {
                        match popup_child.as_ref() {
                            Some(child) if child.instance_id == event.instance_id => {
                                child_navigation_counters.record(&event);
                            }
                            Some(child) => {
                                let expected = child.instance_id;
                                println!(
                                    "{}",
                                    navigation_completed::format_stale_log(&event, expected)
                                );
                                return;
                            }
                            None => {
                                println!(
                                    "{} stale no_live_child",
                                    navigation_completed::format_log(&event)
                                );
                                return;
                            }
                        }
                    }
                }
                println!("{}", navigation_completed::format_log(&event));
                if event.role == process_failure::WebViewRole::Main {
                    match main_trusted_gate.on_outcome(event.is_success) {
                        navigation_gate::OutcomeAction::RunPipeline { generation } => {
                            // Gate invariant: a resolved pending finish
                            // always belongs to the live generation —
                            // any intervening `Started` would have
                            // cleared it — so this pipeline run targets
                            // the current document, never a superseded
                            // one.
                            debug_assert_eq!(generation, document_lifecycle.generation());
                            run_trusted_load_pipeline(TrustedLoadContext {
                                webview: &webview,
                                profile_path: &profile_path,
                                proxy: &proxy,
                                generation,
                                titlebar_plugin_start_script:
                                    &titlebar_plugin_start_script,
                                mod_smoke_script: &mod_smoke_script,
                                settings_ui_probe_script: &settings_ui_probe_script,
                                titlebar_settled_generation:
                                    &mut titlebar_settled_generation,
                                last_quickcss_injected: &mut last_quickcss_injected,
                                last_quickcss_css: &mut last_quickcss_css,
                                last_theme_style_ids: &mut last_theme_style_ids,
                                last_themes: &mut last_themes,
                                flags: TrustedLoadFlags {
                                    bare_page: diagnostic_bare_page,
                                    themes_smoke: diagnostic_themes_smoke,
                                    quickcss_smoke: diagnostic_quickcss_smoke,
                                    mod_smoke: diagnostic_mod_smoke,
                                    interaction: diagnostic_interaction,
                                    settings_smoke: diagnostic_settings_smoke,
                                    settings_mutation_smoke:
                                        diagnostic_settings_mutation_smoke,
                                    settings_set_smoke: diagnostic_settings_set_smoke,
                                    getlang_smoke: diagnostic_getlang_smoke,
                                    app_version_smoke: diagnostic_app_version_smoke,
                                    graphics_smoke: diagnostic_graphics_smoke,
                                    plugin_storage_smoke: diagnostic_plugin_storage_smoke,
                                    theme_install_smoke: diagnostic_theme_install_smoke,
                                    theme_write_smoke: diagnostic_theme_write_smoke,
                                    theme_import_smoke: diagnostic_theme_import_smoke,
                                    theme_set_enabled_smoke:
                                        diagnostic_theme_set_enabled_smoke,
                                    theme_list_smoke: diagnostic_theme_list_smoke,
                                    backup_save_smoke: diagnostic_backup_save_smoke,
                                    backup_restore_smoke:
                                        diagnostic_backup_restore_smoke,
                                    shelter_loader_probe: diagnostic_shelter_loader_probe,
                                    settings_boot_smoke: diagnostic_settings_boot_smoke,
                                    remcord_protocol_smoke:
                                        diagnostic_remcord_protocol_smoke,
                                    settings_ui_smoke: diagnostic_settings_ui_smoke,
                                    permissions_smoke: diagnostic_permissions_smoke,
                                    mutation_stage: mutation_smoke_stage,
                                    mutation_before: mutation_smoke_before,
                                    settings_set_stage: settings_set_smoke_stage,
                                    settings_set_before: settings_set_smoke_before,
                                },
                            });
                        }
                        navigation_gate::OutcomeAction::SkipFailed { generation } => {
                            println!(
                                "{}",
                                main_trusted_gate.format_skip_log("main", generation)
                            );
                        }
                        navigation_gate::OutcomeAction::Absorbed => {}
                    }
                }
            }
            Event::UserEvent(NativeEvent::PageLoadStarted) => {
                document_lifecycle.begin_document();
                request_tracker.clear();
                pending_mods.clear();
                pending_graphics.clear();
                // Phase 2C: the new navigation attempt supersedes every
                // pending finish and stored outcome — a previous
                // navigation's verdict can never decide this document. A
                // discarded stranded finish is fail-closed (its pipeline
                // never ran) and logged for observability.
                if let Some(stranded) =
                    main_trusted_gate.on_started(document_lifecycle.generation())
                {
                    println!(
                        "{}",
                        main_trusted_gate.format_stranded_log("main", stranded)
                    );
                }
                // Phase 2A: `Started` is WebView2 `ContentLoading` — a live
                // renderer began loading new content — so an approved reload
                // has taken effect and the single-flight guard releases. The
                // attempt history is untouched: budget and cooldown still
                // bound the next decision.
                main_recovery.on_page_load_started();
                // Phase 2G: browser-process recreation guard release.
                browser_recovery.on_page_load_started();
                page_load_started_at = Some(Instant::now());
                println!(
                    "[Navigation] new document generation={} (bridge state reset)",
                    document_lifecycle.generation()
                );
            }
            Event::UserEvent(NativeEvent::PageLoadFinished(url)) => {
                // Phase 2A: deliberately no recovery state change here. Wry
                // forwards NavigationCompleted as Finished even for failed
                // navigations (IsSuccess unchecked), so Finished cannot
                // prove the reload recovered anything; only Started
                // (ContentLoading) releases the single-flight guard.
                // One host-owned ShowWindow at the native-ready point: the
                // first finished non-boot document that passes the
                // navigation clamp. Hidden startup is preserved until then.
                if !startup_visibility_applied
                    && url != "about:blank"
                    && is_allowed_navigation(&url)
                {
                    startup_visibility_applied = true;
                    let (mode_name, tray_name) = match load_settings(&profile_path) {
                        Ok(loaded) => (loaded.snapshot.start_minimized, loaded.snapshot.tray),
                        Err(error) => {
                            eprintln!("[Window] startup settings unreadable, showing ({error})");
                            ("off".to_owned(), "dynamic".to_owned())
                        }
                    };
                    let mode = startup_visibility(mode_name.as_str());
                    println!("[Window] startup visibility mode={mode:?}");
                    apply_startup_visibility(&window, mode, tray_name.as_str());
                }
                if let Some(started_at) = page_load_started_at.take() {
                    println!(
                        "[Navigation][Metrics] document load completed in {:.3} ms",
                        started_at.elapsed().as_secs_f64() * 1000.0
                    );
                }
                if url
                    .parse::<Uri>()
                    .is_ok_and(|uri| is_trusted_bridge_origin(&uri))
                {
                    // Phase 2C: a finished Discord document enters the
                    // trusted pipeline only on a proven successful
                    // navigation. The gate pairs this finish with the raw
                    // observer outcome by document generation (see
                    // `navigation_gate.rs`): both arrival orders resolve
                    // correctly, and a failed or still-unknown navigation
                    // never runs the pipeline. No reload, no retry, no
                    // timers — a failure only skips, and the manual F5 path
                    // still recovers through a fresh gated navigation.
                    let generation = document_lifecycle.generation();
                    match main_trusted_gate.on_finished(generation) {
                        navigation_gate::FinishAction::RunPipeline => {
                            run_trusted_load_pipeline(TrustedLoadContext {
                                webview: &webview,
                                profile_path: &profile_path,
                                proxy: &proxy,
                                generation,
                                titlebar_plugin_start_script:
                                    &titlebar_plugin_start_script,
                                mod_smoke_script: &mod_smoke_script,
                                settings_ui_probe_script: &settings_ui_probe_script,
                                titlebar_settled_generation:
                                    &mut titlebar_settled_generation,
                                last_quickcss_injected: &mut last_quickcss_injected,
                                last_quickcss_css: &mut last_quickcss_css,
                                last_theme_style_ids: &mut last_theme_style_ids,
                                last_themes: &mut last_themes,
                                flags: TrustedLoadFlags {
                                    bare_page: diagnostic_bare_page,
                                    themes_smoke: diagnostic_themes_smoke,
                                    quickcss_smoke: diagnostic_quickcss_smoke,
                                    mod_smoke: diagnostic_mod_smoke,
                                    interaction: diagnostic_interaction,
                                    settings_smoke: diagnostic_settings_smoke,
                                    settings_mutation_smoke:
                                        diagnostic_settings_mutation_smoke,
                                    settings_set_smoke: diagnostic_settings_set_smoke,
                                    getlang_smoke: diagnostic_getlang_smoke,
                                    app_version_smoke: diagnostic_app_version_smoke,
                                    graphics_smoke: diagnostic_graphics_smoke,
                                    plugin_storage_smoke: diagnostic_plugin_storage_smoke,
                                    theme_install_smoke: diagnostic_theme_install_smoke,
                                    theme_write_smoke: diagnostic_theme_write_smoke,
                                    theme_import_smoke: diagnostic_theme_import_smoke,
                                    theme_set_enabled_smoke:
                                        diagnostic_theme_set_enabled_smoke,
                                    theme_list_smoke: diagnostic_theme_list_smoke,
                                    backup_save_smoke: diagnostic_backup_save_smoke,
                                    backup_restore_smoke:
                                        diagnostic_backup_restore_smoke,
                                    shelter_loader_probe: diagnostic_shelter_loader_probe,
                                    settings_boot_smoke: diagnostic_settings_boot_smoke,
                                    remcord_protocol_smoke:
                                        diagnostic_remcord_protocol_smoke,
                                    settings_ui_smoke: diagnostic_settings_ui_smoke,
                                    permissions_smoke: diagnostic_permissions_smoke,
                                    mutation_stage: mutation_smoke_stage,
                                    mutation_before: mutation_smoke_before,
                                    settings_set_stage: settings_set_smoke_stage,
                                    settings_set_before: settings_set_smoke_before,
                                },
                            });
                        }
                        navigation_gate::FinishAction::SkipFailed => {
                            // The outcome arrived first and the navigation
                            // failed: the pipeline never ran for it.
                            println!(
                                "{}",
                                main_trusted_gate.format_skip_log("main", generation)
                            );
                        }
                        navigation_gate::FinishAction::AwaitingOutcome => {
                            // The outcome has not arrived yet: the finish is
                            // stashed and the observer arm will resolve it.
                            println!(
                                "{}",
                                main_trusted_gate.format_awaiting_log("main", generation)
                            );
                        }
                    }
                }
            }
            Event::UserEvent(NativeEvent::RemcordProtocolSmokeReadback) => {
                if diagnostic_remcord_protocol_smoke {
                    if let Err(error) = webview.evaluate_script_with_callback(
                        REMCORD_PROTOCOL_SMOKE_READBACK_SCRIPT,
                        |result| {
                            println!("[RemcordProtocol][Smoke] subresources={result}");
                        },
                    ) {
                        eprintln!("[RemcordProtocol][Smoke] readback failed: {error}");
                    }
                }
            }
            Event::UserEvent(NativeEvent::PermissionsSmokeReadback) => {
                if diagnostic_permissions_smoke {
                    if let Err(error) = webview.evaluate_script_with_callback(
                        PERMISSIONS_SMOKE_READBACK_SCRIPT,
                        |result| {
                            println!("[Permissions][Smoke] outcomes={result}");
                        },
                    ) {
                        eprintln!("[Permissions][Smoke] readback failed: {error}");
                    }
                }
            }
            Event::UserEvent(NativeEvent::LiveCssSignal(signal)) => {
                // Raw watcher event: bump the epoch and arm exactly one
                // debounce timer per burst. The timer re-reads authoritative
                // host state; this arm never touches the disk or renderer.
                let generation = document_lifecycle.generation();
                let debouncer = match signal {
                    LiveCssFsSignal::QuickCss => &quickcss_debounce,
                    LiveCssFsSignal::Themes => &themes_debounce,
                };
                let tag = match signal {
                    LiveCssFsSignal::QuickCss => "QuickCss",
                    LiveCssFsSignal::Themes => "Themes",
                };
                let should_spawn = match debouncer.lock() {
                    Ok(mut guard) => guard.on_fs_event(generation).1,
                    Err(_) => false,
                };
                if should_spawn {
                    let timer_debouncer = Arc::clone(debouncer);
                    let timer_proxy = proxy.clone();
                    if let Err(error) = thread::Builder::new()
                        .name(format!("remcord-livecss-{tag}"))
                        .spawn(move || {
                            thread::sleep(Duration::from_millis(CSS_DEBOUNCE_MS));
                            let (epoch, arm_generation) = match timer_debouncer.lock() {
                                Ok(mut guard) => guard.take_fire_epoch(),
                                Err(_) => return,
                            };
                            queue_event(
                                &timer_proxy,
                                NativeEvent::LiveCssFire {
                                    resource: signal,
                                    epoch,
                                    generation: arm_generation,
                                },
                            );
                        })
                    {
                        eprintln!("[LiveCss][{tag}] could not arm the debounce timer: {error}");
                        // Un-arm so the next event retries instead of
                        // stalling the resource forever.
                        if let Ok(mut guard) = debouncer.lock() {
                            if guard.is_armed() {
                                guard.take_fire_epoch();
                            }
                        }
                    }
                }
            }
            Event::UserEvent(NativeEvent::LiveCssFire {
                resource,
                epoch,
                generation,
            }) => {
                // Stale gate first: a superseded burst epoch or a document
                // reload mid-debounce discards the callback before any
                // filesystem or renderer work.
                let current_generation = document_lifecycle.generation();
                let current_epoch = match resource {
                    LiveCssFsSignal::QuickCss => {
                        quickcss_debounce.lock().map(|guard| guard.epoch()).unwrap_or(u64::MAX)
                    }
                    LiveCssFsSignal::Themes => {
                        themes_debounce.lock().map(|guard| guard.epoch()).unwrap_or(u64::MAX)
                    }
                };
                if !ResourceDebouncer::is_fresh(epoch, generation, current_epoch, current_generation) {
                    if let Ok(mut guard) = live_counters.lock() {
                        guard.stale_discarded += 1;
                    }
                    println!(
                        "[LiveCss][stale] discarded {:?} fire (epoch={epoch} gen={generation} vs current epoch={current_epoch} gen={current_generation})",
                        resource
                    );
                    return;
                }                match resource {
                    LiveCssFsSignal::QuickCss => {
                        let read_started_at = Instant::now();
                        let report = load_quick_css(&profile_path);
                        let read_ms = read_started_at.elapsed().as_secs_f64() * 1000.0;
                        if let Ok(mut guard) = live_counters.lock() {
                            guard.debounced_reloads += 1;
                        }
                        match quick_css_transition(
                            last_quickcss_css.as_deref(),
                            report.css.as_deref(),
                        ) {
                            QuickCssTransition::NoChange => {
                                println!(
                                    "[QuickCss][Live] no-change state={} read_ms={read_ms:.3}",
                                    report.state
                                );
                            }
                            QuickCssTransition::InjectOrUpdate { css } => {
                                let created = last_quickcss_css.is_none();
                                let inject_started_at = Instant::now();
                                let applied = apply_managed_css_update(
                                    &webview,
                                    QUICK_CSS_STYLE_ID,
                                    &css,
                                    "QuickCss",
                                );
                                let inject_ms =
                                    inject_started_at.elapsed().as_secs_f64() * 1000.0;
                                if applied {
                                    last_quickcss_css = Some(css);
                                    last_quickcss_injected = true;
                                    if let Ok(mut guard) = live_counters.lock() {
                                        guard.css_updates += 1;
                                    }
                                    // A recreated node appends at the head
                                    // end; repair order so Quick CSS still
                                    // precedes themes.
                                    if created && !last_themes.is_empty() {
                                        let mut ordered: Vec<String> =
                                            vec![QUICK_CSS_STYLE_ID.to_owned()];
                                        ordered.extend(
                                            last_themes
                                                .iter()
                                                .map(|theme| theme.style_id.clone()),
                                        );
                                        if apply_managed_css_order(&webview, &ordered, "QuickCss")
                                        {
                                            if let Ok(mut guard) = live_counters.lock() {
                                                guard.order_repairs += 1;
                                            }
                                        }
                                    }
                                }
                                println!(
                                    "[QuickCss][Live] update state={} created={created} applied={applied} read_ms={read_ms:.3} inject_ms={inject_ms:.3}",
                                    report.state
                                );
                            }
                            QuickCssTransition::Remove => {
                                let removed = if last_quickcss_injected {
                                    apply_managed_css_removal(&webview, QUICK_CSS_STYLE_ID, "QuickCss")
                                } else {
                                    true
                                };
                                if removed {
                                    last_quickcss_css = None;
                                    last_quickcss_injected = false;
                                    if let Ok(mut guard) = live_counters.lock() {
                                        guard.css_removes += 1;
                                    }
                                }
                                println!(
                                    "[QuickCss][Live] remove state={} applied={removed} read_ms={read_ms:.3}",
                                    report.state
                                );
                            }
                        }
                    }
                    LiveCssFsSignal::Themes => {
                        let read_started_at = Instant::now();
                        let report = load_enabled_themes(&profile_path);
                        let read_ms = read_started_at.elapsed().as_secs_f64() * 1000.0;
                        if let Ok(mut guard) = live_counters.lock() {
                            guard.debounced_reloads += 1;
                        }
                        let next = report.injected.clone();
                        let diff = diff_theme_snapshots(&last_themes, &next);
                        // Per-theme skip audit (mirrors the startup path):
                        // a rejected manifest/CSS must name its reason live.
                        for skipped in &report.skipped {
                            println!(
                                "[Themes][Skip] id={} reason={}",
                                skipped.id, skipped.reason
                            );
                        }
                        if diff.is_empty() {
                            println!(
                                "[Themes][Live] no-change discovered={} enabled={} read_ms={read_ms:.3}",
                                report.discovered, report.enabled
                            );
                        } else {
                            let inject_started_at = Instant::now();
                            // Merge into the last-pushed snapshot only on
                            // per-item success, so a failed push is retried
                            // instead of mistaken for present.
                            let mut merged: HashMap<String, LoadedTheme> = last_themes
                                .iter()
                                .map(|theme| (theme.style_id.clone(), theme.clone()))
                                .collect();
                            for theme in diff.updated.iter().chain(diff.added.iter()) {
                                if apply_managed_css_update(
                                    &webview,
                                    &theme.style_id,
                                    &theme.css,
                                    "Themes",
                                ) {
                                    merged.insert(theme.style_id.clone(), theme.clone());
                                    if let Ok(mut guard) = live_counters.lock() {
                                        guard.css_updates += 1;
                                    }
                                }
                            }
                            for style_id in &diff.removed_style_ids {
                                if apply_managed_css_removal(&webview, style_id, "Themes") {
                                    merged.remove(style_id);
                                    if let Ok(mut guard) = live_counters.lock() {
                                        guard.css_removes += 1;
                                    }
                                }
                            }
                            let mut merged_list: Vec<LoadedTheme> =
                                merged.into_values().collect();
                            merged_list.sort_by(|left, right| left.id.cmp(&right.id));
                            last_themes = merged_list;
                            last_theme_style_ids = last_themes
                                .iter()
                                .map(|theme| theme.style_id.clone())
                                .collect();
                            // Structural changes can misplace newly appended
                            // nodes; re-assert Quick CSS-first + sorted
                            // themes. Content-only updates skip this.
                            if diff.is_structural() {
                                let mut ordered: Vec<String> = Vec::new();
                                if last_quickcss_injected {
                                    ordered.push(QUICK_CSS_STYLE_ID.to_owned());
                                }
                                ordered.extend(
                                    last_themes.iter().map(|theme| theme.style_id.clone()),
                                );
                                if apply_managed_css_order(&webview, &ordered, "Themes") {
                                    if let Ok(mut guard) = live_counters.lock() {
                                        guard.order_repairs += 1;
                                    }
                                }
                            }
                            let inject_ms =
                                inject_started_at.elapsed().as_secs_f64() * 1000.0;
                            println!(
                                "[Themes][Live] updated={} added={} removed={} structural={} read_ms={read_ms:.3} inject_ms={inject_ms:.3}",
                                diff.updated.len(),
                                diff.added.len(),
                                diff.removed_style_ids.len(),
                                diff.is_structural(),
                            );
                        }
                    }
                }
                if let Ok(guard) = live_counters.lock() {
                    let snapshot = guard.snapshot();
                    println!(
                        "[LiveCss][Metrics] watchers={} raw_fs_events={} debounced_reloads={} css_updates={} css_removes={} stale_discarded={} order_repairs={}",
                        snapshot.watcher_count,
                        snapshot.raw_fs_events,
                        snapshot.debounced_reloads,
                        snapshot.css_updates,
                        snapshot.css_removes,
                        snapshot.stale_discarded,
                        snapshot.order_repairs,
                    );
                }
            }
            Event::UserEvent(NativeEvent::BridgeRequest(request, received_at)) => {
                incoming_messages += 1;
                request_count += 1;
                match request_tracker.begin(&request.request_id) {
                    Err(error) => eprintln!("[Bridge] rejected request: {error}"),
                    Ok(()) => match request.command {
                            BridgeCommand::WindowShow(_) => {
                                window.set_visible(true);
                                if window.is_minimized() {
                                    window.set_minimized(false);
                                }
                                window.set_focus();
                                respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_show", &received_at);
                                response_count += 1;
                            }
                            BridgeCommand::WindowHide(_) => {
                                // The titlebar × sends hide when
                                // minimizeToTray=true, but no native tray
                                // icon exists to reopen from: hiding would
                                // strand the app running with no visible
                                // window. With no usable tray every close
                                // takes the orderly quit path instead,
                                // regardless of minimizeToTray. Minimize,
                                // maximize/restore, and explicit quit are
                                // untouched.
                                let action = close_behavior::decide(
                                    true,
                                    close_behavior::tray_available(),
                                );
                                println!(
                                    "{}",
                                    close_behavior::format_log(&action, true, close_behavior::tray_available())
                                );
                                match action {
                                    close_behavior::CloseAction::Hide => {
                                        window.set_visible(false);
                                        respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_hide", &received_at);
                                        response_count += 1;
                                    }
                                    close_behavior::CloseAction::Quit => {
                                        #[cfg(target_os = "windows")]
                                        perform_orderly_shutdown(
                                            &mut *control_flow,
                                            &mut shutdown_started,
                                            &main_process_failed_counters,
                                            &child_process_failed_counters,
                                            &main_navigation_counters,
                                            &child_navigation_counters,
                                            &mut main_process_failed,
                                            &mut main_navigation_completed,
                                            &mut popup_child,
                                            &mut env_browser_exited_token,
                                            &webview_env,
                                        );
                                        #[cfg(not(target_os = "windows"))]
                                        perform_orderly_shutdown(
                                            &mut *control_flow,
                                            &mut shutdown_started,
                                            &main_process_failed_counters,
                                            &child_process_failed_counters,
                                            &main_navigation_counters,
                                            &child_navigation_counters,
                                        );
                                        respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_hide", &received_at);
                                        response_count += 1;
                                    }
                                }
                            }
                            BridgeCommand::WindowMinimize(_) => {
                                window.set_minimized(true);
                                respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_minimize", &received_at);
                                response_count += 1;
                            }
                            BridgeCommand::WindowMaximize(_) => {
                                // A maximize request while borderless
                                // fullscreen would change nothing visible:
                                // leave fullscreen first, then maximize
                                // (the standard caption-button behavior).
                                if window.fullscreen().is_some() {
                                    window.set_fullscreen(None);
                                    println!("[Window] fullscreen=false (maximize requested)");
                                }
                                window.set_maximized(true);
                                respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_maximize", &received_at);
                                response_count += 1;
                            }
                            BridgeCommand::WindowUnmaximize(_) => {
                                // Same invisible-no-op hazard as maximize:
                                // restoring while fullscreen must first leave
                                // fullscreen to reach the normal state.
                                if window.fullscreen().is_some() {
                                    window.set_fullscreen(None);
                                    println!("[Window] fullscreen=false (unmaximize requested)");
                                }
                                window.set_maximized(false);
                                respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_unmaximize", &received_at);
                                response_count += 1;
                            }
                            BridgeCommand::WindowDrag(_) => {
                                // Frameless chrome has no OS drag region
                                // (wry honors no `-webkit-app-region`): the
                                // titlebar plugin calls this fixed operation
                                // from its explicit drag handle on mousedown.
                                // A failed drag start is loud but never
                                // fatal: the window simply stays put.
                                if let Err(error) = window.drag_window() {
                                    eprintln!("[Window] drag start failed: {error:?}");
                                }
                                respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_drag", &received_at);
                                response_count += 1;
                            }
                            BridgeCommand::WindowToggleFullscreen(_) => {
                                // The F11 contract: flip between windowed and
                                // borderless fullscreen. The renderer owns
                                // the key binding; the host owns the state,
                                // so no boolean crosses the boundary and a
                                // stuck renderer view cannot desync it.
                                let next = if window.fullscreen().is_some() {
                                    None
                                } else {
                                    Some(tao::window::Fullscreen::Borderless(None))
                                };
                                window.set_fullscreen(next);
                                println!(
                                    "[Window] fullscreen={}",
                                    if window.fullscreen().is_some() { "true" } else { "false" }
                                );
                                respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_toggle_fullscreen", &received_at);
                                response_count += 1;
                            }
                            BridgeCommand::WindowQuit(_) => {
                                // Renderer-initiated quit takes the same
                                // shutdown path as CloseRequested: remove
                                // observer subscriptions before exiting.
                                #[cfg(target_os = "windows")]
                                perform_orderly_shutdown(
                                    &mut *control_flow,
                                    &mut shutdown_started,
                                    &main_process_failed_counters,
                                    &child_process_failed_counters,
                                    &main_navigation_counters,
                                    &child_navigation_counters,
                                    &mut main_process_failed,
                                    &mut main_navigation_completed,
                                    &mut popup_child,
                                    &mut env_browser_exited_token,
                                    &webview_env,
                                );
                                #[cfg(not(target_os = "windows"))]
                                perform_orderly_shutdown(
                                    &mut *control_flow,
                                    &mut shutdown_started,
                                    &main_process_failed_counters,
                                    &child_process_failed_counters,
                                    &main_navigation_counters,
                                    &child_navigation_counters,
                                );
                                respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_quit", &received_at);
                                response_count += 1;
                            }
                            BridgeCommand::Ping(payload) => {
                            let request_id = request.request_id;
                            let response = BridgeResponse {
                                version: PROTOCOL_VERSION,
                                request_id: request_id.clone(),
                                ok: true,
                                payload: Some(BridgeResponsePayload::Ping(PingResponse {
                                    nonce: payload.nonce,
                                    message: "pong",
                                })),
                                error: None,
                            };
                            send_bridge_response(&webview, &response);
                            if let Err(error) = request_tracker.complete(&request_id) {
                                eprintln!("[Bridge] correlation failure after response: {error}");
                            }
                            response_count += 1;
                            println!(
                                "[Bridge][Metrics] ping request-response latency: {:.3} ms",
                                received_at.elapsed().as_secs_f64() * 1000.0
                            );
                        }
                        BridgeCommand::SettingsGetConfig(_) => {
                            // Read-only capability: no renderer parameters exist
                            // by protocol, and the settings path is built from
                            // the fixed host profile directory only.
                            let request_id = request.request_id;
                            match load_settings(&profile_path) {
                                Ok(load) => {
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::SettingsConfig(
                                            Box::new(SettingsConfigResponse {
                                                settings: load.snapshot,
                                                source: load.source.as_str(),
                                                payload_bytes: load.payload_bytes,
                                            }),
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Settings][Metrics] source={} payload_bytes={} request_latency_ms={:.3}",
                                        load.source.as_str(),
                                        load.payload_bytes,
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(error) => {
                                    eprintln!("[Settings][settings_unavailable] {error}");
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: "SETTINGS_UNAVAILABLE",
                                            message: error,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::SettingsSetBlockPowerSaving(payload) => {
                            // Allowlisted mutation: the field is fixed at
                            // compile time and the value is a plain bool
                            // validated by the protocol parser. The settings
                            // path is built from the fixed host profile only.
                            let request_id = request.request_id;
                            match apply_block_power_saving(&profile_path, payload.value) {
                                Ok(mutation) => {
                                    let payload_bytes = mutation.payload_bytes;
                                    let applied = mutation.block_power_saving_in_voice_chat;
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::SettingsMutation(
                                            SettingsMutationResponse {
                                                block_power_saving_in_voice_chat: applied,
                                                source: mutation.source.as_str(),
                                                payload_bytes,
                                            },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Settings][Mutation] value={applied} source=file payload_bytes={payload_bytes} request_latency_ms={:.3}",
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(error) => {
                                    eprintln!("[Settings][settings_unavailable] {error}");
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: "SETTINGS_UNAVAILABLE",
                                            message: error,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::SettingsSet(payload) => {
                            // Closed allowlist mutation: the key is an enum
                            // and the value union is validated by the
                            // settings owner (re-checked here defensively).
                            // The settings path is built from the fixed host
                            // profile only.
                            let request_id = request.request_id;
                            if let Err(error) = settings::validate_settings_set(
                                payload.key,
                                &payload.value,
                            ) {
                                eprintln!("[Settings][invalid_set] {error}");
                                let response = BridgeResponse {
                                    version: PROTOCOL_VERSION,
                                    request_id: request_id.clone(),
                                    ok: false,
                                    payload: None,
                                    error: Some(BridgeError {
                                        code: "INVALID_SETTINGS_SET",
                                        message: error,
                                    }),
                                };
                                send_bridge_response(&webview, &response);
                                if let Err(error) = request_tracker.complete(&request_id) {
                                    eprintln!("[Bridge] correlation failure after response: {error}");
                                }
                                response_count += 1;
                                return;
                            }
                            match apply_settings_set(&profile_path, payload.key, &payload.value) {
                                Ok(mutation) => {
                                    let payload_bytes = mutation.payload_bytes;
                                    let key = mutation.key;
                                    let value = mutation.value.clone();
                                    // The Quick CSS toggle applies live: the
                                    // flag flip alone would otherwise wait for
                                    // the next trusted load, so re-push the
                                    // managed style into the running document
                                    // (enable injects, disable removes).
                                    if key == SettingsSetKey::QuickCss {
                                        repush_quick_css(
                                            &webview,
                                            &profile_path,
                                            &mut last_quickcss_injected,
                                            &mut last_quickcss_css,
                                        );
                                    }
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::SettingsSet(
                                            SettingsSetResponse {
                                                key,
                                                value,
                                                source: mutation.source.as_str(),
                                                payload_bytes,
                                            },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Settings][Set] key={} source=file payload_bytes={payload_bytes} request_latency_ms={:.3}",
                                        key.storage_key(),
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(error) => {
                                    eprintln!("[Settings][settings_unavailable] {error}");
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: "SETTINGS_UNAVAILABLE",
                                            message: error,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::SettingsImportElectron(_) => {
                            // Copy-only import from the Electron profile:
                            // the source path is fixed host-side
                            // (%APPDATA%\Remcord\storage\settings.json),
                            // the Electron file is read-only and never
                            // written, and only validated allowlisted keys
                            // merge into the native document. Runs once per
                            // explicit request — never at startup, never on
                            // a timer. A quickCss flip re-pushes the managed
                            // style live, like the settings.set path.
                            let request_id = request.request_id;
                            let outcome = std::env::var("APPDATA")
                                .map_err(|_| {
                                    settings_import::SettingsImportFailure {
                                        code: "ELECTRON_SETTINGS_NOT_FOUND",
                                        message: "the Electron app profile is unavailable".to_owned(),
                                    }
                                })
                                .and_then(|appdata| {
                                    settings_import::electron_settings_path_from_appdata(&appdata)
                                })
                                .and_then(|source| settings_import::read_electron_settings(&source))
                                .and_then(|document| {
                                    settings_import::import_electron_settings(&profile_path, &document)
                                });
                            match outcome {
                                Ok(summary) => {
                                    if summary.applied.iter().any(|key| key == "quickCss") {
                                        repush_quick_css(
                                            &webview,
                                            &profile_path,
                                            &mut last_quickcss_injected,
                                            &mut last_quickcss_css,
                                        );
                                    }
                                    let applied = summary.applied.len();
                                    let skipped = summary.skipped.len();
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::SettingsImported(
                                            SettingsImportedResponse {
                                                applied: summary.applied,
                                                skipped: summary.skipped,
                                            },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Settings][Import] applied={applied} skipped={skipped} request_latency_ms={:.3}",
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[Settings][{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::LocalizationGetLang(payload) => {
                            // The key shape was validated by the protocol
                            // parser; re-check defensively before lookup.
                            let request_id = request.request_id;
                            if let Err(error) = validate_lang_key(&payload.key) {
                                eprintln!("[Localization][invalid_key] {error}");
                                let response = BridgeResponse {
                                    version: PROTOCOL_VERSION,
                                    request_id: request_id.clone(),
                                    ok: false,
                                    payload: None,
                                    error: Some(BridgeError {
                                        code: "INVALID_LANG_KEY",
                                        message: error,
                                    }),
                                };
                                send_bridge_response(&webview, &response);
                                if let Err(error) = request_tracker.complete(&request_id) {
                                    eprintln!("[Bridge] correlation failure after response: {error}");
                                }
                                response_count += 1;
                                return;
                            }
                            let found = lookup_lang_string(&profile_path, &payload.key);
                            let payload_bytes = found.value.len();
                            let response = BridgeResponse {
                                version: PROTOCOL_VERSION,
                                request_id: request_id.clone(),
                                ok: true,
                                payload: Some(BridgeResponsePayload::LocalizationString(
                                    LocalizationStringResponse {
                                        value: found.value,
                                        lang: found.lang.clone(),
                                    },
                                )),
                                error: None,
                            };
                            send_bridge_response(&webview, &response);
                            if let Err(error) = request_tracker.complete(&request_id) {
                                eprintln!("[Bridge] correlation failure after response: {error}");
                            }
                            response_count += 1;
                            println!(
                                "[Localization][Metrics] lang={} fallback={} key_len={} payload_bytes={} request_latency_ms={:.3}",
                                found.lang,
                                found.from_fallback,
                                payload.key.len(),
                                payload_bytes,
                                received_at.elapsed().as_secs_f64() * 1000.0
                            );
                        }
                        BridgeCommand::AppGetVersion(_) => {
                            // Parameter-free read: the version is a
                            // compile-time constant, so there is no path to
                            // build and no file to be missing or corrupt.
                            let request_id = request.request_id;
                            let version = current_app_version();
                            let response = BridgeResponse {
                                version: PROTOCOL_VERSION,
                                request_id: request_id.clone(),
                                ok: true,
                                payload: Some(BridgeResponsePayload::AppVersion(
                                    AppVersionResponse {
                                        version: version.to_owned(),
                                    },
                                )),
                                error: None,
                            };
                            send_bridge_response(&webview, &response);
                            if let Err(error) = request_tracker.complete(&request_id) {
                                eprintln!("[Bridge] correlation failure after response: {error}");
                            }
                            response_count += 1;
                            println!(
                                "[App][Metrics] version={version} payload_bytes={} request_latency_ms={:.3}",
                                version.len(),
                                received_at.elapsed().as_secs_f64() * 1000.0
                            );
                        }
                        BridgeCommand::PluginStorageRead(payload) => {
                            // Scoped read: the id/path shape was checked by
                            // the protocol parser; strict validation,
                            // gate, and canonical containment run here.
                            // The host builds the root from the fixed
                            // profile directory only.
                            let request_id = request.request_id;
                            match read_plugin_file(
                                &profile_path,
                                &payload.plugin_id,
                                &payload.relative_path,
                            ) {
                                Ok(read) => {
                                    let payload_bytes = read.payload_bytes;
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(
                                            BridgeResponsePayload::PluginStorageRead(
                                                PluginStorageReadResponse {
                                                    data: read.data,
                                                    payload_bytes,
                                                },
                                            ),
                                        ),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[PluginStorage][Metrics] op=read payload_bytes={payload_bytes} request_latency_ms={:.3}",
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[PluginStorage][{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::PluginStorageWrite(payload) => {
                            // Scoped write: same addressing as the read plus
                            // bounded text data. Parent directories are
                            // created ONLY inside the validated namespace
                            // and the write is atomic (temp + rename).
                            let request_id = request.request_id;
                            match write_plugin_file(
                                &profile_path,
                                &payload.plugin_id,
                                &payload.relative_path,
                                &payload.data,
                            ) {
                                Ok(written) => {
                                    let payload_bytes = written.payload_bytes;
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(
                                            BridgeResponsePayload::PluginStorageWrite(
                                                PluginStorageWriteResponse { payload_bytes },
                                            ),
                                        ),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[PluginStorage][Metrics] op=write payload_bytes={payload_bytes} request_latency_ms={:.3}",
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[PluginStorage][{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::PluginsList(_) => {
                            // Read-only inventory: parameter-free by
                            // protocol. Fresh discovery plus enablement from
                            // the authoritative settings on every request, so
                            // manifest edits and new plugins apply without a
                            // restart. Display metadata only.
                            let request_id = request.request_id;
                            match plugins_list_compute(&profile_path) {
                                Ok(infos) => {
                                    let count = infos.len();
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::PluginsList(
                                            PluginsListResponse { plugins: infos },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Plugins][Metrics] op=list count={count} request_latency_ms={:.3}",
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[Plugins][{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::PluginsSetEnabled(payload) => {
                            // Enablement flip: logical id plus boolean only.
                            // Unknown ids and incompatible enables resolve
                            // `{ok:false}` with envelope-ok (Electron
                            // parity); the flip persists into the
                            // `pluginStates` settings map. Takes effect on
                            // the next document (restart-required model is
                            // NOT needed: the loader re-resolves per
                            // document, unlike the baked mod adapters).
                            let request_id = request.request_id;
                            match plugins_set_enabled_compute(
                                &profile_path,
                                &payload.plugin_id,
                                payload.enabled,
                            ) {
                                Ok(flipped) => {
                                    let ok = flipped.ok;
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(
                                            BridgeResponsePayload::PluginsSetEnabled(flipped),
                                        ),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Plugins][Metrics] op=set_enabled plugin='{}' ok={ok} request_latency_ms={:.3}",
                                        payload.plugin_id,
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[Plugins][{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::PluginsReload(payload) => {
                            // Re-validation: id only. The re-discovery IS
                            // the reload in the stateless design; the loader
                            // picks it up on the next document. Unknown or
                            // incompatible ids resolve `{ok:false}`.
                            let request_id = request.request_id;
                            match plugins_reload_compute(&profile_path, &payload.plugin_id) {
                                Ok(reloaded) => {
                                    let ok = reloaded.ok;
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::PluginsReload(
                                            reloaded,
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Plugins][Metrics] op=reload ok={ok} request_latency_ms={:.3}",
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[Plugins][{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::PluginsGetRuntimeEntries(payload) => {
                            // Loader inventory: closed target only. Logical
                            // `{id, name}` pairs for every enabled+compatible
                            // plugin declaring the target — never paths.
                            let request_id = request.request_id;
                            let target = payload.target;
                            match plugins_entries_compute(&profile_path, target) {
                                Ok(entries) => {
                                    let count = entries.len();
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(
                                            BridgeResponsePayload::PluginsRuntimeEntries(
                                                PluginsRuntimeEntriesResponse { target, entries },
                                            ),
                                        ),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Plugins][Metrics] op=entries target={} count={count} request_latency_ms={:.3}",
                                        target.as_str(),
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[Plugins][{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::PluginsGetRuntimeScript(payload) => {
                            // Loader source: logical id plus closed target.
                            // Absent entries resolve `{source:null}` (the
                            // loader skips falsy sources like Electron);
                            // hard failures are typed errors.
                            let request_id = request.request_id;
                            let target = payload.target;
                            match plugins_script_compute(
                                &profile_path,
                                &payload.plugin_id,
                                target,
                            ) {
                                Ok(source) => {
                                    let bytes = source.as_ref().map(|text| text.len());
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(
                                            BridgeResponsePayload::PluginsRuntimeScript(
                                                PluginsRuntimeScriptResponse {
                                                    plugin_id: payload.plugin_id,
                                                    target,
                                                    source,
                                                },
                                            ),
                                        ),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Plugins][Metrics] op=script target={} bytes={} request_latency_ms={:.3}",
                                        target.as_str(),
                                        bytes
                                            .map(|size| size.to_string())
                                            .unwrap_or_else(|| "none".to_owned()),
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[Plugins][{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::BundlesList(_) => {
                            // Read-only discovery: parameter-free by
                            // protocol, so the host inventories exactly the
                            // fixed allowlist under the profile root. No
                            // contents, paths, or URLs cross the boundary.
                            let request_id = request.request_id;
                            match describe_bundle_files(&profile_path) {
                                Ok(files) => {
                                    let present =
                                        files.iter().filter(|entry| entry.present).count();
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::BundlesList(
                                            BundlesListResponse {
                                                bundles: files
                                                    .into_iter()
                                                    .map(|entry| BundleFileEntry {
                                                        name: entry.name.to_owned(),
                                                        present: entry.present,
                                                        bytes: entry.bytes,
                                                    })
                                                    .collect(),
                                            },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Bundles][Metrics] files={} present={} request_latency_ms={:.3}",
                                        BUNDLE_FILES.len(),
                                        present,
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[Bundles][{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::FoldersOpen(payload) => {
                            // Reveal semantics (showItemInFolder equivalent):
                            // the location is a closed enum, so the host
                            // reveals exactly one fixed root. No path, URL,
                            // or selector crosses the boundary.
                            let request_id = request.request_id;
                            let location = payload.location;
                            let location_label = match location {
                                FolderLocation::Storage => "storage",
                                FolderLocation::Themes => "themes",
                                FolderLocation::Plugins => "plugins",
                                FolderLocation::Gallery => "gallery",
                            };
                            match ensure_folder_root(&profile_path, location)
                                .and_then(|dir| reveal_folder(&dir))
                            {
                                Ok(_) => {
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::FoldersOpened(
                                            FoldersOpenedResponse { location },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Folders][Metrics] op=reveal target={location_label} request_latency_ms={:.3}",
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[Folders][{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::FoldersOpenTheme(payload) => {
                            // Open semantics (openPath equivalent for a
                            // directory): Explorer opens inside the validated
                            // theme folder itself. The id is validated and
                            // the path is built plus contained by the host.
                            let request_id = request.request_id;
                            let theme_id = payload.id;
                            match resolve_theme_folder(&profile_path, &theme_id)
                                .and_then(|dir| open_folder(&dir))
                            {
                                Ok(_) => {
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(
                                            BridgeResponsePayload::ThemeFolderOpened(
                                                ThemeFolderOpenedResponse { id: theme_id },
                                            ),
                                        ),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Folders][Metrics] op=open target=theme request_latency_ms={:.3}",
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[Folders][{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::GalleryList(_) => {
                            // Newest-first items plus categories in
                            // Electron's `{items, categories}` shape. The
                            // host owns the root, the read, and the order.
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match gallery::list_gallery_items(&profile_path)
                                .and_then(|items| {
                                    gallery::list_gallery_categories(&profile_path)
                                        .map(|categories| (items, categories))
                                }) {
                                Ok((items, categories)) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryList(GalleryListResponse {
                                        items: items.iter().map(gallery::gallery_item_json).collect(),
                                        categories: categories
                                            .iter()
                                            .map(gallery::gallery_category_json)
                                            .collect(),
                                    }),
                                    "op=list",
                                    latency_ms,
                                ),
                                Err(failure) => respond_gallery_err(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    failure.code,
                                    failure.message,
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::GalleryAddUrl(payload) => {
                            // Save-by-URL: the host validates the scheme,
                            // derives kind/file/id, and downloads
                            // image/video bytes itself. Typed failures
                            // resolve `{ok: false, error}` (Electron parity)
                            // so the page can toast the reason.
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            let meta = gallery_meta_from_request(&payload.meta);
                            match gallery::add_gallery_url(&profile_path, &payload.url, &meta) {
                                Ok(item) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryAdd(GalleryAddResponse {
                                        ok: true,
                                        item: Some(gallery::gallery_item_json(&item)),
                                        error: None,
                                    }),
                                    "op=add_url",
                                    latency_ms,
                                ),
                                Err(failure) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryAdd(GalleryAddResponse {
                                        ok: false,
                                        item: None,
                                        error: Some(failure.code.to_owned()),
                                    }),
                                    "op=add_url",
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::GalleryAddLink(payload) => {
                            // Metadata-only save; no download. Same
                            // envelope as `gallery.add_url`.
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            let meta = gallery_meta_from_request(&payload.meta);
                            match gallery::add_gallery_link(&profile_path, &payload.url, &meta) {
                                Ok(item) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryAdd(GalleryAddResponse {
                                        ok: true,
                                        item: Some(gallery::gallery_item_json(&item)),
                                        error: None,
                                    }),
                                    "op=add_link",
                                    latency_ms,
                                ),
                                Err(failure) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryAdd(GalleryAddResponse {
                                        ok: false,
                                        item: None,
                                        error: Some(failure.code.to_owned()),
                                    }),
                                    "op=add_link",
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::GalleryRemove(payload) => {
                            // Single removal; unknown ids resolve
                            // `{ok: false}` (Electron parity). The cached
                            // file dies only inside the gallery root.
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match gallery::remove_gallery_item(&profile_path, &payload.id) {
                                Ok(removed) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryRemove(GalleryRemoveResponse {
                                        ok: removed,
                                    }),
                                    "op=remove",
                                    latency_ms,
                                ),
                                Err(failure) => respond_gallery_err(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    failure.code,
                                    failure.message,
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::GalleryBulkRemove(payload) => {
                            // At most 2000 ids (Electron slice parity);
                            // invalid ids are skipped, never dispatched.
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match gallery::bulk_remove_gallery_items(&profile_path, &payload.ids) {
                                Ok(removed) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryBulkRemove(
                                        GalleryBulkRemoveResponse { removed },
                                    ),
                                    "op=bulk_remove",
                                    latency_ms,
                                ),
                                Err(failure) => respond_gallery_err(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    failure.code,
                                    failure.message,
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::GalleryUpdate(payload) => {
                            // Title/tag patch only; unknown ids resolve
                            // `{ok: false, error}` (Electron parity).
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match gallery::update_gallery_item(
                                &profile_path,
                                &payload.id,
                                payload.title.as_deref(),
                                payload.categories.as_deref(),
                            ) {
                                Ok(item) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryUpdate(GalleryUpdateResponse {
                                        ok: true,
                                        item: Some(gallery::gallery_item_json(&item)),
                                        error: None,
                                    }),
                                    "op=update",
                                    latency_ms,
                                ),
                                Err(failure) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryUpdate(GalleryUpdateResponse {
                                        ok: false,
                                        item: None,
                                        error: Some(failure.code.to_owned()),
                                    }),
                                    "op=update",
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::GalleryCategories(_) => {
                            // Alphabetical display metadata only.
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match gallery::list_gallery_categories(&profile_path) {
                                Ok(categories) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryCategories(
                                        GalleryCategoriesResponse {
                                            categories: categories
                                                .iter()
                                                .map(gallery::gallery_category_json)
                                                .collect(),
                                        },
                                    ),
                                    "op=categories",
                                    latency_ms,
                                ),
                                Err(failure) => respond_gallery_err(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    failure.code,
                                    failure.message,
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::GalleryCategoryAdd(payload) => {
                            // Typed failures resolve `{ok: false, error}`
                            // (Electron parity).
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match gallery::add_gallery_category(&profile_path, &payload.name) {
                                Ok(category) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryCategoryAdd(
                                        GalleryCategoryAddResponse {
                                            ok: true,
                                            category: Some(gallery::gallery_category_json(
                                                &category,
                                            )),
                                            error: None,
                                        },
                                    ),
                                    "op=category_add",
                                    latency_ms,
                                ),
                                Err(failure) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryCategoryAdd(
                                        GalleryCategoryAddResponse {
                                            ok: false,
                                            category: None,
                                            error: Some(failure.code.to_owned()),
                                        },
                                    ),
                                    "op=category_add",
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::GalleryCategoryRename(payload) => {
                            // Retags items host-side; typed failures
                            // resolve `{ok: false, error}` (Electron parity).
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match gallery::rename_gallery_category(
                                &profile_path,
                                &payload.old_name,
                                &payload.new_name,
                            ) {
                                Ok(_) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryCategoryRename(
                                        GalleryCategoryRenameResponse { ok: true, error: None },
                                    ),
                                    "op=category_rename",
                                    latency_ms,
                                ),
                                Err(failure) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryCategoryRename(
                                        GalleryCategoryRenameResponse {
                                            ok: false,
                                            error: Some(failure.code.to_owned()),
                                        },
                                    ),
                                    "op=category_rename",
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::GalleryCategoryRemove(payload) => {
                            // Untags items host-side; always `{ok: true}`
                            // once the index persists (Electron parity).
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match gallery::remove_gallery_category(&profile_path, &payload.name) {
                                Ok(_) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryCategoryRemove(
                                        GalleryCategoryRemoveResponse { ok: true },
                                    ),
                                    "op=category_remove",
                                    latency_ms,
                                ),
                                Err(failure) => respond_gallery_err(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    failure.code,
                                    failure.message,
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::GalleryUsage(_) => {
                            // Exact bytes/files/items measured from the
                            // filesystem by the host.
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match gallery::gallery_disk_usage(&profile_path) {
                                Ok((bytes, files, items)) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryUsage(GalleryUsageResponse {
                                        bytes,
                                        files,
                                        items,
                                    }),
                                    "op=usage",
                                    latency_ms,
                                ),
                                Err(failure) => respond_gallery_err(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    failure.code,
                                    failure.message,
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::GalleryFile(payload) => {
                            // Inline base64 preview bounded by the 48 MiB
                            // transport cap; larger files play via their
                            // remote `sourceUrl` (Electron parity). Typed
                            // failures resolve `{ok: false, error}` so the
                            // thumbnail can fall back.
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match gallery::read_gallery_file(&profile_path, &payload.id) {
                                Ok((base64, mime, file_name)) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryFile(GalleryFileResponse {
                                        ok: true,
                                        base64: Some(base64),
                                        mime: Some(mime),
                                        file_name: Some(file_name),
                                        error: None,
                                    }),
                                    "op=file",
                                    latency_ms,
                                ),
                                Err(failure) => respond_gallery_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::GalleryFile(GalleryFileResponse {
                                        ok: false,
                                        base64: None,
                                        mime: None,
                                        file_name: None,
                                        error: Some(failure.code.to_owned()),
                                    }),
                                    "op=file",
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::GalleryRoot(_) => {
                            // Fixed profile-relative label only; an
                            // absolute path never crosses (deliberate
                            // difference: Electron reports the absolute
                            // custom root, native fixes the root).
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            respond_gallery_ok(
                                &webview,
                                &mut request_tracker,
                                &mut response_count,
                                request_id,
                                BridgeResponsePayload::GalleryRoot(GalleryRootResponse {
                                    root: gallery::GALLERY_ROOT_LABEL.to_owned(),
                                }),
                                "op=root",
                                latency_ms,
                            );
                        }
                        BridgeCommand::KeybindsList(_) => {
                            // Validated inventory, metadata only. Entries
                            // that cannot validate (including
                            // `runJavascript` survivors) are already
                            // excluded by the keybind owner.
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match keybinds::list_keybinds(&profile_path) {
                                Ok(entries) => respond_keybinds_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::KeybindsList(KeybindsListResponse {
                                        keybinds: entries
                                            .iter()
                                            .map(|entry| {
                                                serde_json::json!({
                                                    "id": entry.id,
                                                    "accelerator": entry.accelerator,
                                                    "action": entry.action.as_str(),
                                                    "global": entry.global,
                                                    "enabled": entry.enabled,
                                                })
                                            })
                                            .collect(),
                                    }),
                                    "op=list",
                                    latency_ms,
                                ),
                                Err(failure) => respond_keybinds_err(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    failure.code,
                                    failure.message,
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::KeybindsAdd(payload) => {
                            // Validated save; `runJavascript` and `js`
                            // members fail validation (never stored, never
                            // executed). Success re-registers the global
                            // set, mirroring refreshGlobalKeybinds.
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match keybinds::add_keybind(
                                &profile_path,
                                &payload.id,
                                &payload.accelerator,
                                &payload.action,
                                payload.global,
                                payload.enabled,
                            ) {
                                Ok(entry) => {
                                    sync_hotkey_worker(&keybind_hotkey_tx, &profile_path);
                                    respond_keybinds_ok(
                                        &webview,
                                        &mut request_tracker,
                                        &mut response_count,
                                        request_id,
                                        BridgeResponsePayload::KeybindsAdd(KeybindsAddResponse {
                                            ok: true,
                                            id: Some(entry.id),
                                            error: None,
                                        }),
                                        "op=add",
                                        latency_ms,
                                    );
                                }
                                Err(failure) => respond_keybinds_ok(
                                    &webview,
                                    &mut request_tracker,
                                    &mut response_count,
                                    request_id,
                                    BridgeResponsePayload::KeybindsAdd(KeybindsAddResponse {
                                        ok: false,
                                        id: None,
                                        error: Some(failure.code.to_owned()),
                                    }),
                                    "op=add",
                                    latency_ms,
                                ),
                            }
                        }
                        BridgeCommand::KeybindsToggle(payload) => {
                            // Flip + re-register; unknown ids resolve
                            // `{ok: false}` without writing.
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match keybinds::toggle_keybind(&profile_path, &payload.id) {
                                Ok(enabled) => {
                                    sync_hotkey_worker(&keybind_hotkey_tx, &profile_path);
                                    respond_keybinds_ok(
                                        &webview,
                                        &mut request_tracker,
                                        &mut response_count,
                                        request_id,
                                        BridgeResponsePayload::KeybindsToggle(
                                            KeybindsToggleResponse {
                                                ok: true,
                                                enabled: Some(enabled),
                                            },
                                        ),
                                        "op=toggle",
                                        latency_ms,
                                    );
                                }
                                Err(failure) => {
                                    if failure.code == "NOT_FOUND" {
                                        respond_keybinds_ok(
                                            &webview,
                                            &mut request_tracker,
                                            &mut response_count,
                                            request_id,
                                            BridgeResponsePayload::KeybindsToggle(
                                                KeybindsToggleResponse { ok: false, enabled: None },
                                            ),
                                            "op=toggle",
                                            latency_ms,
                                        );
                                    } else {
                                        respond_keybinds_err(
                                            &webview,
                                            &mut request_tracker,
                                            &mut response_count,
                                            request_id,
                                            failure.code,
                                            failure.message,
                                            latency_ms,
                                        );
                                    }
                                }
                            }
                        }
                        BridgeCommand::KeybindsRemove(payload) => {
                            // Remove + re-register; unknown ids resolve
                            // `{ok: false}` without writing.
                            let request_id = request.request_id;
                            let latency_ms = received_at.elapsed().as_secs_f64() * 1000.0;
                            match keybinds::remove_keybind(&profile_path, &payload.id) {
                                Ok(_) => {
                                    sync_hotkey_worker(&keybind_hotkey_tx, &profile_path);
                                    respond_keybinds_ok(
                                        &webview,
                                        &mut request_tracker,
                                        &mut response_count,
                                        request_id,
                                        BridgeResponsePayload::KeybindsRemove(
                                            KeybindsRemoveResponse { ok: true },
                                        ),
                                        "op=remove",
                                        latency_ms,
                                    );
                                }
                                Err(failure) => {
                                    if failure.code == "NOT_FOUND" {
                                        respond_keybinds_ok(
                                            &webview,
                                            &mut request_tracker,
                                            &mut response_count,
                                            request_id,
                                            BridgeResponsePayload::KeybindsRemove(
                                                KeybindsRemoveResponse { ok: false },
                                            ),
                                            "op=remove",
                                            latency_ms,
                                        );
                                    } else {
                                        respond_keybinds_err(
                                            &webview,
                                            &mut request_tracker,
                                            &mut response_count,
                                            request_id,
                                            failure.code,
                                            failure.message,
                                            latency_ms,
                                        );
                                    }
                                }
                            }
                        }
                        BridgeCommand::ThemeInstallCode(payload) => {
                            // Install from CSS text only: the id is derived
                            // from `@name`, the filename is fixed, and the
                            // theme stays disabled (Electron parity). The
                            // existing live watcher observes the written
                            // files and drives the managed CSS lifecycle.
                            let request_id = request.request_id;
                            match install_theme_from_code(&profile_path, &payload.css) {
                                Ok(installed) => {
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::ThemeInstalled(
                                            ThemeInstalledResponse {
                                                id: installed.id.clone(),
                                                enabled: installed.enabled,
                                                css_bytes: installed.css_bytes,
                                            },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Themes][Install] id={} enabled={} css_bytes={} request_latency_ms={:.3}",
                                        installed.id,
                                        installed.enabled,
                                        installed.css_bytes,
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[Themes][Install:{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::ThemeWriteCss(payload) => {
                            // Replace one theme's CSS text: the destination
                            // is the manifest-declared file resolved by the
                            // host, and the manifest itself is never
                            // touched. The existing live watcher observes
                            // the replacement and drives the managed CSS
                            // lifecycle.
                            let request_id = request.request_id;
                            match write_theme_css(&profile_path, &payload.id, &payload.css) {
                                Ok(written) => {
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::ThemeWritten(
                                            ThemeWrittenResponse {
                                                id: written.id.clone(),
                                                enabled: written.enabled,
                                                css_bytes: written.css_bytes,
                                            },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Themes][Write] id={} enabled={} css_bytes={} request_latency_ms={:.3}",
                                        written.id,
                                        written.enabled,
                                        written.css_bytes,
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[Themes][Write:{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::ThemeImportFiles(_) => {
                            // Host-owned picker: the request carries no
                            // parameters by protocol. The modal dialog plus
                            // the sequential bounded ingestion run on the
                            // dedicated STA worker; the typed summary comes
                            // back as ThemeImportReady on the event loop, so
                            // the WebView/UI thread never blocks. The
                            // renderer waits with the long-operation timeout
                            // (picker stays open as long as the user needs).
                            let import_request_id = request.request_id.clone();
                            let job = ThemeImportJob {
                                request_id: import_request_id.clone(),
                                received_at,
                                profile: profile_path.clone(),
                                proxy: proxy.clone(),
                            };
                            match import_tx.send(job) {
                                Ok(()) => {
                                    println!("[Themes][Import] picker request accepted");
                                }
                                Err(_) => {
                                    eprintln!("[Themes][Import:IMPORT_UNAVAILABLE] picker worker is unavailable");
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: import_request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: "IMPORT_UNAVAILABLE",
                                            message: "theme import is unavailable".to_owned(),
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) =
                                        request_tracker.complete(&import_request_id)
                                    {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::ThemeSetEnabled(payload) => {
                            // Flip one theme's flag: the id is validated
                            // (never sanitized), the manifest path is
                            // host-derived, and only `enabled` changes. The
                            // existing live watcher observes the rewrite and
                            // drives the managed CSS lifecycle; nothing here
                            // touches styles directly.
                            let request_id = request.request_id;
                            match set_theme_enabled(&profile_path, &payload.id, payload.enabled) {
                                Ok(flipped) => {
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::ThemeEnabled(
                                            ThemeEnabledResponse {
                                                id: flipped.id.clone(),
                                                enabled: flipped.enabled,
                                            },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Themes][SetEnabled] id={} enabled={} request_latency_ms={:.3}",
                                        flipped.id,
                                        flipped.enabled,
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[Themes][SetEnabled:{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::ThemesList(_) => {
                            // Read-only inventory: the request carries no
                            // parameters by protocol. The host owns the root,
                            // enumeration, ordering, and bounds; the renderer
                            // receives bounded display metadata only.
                            let request_id = request.request_id;
                            let report = list_themes(&profile_path);
                            for skipped in &report.skipped {
                                println!(
                                    "[Themes][ListSkip] id={} reason={}",
                                    skipped.id, skipped.reason
                                );
                            }
                            let response = BridgeResponse {
                                version: PROTOCOL_VERSION,
                                request_id: request_id.clone(),
                                ok: true,
                                payload: Some(BridgeResponsePayload::ThemesList(
                                    ThemesListResponse {
                                        themes: report.listed.clone(),
                                    },
                                )),
                                error: None,
                            };
                            send_bridge_response(&webview, &response);
                            if let Err(error) = request_tracker.complete(&request_id) {
                                eprintln!("[Bridge] correlation failure after response: {error}");
                            }
                            response_count += 1;
                            println!(
                                "[Themes][List] listed={} skipped={} request_latency_ms={:.3}",
                                report.listed.len(),
                                report.skipped.len(),
                                received_at.elapsed().as_secs_f64() * 1000.0
                            );
                        }
                        BridgeCommand::QuickCssRead(_) => {
                            // Editor read: the request carries no parameters
                            // by protocol. The host reads its own fixed file
                            // only; the renderer receives the text plus its
                            // byte length.
                            let request_id = request.request_id;
                            match read_quick_css_text(&profile_path) {
                                Ok((css, bytes)) => {
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::QuickCssRead(
                                            QuickCssReadResponse { css, bytes },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[QuickCss][Read] bytes={} request_latency_ms={:.3}",
                                        bytes,
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(message) => {
                                    eprintln!("[QuickCss][Read:QUICKCSS_UNAVAILABLE] {message}");
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: "QUICKCSS_UNAVAILABLE",
                                            message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::QuickCssWrite(payload) => {
                            // Editor save: exactly one bounded CSS text; the
                            // destination is always the host-fixed file,
                            // persisted atomically. The shared re-push
                            // applies the save to the running document, so no
                            // reload or restart is needed.
                            let request_id = request.request_id;
                            match write_quick_css_text(&profile_path, &payload.css) {
                                Ok(bytes) => {
                                    repush_quick_css(
                                        &webview,
                                        &profile_path,
                                        &mut last_quickcss_injected,
                                        &mut last_quickcss_css,
                                    );
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::QuickCssWritten(
                                            QuickCssWrittenResponse { bytes },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[QuickCss][Write] bytes={} request_latency_ms={:.3}",
                                        bytes,
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(message) => {
                                    eprintln!("[QuickCss][Write:QUICKCSS_UNAVAILABLE] {message}");
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: "QUICKCSS_UNAVAILABLE",
                                            message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::DiagnosticsGetInfo(_) => {
                            // Read-only facts for the debug/GPU copy actions:
                            // compile-time app version, boot-time Runtime
                            // version, profile-kind label (never a path),
                            // validated channel/csp modes, and ValidMods-only
                            // ids from the authoritative snapshot. A corrupt
                            // snapshot fails closed like every other reader.
                            let request_id = request.request_id;
                            match load_settings(&profile_path) {
                                Ok(loaded) => {
                                    let info = diagnostics::collect_debug_info(
                                        current_app_version(),
                                        runtime_version.as_deref(),
                                        resolved_profile.kind.as_str(),
                                        &loaded.snapshot.channel,
                                        &loaded.snapshot.csp,
                                        &loaded.snapshot.mods,
                                    );
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::DiagnosticsInfo(
                                            DiagnosticsInfoResponse {
                                                app_version: info.app_version,
                                                webview2_runtime: info.webview2_runtime,
                                                profile_kind: info.profile_kind,
                                                channel: info.channel,
                                                csp: info.csp,
                                                mods: info.mods,
                                            },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Diagnostics][Metrics] request_latency_ms={:.3}",
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(error) => {
                                    eprintln!("[Diagnostics][DIAGNOSTICS_UNAVAILABLE] {error}");
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: "DIAGNOSTICS_UNAVAILABLE",
                                            message: error,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::GraphicsGetInfo(_) => {
                            // CDP-backed graphics report: the two fixed probes
                            // (`System.getInfo`, `Browser.getVersion`) are
                            // issued on this thread and each completion
                            // forwards one `GraphicsCdpPart` event; the report
                            // goes out only when both parts arrive. The UI
                            // thread never blocks, and sections the runtime
                            // withholds report as `unsupported`/`unavailable`
                            // — never invented.
                            let request_id = request.request_id;
                            let runtime = runtime_version.clone();
                            let browser_args =
                                crate::remcord_protocol::scheme::ADDITIONAL_BROWSER_ARGUMENTS;
                            #[cfg(target_os = "windows")]
                            {
                                if pending_graphics.len() >= graphics::MAX_PENDING_GRAPHICS {
                                    eprintln!(
                                        "[Graphics][GRAPHICS_BUSY] too many in-flight probes"
                                    );
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: "GRAPHICS_BUSY",
                                            message: "too many in-flight graphics probes"
                                                .to_owned(),
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                } else {
                                    let core = webview.webview();
                                    let (system_issued, browser_issued) =
                                        graphics::issue_cdp_calls(&core, &proxy, &request_id);
                                    let pending = graphics::PendingGraphics::new(
                                        runtime,
                                        browser_args,
                                        if system_issued {
                                            None
                                        } else {
                                            Some(Err("the System.getInfo probe could not start"
                                                .to_owned()))
                                        },
                                        if browser_issued {
                                            None
                                        } else {
                                            Some(Err(
                                                "the Browser.getVersion probe could not start"
                                                    .to_owned(),
                                            ))
                                        },
                                    );
                                    if pending.is_complete() {
                                        let report = pending.build_response();
                                        let response = BridgeResponse {
                                            version: PROTOCOL_VERSION,
                                            request_id: request_id.clone(),
                                            ok: true,
                                            payload: Some(
                                                BridgeResponsePayload::GraphicsInfo(Box::new(report)),
                                            ),
                                            error: None,
                                        };
                                        send_bridge_response(&webview, &response);
                                        if let Err(error) =
                                            request_tracker.complete(&request_id)
                                        {
                                            eprintln!("[Bridge] correlation failure after response: {error}");
                                        }
                                        response_count += 1;
                                        println!(
                                            "[Graphics][Metrics] probes_refused_at_issue request_latency_ms={:.3}",
                                            received_at.elapsed().as_secs_f64() * 1000.0
                                        );
                                    } else {
                                        pending_graphics.insert(request_id, pending);
                                    }
                                }
                            }
                            #[cfg(not(target_os = "windows"))]
                            {
                                // No WebView2 shell exists here: answer with
                                // both sections unavailable, never invented.
                                let unavailable = "the WebView2 graphics probe is unavailable on this platform".to_owned();
                                let report = graphics::assemble_report(
                                    None,
                                    browser_args,
                                    &Err(unavailable.clone()),
                                    &Err(unavailable),
                                );
                                let response = BridgeResponse {
                                    version: PROTOCOL_VERSION,
                                    request_id: request_id.clone(),
                                    ok: true,
                                    payload: Some(BridgeResponsePayload::GraphicsInfo(Box::new(report))),
                                    error: None,
                                };
                                send_bridge_response(&webview, &response);
                                if let Err(error) = request_tracker.complete(&request_id) {
                                    eprintln!("[Bridge] correlation failure after response: {error}");
                                }
                                response_count += 1;
                            }
                        }
                        BridgeCommand::ModClearCache(_) => {
                            // Client-mod cache wipe: the five host-managed
                            // downloaded bundle caches plus the stored
                            // `modCache` map reset (Electron parity for the
                            // map). User-owned custom bundles are never
                            // touched; links are skipped, never followed.
                            let request_id = request.request_id;
                            match mod_cache::clear_mod_cache(&profile_path) {
                                Ok(cleared) => {
                                    let cleared_count = cleared.cleared.len();
                                    let mod_cache_cleared = cleared.mod_cache_cleared;
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::ModCacheCleared(
                                            ModCacheClearedResponse {
                                                cleared: cleared.cleared,
                                                mod_cache_cleared,
                                            },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[ModCache][Metrics] cleared={cleared_count} mod_cache_cleared={mod_cache_cleared} request_latency_ms={:.3}",
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(failure) => {
                                    eprintln!("[ModCache][{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::SpellcheckGetState(_) => {
                            // Read-only validated spellcheck state (F18):
                            // the closed enabled flag plus the normalized
                            // language list and primary tag from the
                            // authoritative snapshot (Electron
                            // `webPreferences.spellcheck` plus the session
                            // language list parity). A corrupt
                            // snapshot fails closed like every other reader.
                            let request_id = request.request_id;
                            match load_settings(&profile_path) {
                                Ok(loaded) => {
                                    let state = spellcheck::resolve_state(
                                        loaded.snapshot.spellcheck,
                                        &loaded.snapshot.spellcheck_language,
                                    );
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: true,
                                        payload: Some(BridgeResponsePayload::SpellcheckState(
                                            SpellcheckStateResponse {
                                                enabled: state.enabled,
                                                languages: state.languages,
                                                primary_language: state.primary_language,
                                            },
                                        )),
                                        error: None,
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                    println!(
                                        "[Spellcheck][Metrics] request_latency_ms={:.3}",
                                        received_at.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Err(error) => {
                                    eprintln!("[Spellcheck][SPELLCHECK_UNAVAILABLE] {error}");
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: "SPELLCHECK_UNAVAILABLE",
                                            message: error,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) = request_tracker.complete(&request_id) {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::BackupSave(payload) => {
                            // Export only: the renderer supplies the include
                            // flags plus the bounded clientMods envelope (no
                            // path, filename, or dialog option). The payload
                            // is validated through the single typed owner
                            // (`validate_backup_request`, the same function
                            // the unit tests cover); collection, ZIP
                            // creation, the modal dialog, and the final write
                            // run on the dedicated STA worker, and the bounded
                            // outcome comes back as BackupSaveReady. The
                            // renderer waits with the long-operation timeout
                            // (the picker stays open as long as the user
                            // needs).
                            let save_request_id = request.request_id.clone();
                            let validated = serde_json::to_string(&payload)
                                .map_err(|_| BackupFailure {
                                    code: "INVALID_BACKUP",
                                    message: "backup request is invalid".to_owned(),
                                })
                                .and_then(|raw| validate_backup_request(&raw));
                            match validated {
                                Ok(validated) => {
                                    let job = BackupSaveJob {
                                        request_id: save_request_id.clone(),
                                        received_at,
                                        profile: profile_path.clone(),
                                        includes: validated.includes,
                                        client_mods_manifest: validated.client_mods_manifest,
                                        proxy: proxy.clone(),
                                    };
                                    match backup_tx.send(job) {
                                        Ok(()) => {
                                            println!("[Backup][Save] save request accepted");
                                        }
                                        Err(_) => {
                                            eprintln!("[Backup][Save:BACKUP_UNAVAILABLE] save worker is unavailable");
                                            let response = BridgeResponse {
                                                version: PROTOCOL_VERSION,
                                                request_id: save_request_id.clone(),
                                                ok: false,
                                                payload: None,
                                                error: Some(BridgeError {
                                                    code: "BACKUP_UNAVAILABLE",
                                                    message: "backup save is unavailable".to_owned(),
                                                }),
                                            };
                                            send_bridge_response(&webview, &response);
                                            if let Err(error) =
                                                request_tracker.complete(&save_request_id)
                                            {
                                                eprintln!("[Bridge] correlation failure after response: {error}");
                                            }
                                            response_count += 1;
                                        }
                                    }
                                }
                                Err(failure) => {
                                    eprintln!("[Backup][Save:{}] {}", failure.code, failure.message);
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: save_request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: failure.code,
                                            message: failure.message,
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) =
                                        request_tracker.complete(&save_request_id)
                                    {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
                        }
                        BridgeCommand::BackupInspect(_) => {
                            // Preview only: the renderer supplies NOTHING
                            // (the empty payload is enforced by
                            // `deny_unknown_fields` at parse). The modal
                            // open dialog, the bounded archive read, full
                            // validation, and staging run on the dedicated
                            // STA restore worker; the staged bytes are HELD
                            // worker-side and only the preview crosses back
                            // as BackupInspectReady. The renderer waits with
                            // the long-operation timeout (the picker stays
                            // open as long as the user needs).
                            send_backup_restore_job(
                                &restore_tx,
                                &webview,
                                &mut request_tracker,
                                &mut response_count,
                                BackupRestoreKind::Inspect,
                                request.request_id.clone(),
                                received_at,
                                &profile_path,
                                &proxy,
                                "inspect",
                            );
                        }
                        BridgeCommand::BackupCommitRestore(_) => {
                            // Commit the held staged restore: no parameters
                            // by protocol, so the commit applies exactly the
                            // bytes the preview described. Runs on the same
                            // worker (which owns the hold) and comes back as
                            // BackupRestoreReady.
                            send_backup_restore_job(
                                &restore_tx,
                                &webview,
                                &mut request_tracker,
                                &mut response_count,
                                BackupRestoreKind::Commit,
                                request.request_id.clone(),
                                received_at,
                                &profile_path,
                                &proxy,
                                "commit",
                            );
                        }
                        BridgeCommand::BackupDiscardRestore(_) => {
                            // Drop the held staged restore without touching
                            // the profile. Same worker, BackupDiscardReady.
                            send_backup_restore_job(
                                &restore_tx,
                                &webview,
                                &mut request_tracker,
                                &mut response_count,
                                BackupRestoreKind::Discard,
                                request.request_id.clone(),
                                received_at,
                                &profile_path,
                                &proxy,
                                "discard",
                            );
                        }
                        BridgeCommand::ModGetBundle(payload) => {                            let request_id = request.request_id;
                            if let Err(error) = validate_mod_bundle_id(&payload.mod_id) {                                eprintln!("[Mod:Registry][invalid_mod_id] {error}");
                                let response = BridgeResponse {
                                    version: PROTOCOL_VERSION,
                                    request_id: request_id.clone(),
                                    ok: false,
                                    payload: None,
                                    error: Some(BridgeError {
                                        code: "INVALID_MOD_ID",
                                        message: error,
                                    }),
                                };
                                send_bridge_response(&webview, &response);
                                if let Err(error) = request_tracker.complete(&request_id) {
                                    eprintln!("[Bridge] correlation failure after response: {error}");
                                }
                                response_count += 1;
                                return;
                            }
                            let Some(descriptor) = lookup_mod_descriptor(&payload.mod_id) else {
                                eprintln!(
                                    "[Mod:Registry][unknown_mod] mod id '{}' is not registered in the native host",
                                    payload.mod_id
                                );
                                let response = BridgeResponse {
                                    version: PROTOCOL_VERSION,
                                    request_id: request_id.clone(),
                                    ok: false,
                                    payload: None,
                                    error: Some(BridgeError {
                                        code: "UNKNOWN_MOD",
                                        message: format!(
                                            "mod '{}' is not registered in the native host",
                                            payload.mod_id
                                        ),
                                    }),
                                };
                                send_bridge_response(&webview, &response);
                                if let Err(error) = request_tracker.complete(&request_id) {
                                    eprintln!("[Bridge] correlation failure after response: {error}");
                                }
                                response_count += 1;
                                return;
                            };

                            mod_metrics.record_request(descriptor.id);
                            if !document_lifecycle.begin_mod(descriptor.id) {
                                mod_metrics.record_duplicate(descriptor.id);
                                send_mod_failure(
                                    &descriptor,
                                    &webview,
                                    &mut request_tracker,
                                    request_id.clone(),
                                    mod_stage::DUPLICATE_BOOTSTRAP,
                                    format!(
                                        "{} bootstrap was already requested for this document",
                                        descriptor.id
                                    ),
                                );
                                response_count += 1;
                                return;
                            }

                            let generation = document_lifecycle.generation();
                            // User-owned bundles bypass the in-memory cache
                            // and are re-read per request: the files are the
                            // user's live editor surface, so an edit applies
                            // on the next document without a restart.
                            // Download-published bundles are immutable per
                            // session and stay cached.
                            let is_user_owned = matches!(
                                descriptor.identity,
                                ModArtifactIdentity::UserOwned
                            );
                            let cached_bundle = if is_user_owned {
                                None
                            } else {
                                bundle_cache
                                    .lock()
                                    .ok()
                                    .and_then(|guard| guard.get(descriptor.id))
                            };
                            if let Some(bundle) = cached_bundle {
                                println!(
                                    "[{}][capability] request accepted from in-memory cache (document_generation={generation})",
                                    descriptor.log_tag
                                );
                                queue_event(
                                    &proxy,
                                    NativeEvent::ModBundleReady {
                                        mod_id: descriptor.id,
                                        generation,
                                        request_id,
                                        received_at,
                                        retrieval_ms: 0.0,
                                        result: Ok((*bundle).clone()),
                                    },
                                );
                            } else {
                                let worker_proxy = proxy.clone();
                                let profile = profile_path.clone();
                                let cache_ref = Arc::clone(&bundle_cache);
                                let worker_request_id = request_id.clone();
                                if let Err(error) = thread::Builder::new()
                                    .name(format!("remcord-{}-bundle", descriptor.id))
                                    .spawn(move || {
                                        let retrieval_started_at = Instant::now();
                                        let result = if is_user_owned {
                                            load_custom_bundle(&descriptor, profile)
                                        } else {
                                            load_mod_bundle(&descriptor, profile)
                                        };
                                        if let Ok(ref bundle) = result {
                                            if !is_user_owned {
                                                if let Ok(mut guard) = cache_ref.lock() {
                                                    guard.insert(descriptor.id, bundle.clone());
                                                }
                                            }
                                        }
                                        let retrieval_ms =
                                            retrieval_started_at.elapsed().as_secs_f64() * 1000.0;
                                        queue_event(
                                            &worker_proxy,
                                            NativeEvent::ModBundleReady {
                                                mod_id: descriptor.id,
                                                generation,
                                                request_id: worker_request_id,
                                                received_at,
                                                retrieval_ms,
                                                result,
                                            },
                                        );
                                    })
                                {
                                    send_mod_failure(
                                        &descriptor,
                                        &webview,
                                        &mut request_tracker,
                                        request_id,
                                        mod_stage::BUNDLE_RETRIEVAL,
                                        format!("Could not start {} bundle retrieval: {error}", descriptor.id),
                                    );
                                    response_count += 1;
                                } else {
                                    println!(
                                        "[{}][capability] request accepted (document_generation={generation})",
                                        descriptor.log_tag
                                    );
                                }
                            }
                        },
                    },
                }
            },
            Event::UserEvent(NativeEvent::GraphicsCdpPart {
                request_id,
                method,
                outcome,
            }) => {
                // One fixed CDP probe completed: record its bounded outcome
                // and answer only when both parts arrived. Unknown ids are
                // late renderer timeouts — logged, never answered twice.
                let Some(mut pending) = pending_graphics.remove(&request_id) else {
                    eprintln!("[Graphics] late CDP part for unknown request");
                    return;
                };
                pending.record(method, outcome);
                if !pending.is_complete() {
                    pending_graphics.insert(request_id, pending);
                    return;
                }
                let system_status = pending.system_status();
                let browser_status = pending.browser_status();
                let elapsed_ms = pending.elapsed_ms();
                let report = pending.build_response();
                let system_error = report.system.error.clone().unwrap_or_default();
                let browser_error = report.browser.error.clone().unwrap_or_default();
                let response = BridgeResponse {
                    version: PROTOCOL_VERSION,
                    request_id: request_id.clone(),
                    ok: true,
                    payload: Some(BridgeResponsePayload::GraphicsInfo(Box::new(report))),
                    error: None,
                };
                send_bridge_response(&webview, &response);
                if let Err(error) = request_tracker.complete(&request_id) {
                    eprintln!("[Bridge] correlation failure after response: {error}");
                }
                response_count += 1;
                println!(
                    "[Graphics][Metrics] system={system_status} browser={browser_status} request_latency_ms={elapsed_ms:.3} system_error={system_error:?} browser_error={browser_error:?}"
                );
            }
            Event::UserEvent(NativeEvent::ModBundleReady {
                mod_id,
                generation,
                request_id,
                received_at,
                retrieval_ms,
                result,
            }) => {
                let Some(descriptor) = lookup_mod_descriptor(mod_id) else {
                    eprintln!("[Mod:{mod_id}][lifecycle] bundle result for unknown mod");
                    return;
                };
                if generation != document_lifecycle.generation() {
                    println!(
                        "[{}][lifecycle] discarded stale bundle result (generation={generation})",
                        descriptor.log_tag
                    );
                    return;
                }
                let bundle = match result {
                    Ok(bundle) => bundle,
                    Err(error) => {
                        send_mod_failure(
                            &descriptor,
                            &webview,
                            &mut request_tracker,
                            request_id,
                            mod_stage::BUNDLE_RETRIEVAL,
                            error,
                        );
                        response_count += 1;
                        return;
                    }
                };
                println!(
                    "[{}][Metrics] bundle_version={} bundle_bytes={} css_bytes={} retrieval_ms={retrieval_ms:.3}",
                    descriptor.log_tag,
                    bundle.version,
                    bundle.js_bytes,
                    bundle
                        .css_bytes
                        .map(|bytes| bytes.to_string())
                        .unwrap_or_else(|| "none".to_owned())
                );

                let execution_started_at = Instant::now();
                let execution_script =
                    execution_script_for(&descriptor, &bundle.js, &shelter_injector_decl);
                if let Err(error) = webview.evaluate_script(&execution_script) {
                    send_mod_failure(
                        &descriptor,
                        &webview,
                        &mut request_tracker,
                        request_id,
                        mod_stage::SCRIPT_EXECUTION,
                        format!(
                            "WebView2 could not execute the {} bundle: {error}",
                            descriptor.id
                        ),
                    );
                    response_count += 1;
                    return;
                }
                let execution_ms = execution_started_at.elapsed().as_secs_f64() * 1000.0;
                pending_mods.insert(
                    request_id.clone(),
                    PendingModBootstrap {
                        mod_id: descriptor.id,
                        generation,
                        received_at,
                        retrieval_ms,
                        bundle_version: bundle.version,
                        bundle_bytes: bundle.js_bytes,
                        css_bytes: bundle.css_bytes,
                        execution_ms,
                        css_started_at: Instant::now(),
                        css_injection_ms: None,
                    },
                );
                println!(
                    "[{}][Metrics] bundle execution call completed in {execution_ms:.3} ms",
                    descriptor.log_tag
                );

                // The CSS stage exists only when the descriptor declares a
                // stylesheet. A mod that ships none (Shelter) never gets an
                // empty `<style>` and never waits for a CSS result: it goes
                // straight to the execution marker.
                match (&descriptor.css, &bundle.css) {
                    (Some(source), Some(css)) => {
                        let css_script = match mod_css_script(&descriptor, source, &request_id, css) {
                            Ok(script) => script,
                            Err(error) => {
                                pending_mods.remove(&request_id);
                                send_mod_failure(
                                    &descriptor,
                                    &webview,
                                    &mut request_tracker,
                                    request_id,
                                    mod_stage::ADAPTER_INITIALIZATION,
                                    format!("Could not prepare the {} CSS adapter: {error}", descriptor.id),
                                );
                                response_count += 1;
                                return;
                            }
                        };
                        if let Err(error) = webview.evaluate_script(&css_script) {
                            pending_mods.remove(&request_id);
                            send_mod_failure(
                                &descriptor,
                                &webview,
                                &mut request_tracker,
                                request_id,
                                mod_stage::CSS_INJECTION,
                                format!(
                                    "WebView2 could not run {} CSS injection: {error}",
                                    descriptor.id
                                ),
                            );
                            response_count += 1;
                        }
                    }
                    _ => {
                        let script = match mod_marker_script(&descriptor, &request_id, execution_ms) {
                            Ok(script) => script,
                            Err(error) => {
                                pending_mods.remove(&request_id);
                                send_mod_failure(
                                    &descriptor,
                                    &webview,
                                    &mut request_tracker,
                                    request_id,
                                    mod_stage::ADAPTER_INITIALIZATION,
                                    format!(
                                        "Could not prepare the {} execution marker: {error}",
                                        descriptor.id
                                    ),
                                );
                                response_count += 1;
                                return;
                            }
                        };
                        if let Err(error) = webview.evaluate_script(&script) {
                            pending_mods.remove(&request_id);
                            send_mod_failure(
                                &descriptor,
                                &webview,
                                &mut request_tracker,
                                request_id,
                                mod_stage::ADAPTER_INITIALIZATION,
                                format!(
                                    "WebView2 could not verify {} bootstrap execution: {error}",
                                    descriptor.id
                                ),
                            );
                            response_count += 1;
                        }
                    }
                }
            }
            Event::UserEvent(NativeEvent::KeybindFired { keybind_id }) => {
                // Global-hotkey firing from the worker thread: re-validate
                // (known, enabled, global-scope) and execute the stored
                // action. Anything else is logged and dropped — a stale
                // worker table can outlive a management mutation by one
                // sync, and ids never come from the renderer here either.
                fire_keybind(&webview, &profile_path, &keybind_id, true);
            }
            Event::UserEvent(NativeEvent::ThemeImportReady {
                request_id,
                received_at,
                result,
            }) => {
                // Cancellation is normal control flow: it resolves the
                // bounded canceled summary, never a bridge rejection, so no
                // error toast and no unhandled rejection can follow.
                match result {
                    Ok(summary) => {
                        let response = BridgeResponse {
                            version: PROTOCOL_VERSION,
                            request_id: request_id.clone(),
                            ok: true,
                            payload: Some(BridgeResponsePayload::ThemeImported(
                                ThemeImportedResponse {
                                    canceled: summary.canceled,
                                    imported: summary.imported,
                                    failed: summary.failed,
                                },
                            )),
                            error: None,
                        };
                        send_bridge_response(&webview, &response);
                        if let Err(error) = request_tracker.complete(&request_id) {
                            eprintln!("[Bridge] correlation failure after response: {error}");
                        }
                        response_count += 1;
                        println!(
                            "[Themes][Import] canceled={} imported={} failed={} request_latency_ms={:.3}",
                            summary.canceled,
                            summary.imported,
                            summary.failed,
                            received_at.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                    Err(failure) => {
                        eprintln!("[Themes][Import:{}] {}", failure.code, failure.message);
                        let response = BridgeResponse {
                            version: PROTOCOL_VERSION,
                            request_id: request_id.clone(),
                            ok: false,
                            payload: None,
                            error: Some(BridgeError {
                                code: failure.code,
                                message: failure.message,
                            }),
                        };
                        send_bridge_response(&webview, &response);
                        if let Err(error) = request_tracker.complete(&request_id) {
                            eprintln!("[Bridge] correlation failure after response: {error}");
                        }
                        response_count += 1;
                    }
                }
            }
            Event::UserEvent(NativeEvent::BackupSaveReady {
                request_id,
                received_at,
                result,
            }) => {
                // Cancellation is normal control flow: it resolves the
                // bounded canceled outcome, never a bridge rejection, so the
                // existing backup UI shows its silent cancelled toast and no
                // unhandled rejection can follow. The response carries the
                // byte count only — never the output path.
                match result {
                    Ok(summary) => {
                        let response = BridgeResponse {
                            version: PROTOCOL_VERSION,
                            request_id: request_id.clone(),
                            ok: true,
                            payload: Some(BridgeResponsePayload::BackupSaved(
                                BackupSavedResponse {
                                    canceled: summary.canceled,
                                    bytes: summary.bytes,
                                },
                            )),
                            error: None,
                        };
                        send_bridge_response(&webview, &response);
                        if let Err(error) = request_tracker.complete(&request_id) {
                            eprintln!("[Bridge] correlation failure after response: {error}");
                        }
                        response_count += 1;
                        println!(
                            "[Backup][Save] canceled={} bytes={} request_latency_ms={:.3}",
                            summary.canceled,
                            summary.bytes,
                            received_at.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                    Err(failure) => {
                        eprintln!("[Backup][Save:{}] {}", failure.code, failure.message);
                        let response = BridgeResponse {
                            version: PROTOCOL_VERSION,
                            request_id: request_id.clone(),
                            ok: false,
                            payload: None,
                            error: Some(BridgeError {
                                code: failure.code,
                                message: failure.message,
                            }),
                        };
                        send_bridge_response(&webview, &response);
                        if let Err(error) = request_tracker.complete(&request_id) {
                            eprintln!("[Bridge] correlation failure after response: {error}");
                        }
                        response_count += 1;
                    }
                }
            }
            Event::UserEvent(NativeEvent::BackupRestoreReady {
                request_id,
                received_at,
                result,
            }) => {
                // Cancellation is normal control flow: it resolves the
                // bounded canceled outcome, never a bridge rejection, so the
                // future backup UI shows its silent cancelled toast and no
                // unhandled rejection can follow. The response carries the
                // clientMods envelope only — never archive or profile paths.
                // Cancellation resolves `canceled: true` with a null
                // envelope; the facade maps it to the silent
                // `{ ok: false, error: "CANCELLED" }` shape.
                match result {
                    Ok(summary) => {
                        let canceled = summary.canceled;
                        let response = BridgeResponse {
                            version: PROTOCOL_VERSION,
                            request_id: request_id.clone(),
                            ok: true,
                            payload: Some(BridgeResponsePayload::BackupRestored(
                                BackupRestoredResponse {
                                    canceled,
                                    client_mods: summary.client_mods,
                                },
                            )),
                            error: None,
                        };
                        send_bridge_response(&webview, &response);
                        if let Err(error) = request_tracker.complete(&request_id) {
                            eprintln!("[Bridge] correlation failure after response: {error}");
                        }
                        response_count += 1;
                        println!(
                            "[Backup][Restore] canceled={} entries={} client_mods={} request_latency_ms={:.3}",
                            canceled,
                            summary.entry_count,
                            summary.client_mods_bytes,
                            received_at.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                    Err(failure) => {
                        eprintln!("[Backup][Restore:{}] {}", failure.code, failure.message);
                        let response = BridgeResponse {
                            version: PROTOCOL_VERSION,
                            request_id: request_id.clone(),
                            ok: false,
                            payload: None,
                            error: Some(BridgeError {
                                code: failure.code,
                                message: failure.message,
                            }),
                        };
                        send_bridge_response(&webview, &response);
                        if let Err(error) = request_tracker.complete(&request_id) {
                            eprintln!("[Bridge] correlation failure after response: {error}");
                        }
                        response_count += 1;
                    }
                }
            }
            Event::UserEvent(NativeEvent::BackupInspectReady {
                request_id,
                received_at,
                result,
            }) => {
                // Preview delivery: `None` (dismissed picker) resolves the
                // bounded canceled outcome, never a rejection. `Some` maps
                // the held restore's preview (counts, flags, bounded display
                // listing — never contents or paths) into the typed
                // response. The staged bytes stay held worker-side for the
                // explicit commit or discard.
                match result {
                    Ok(preview) => {
                        let canceled = preview.is_none();
                        let response = BridgeResponse {
                            version: PROTOCOL_VERSION,
                            request_id: request_id.clone(),
                            ok: true,
                            payload: Some(BridgeResponsePayload::BackupInspected(
                                BackupInspectedResponse {
                                    canceled,
                                    includes: preview.as_ref().map(|preview| preview.includes).unwrap_or(
                                        crate::backup::BackupIncludes {
                                            remcord_config: false,
                                            remcord_themes_and_quick_css: false,
                                            remcord_extension_plugins: false,
                                            vencord_mod_data: false,
                                            equicord_mod_data: false,
                                            shelter_mod_data: false,
                                            mod_bundles: false,
                                        },
                                    ),
                                    archive_bytes: preview.as_ref().map(|preview| preview.archive_bytes).unwrap_or(0),
                                    uncompressed_bytes: preview
                                        .as_ref()
                                        .map(|preview| preview.uncompressed_bytes)
                                        .unwrap_or(0),
                                    entry_count: preview.as_ref().map(|preview| preview.entry_count).unwrap_or(0),
                                    client_mods_bytes: preview
                                        .as_ref()
                                        .map(|preview| preview.client_mods_bytes)
                                        .unwrap_or(0),
                                    has_vencord_mod_data: preview
                                        .as_ref()
                                        .map(|preview| preview.has_vencord_mod_data)
                                        .unwrap_or(false),
                                    has_equicord_mod_data: preview
                                        .as_ref()
                                        .map(|preview| preview.has_equicord_mod_data)
                                        .unwrap_or(false),
                                    has_shelter_mod_data: preview
                                        .as_ref()
                                        .map(|preview| preview.has_shelter_mod_data)
                                        .unwrap_or(false),
                                    entries: preview.as_ref().map(|preview| preview.entry_names.clone()).unwrap_or_default(),
                                    entries_truncated: preview
                                        .as_ref()
                                        .map(|preview| preview.entries_truncated)
                                        .unwrap_or(false),
                                },
                            )),
                            error: None,
                        };
                        send_bridge_response(&webview, &response);
                        if let Err(error) = request_tracker.complete(&request_id) {
                            eprintln!("[Bridge] correlation failure after response: {error}");
                        }
                        response_count += 1;
                        println!(
                            "[Backup][Inspect] canceled={} entries={} request_latency_ms={:.3}",
                            canceled,
                            preview.map(|preview| preview.entry_count).unwrap_or(0),
                            received_at.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                    Err(failure) => {
                        eprintln!("[Backup][Inspect:{}] {}", failure.code, failure.message);
                        let response = BridgeResponse {
                            version: PROTOCOL_VERSION,
                            request_id: request_id.clone(),
                            ok: false,
                            payload: None,
                            error: Some(BridgeError {
                                code: failure.code,
                                message: failure.message,
                            }),
                        };
                        send_bridge_response(&webview, &response);
                        if let Err(error) = request_tracker.complete(&request_id) {
                            eprintln!("[Bridge] correlation failure after response: {error}");
                        }
                        response_count += 1;
                    }
                }
            }
            Event::UserEvent(NativeEvent::BackupDiscardReady {
                request_id,
                received_at,
                discarded,
            }) => {
                let response = BridgeResponse {
                    version: PROTOCOL_VERSION,
                    request_id: request_id.clone(),
                    ok: true,
                    payload: Some(BridgeResponsePayload::BackupRestoreDiscarded(
                        BackupRestoreDiscardedResponse { discarded },
                    )),
                    error: None,
                };
                send_bridge_response(&webview, &response);
                if let Err(error) = request_tracker.complete(&request_id) {
                    eprintln!("[Bridge] correlation failure after response: {error}");
                }
                response_count += 1;
                println!(
                    "[Backup][Discard] discarded={} request_latency_ms={:.3}",
                    discarded,
                    received_at.elapsed().as_secs_f64() * 1000.0
                );
            }
            Event::UserEvent(NativeEvent::BridgeEvent(message)) => {
                incoming_messages += 1;
                event_count += 1;
                match message.event {
                    BridgeEvent::RoundTripAck(payload) => {
                        println!(
                            "[Bridge] renderer received Rust reply; round-trip complete (nonce={})",
                            payload.nonce
                        );
                    }
                    BridgeEvent::CssProbe(payload) => {
                        println!("[Injection] CSS probe result: {:?}", payload.value);
                    }
                    BridgeEvent::RendererError(payload) => {
                        eprintln!("[Renderer][{}] {}", payload.kind, payload.message);
                    }
                    BridgeEvent::BootstrapMetrics(payload) => {
                        println!(
                            "[Bridge][Metrics] document-start execution: {:.3} ms",
                            payload.execution_ms
                        );
                    }
                    BridgeEvent::ModCssResult(payload) => {
                        // The claimed mod id must name a registered mod; the
                        // handler then cross-checks it against the mod that owns
                        // the correlated request, so one mod's adapter can never
                        // satisfy another mod's CSS gate.
                        if let Some(descriptor) = lookup_mod_descriptor(&payload.mod_id) {
                            handle_mod_css_result(
                                &descriptor,
                                payload.request_id,
                                payload.ok,
                                payload.error,
                                &webview,
                                &mut pending_mods,
                                &mut document_lifecycle,
                                &mut request_tracker,
                                &mut response_count,
                            );
                        } else {
                            eprintln!(
                                "[Mod:Registry][unknown_mod] CSS result for unregistered mod '{}'",
                                payload.mod_id
                            );
                        }
                    }
                    BridgeEvent::ModBootstrapResult(payload) => {
                        if let Some(descriptor) = lookup_mod_descriptor(&payload.mod_id) {
                            handle_mod_bootstrap_result(
                                &descriptor,
                                payload.request_id,
                                payload.ok,
                                payload.marker,
                                payload.error,
                                payload.local_storage_api,
                                payload.settings_state,
                                &webview,
                                &mut pending_mods,
                                &mut document_lifecycle,
                                &mut mod_metrics,
                                &mut request_tracker,
                                &mut smoke_reloads_remaining,
                                &mut smoke_reload_request_id,
                                &mut response_count,
                                diagnostic_mod_smoke.then_some(mod_smoke_script.as_str()),
                                diagnostic_shelter_loader_probe
                                    .then_some(SHELTER_LOADER_PROBE_SCRIPT),
                                diagnostic_injector_probe.then_some(INJECTOR_PROBE_SCRIPT),
                                Some(ShelterPluginStart {
                                    script: &settings_plugin_start_script,
                                    started_generation: &mut settings_plugin_started_generation,
                                    start_count: &mut settings_plugin_start_count,
                                }),
                                Some(ShelterPluginStart {
                                    script: &titlebar_plugin_start_script,
                                    started_generation: &mut titlebar_started_generation,
                                    start_count: &mut titlebar_start_count,
                                }),
                            );
                        } else {
                            eprintln!(
                                "[Mod:Registry][unknown_mod] bootstrap result for unregistered mod '{}'",
                                payload.mod_id
                            );
                        }
                    }
                    BridgeEvent::ModAdapterError(payload) => {
                        if let Some(descriptor) = lookup_mod_descriptor(&payload.mod_id) {
                            eprintln!(
                                "[{}][adapter_initialization] {}",
                                descriptor.log_tag, payload.message
                            );
                        } else {
                            eprintln!(
                                "[Mod:Registry][unknown_mod] adapter error for unregistered mod '{}': {}",
                                payload.mod_id, payload.message
                            );
                        }
                    }
                    BridgeEvent::ModResponseAck(payload) => {
                        // The reload driver is host-side DEV instrumentation and
                        // is mod-agnostic: the first registered mod to finish a
                        // bootstrap drives the next forced reload, so both a
                        // single-mod and a multi-mod run work.
                        handle_mod_response_ack(
                            find_mod_descriptor(&payload.mod_id),
                            payload.request_id,
                            &webview,
                            &mut smoke_reloads_remaining,
                            &mut smoke_reload_request_id,
                        );
                    }
                    BridgeEvent::PluginResult(payload) => {
                        // Shipped loader telemetry: log-only, no response.
                        // The id is verified against fresh discovery so the
                        // logs stay trustworthy; a report for an undiscovered
                        // id is dropped loudly instead of recorded.
                        if plugins::validate_plugin_id(&payload.plugin_id).is_err() {
                            eprintln!(
                                "[Plugins][result] report with an invalid plugin id was dropped"
                            );
                        } else {
                            let discovered = plugins::discover_plugins(
                                &profile_path,
                                current_app_version(),
                            );
                            if plugins::find_plugin(&discovered, &payload.plugin_id).is_none() {
                                eprintln!(
                                    "[Plugins][result] report for undiscovered plugin '{}' was dropped",
                                    payload.plugin_id
                                );
                            } else if payload.ok {
                                println!(
                                    "[Plugins][Result] plugin='{}' target={} ok=true",
                                    payload.plugin_id,
                                    payload.target.as_str(),
                                );
                            } else {
                                let error = payload.error.unwrap_or_else(|| {
                                    "plugin entry failed without details".to_owned()
                                });
                                // Byte-safe truncation for the log line; the
                                // loader already bounds the message at 2 KiB.
                                let shown: String = error.chars().take(512).collect();
                                eprintln!(
                                    "[Plugins][Result] plugin='{}' target={} ok=false error={shown}",
                                    payload.plugin_id,
                                    payload.target.as_str(),
                                );
                            }
                        }
                    }
                    // The historically Vencord-named variants remain accepted so
                    // the already-validated Vencord wire contract is unchanged;
                    // each is normalized into the same mod-agnostic handlers.
                    BridgeEvent::VencordCssResult(payload) => {
                        // The shared CSS builder echoes the registry id; the
                        // legacy shape predates the echo and omits it. A
                        // foreign claim is refused loudly before any handler
                        // runs (the fixed-descriptor gate below re-checks the
                        // correlated request as well).
                        if let Err(error) = validate_vencord_css_mod_id(&payload.mod_id) {
                            eprintln!("[Mod:Vencord][css_injection] {error}");
                        } else {
                            handle_mod_css_result(
                                &VENCORD_MOD,
                                payload.request_id,
                                payload.ok,
                                payload.error,
                                &webview,
                                &mut pending_mods,
                                &mut document_lifecycle,
                                &mut request_tracker,
                                &mut response_count,
                            );
                        }
                    }
                    BridgeEvent::VencordBootstrapResult(payload) => {
                        handle_mod_bootstrap_result(
                            &VENCORD_MOD,
                            payload.request_id,
                            payload.ok,
                            payload.marker,
                            payload.error,
                            payload.local_storage_api,
                            payload.settings_state,
                            &webview,
                            &mut pending_mods,
                            &mut document_lifecycle,
                            &mut mod_metrics,
                            &mut request_tracker,
                            &mut smoke_reloads_remaining,
                            &mut smoke_reload_request_id,
                            &mut response_count,
                            None,
                            None,
                            None,
                            None,
                            None,
                        );
                    }
                    BridgeEvent::VencordAdapterError(payload) => {
                        eprintln!(
                            "[{}][adapter_initialization] {}",
                            VENCORD_MOD.log_tag, payload.message
                        );
                    }
                    BridgeEvent::VencordResponseAck(payload) => {
                        handle_mod_response_ack(
                            Some(&VENCORD_MOD),
                            payload.request_id,
                            &webview,
                            &mut smoke_reloads_remaining,
                            &mut smoke_reload_request_id,
                        );
                    }
                    BridgeEvent::InteractionProbe(payload) => {
                        println!("[Renderer][Interaction] {}", payload.phase);
                    }
                    BridgeEvent::SettingsProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[Settings][Smoke] ok=true source={} payload_bytes={} block_power_saving={}",
                                payload.source, payload.payload_bytes, payload.block_power_saving
                            );
                        } else {
                            eprintln!(
                                "[Settings][Smoke] ok=false source={} error={}",
                                payload.source,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::SettingsMutationProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[Settings][MutationSmoke] ok=true stage={} before={} value={} reread_matches={}",
                                payload.stage,
                                payload.before,
                                payload.value,
                                payload.reread_matches
                            );
                        } else {
                            eprintln!(
                                "[Settings][MutationSmoke] ok=false stage={} error={}",
                                payload.stage,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                        if !diagnostic_settings_mutation_smoke {
                            return;
                        }
                        if payload.stage == "mutated" && mutation_smoke_stage == 0 {
                            if payload.ok && payload.reread_matches {
                                mutation_smoke_before = payload.before;
                                mutation_smoke_value = payload.value;
                                mutation_smoke_stage = 1;
                                println!(
                                    "[Settings][MutationSmoke] mutation persisted; reloading to verify reload survival"
                                );
                                if let Err(error) =
                                    webview.evaluate_script("window.location.reload();")
                                {
                                    eprintln!(
                                        "[Settings][MutationSmoke] verification reload failed: {error}"
                                    );
                                }
                            } else {
                                eprintln!(
                                    "[Settings][MutationSmoke] mutate stage failed; not reloading"
                                );
                            }
                        } else if payload.stage == "restored" && mutation_smoke_stage == 1 {
                            if payload.ok
                                && payload.reread_matches
                                && payload.before == mutation_smoke_value
                            {
                                mutation_smoke_stage = 2;
                                println!(
                                    "[Settings][MutationSmoke] reload survival verified and original value restored"
                                );
                            } else {
                                eprintln!(
                                    "[Settings][MutationSmoke] restore verification failed (post-reload read did not match the mutated value or restore did not persist)"
                                );
                            }
                        }
                    }
                    BridgeEvent::SettingsSetProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[Settings][SetSmoke] ok=true stage={} key={} reread_matches={} attacks_blocked={}",
                                payload.stage,
                                payload.key,
                                payload.reread_matches,
                                payload.attacks_blocked
                            );
                        } else {
                            eprintln!(
                                "[Settings][SetSmoke] ok=false stage={} key={} error={}",
                                payload.stage,
                                payload.key,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                        if !diagnostic_settings_set_smoke {
                            return;
                        }
                        if payload.stage == "mutated" && settings_set_smoke_stage == 0 {
                            if payload.ok && payload.reread_matches {
                                settings_set_smoke_before = payload.before;
                                settings_set_smoke_stage = 1;
                                println!(
                                    "[Settings][SetSmoke] mutation persisted; reloading to verify reload survival"
                                );
                                if let Err(error) =
                                    webview.evaluate_script("window.location.reload();")
                                {
                                    eprintln!(
                                        "[Settings][SetSmoke] verification reload failed: {error}"
                                    );
                                }
                            } else {
                                eprintln!(
                                    "[Settings][SetSmoke] mutate stage failed; not reloading"
                                );
                            }
                        } else if payload.stage == "restored" && settings_set_smoke_stage == 1 {
                            if payload.ok
                                && payload.reread_matches
                                && payload.before != settings_set_smoke_before
                            {
                                settings_set_smoke_stage = 2;
                                println!(
                                    "[Settings][SetSmoke] reload survival verified and original value restored"
                                );
                            } else {
                                eprintln!(
                                    "[Settings][SetSmoke] restore verification failed (post-reload read did not match the mutated value or restore did not persist)"
                                );
                            }
                        }
                    }
                    BridgeEvent::GetLangProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[Localization][Smoke] ok=true lang={} value={:?} missing_is_empty={}",
                                payload.lang, payload.value, payload.missing_is_empty
                            );
                        } else {
                            eprintln!(
                                "[Localization][Smoke] ok=false lang={} error={}",
                                payload.lang,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::AppVersionProbe(payload) => {
                        if payload.ok {
                            println!("[App][Smoke] ok=true version={}", payload.version);
                        } else {
                            eprintln!(
                                "[App][Smoke] ok=false error={}",
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::GraphicsProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[Graphics][Smoke] ok=true stage={} browser={} system={} devices={} features={} webgl={} device_summary={:?} feature_summary={:?} webgl_summary={:?}",
                                payload.stage,
                                payload.browser_status,
                                payload.system_status,
                                payload.device_count,
                                payload.feature_count,
                                payload.webgl_status,
                                payload.device_summary,
                                payload.feature_summary,
                                payload.webgl_summary
                            );
                        } else {
                            eprintln!(
                                "[Graphics][Smoke] ok=false stage={} error={}",
                                payload.stage,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::PluginStorageProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[PluginStorage][Smoke] ok=true stage={} payload_bytes={} round_trip={} attacks_blocked={}",
                                payload.stage,
                                payload.payload_bytes,
                                payload.round_trip,
                                payload.attacks_blocked
                            );
                        } else {
                            eprintln!(
                                "[PluginStorage][Smoke] ok=false stage={} error={}",
                                payload.stage,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::ThemeInstallProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[Themes][InstallSmoke] ok=true stage={} id={} css_bytes={} enabled={} attacks_blocked={} fetch_observed={}",
                                payload.stage,
                                payload.id,
                                payload.css_bytes,
                                payload.enabled,
                                payload.attacks_blocked,
                                payload.fetch_observed
                            );
                        } else {
                            eprintln!(
                                "[Themes][InstallSmoke] ok=false stage={} error={}",
                                payload.stage,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::ThemeWriteProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[Themes][WriteSmoke] ok=true stage={} id={} css_bytes={} enabled={} attacks_blocked={} reflected={}",
                                payload.stage,
                                payload.id,
                                payload.css_bytes,
                                payload.enabled,
                                payload.attacks_blocked,
                                payload.reflected
                            );
                        } else {
                            eprintln!(
                                "[Themes][WriteSmoke] ok=false stage={} error={}",
                                payload.stage,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::ThemeImportProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[Themes][ImportSmoke] ok=true stage={} canceled={} imported={} failed={}",
                                payload.stage,
                                payload.canceled,
                                payload.imported,
                                payload.failed
                            );
                        } else {
                            eprintln!(
                                "[Themes][ImportSmoke] ok=false stage={} error={}",
                                payload.stage,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::ThemeSetEnabledProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[Themes][SetEnabledSmoke] ok=true stage={} id={} enabled={} attacks_blocked={} reflected={} multi={} clearvision={}",
                                payload.stage,
                                payload.id,
                                payload.enabled,
                                payload.attacks_blocked,
                                payload.reflected,
                                payload.multi,
                                payload.clearvision
                            );
                        } else {
                            eprintln!(
                                "[Themes][SetEnabledSmoke] ok=false stage={} error={}",
                                payload.stage,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::ThemeListProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[Themes][ListSmoke] ok=true stage={} count={} clearvision={}",
                                payload.stage, payload.count, payload.clearvision
                            );
                        } else {
                            eprintln!(
                                "[Themes][ListSmoke] ok=false stage={} error={}",
                                payload.stage,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::BackupSaveProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[Backup][SaveSmoke] ok=true stage={} canceled={} bytes={}",
                                payload.stage, payload.canceled, payload.bytes
                            );
                        } else {
                            eprintln!(
                                "[Backup][SaveSmoke] ok=false stage={} error={}",
                                payload.stage,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::BackupRestoreProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[Backup][RestoreSmoke] ok=true stage={} canceled={} client_mods_bytes={}",
                                payload.stage, payload.canceled, payload.client_mods_bytes
                            );
                        } else {
                            eprintln!(
                                "[Backup][RestoreSmoke] ok=false stage={} error={}",
                                payload.stage,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::ShelterLoaderProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[ShelterLoader][Probe] ok=true stage={} synthetic={}",
                                payload.stage, payload.synthetic
                            );
                        } else {
                            eprintln!(
                                "[ShelterLoader][Probe] ok=false stage={} error={}",
                                payload.stage,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::SettingsBootSetLang(payload) => {
                        // Narrow fire-and-forget write, mirroring Electron's
                        // `send("setLang", lang)`: the charset/length rule is
                        // re-checked by the localization owner before any
                        // write, and no response is ever sent.
                        match crate::localization::apply_lang(&profile_path, &payload.locale) {
                            Ok(()) => println!(
                                "[SettingsBoot] setLang locale={} persisted",
                                payload.locale
                            ),
                            Err(error) => eprintln!("[SettingsBoot] setLang failed: {error}"),
                        }
                    }
                    BridgeEvent::KeybindMatched(payload) => {
                        // In-page matcher report: carries only the entry
                        // id. firings must name non-global entries (global
                        // ones fire through the hotkey worker instead); the
                        // shared validator enforces scope plus known and
                        // enabled, then executes the stored action.
                        fire_keybind(&webview, &profile_path, &payload.id, false);
                    }
                    BridgeEvent::SettingsBootRestart(_) => {                        // Electron relaunches the application
                        // (`app.relaunch()` + `app.exit()`), and the native
                        // shell must match that: mod adapters are resolved
                        // once at startup and baked into the document-start
                        // script, so a document reload could never activate a
                        // newly enabled mod. Relaunch this executable, then
                        // take the existing orderly quit path. A spawn
                        // failure is loud and keeps the app running (fail
                        // safe); the user can still quit manually.
                        match relaunch_command()
                            .and_then(|mut command| {
                                command.spawn().map_err(|error| {
                                    format!("relaunch spawn failed: {error}")
                                })
                            }) {
                            Ok(_) => {
                                println!(
                                    "[SettingsBoot] restart requested; relaunched executable, shutting down"
                                );
                                #[cfg(target_os = "windows")]
                                perform_orderly_shutdown(
                                    &mut *control_flow,
                                    &mut shutdown_started,
                                    &main_process_failed_counters,
                                    &child_process_failed_counters,
                                    &main_navigation_counters,
                                    &child_navigation_counters,
                                    &mut main_process_failed,
                                    &mut main_navigation_completed,
                                    &mut popup_child,
                                    &mut env_browser_exited_token,
                                    &webview_env,
                                );
                                #[cfg(not(target_os = "windows"))]
                                perform_orderly_shutdown(
                                    &mut *control_flow,
                                    &mut shutdown_started,
                                    &main_process_failed_counters,
                                    &child_process_failed_counters,
                                    &main_navigation_counters,
                                    &child_navigation_counters,
                                );
                            }
                            Err(error) => {
                                eprintln!("[SettingsBoot] restart {error}; staying running")
                            }
                        }
                    }
                    BridgeEvent::SettingsBootProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[SettingsBoot][Probe] ok=true settings_source={} settings_bytes={} themes_count={} i18n_keys={} lang={} frozen={} snapshot_bytes={}",
                                payload.settings_source,
                                payload.settings_bytes,
                                payload.themes_count,
                                payload.i18n_keys,
                                payload.lang,
                                payload.frozen,
                                payload.snapshot_bytes
                            );
                        } else {
                            eprintln!(
                                "[SettingsBoot][Probe] ok=false error={}",
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                    BridgeEvent::SettingsUiProbe(payload) => {
                        if payload.ok {
                            println!(
                                "[SettingsUi][Probe] stage={} ok=true detail={}",
                                payload.stage, payload.detail
                            );
                        } else {
                            eprintln!(
                                "[SettingsUi][Probe] stage={} ok=false detail={} error={}",
                                payload.stage,
                                payload.detail,
                                payload.error.unwrap_or_else(|| "unknown".to_owned())
                            );
                        }
                    }
                }
            }
            _ => {
                let _keep_window_alive = &window;
                let _keep_web_context_alive = &web_context;
                // Owned here so the two native CSS watchers live exactly as
                // long as the application: created once at startup, never on
                // document reload, destroyed by RAII drop on shutdown.
                let _keep_live_watchers_alive = &live_watchers;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        document_start_script, execution_script_for, initialization_script, is_allowed_navigation,
        is_trusted_bridge_origin, load_custom_bundle, lookup_mod_descriptor, mod_adapter_script,
        plugins_entries_compute, plugins_list_compute, plugins_reload_compute,
        plugins_script_compute, plugins_set_enabled_compute, relaunch_command, resolve_active_mods,
        settings_plugin_start_due, settings_plugin_start_script, settings_ui_probe_script,
        startup_visibility, titlebar_plugin_start_script, StartupVisibility, CUSTOM_MOD,
        EQUICORD_MOD, KNOWN_MODS, PERMISSIONS_SMOKE_SCRIPT, REMCORD_PROTOCOL_SMOKE_SCRIPT,
        SETTINGS_PLUGIN_ID, SETTINGS_PLUGIN_START_TEMPLATE, SHELTER_MOD, TITLEBAR_PLUGIN_ID,
        TITLEBAR_PLUGIN_START_TEMPLATE, VENCORD_MOD,
    };
    use crate::mod_substrate::{
        is_allowed_mod_download_url, mod_css_script, mod_execution_context_script,
        mod_marker_script, validate_descriptor, validate_mod_bundle, DocumentModLifecycle,
        ModArtifactIdentity, ModEnabling,
    };

    /// Every registered mod, for tests that must hold for all of them.
    fn all_mods() -> Vec<&'static crate::mod_substrate::ModDescriptor> {
        KNOWN_MODS.iter().collect()
    }

    /// Adapters for an explicit set of mods.
    fn adapters_for(ids: &[&str]) -> Vec<&'static crate::mod_substrate::ModDescriptor> {
        all_mods()
            .into_iter()
            .filter(|descriptor| ids.contains(&descriptor.id))
            .collect()
    }

    #[test]
    fn startup_visibility_maps_exactly_like_electron_modes() {
        // Electron `START_MINIMIZED_MODES` parity: only the two spelled
        // modes are special; everything else (unknown, empty, wrong case)
        // fails safe to Show, like the `"off"` fallback.
        assert_eq!(startup_visibility("off"), StartupVisibility::Show);
        assert_eq!(
            startup_visibility("minimized"),
            StartupVisibility::ShowMinimized
        );
        assert_eq!(startup_visibility("tray"), StartupVisibility::HiddenTray);
        for other in ["", "OFF", "Minimized", "TRAY", "hidden", "true", "tray "] {
            assert_eq!(
                startup_visibility(other),
                StartupVisibility::Show,
                "{other:?}"
            );
        }
    }

    #[test]
    fn settings_boot_snapshot_is_spliced_before_every_adapter() {
        // The snapshot must exist before the settings plugin could execute:
        // the serialized snapshot precedes every mod adapter in the same
        // document-start script, and no splice placeholder may remain.
        for id in ["vencord", "shelter", "equicord", "custom"] {
            let script = initialization_script(
                &adapters_for(&[id]),
                crate::settings_boot::test_fixture_json(),
                crate::app::test_fixture_json(),
            )
            .unwrap();
            assert!(
                !script.contains("__REMCORD_BOOT_SNAPSHOT__"),
                "boot placeholder must be spliced"
            );
            assert!(
                !script.contains("__REMCORD_SETTINGS_BOOT_SNAPSHOT__"),
                "settings boot placeholder must be spliced"
            );
            assert!(
                !script.contains("__REMCORD_HOST_PLATFORM__"),
                "host platform placeholder must be spliced"
            );
            let snapshot_at = script
                .find("\"settingsSource\"")
                .expect("snapshot must be embedded");
            let adapter_at = script
                .find("createModAdapter")
                .expect("adapter factory use must be present");
            assert!(
                snapshot_at < adapter_at,
                "{id}: snapshot must precede adapter execution"
            );
        }
    }

    #[test]
    fn settings_boot_snapshot_exposes_the_closed_surface() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("settingsBoot"));
        assert!(script.contains("settings_boot_set_lang"));
        assert!(script.contains("settings_boot_restart"));
        // No generic invocation surface may appear alongside the snapshot.
        assert!(!script.contains("sendSync"));
        assert!(!script.contains("require(\"electron\")"));
    }

    #[test]
    fn remcord_protocol_smoke_probe_uses_only_fixed_bundle_urls() {
        // The DEV probe may request exactly the host-fixed bundle URLs that
        // exist on disk; it must
        // not accept input, execute fetched bytes, or open a generic
        // fetch/eval surface.
        for fixed in [
            "remcord://assets/css/discord.css",
            "remcord://assets/desktop.png",
            "remcord://plugins/settings/plugin.js",
        ] {
            assert!(
                REMCORD_PROTOCOL_SMOKE_SCRIPT.contains(fixed),
                "probe must reference the fixed bundle URL {fixed}"
            );
        }
        assert!(!REMCORD_PROTOCOL_SMOKE_SCRIPT.contains("eval("));
        assert!(!REMCORD_PROTOCOL_SMOKE_SCRIPT.contains("new Function"));
        assert!(!REMCORD_PROTOCOL_SMOKE_SCRIPT.contains("innerHTML"));
        // No query/fragment: the resolver hardens by rejecting them.
        assert!(!REMCORD_PROTOCOL_SMOKE_SCRIPT.contains('?'));
        assert!(!REMCORD_PROTOCOL_SMOKE_SCRIPT.contains('#'));
    }

    #[test]
    fn permissions_smoke_probe_requests_only_fixed_kinds_and_stops_tracks() {
        // The DEV probe exercises exactly four DOM APIs with literal shapes
        // (notification permission, mic-only getUserMedia, camera-only
        // getUserMedia, geolocation) and stops every obtained track in the
        // same expression. No display media, no picker, no audio graph, no
        // eval surface, no bridge traffic.
        assert!(PERMISSIONS_SMOKE_SCRIPT.contains("Notification.requestPermission()"));
        assert!(PERMISSIONS_SMOKE_SCRIPT.contains("getUserMedia({ audio: true })"));
        assert!(PERMISSIONS_SMOKE_SCRIPT.contains("getUserMedia({ video: true })"));
        assert!(PERMISSIONS_SMOKE_SCRIPT.contains("navigator.geolocation.getCurrentPosition("));
        assert!(PERMISSIONS_SMOKE_SCRIPT.contains("track.stop()"));
        assert!(!PERMISSIONS_SMOKE_SCRIPT.contains("getDisplayMedia"));
        assert!(!PERMISSIONS_SMOKE_SCRIPT.contains("desktopCapturer"));
        assert!(!PERMISSIONS_SMOKE_SCRIPT.contains("eval("));
        assert!(!PERMISSIONS_SMOKE_SCRIPT.contains("new Function"));
        assert!(!PERMISSIONS_SMOKE_SCRIPT.contains("postMessage"));
    }

    #[test]
    fn settings_plugin_start_script_is_closed_and_embedded() {
        // The template is closed: host-fixed lifecycle calls and plain status
        // strings only — no protocol, bridge, URL, or path surface of its own.
        // (The embedded artifact carries remcord:// asset image references
        // for its icons; they resolve through the canonical protocol.)
        for token in [
            "removePlugin",
            "addLocalPlugin",
            "startPlugin",
            "settings-plugin-started",
            "settings-plugin-failed",
        ] {
            assert!(
                SETTINGS_PLUGIN_START_TEMPLATE.contains(token),
                "template must carry {token}"
            );
        }
        assert!(
            SETTINGS_PLUGIN_START_TEMPLATE.contains(SETTINGS_PLUGIN_ID),
            "template must use the closed plugin id"
        );
        for token in ["remcord://", "postMessage", "http://", "https://"] {
            assert!(
                !SETTINGS_PLUGIN_START_TEMPLATE.contains(token),
                "template must not carry {token}"
            );
        }
        let script = settings_plugin_start_script().expect("the start script must build");
        assert!(!script.contains("__REMCORD_SETTINGS_PLUGIN_"));
        assert!(script.contains("remcord-settings"));
        assert!(script.contains("Remcord Settings"));
        // The embedded artifact travels with the script (a distinctive boot
        // member proves the splice carries real plugin bytes, not padding).
        assert!(script.contains("showExperimentalPluginMenu"));
    }

    #[test]
    fn settings_plugin_starts_once_per_document_generation() {
        // Fresh generation: due. Same generation again: refused (replay).
        // New generation (reload): due again (fresh start, no stale state).
        assert!(settings_plugin_start_due(None, 7));
        assert!(!settings_plugin_start_due(Some(7), 7));
        assert!(settings_plugin_start_due(Some(7), 8));
    }

    #[test]
    fn titlebar_plugin_start_script_is_closed_and_embedded() {
        // The template is closed: host-fixed lifecycle calls, inline style
        // management over build-embedded text (no remcord:// fetch, no
        // generic fetch at all), platform-gated attributes, and plain status
        // strings — no bridge, no renderer input.
        for token in [
            "removePlugin",
            "addLocalPlugin",
            "startPlugin",
            "titlebar-started:controls-",
            "titlebar-started:controls-missing",
            "titlebar-pending: shelter plugins unavailable",
            "titlebar-failed",
            "remcord-platform",
            "data-remcord-titlebar-css",
            "__REMCORD_TITLEBAR_CSS__",
        ] {
            assert!(
                TITLEBAR_PLUGIN_START_TEMPLATE.contains(token),
                "template must carry {token}"
            );
        }
        assert!(
            TITLEBAR_PLUGIN_START_TEMPLATE.contains(TITLEBAR_PLUGIN_ID),
            "template must use the closed plugin id"
        );
        for token in [
            "remcord://",
            "postMessage",
            "http://",
            "https://",
            "eval(",
            "new Function",
            "fetch(",
        ] {
            assert!(
                !TITLEBAR_PLUGIN_START_TEMPLATE.contains(token),
                "template must not carry {token}"
            );
        }
        let script = titlebar_plugin_start_script().expect("the start script must build");
        assert!(!script.contains("__REMCORD_TITLEBAR_PLUGIN_"));
        assert!(!script.contains("__REMCORD_TITLEBAR_CSS__"));
        assert!(script.contains("remcord-titlebar"));
        assert!(script.contains("Remcord Titlebar Controller"));
        // The embedded artifact travels with the script (a structural member
        // proves the splice carries real plugin bytes, not padding).
        assert!(script.contains("remcordNavControls"));
        assert!(script.contains("window-controls-container"));
        // The embedded stylesheets travel with the script (a controls rule
        // proves real CSS bytes, not padding).
        assert!(script.contains("#window-controls-container"));
    }

    #[test]
    fn settings_ui_probe_script_carries_a_spliced_run_nonce() {
        let first = settings_ui_probe_script(11).expect("probe script must build");
        let second = settings_ui_probe_script(22).expect("probe script must build");
        assert!(!first.contains("__REMCORD_SETTINGS_UI_RUN_NONCE__"));
        assert!(first.contains("const RUN_NONCE = \"11\""));
        assert!(!second.contains("__REMCORD_SETTINGS_UI_RUN_NONCE__"));
        assert!(
            first != second,
            "distinct nonces must build distinct scripts"
        );
    }

    #[test]
    fn malformed_settings_boot_snapshot_fails_startup() {
        assert!(initialization_script(&[], "{not json", crate::app::test_fixture_json()).is_err());
        assert!(initialization_script(&[], "", crate::app::test_fixture_json()).is_err());
        assert!(
            initialization_script(&[], crate::settings_boot::test_fixture_json(), "{not json")
                .is_err()
        );
    }

    #[test]
    fn document_start_script_appends_quality_after_adapters() {
        // Ordering: the media-hardening layer runs first (SDP/camera wrappers
        // must exist before any page script binds RTCPeerConnection), then
        // the boot snapshot precedes adapters, the spellcheck layer follows
        // the adapters (editable roots carry the host language before the
        // user types), the capture script follows it, and the stream setup
        // popup is the last layer — it can never perturb snapshot/mod boot.
        let boot = crate::settings_boot::test_fixture_json();
        let platform = crate::app::test_fixture_json();
        let script = document_start_script(
            &adapters_for(&["shelter"]),
            boot,
            platform,
            "balanced",
            true,
            true,
            "en-US",
        )
        .expect("must build");
        let media_at = script
            .find("__remcordScreenshareMedia")
            .expect("media status present");
        let snapshot_at = script
            .find("\"settingsSource\"")
            .expect("snapshot embedded");
        let adapter_at = script.find("createModAdapter").expect("adapter present");
        let spell_at = script
            .find("__remcordSpellcheck")
            .expect("spellcheck status present");
        let quality_at = script
            .find("__remcordScreenshareQuality")
            .expect("quality status present");
        let picker_at = script
            .find("__remcordStreamPicker")
            .expect("picker status present");
        assert!(media_at < snapshot_at);
        assert!(snapshot_at < adapter_at);
        assert!(adapter_at < spell_at);
        assert!(spell_at < quality_at);
        assert!(quality_at < picker_at);
        assert!(!script.contains("__REMCORD_SCREENSHARE_QUALITY_MODE__"));
        assert!(!script.contains("__REMCORD_SCREENSHARE_PREFER_HW_H264__"));
        assert!(!script.contains("__REMCORD_SPELLCHECK_ENABLED__"));
        assert!(!script.contains("__REMCORD_SPELLCHECK_LANG__"));
        assert!(script.contains("\"balanced\""));
        assert!(script.contains("const preferHwH264 = true"));
        assert!(script.contains("const spellcheckEnabled = true"));
        assert!(script.contains("const spellcheckLang = \"en-US\""));
    }

    #[test]
    fn document_start_script_sdp_toggle_splices_the_closed_boolean() {
        let boot = crate::settings_boot::test_fixture_json();
        let platform = crate::app::test_fixture_json();
        let on = document_start_script(&[], boot, platform, "default", true, true, "en-US")
            .expect("must build");
        assert!(on.contains("const preferHwH264 = true"));
        let off = document_start_script(&[], boot, platform, "default", false, true, "en-US")
            .expect("must build");
        assert!(off.contains("const preferHwH264 = false"));
    }

    #[test]
    fn document_start_script_spellcheck_toggle_splices_the_closed_pair() {
        // F18: the validated spellcheck pair splices as JSON literals; a
        // hostile language falls back to en-US and can never cross raw.
        let boot = crate::settings_boot::test_fixture_json();
        let platform = crate::app::test_fixture_json();
        let on = document_start_script(&[], boot, platform, "default", true, true, "fr-FR")
            .expect("must build");
        assert!(on.contains("const spellcheckEnabled = true"));
        assert!(on.contains("const spellcheckLang = \"fr-FR\""));
        let off = document_start_script(&[], boot, platform, "default", true, false, "en-US")
            .expect("must build");
        assert!(off.contains("const spellcheckEnabled = false"));
        let hostile =
            document_start_script(&[], boot, platform, "default", true, true, "../escape")
                .expect("must build");
        assert!(hostile.contains("const spellcheckLang = \"en-US\""));
        assert!(!hostile.contains("../escape"));
    }

    #[test]
    fn document_start_script_default_mode_is_capture_inert() {
        // Default quality preserves existing behavior exactly: the quality
        // segment publishes status but contributes no capture wrapper. The
        // stream setup popup is a separate layer and is always present.
        let boot = crate::settings_boot::test_fixture_json();
        let platform = crate::app::test_fixture_json();
        let script = document_start_script(&[], boot, platform, "default", true, true, "en-US")
            .expect("must build");
        assert!(script.contains("__remcordScreenshareMedia"));
        assert!(script.contains("__remcordSpellcheck"));
        assert!(script.contains("__remcordScreenshareQuality"));
        assert!(script.contains("__remcordStreamPicker"));
        // Picker contract anchors: last-choice record and idempotence key.
        assert!(script.contains("__remcordStreamPrefs"));
        assert!(script.contains("remcord.screenshare.pickerPatch"));
        assert!(
            !script.contains("[Remcord][ScreenshareQuality] capture constrained"),
            "default quality must contribute no capture wrapper"
        );
    }

    #[test]
    fn document_start_script_unknown_mode_fails_safe_to_default() {
        let boot = crate::settings_boot::test_fixture_json();
        let platform = crate::app::test_fixture_json();
        let script = document_start_script(&[], boot, platform, "ultra-hd", true, true, "en-US")
            .expect("must build");
        assert!(script.contains("\"default\""));
        assert!(script.contains("__remcordStreamPicker"));
        assert!(
            !script.contains("[Remcord][ScreenshareQuality] capture constrained"),
            "unknown quality mode must contribute no capture wrapper"
        );
    }

    #[test]
    fn document_start_script_rejects_malformed_snapshots_like_base() {
        assert!(document_start_script(
            &[],
            "{not json",
            crate::app::test_fixture_json(),
            "default",
            true,
            true,
            "en-US"
        )
        .is_err());
        assert!(document_start_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            "{not json",
            "balanced",
            false,
            true,
            "en-US"
        )
        .is_err());
    }

    #[test]
    fn diagnostic_keeps_facade_and_substrate_but_omits_unselected_adapters() {
        let with_adapter = initialization_script(
            &adapters_for(&["vencord"]),
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        let without_adapter = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(with_adapter.contains("native.getModBundle"));
        assert!(with_adapter.contains("__remcordNativeVencordAdapter"));
        assert!(with_adapter.contains("__remcordNativeModSubstrate"));
        assert!(without_adapter.contains("        getModBundle,"));
        assert!(without_adapter.contains("        getVencordBundle,"));
        assert!(without_adapter.contains("__remcordNativeModSubstrate"));
        assert!(!without_adapter.contains("__remcordNativeVencordAdapter"));
    }

    #[test]
    fn every_registered_mod_gets_exactly_its_own_adapter() {
        // The adapter appended for a mod must be that mod's file and no other:
        // one mod's identity can never be substituted for another's.
        let expectations = [
            ("vencord", "__remcordNativeVencordAdapter"),
            ("shelter", "__remcordNativeShelterAdapter"),
            ("equicord", "__remcordNativeEquicordAdapter"),
            ("custom", "__remcordNativeCustomAdapter"),
        ];
        for (id, own_flag) in expectations {
            let script = initialization_script(
                &adapters_for(&[id]),
                crate::settings_boot::test_fixture_json(),
                crate::app::test_fixture_json(),
            )
            .unwrap();
            assert!(script.contains(own_flag), "{id} adapter must be injected");
            for (_, other_flag) in expectations {
                if other_flag != own_flag {
                    assert!(
                        !script.contains(other_flag),
                        "{id} run must not inject {other_flag}"
                    );
                }
            }
        }
        // Every registered descriptor resolves to a real adapter file, and the
        // script builder refuses a registry entry that has none.
        for descriptor in all_mods() {
            assert!(
                mod_adapter_script(descriptor).is_some(),
                "{} must have an adapter",
                descriptor.id
            );
            assert!(!initialization_script(
                &[],
                crate::settings_boot::test_fixture_json(),
                crate::app::test_fixture_json()
            )
            .unwrap()
            .contains("getModBundle(\"shelter\""));
        }
    }

    #[test]
    fn persisted_mods_rebuild_the_matching_document_start_adapters() {
        // The restart contract: a new process resolves adapters from the
        // persisted `mods` value, so enabling Vencord then restarting injects
        // exactly the Vencord (+ always-on Shelter) adapters, and disabling
        // drops them again. A document reload cannot do this — the script
        // below is built once per process — which is why the settings-restart
        // path must be a real relaunch rather than a reload.
        let boot = crate::settings_boot::test_fixture_json();
        let platform = crate::app::test_fixture_json();
        let script_for = |mods: &[&str]| {
            let settings_mods = mods
                .iter()
                .map(|entry| entry.to_string())
                .collect::<Vec<_>>();
            let active = resolve_active_mods(&settings_mods, None);
            initialization_script(&active, boot, platform).unwrap()
        };
        let enabled = script_for(&["vencord"]);
        assert!(enabled.contains("__remcordNativeVencordAdapter"));
        assert!(enabled.contains("__remcordNativeShelterAdapter"));
        assert!(!enabled.contains("__remcordNativeEquicordAdapter"));
        assert!(!enabled.contains("__remcordNativeCustomAdapter"));
        let equicord = script_for(&["equicord"]);
        assert!(equicord.contains("__remcordNativeEquicordAdapter"));
        assert!(equicord.contains("__remcordNativeShelterAdapter"));
        assert!(!equicord.contains("__remcordNativeVencordAdapter"));
        assert!(!equicord.contains("__remcordNativeCustomAdapter"));
        // Custom follows the same settings gate: present only when
        // `settings.mods` names it, alongside always-on Shelter.
        let custom = script_for(&["custom"]);
        assert!(custom.contains("__remcordNativeCustomAdapter"));
        assert!(custom.contains("getModBundle(\"custom\")"));
        assert!(custom.contains("__remcordNativeShelterAdapter"));
        assert!(!custom.contains("__remcordNativeVencordAdapter"));
        let disabled = script_for(&[]);
        assert!(!disabled.contains("__remcordNativeVencordAdapter"));
        assert!(!disabled.contains("__remcordNativeEquicordAdapter"));
        assert!(!disabled.contains("__remcordNativeCustomAdapter"));
        assert!(disabled.contains("__remcordNativeShelterAdapter"));
    }

    #[test]
    fn relaunch_command_targets_the_running_executable_with_no_args() {
        // The settings-restart path re-executes this binary with no arguments
        // (the shell takes none; profile and diagnostics inherit through the
        // environment). Spawning itself is never exercised here — that would
        // launch a real process.
        let command = relaunch_command().expect("relaunch must resolve");
        let exe = std::env::current_exe().expect("the test process has an image");
        assert_eq!(command.get_program(), exe.as_os_str());
        assert_eq!(command.get_args().len(), 0);
    }

    #[test]
    fn initialization_script_preserves_window_prototype_storage() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("__remcordNativeStorageAtDocumentStart"));
        assert!(script.contains("Window.prototype"));
        assert!(script.contains("defineProperty(Window.prototype, \"localStorage\""));
    }

    #[test]
    fn every_registered_descriptor_declares_its_traced_contract() {
        assert_eq!(KNOWN_MODS.len(), 4);
        let vencord = lookup_mod_descriptor("vencord").unwrap();
        assert_eq!(vencord.id, VENCORD_MOD.id);
        assert_eq!(vencord.capability, "mod.get_bundle");
        assert_eq!(
            vencord.js_url,
            "https://github.com/Vendicated/Vencord/releases/download/devbuild/browser.js"
        );
        assert_eq!(vencord.js_file, "vencord.js");
        let vencord_css = vencord.css.expect("Vencord ships CSS");
        assert_eq!(vencord_css.file, "vencord.css");
        assert_eq!(vencord_css.style_element_id, "remcord-native-vencord-css");
        assert_eq!(vencord.page_global, "Vencord");
        assert_eq!(vencord.success_marker, "vencord-bootstrap-executed");
        assert_eq!(vencord.settings_key, Some("VencordSettings"));

        // Shelter: JS only, fixed raw.githubusercontent source, header-less
        // identity, asynchronous global, no invented settings key, and
        // unconditional enablement (Electron hardcodes `enabled: true`).
        let shelter = lookup_mod_descriptor("shelter").unwrap();
        assert_eq!(shelter.id, SHELTER_MOD.id);
        assert_eq!(
            shelter.js_url,
            "https://raw.githubusercontent.com/uwu/shelter-builds/main/shelter.js"
        );
        assert_eq!(shelter.js_file, "shelter.js");
        assert!(shelter.css.is_none(), "Shelter ships no stylesheet");
        assert_eq!(shelter.page_global, "shelter");
        assert_eq!(shelter.page_global_requires, Some("plugins"));
        assert_eq!(shelter.settings_key, None);
        assert_eq!(shelter.enabling, ModEnabling::AlwaysEnabled);
        assert_eq!(shelter.success_marker, "shelter-bootstrap-executed");

        // Equicord: JS + CSS (its Electron preload requires both), banner
        // identity, synchronous global, settings key, and a settings gate.
        let equicord = lookup_mod_descriptor("equicord").unwrap();
        assert_eq!(equicord.id, EQUICORD_MOD.id);
        assert_eq!(
            equicord.js_url,
            "https://github.com/Equicord/Equicord/releases/download/latest/browser.js"
        );
        let equicord_css = equicord.css.expect("Equicord requires CSS");
        assert_eq!(
            equicord_css.url,
            "https://github.com/Equicord/Equicord/releases/download/latest/browser.css"
        );
        assert_eq!(equicord_css.file, "equicord.css");
        assert_eq!(equicord.settings_key, Some("EquicordSettings"));
        assert_eq!(equicord.enabling, ModEnabling::SettingsMods);
        assert_eq!(equicord.success_marker, "equicord-bootstrap-executed");

        // Custom: user-owned `<profile>/custom.js` + `<profile>/custom.css`
        // (host-built fixed paths, never a download), `UserOwned` identity,
        // optional CSS, no page-world global of its own (the marker checks
        // the host storage-context probe), and a settings gate matching
        // Electron's `getConfig("mods").includes("custom")`.
        let custom = lookup_mod_descriptor("custom").unwrap();
        assert_eq!(custom.id, CUSTOM_MOD.id);
        assert_eq!(custom.capability, "mod.get_bundle");
        assert_eq!(custom.js_file, "custom.js");
        let custom_css = custom.css.expect("Custom declares an optional stylesheet");
        assert_eq!(custom_css.file, "custom.css");
        assert_eq!(custom_css.style_element_id, "remcord-native-custom-css");
        assert!(matches!(custom.identity, ModArtifactIdentity::UserOwned));
        assert_eq!(
            custom.page_global,
            "__remcordNativeCustomStorageAtExecution"
        );
        assert_eq!(
            custom.storage_context_global,
            "__remcordNativeCustomStorageAtExecution"
        );
        assert_eq!(custom.page_global_requires, None);
        assert_eq!(custom.settings_key, None);
        assert_eq!(custom.enabling, ModEnabling::SettingsMods);
        assert_eq!(custom.success_marker, "custom-bootstrap-executed");
        assert_eq!(
            custom.bootstrap_dom_event,
            "remcord-native:custom-bootstrap"
        );
        // Single source of truth for the size caps: the descriptor carries
        // the exact constants the local-file reader enforces.
        assert_eq!(
            custom.max_js_bytes,
            crate::custom_bundle::MAX_CUSTOM_JS_BYTES
        );
        assert_eq!(
            custom_css.max_bytes,
            crate::custom_bundle::MAX_CUSTOM_CSS_BYTES
        );
        // The placeholder URLs are never fetched (retrieval is local-file
        // only, and the downloader refuses `UserOwned` outright), but they
        // must still satisfy structural validation like every descriptor.
        assert!(is_allowed_mod_download_url(&custom, custom.js_url));
        assert!(is_allowed_mod_download_url(&custom, custom_css.url));

        // Nothing that is not registered resolves, and the registry is
        // EXACT-match only: no prefix, case, path, or URL shape selects one.
        for unregistered in [
            "arbitrary",
            "Vencord",
            "SHELTER",
            "Equicord",
            "EQUICORD",
            "vencord.js",
            "shelter/../x",
            "../shelter",
            "../vencord",
            "equicord/foo",
            "https://attacker.invalid/x.js",
            "C:\\Windows\\x.js",
            "javascript:alert(1)",
            "ven ",
        ] {
            assert!(
                lookup_mod_descriptor(unregistered).is_none(),
                "'{unregistered}' must not resolve"
            );
        }
    }

    #[test]
    fn adapter_enablement_follows_each_mods_traced_electron_policy() {
        // Electron: `getConfig("mods").includes(<id>)` for Vencord/Equicord and
        // a hardcoded `enabled: true` for Shelter.
        let settings = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();

        // Empty settings: only the unconditionally enabled mod runs.
        let active = resolve_active_mods(&settings(&[]), None);
        assert_eq!(
            active.iter().map(|d| d.id).collect::<Vec<_>>(),
            vec!["shelter"]
        );

        let active = resolve_active_mods(&settings(&["vencord"]), None);
        assert_eq!(
            active.iter().map(|d| d.id).collect::<Vec<_>>(),
            vec!["vencord", "shelter"]
        );

        let active = resolve_active_mods(&settings(&["equicord"]), None);
        assert_eq!(
            active.iter().map(|d| d.id).collect::<Vec<_>>(),
            vec!["shelter", "equicord"]
        );

        // A host-controlled diagnostic allowlist can force a subset, which is
        // how the "native without <mod>" control runs are produced. It can only
        // REMOVE mods, never introduce one that is not registered.
        let active = resolve_active_mods(&settings(&["vencord", "equicord"]), Some("shelter"));
        assert_eq!(
            active.iter().map(|d| d.id).collect::<Vec<_>>(),
            vec!["shelter"]
        );
        // The allowlist can only REMOVE mods; it never bypasses a settings
        // gate. With no mods in settings, an allowlist naming Vencord and
        // Equicord still yields nothing.
        assert!(resolve_active_mods(&settings(&[]), Some("vencord, equicord")).is_empty());
        let active = resolve_active_mods(
            &settings(&["vencord", "equicord"]),
            Some("vencord, equicord"),
        );
        assert_eq!(
            active.iter().map(|d| d.id).collect::<Vec<_>>(),
            vec!["vencord", "equicord"]
        );
        // A settings entry for an unregistered name selects nothing, while
        // the registered `custom` id selects the custom adapter (its bytes
        // still come from the host-built profile files, resolved per
        // request — an absent `custom.js`/`custom.css` fails retrieval
        // instead of executing nothing).
        let active = resolve_active_mods(&settings(&["DoNotChange"]), None);
        assert_eq!(
            active.iter().map(|d| d.id).collect::<Vec<_>>(),
            vec!["shelter"]
        );
        let active = resolve_active_mods(&settings(&["custom", "DoNotChange"]), None);
        assert_eq!(
            active.iter().map(|d| d.id).collect::<Vec<_>>(),
            vec!["shelter", "custom"]
        );
        // The settings gate still applies: an allowlist naming custom
        // cannot enable it without the settings entry.
        assert!(resolve_active_mods(&settings(&[]), Some("custom")).is_empty());
        // Whitespace and empty entries in the allowlist are ignored.
        let active = resolve_active_mods(&settings(&[]), Some(" shelter , "));
        assert_eq!(
            active.iter().map(|d| d.id).collect::<Vec<_>>(),
            vec!["shelter"]
        );
    }

    #[test]
    fn shelter_identity_does_not_invent_a_build_header() {
        use crate::mod_substrate::ModArtifactIdentity;
        // Shelter's published artifact carries NO banner, so the descriptor
        // must NOT claim a header prefix. Identity is sentinel-based.
        match SHELTER_MOD.identity {
            ModArtifactIdentity::SourceSentinels { sentinels } => {
                assert!(
                    sentinels.len() >= 2,
                    "identity needs more than one sentinel"
                );
                for sentinel in sentinels {
                    assert!(!sentinel.is_empty() && sentinel.len() <= 256);
                    assert!(
                        sentinel.contains("shelter"),
                        "sentinels must be Shelter-specific: {sentinel}"
                    );
                }
            }
            ModArtifactIdentity::BuildHeader { .. } => {
                panic!("Shelter must not claim a build header it does not publish")
            }
            ModArtifactIdentity::UserOwned => {
                panic!("Shelter is download-published; only custom is user-owned")
            }
        }
        // Equicord DOES publish a banner, so it uses the header shape.
        assert!(matches!(
            EQUICORD_MOD.identity,
            ModArtifactIdentity::BuildHeader { prefix } if prefix == "// Equicord "
        ));
        // A real sentinel-bearing artifact validates; a Vencord artifact (or
        // anything else without the sentinels) is refused.
        let shelter_js = b"(()=>{console.error(\"shelter is initializing...\");\
            (async()=>{window.shelter={plugins:{}}})();\
            console.error(\"shelter is initialized. took: 4ms\");})();"
            .to_vec();
        let bundle = validate_mod_bundle(&SHELTER_MOD, shelter_js.clone(), None).unwrap();
        assert!(bundle.version.starts_with("sha256-"), "{}", bundle.version);
        assert_eq!(bundle.css_bytes, None);
        assert!(validate_mod_bundle(
            &SHELTER_MOD,
            b"// Vencord b52ed36\nvar Vencord = {};\n".to_vec(),
            None
        )
        .is_err());
        assert!(validate_mod_bundle(&SHELTER_MOD, shelter_js, Some(b".x{}".to_vec())).is_err());
    }

    #[test]
    fn every_registered_descriptor_passes_substrate_validation() {
        for descriptor in all_mods() {
            validate_descriptor(descriptor)
                .unwrap_or_else(|error| panic!("{}: {error}", descriptor.id));
            assert_eq!(descriptor.capability, "mod.get_bundle");
            assert!(crate::protocol::validate_mod_bundle_id(descriptor.id).is_ok());
            assert!(is_allowed_mod_download_url(descriptor, descriptor.js_url));
            if let Some(css) = descriptor.css {
                assert!(is_allowed_mod_download_url(descriptor, css.url));
            }
            // Every JS and CSS cache name is a bare host-chosen file name.
            assert!(!descriptor.js_file.contains('/') && !descriptor.js_file.contains('\\'));
            assert!(!descriptor.js_file.contains(".."));
        }
    }

    #[test]
    fn f4_in_mod_settings_pages_need_no_host_capability() {
        // Vencord/Equicord/Shelter render their own settings UI page-side
        // once the substrate executes their bundles; the host contributes
        // storage reachability + the QuickCSS editor handoff only. This pins
        // that contract to the registered descriptors and the boot snapshot.
        crate::mod_settings::validate_mod_settings_contract(
            super::VENCORD_MOD.settings_key,
            super::EQUICORD_MOD.settings_key,
            super::SHELTER_MOD.settings_key,
            super::CUSTOM_MOD.settings_key,
            super::BOOT_SNAPSHOT.capabilities,
        )
        .expect("F4 contract must hold for the registered mods");
        assert_eq!(
            crate::mod_settings::QUICKCSS_HANDOFF_EVENT,
            "remcord-native:open-quickcss-editor"
        );
        assert_eq!(
            crate::mod_settings::QUICKCSS_HANDOFF_MODES,
            ["vencord", "equicord", "remcord"]
        );
    }

    #[test]
    fn f7_settings_import_is_copy_only_with_a_documented_allowlist() {
        // Copy-only, never auto-migrate: the capability exists in the boot
        // snapshot, the source path is fixed host-side, every allowlisted
        // key carries a rationale, and restart-requiring keys are named so
        // the UI can raise the restart banner.
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"settings.import_electron"));
        assert!(!crate::settings_import::IMPORTABLE_KEYS.is_empty());
        for entry in crate::settings_import::IMPORTABLE_KEYS {
            assert!(!entry.key.is_empty());
            assert!(
                !entry.rationale.is_empty(),
                "{} must document why it is safe",
                entry.key
            );
        }
        assert_eq!(
            crate::settings_import::RESTART_REQUIRED_KEYS,
            ["mods", "channel", "csp"]
        );
        // Dangerous shapes are structurally absent from the allowlist.
        for forbidden in [
            "windowStyle",
            "additionalArguments",
            "customIcon",
            "galleryRoot",
            "keybinds",
            "proxyMode",
            "customJsBundle",
        ] {
            assert!(
                !crate::settings_import::IMPORTABLE_KEYS
                    .iter()
                    .any(|entry| entry.key == forbidden),
                "{forbidden} must never be importable"
            );
        }
    }

    #[test]
    fn f8_keybind_execution_registers_validated_entries_without_code() {
        // Management capabilities exist in the boot snapshot; the matcher
        // ships in the document-start script; runJavascript has no wire
        // shape anywhere (no command, no event, no action variant).
        for capability in [
            "keybinds.list",
            "keybinds.add",
            "keybinds.toggle",
            "keybinds.remove",
        ] {
            assert!(super::BOOT_SNAPSHOT.capabilities.contains(&capability));
        }
        assert!(super::KEYBIND_MATCHER_SCRIPT.contains("keybind_matched"));
        assert!(super::KEYBIND_MATCHER_SCRIPT.contains("remcordNativeKeybindMatcher"));
        assert!(crate::keybinds::KeybindAction::parse("runJavascript").is_none());
        // The action scripts carry fixed Electron shapes and no
        // interpolation surface.
        let mute = crate::keybinds::keybind_action_script(crate::keybinds::KeybindAction::Mute);
        assert!(mute.contains("AUDIO_TOGGLE_SELF_MUTE"));
        // Voice actions stay flux dispatches (harmless without a call);
        // settings navigation uses the fixed path.
        assert!(crate::keybinds::KEYBIND_OPEN_SETTINGS_PATH.starts_with('/'));
    }

    #[test]
    fn the_bundle_capability_is_shared_and_registry_backed() {
        // ONE capability serves every registered mod; the renderer only ever
        // supplies a logical id. No filesystem, URL, or source capability is
        // granted alongside it.
        let descriptor = lookup_mod_descriptor("vencord").unwrap();
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&descriptor.capability));
        assert_eq!(
            super::BOOT_SNAPSHOT
                .capabilities
                .iter()
                .filter(|capability| **capability == "mod.get_bundle")
                .count(),
            1
        );
        for capability in super::BOOT_SNAPSHOT.capabilities {
            assert!(!capability.contains("filesystem"));
            assert!(!capability.contains("bundle_file"));
            assert!(!capability.contains("eval"));
        }
        for capability in [
            "filesystem",
            "invoke",
            "read_file",
            "load_script",
            "execute",
        ] {
            assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&capability));
        }
        // Every registered mod must declare the shared capability and pass
        // id validation, so a request id can never be silently mismatched
        // against a descriptor.
        let mut ids: Vec<&str> = Vec::new();
        for descriptor in KNOWN_MODS {
            assert_eq!(descriptor.capability, "mod.get_bundle");
            assert!(crate::protocol::validate_mod_bundle_id(descriptor.id).is_ok());
            assert_eq!(
                lookup_mod_descriptor(descriptor.id).map(|found| found.id),
                Some(descriptor.id)
            );
            assert!(!ids.contains(&descriptor.id), "duplicate registry id");
            ids.push(descriptor.id);
        }
    }

    #[test]
    fn every_registered_mod_binds_the_shared_bundle_capability_at_startup() {
        // Fail-fast parity with the existing startup descriptor check: a
        // descriptor without the shared capability must abort startup rather
        // than silently serve no bundle.
        for descriptor in KNOWN_MODS {
            assert!(super::BOOT_SNAPSHOT
                .capabilities
                .contains(&descriptor.capability));
        }
    }

    #[test]
    fn custom_bundles_execute_from_host_built_profile_files() {
        // End-to-end through the real loader + shared validator: bytes the
        // user drops at `<profile>/custom.js` / `<profile>/custom.css`
        // validate into a servable bundle with a content-digest version.
        // No path, URL, or filename crosses from the renderer at any point.
        let profile = std::env::temp_dir().join(format!(
            "remcord-custom-e2e-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&profile).expect("temp profile must be creatable");
        // Nothing installed: the capability must fail typed, never serve
        // an empty bundle.
        let error = load_custom_bundle(&CUSTOM_MOD, profile.clone())
            .expect_err("absent custom files must fail");
        assert!(
            error.contains("CUSTOM_NOT_INSTALLED"),
            "unexpected error: {error}"
        );
        assert!(!error.contains(&profile.to_string_lossy().into_owned()));
        // JS-only: CSS stage is skipped, validation passes.
        std::fs::write(profile.join("custom.js"), b"window.__custom = 1;").unwrap();
        let bundle =
            load_custom_bundle(&CUSTOM_MOD, profile.clone()).expect("js-only custom must validate");
        assert_eq!(bundle.js, "window.__custom = 1;");
        assert!(bundle.css.is_none());
        assert!(bundle.version.starts_with("sha256-"));
        // Adding CSS changes the version (the digest covers both halves).
        std::fs::write(profile.join("custom.css"), b".x{color:red}").unwrap();
        let with_css =
            load_custom_bundle(&CUSTOM_MOD, profile.clone()).expect("js+css custom must validate");
        assert_eq!(with_css.css.unwrap(), ".x{color:red}");
        assert_ne!(with_css.version, bundle.version);
        // Identical bytes always produce the identical version.
        let again = load_custom_bundle(&CUSTOM_MOD, profile.clone()).expect("reread must validate");
        assert_eq!(again.version, with_css.version);
        // CSS-only: the empty-implicit JS half is accepted for user-owned
        // bundles (a download-published bundle would be refused here).
        std::fs::remove_file(profile.join("custom.js")).unwrap();
        let css_only = load_custom_bundle(&CUSTOM_MOD, profile.clone())
            .expect("css-only custom must validate");
        assert_eq!(css_only.js, "");
        assert_eq!(css_only.js_bytes, 0);
        // Non-UTF8 bytes are refused, never served.
        std::fs::write(profile.join("custom.js"), vec![0xff, 0xfe, 0x00]).unwrap();
        assert!(load_custom_bundle(&CUSTOM_MOD, profile.clone()).is_err());
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn custom_bundles_never_reach_the_downloader() {
        // Defense in depth: even a mistaken direct call fails closed with
        // a message naming the local-file path instead of attempting the
        // placeholder URL.
        let error = crate::mod_substrate::load_mod_bundle(
            &CUSTOM_MOD,
            std::env::temp_dir().join("remcord-custom-never"),
        )
        .expect_err("UserOwned must never download");
        assert!(error.contains("local profile files"));
    }

    #[test]
    fn custom_marker_checks_the_host_context_probe_not_a_page_global() {
        // No custom bundle can be required to publish a page-world global,
        // so the marker proves the host's own storage-context probe object
        // (defined immediately before the user JS) instead.
        let context = mod_execution_context_script(&CUSTOM_MOD);
        assert!(context.contains("__remcordNativeCustomStorageAtExecution"));
        // Custom keeps its state outside localStorage: no settings key is
        // invented for the storage diagnostic.
        assert!(context.contains(r#"settingsState = "not_applicable";"#));
        let marker = mod_marker_script(&CUSTOM_MOD, "request-1", 1.5).unwrap();
        assert!(marker.contains("globalThis.__remcordNativeCustomStorageAtExecution"));
        assert!(marker.contains("custom-bootstrap-executed"));
        assert!(marker.contains("remcord-native:custom-bootstrap"));
        assert!(!marker.contains("eval("));
        assert!(!marker.contains("new Function"));
        // The CSS stage uses the mod-agnostic result family with the
        // host-supplied id echoed, exactly like Equicord.
        let css_source = CUSTOM_MOD.css.expect("custom declares optional CSS");
        let css = mod_css_script(&CUSTOM_MOD, &css_source, "request-1", ".x{}").unwrap();
        assert!(css.contains("remcord-native-custom-css"));
        assert!(css.contains("mod_css_result"));
        assert!(css.contains("mod_id: \"custom\""));
    }

    /// Temporary plugin profile for compute tests: installs manifests,
    /// entry files, and an optional `pluginStates` map, returning the
    /// profile root. Callers delete it when done.
    type FixtureEntry<'a> = (&'a str, &'a [u8]);
    type FixturePlugin<'a> = (&'a str, &'a str, &'a [FixtureEntry<'a>]);
    fn plugin_profile(
        tag: &str,
        plugins: &[FixturePlugin<'_>],
        states: Option<&str>,
    ) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "remcord-plugin-compute-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        for (child, manifest, files) in plugins {
            let dir = root.join("plugins").join(child);
            std::fs::create_dir_all(&dir).expect("plugin dir must be creatable");
            std::fs::write(dir.join("manifest.json"), manifest).expect("manifest must write");
            for (name, bytes) in *files {
                let path = dir.join(name);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("entry parent must be creatable");
                }
                std::fs::write(path, bytes).expect("entry must write");
            }
        }
        if let Some(states) = states {
            std::fs::create_dir_all(root.join("storage")).expect("storage must be creatable");
            std::fs::write(
                root.join("storage").join("settings.json"),
                format!(r#"{{"pluginStates":{states}}}"#),
            )
            .expect("settings must write");
        }
        root
    }

    const PLUGIN_A_MANIFEST: &str = r#"{"id":"alpha","name":"Alpha","version":"1.0.0","description":"First plugin","author":"tester","preload":"preload.js","renderer":"renderer.js","compatibleVersions":["*"]}"#;
    const PLUGIN_B_MANIFEST: &str = r#"{"id":"beta","name":"Beta","version":"2.0.0","preload":"preload.js","compatibleVersions":["9.9.x"]}"#;
    const PLUGIN_MAIN_ONLY_MANIFEST: &str =
        r#"{"id":"mainonly","name":"MainOnly","version":"1.0.0","main":"main.js"}"#;

    #[test]
    fn plugins_list_reports_display_metadata_with_authoritative_enablement() {
        let root = plugin_profile(
            "list",
            &[
                ("a", PLUGIN_A_MANIFEST, &[("preload.js", b"void 0;")]),
                ("b", PLUGIN_B_MANIFEST, &[("preload.js", b"void 0;")]),
                ("m", PLUGIN_MAIN_ONLY_MANIFEST, &[("main.js", b"void 0;")]),
            ],
            Some(r#"{"alpha":true,"beta":true,"ghost":true}"#),
        );
        let infos = plugins_list_compute(&root).expect("a valid profile must list");
        assert_eq!(infos.len(), 3);
        let alpha = infos.iter().find(|info| info.id == "alpha").unwrap();
        assert_eq!(alpha.name, "Alpha");
        assert_eq!(alpha.description.as_deref(), Some("First plugin"));
        assert_eq!(alpha.author.as_deref(), Some("tester"));
        assert!(alpha.enabled);
        assert!(alpha.compatible);
        assert!(alpha.compatibility_message.is_none());
        assert_eq!(alpha.compatible_versions, vec!["*".to_owned()]);
        assert!(!alpha.has_main);
        assert!(alpha.has_preload);
        assert!(alpha.has_renderer);
        // Pinned to a foreign major: listed but incompatible, still enabled
        // (enablement and compatibility are independent facts).
        let beta = infos.iter().find(|info| info.id == "beta").unwrap();
        assert!(beta.enabled);
        assert!(!beta.compatible);
        assert!(beta.compatibility_message.is_some());
        // A main-only plugin is listed honestly (its entry never executes).
        let main_only = infos.iter().find(|info| info.id == "mainonly").unwrap();
        assert!(main_only.has_main);
        assert!(!main_only.has_preload);
        // States for undiscovered ids change nothing.
        assert!(infos.iter().all(|info| info.id != "ghost"));
        // Missing settings file: defaults (everything disabled).
        let bare = plugin_profile("list-bare", &[("a", PLUGIN_A_MANIFEST, &[])], None);
        let infos = plugins_list_compute(&bare).expect("missing settings must list defaults");
        assert!(!infos[0].enabled);
        // Corrupt settings: fail closed, never guessed enablement.
        std::fs::create_dir_all(bare.join("storage")).unwrap();
        std::fs::write(bare.join("storage").join("settings.json"), b"{not json").unwrap();
        let error = plugins_list_compute(&bare).expect_err("corrupt settings must fail");
        assert_eq!(error.code, "PLUGINS_UNAVAILABLE");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&bare);
    }

    #[test]
    fn plugins_set_enabled_follows_the_electron_decision_table() {
        let root = plugin_profile(
            "flip",
            &[
                ("a", PLUGIN_A_MANIFEST, &[("preload.js", b"void 0;")]),
                ("b", PLUGIN_B_MANIFEST, &[("preload.js", b"void 0;")]),
            ],
            Some(r#"{}"#),
        );
        // Malformed ids are typed errors before any filesystem work.
        for hostile in ["", ".", "..", "../x", "x/y", "https://a.invalid/x"] {
            let error = plugins_set_enabled_compute(&root, hostile, true)
                .expect_err("a malformed id must fail");
            assert_eq!(error.code, "INVALID_PLUGIN_ID", "{hostile:?}");
        }
        // Unknown ids resolve {ok:false} without writing.
        let denied =
            plugins_set_enabled_compute(&root, "ghost", true).expect("unknown must resolve");
        assert!(!denied.ok);
        assert!(denied.plugin_id.is_none());
        // Enabling an incompatible plugin resolves {ok:false} without writing.
        let denied = plugins_set_enabled_compute(&root, "beta", true)
            .expect("incompatible enable must resolve");
        assert!(!denied.ok);
        // Enabling a compatible plugin persists and echoes.
        let flipped = plugins_set_enabled_compute(&root, "alpha", true)
            .expect("compatible enable must apply");
        assert!(flipped.ok);
        assert_eq!(flipped.plugin_id.as_deref(), Some("alpha"));
        assert_eq!(flipped.enabled, Some(true));
        assert_eq!(flipped.source, Some("file"));
        assert!(flipped.payload_bytes.is_some());
        let infos = plugins_list_compute(&root).expect("reread must list");
        assert!(
            infos
                .iter()
                .find(|info| info.id == "alpha")
                .unwrap()
                .enabled
        );
        assert!(!infos.iter().find(|info| info.id == "beta").unwrap().enabled);
        // Disabling an incompatible plugin is allowed (Electron parity).
        let flipped =
            plugins_set_enabled_compute(&root, "beta", false).expect("disable must apply");
        assert!(flipped.ok);
        // Idempotent same-state flip persists the same value.
        let flipped =
            plugins_set_enabled_compute(&root, "alpha", true).expect("idempotent flip must apply");
        assert!(flipped.ok);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plugins_reload_revalidates_without_enabling() {
        let root = plugin_profile(
            "reload",
            &[
                ("a", PLUGIN_A_MANIFEST, &[("preload.js", b"void 0;")]),
                ("b", PLUGIN_B_MANIFEST, &[("preload.js", b"void 0;")]),
            ],
            Some(r#"{"alpha":true}"#),
        );
        assert!(plugins_reload_compute(&root, "../x").is_err());
        let denied = plugins_reload_compute(&root, "ghost").expect("unknown must resolve");
        assert!(!denied.ok);
        // Incompatible reloads fail even when disabled (Electron parity).
        let denied = plugins_reload_compute(&root, "beta").expect("incompatible must resolve");
        assert!(!denied.ok);
        // Compatible reloads succeed enabled or not, and change nothing.
        let reloaded = plugins_reload_compute(&root, "alpha").expect("enabled reload must succeed");
        assert!(reloaded.ok);
        assert_eq!(reloaded.plugin_id.as_deref(), Some("alpha"));
        plugins_set_enabled_compute(&root, "alpha", false).expect("disable must apply");
        let reloaded =
            plugins_reload_compute(&root, "alpha").expect("disabled reload must succeed");
        assert!(reloaded.ok);
        let infos = plugins_list_compute(&root).expect("reread must list");
        assert!(
            !infos
                .iter()
                .find(|info| info.id == "alpha")
                .unwrap()
                .enabled
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plugins_entries_and_scripts_serve_only_enabled_compatible_resolvable() {
        use crate::plugins::PluginTarget;
        let root = plugin_profile(
            "serve",
            &[
                (
                    "a",
                    PLUGIN_A_MANIFEST,
                    &[
                        ("preload.js", b"module.exports.activate = () => {};"),
                        ("renderer.js", b"window.__a = 1;"),
                    ],
                ),
                ("b", PLUGIN_B_MANIFEST, &[("preload.js", b"void 0;")]),
                (
                    "gone",
                    r#"{"id":"gone","name":"Gone","version":"1.0.0","preload":"missing.js"}"#,
                    &[],
                ),
            ],
            Some(r#"{"alpha":true,"beta":true,"gone":true}"#),
        );
        // Entries: only alpha (enabled+compatible+resolvable). Beta is
        // enabled but incompatible; gone resolves to nothing.
        let entries =
            plugins_entries_compute(&root, PluginTarget::Preload).expect("entries must compute");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "alpha");
        assert_eq!(entries[0].name, "Alpha");
        let entries =
            plugins_entries_compute(&root, PluginTarget::Renderer).expect("entries must compute");
        assert_eq!(entries.len(), 1);
        // Scripts: alpha serves; beta (incompatible), unknown ids, and
        // disabled plugins resolve None; missing files resolve None.
        let source = plugins_script_compute(&root, "alpha", PluginTarget::Preload)
            .expect("alpha preload must serve")
            .expect("alpha preload must have source");
        assert!(source.contains("activate"));
        let renderer = plugins_script_compute(&root, "alpha", PluginTarget::Renderer)
            .expect("alpha renderer must serve")
            .expect("alpha renderer must have source");
        assert!(renderer.contains("__a"));
        assert!(plugins_script_compute(&root, "beta", PluginTarget::Preload)
            .expect("beta must resolve")
            .is_none());
        assert!(
            plugins_script_compute(&root, "ghost", PluginTarget::Preload)
                .expect("unknown must resolve")
                .is_none()
        );
        assert!(plugins_script_compute(&root, "gone", PluginTarget::Preload)
            .expect("missing file must resolve")
            .is_none());
        // Disabling removes servability without touching files.
        plugins_set_enabled_compute(&root, "alpha", false).expect("disable must apply");
        assert!(
            plugins_script_compute(&root, "alpha", PluginTarget::Preload)
                .expect("disabled must resolve")
                .is_none()
        );
        assert!(plugins_entries_compute(&root, PluginTarget::Preload)
            .expect("entries must compute")
            .is_empty());
        // Malformed ids are typed errors, never resolutions.
        assert!(plugins_script_compute(&root, "../x", PluginTarget::Preload).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn shelter_execution_leads_with_the_injector_declaration() {
        // Electron's wrapper order: the injector global is declared first,
        // then the bundle evaluates. Other mods keep the exact previous
        // shape (context probe, then bytes).
        let injector = crate::shelter_injector::injector_declaration()
            .expect("the injector declaration must build");
        let shelter_script = execution_script_for(&SHELTER_MOD, "/*bundle*/", &injector);
        let injector_at = shelter_script
            .find("const SHELTER_INJECTOR_PLUGINS=")
            .expect("the declaration must lead");
        let context_at = shelter_script
            .find("__remcordNativeShelterStorageAtExecution")
            .expect("the context probe must follow");
        let bundle_at = shelter_script
            .find("/*bundle*/")
            .expect("bundle must trail");
        assert!(injector_at < context_at && context_at < bundle_at);
        for descriptor in [&VENCORD_MOD, &EQUICORD_MOD, &CUSTOM_MOD] {
            let script = execution_script_for(descriptor, "/*bundle*/", &injector);
            assert!(
                !script.contains("SHELTER_INJECTOR_PLUGINS"),
                "{} must not carry the injector declaration",
                descriptor.id
            );
            assert!(script.contains("/*bundle*/"));
        }
        // The injector probe script is read-only reporting: no
        // registration, start, fetch, or eval surface.
        assert!(super::INJECTOR_PROBE_SCRIPT.contains("window.remcordRPC"));
        assert!(super::INJECTOR_PROBE_SCRIPT.contains("SHELTER_INJECTOR_PLUGINS"));
        assert!(!super::INJECTOR_PROBE_SCRIPT.contains("addLocalPlugin"));
        assert!(!super::INJECTOR_PROBE_SCRIPT.contains("startPlugin"));
        assert!(!super::INJECTOR_PROBE_SCRIPT.contains("fetch("));
        assert_eq!(super::INJECTOR_PROBE_ENV, "REMCORD_NATIVE_INJECTOR_PROBE");
    }

    #[test]
    fn custom_adapter_requests_only_the_logical_id() {
        // The adapter carries identity only: no custom filename, path,
        // plain-http URL, file-read, sync-IPC, or eval surface may appear
        // anywhere in the injected file. (The three `https://discord.com`
        // entry origins are the required origin gate, asserted below.)
        assert!(super::CUSTOM_ADAPTER_SCRIPT.contains("getModBundle(\"custom\")"));
        assert!(super::CUSTOM_ADAPTER_SCRIPT.contains("__remcordNativeCustomAdapter"));
        assert!(super::CUSTOM_ADAPTER_SCRIPT.contains("__remcordNativeCustom"));
        assert!(super::CUSTOM_ADAPTER_SCRIPT.contains("remcord-native:custom-bootstrap"));
        assert!(super::CUSTOM_ADAPTER_SCRIPT.contains("custom-bootstrap-executed"));
        assert!(super::CUSTOM_ADAPTER_SCRIPT.contains("https://discord.com"));
        assert!(super::CUSTOM_ADAPTER_SCRIPT.contains("https://canary.discord.com"));
        assert!(super::CUSTOM_ADAPTER_SCRIPT.contains("https://ptb.discord.com"));
        for forbidden in [
            "custom.js",
            "custom.css",
            "example.invalid",
            "http://",
            "readFile",
            "sendSync",
            "eval(",
            "new Function",
        ] {
            assert!(
                !super::CUSTOM_ADAPTER_SCRIPT.contains(forbidden),
                "custom adapter must not contain {forbidden:?}"
            );
        }
    }

    #[test]
    fn native_document_lifecycle_allows_one_bootstrap_and_resets_after_navigation() {
        // Every registered mod gets exactly the same lifecycle: one start, one
        // execution, per document, with a clean reset on navigation.
        for descriptor in all_mods() {
            let mut lifecycle = DocumentModLifecycle::default();
            lifecycle.begin_document();
            assert_eq!(lifecycle.generation(), 1);
            assert!(lifecycle.begin_mod(descriptor.id));
            assert!(!lifecycle.begin_mod(descriptor.id), "{}", descriptor.id);
            assert!(lifecycle.record_mod_execution(descriptor.id));
            assert!(
                !lifecycle.record_mod_execution(descriptor.id),
                "{} must not execute twice",
                descriptor.id
            );

            lifecycle.begin_document();
            assert_eq!(lifecycle.generation(), 2);
            assert!(lifecycle.begin_mod(descriptor.id));
            assert_eq!(lifecycle.mod_execution_count(descriptor.id), 0);
        }
        // Independently: three documents, three bootstraps, no cross-talk.
        let mut lifecycle = DocumentModLifecycle::default();
        for generation in 1..=3u64 {
            lifecycle.begin_document();
            assert_eq!(lifecycle.generation(), generation);
            for descriptor in all_mods() {
                assert!(lifecycle.begin_mod(descriptor.id));
                assert!(!lifecycle.begin_mod(descriptor.id));
                assert!(lifecycle.record_mod_execution(descriptor.id));
                assert_eq!(lifecycle.mod_execution_count(descriptor.id), 1);
            }
        }
    }

    #[test]
    fn accepts_only_each_mods_fixed_upstream_hosts() {
        assert!(is_allowed_mod_download_url(
            &VENCORD_MOD,
            "https://github.com/Vendicated/Vencord/releases/download/devbuild/browser.js"
        ));
        assert!(is_allowed_mod_download_url(
            &VENCORD_MOD,
            "https://release-assets.githubusercontent.com/asset/browser.js"
        ));
        // Shelter's fixed source is raw.githubusercontent.com only: the GitHub
        // release hosts Vencord/Equicord use are NOT approved for Shelter, so a
        // redirect cannot substitute a different artifact.
        assert!(is_allowed_mod_download_url(
            &SHELTER_MOD,
            "https://raw.githubusercontent.com/uwu/shelter-builds/main/shelter.js"
        ));
        assert!(!is_allowed_mod_download_url(
            &SHELTER_MOD,
            "https://github.com/uwu/shelter-builds/main/shelter.js"
        ));
        assert!(!is_allowed_mod_download_url(
            &SHELTER_MOD,
            "http://raw.githubusercontent.com/uwu/shelter-builds/main/shelter.js"
        ));
        assert!(!is_allowed_mod_download_url(
            &SHELTER_MOD,
            "https://raw.githubusercontent.com.attacker.invalid/uwu/shelter.js"
        ));
        // And Vencord does not inherit Shelter's host.
        assert!(!is_allowed_mod_download_url(
            &VENCORD_MOD,
            "https://raw.githubusercontent.com/uwu/shelter-builds/main/shelter.js"
        ));
        for descriptor in all_mods() {
            assert!(!is_allowed_mod_download_url(
                descriptor,
                "http://github.com/x/browser.js"
            ));
            assert!(!is_allowed_mod_download_url(
                descriptor,
                "https://github.com.attacker.invalid/browser.js"
            ));
            assert!(!is_allowed_mod_download_url(
                descriptor,
                "https://badges.example.invalid/badges.json"
            ));
        }
    }

    #[test]
    fn validates_bundle_sizes_and_execution_markers() {
        let bundle = validate_mod_bundle(
            &VENCORD_MOD,
            b"// Vencord test-build\nvar Vencord = {};\n".to_vec(),
            Some(b"/* test CSS */".to_vec()),
        )
        .unwrap();
        assert_eq!(bundle.version, "test-build");
        assert_eq!(bundle.js_bytes, bundle.js.len());
        assert_eq!(bundle.css_bytes, Some(bundle.css.as_ref().unwrap().len()));
        assert_eq!(VENCORD_MOD.success_marker, "vencord-bootstrap-executed");
        assert!(validate_mod_bundle(
            &VENCORD_MOD,
            b"var notVencord = {};".to_vec(),
            Some(b"/*css*/".to_vec())
        )
        .is_err());
        // Equicord's banner is its own: a Vencord artifact is refused, and its
        // 40-character commit hash is a valid version token.
        let equicord = validate_mod_bundle(
            &EQUICORD_MOD,
            b"// Equicord 1489c0e2a4435d854d7343a5cfb0abb894772318\nvar Vencord={};".to_vec(),
            Some(b".vc{}".to_vec()),
        )
        .unwrap();
        assert_eq!(equicord.version, "1489c0e2a4435d854d7343a5cfb0abb894772318");
        assert!(validate_mod_bundle(
            &EQUICORD_MOD,
            b"// Vencord b52ed36\nvar Vencord = {};\n".to_vec(),
            Some(b".vc{}".to_vec())
        )
        .is_err());
        // Every registered mod reports a bounded, non-empty version.
        for descriptor in all_mods() {
            assert!(descriptor.success_marker.len() <= 64);
            assert!(!descriptor.success_marker.is_empty());
        }
    }

    #[test]
    fn generates_fixed_css_and_marker_scripts_without_an_eval_surface() {
        let vencord_css = VENCORD_MOD.css.unwrap();
        let css = mod_css_script(
            &VENCORD_MOD,
            &vencord_css,
            "request-1",
            r#"body::after{content:"ok"}"#,
        )
        .unwrap();
        assert!(css.contains("document.head"));
        assert!(css.contains("remcord-native-vencord-css"));
        assert!(css.contains("vencord_css_result"));
        assert!(css.contains(r#"body::after{content:\"ok\"}"#));

        let context = mod_execution_context_script(&VENCORD_MOD);
        assert!(context.contains("VencordSettings"));
        assert!(context.contains("__remcordNativeVencordStorageAtExecution"));

        let marker = mod_marker_script(&VENCORD_MOD, "request-1", 1.5).unwrap();
        assert!(marker.contains("globalThis.Vencord"));
        assert!(marker.contains("local_storage_api"));
        assert!(marker.contains(VENCORD_MOD.success_marker));
        assert!(!marker.contains("eval("));
        assert!(!marker.contains("new Function"));

        // Equicord's CSS uses its OWN stable identity and result event, and the
        // generic managed CSS substrate is reused rather than reimplemented.
        let equicord_css = EQUICORD_MOD.css.unwrap();
        let equicord_script =
            mod_css_script(&EQUICORD_MOD, &equicord_css, "request-1", ".vc{}").unwrap();
        assert!(equicord_script.contains("remcord-native-equicord-css"));
        // Equicord reports through the MOD-AGNOSTIC CSS event family and
        // echoes its own host-supplied id, so the host can refuse a result
        // claimed for another mod. Vencord's legacy event name is untouched.
        assert!(equicord_script.contains("mod_css_result"));
        assert!(equicord_script.contains("mod_id: \"equicord\""));
        assert!(!equicord_script.contains("remcord-native-vencord-css"));
        assert!(!equicord_script.contains("vencord_css_result"));
        // Shelter declares no stylesheet, so no CSS script is ever built for it.
        assert!(SHELTER_MOD.css.is_none());
        // No generated script for ANY registered mod exposes an eval surface.
        for descriptor in all_mods() {
            let context = mod_execution_context_script(descriptor);
            let marker = mod_marker_script(descriptor, "request-1", 1.0).unwrap();
            for script in [context, marker] {
                assert!(!script.contains("eval("), "{}", descriptor.id);
                assert!(!script.contains("new Function"), "{}", descriptor.id);
                assert!(!script.contains("executeScript"), "{}", descriptor.id);
            }
        }
    }

    #[test]
    fn navigation_is_limited_to_https_discord_hosts() {
        assert!(is_allowed_navigation("https://discord.com/app"));
        assert!(is_allowed_navigation("https://canary.discord.com/app"));
        assert!(!is_allowed_navigation("http://discord.com/app"));
        assert!(!is_allowed_navigation(
            "https://discord.com.attacker.invalid/app"
        ));
        assert!(!is_allowed_navigation("https://example.com/app"));
    }

    #[test]
    fn bridge_origin_trusts_exactly_the_three_entry_hosts() {
        // The channel setting selects which entry host the app runs on, so
        // the bridge must initialize on stable, canary, and ptb alike.
        for url in [
            "https://discord.com/app",
            "https://canary.discord.com/app",
            "https://ptb.discord.com/app",
            "HTTPS://CANARY.DISCORD.COM/app",
        ] {
            assert!(
                is_trusted_bridge_origin(&url.parse().unwrap()),
                "{url} must be trusted"
            );
        }
        // Everything else still denies: ports, userinfo, lookalikes,
        // non-entry subdomains, other schemes, other hosts.
        for url in [
            "https://canary.discord.com:443/app",
            "https://discord.com:443/app",
            "https://evil@discord.com/app",
            "https://evil@canary.discord.com/app",
            "https://discord.com.attacker.invalid/app",
            "https://evil.discord.com/app",
            "https://deep.sub.discord.com/app",
            "http://discord.com/app",
            "https://example.com/app",
        ] {
            assert!(
                !is_trusted_bridge_origin(&url.parse().unwrap()),
                "{url} must not be trusted"
            );
        }
    }

    #[test]
    fn settings_requests_require_an_entry_discord_origin() {
        // The origin gate runs before command dispatch, so a settings
        // capability request from any other origin never reaches the
        // settings reader, including lookalike hosts.
        assert!(is_trusted_bridge_origin(
            &"https://discord.com/app".parse().unwrap()
        ));
        assert!(!is_trusted_bridge_origin(
            &"https://discord.com.attacker.invalid/app".parse().unwrap()
        ));
        assert!(!is_trusted_bridge_origin(
            &"https://example.com/app".parse().unwrap()
        ));
    }

    #[test]
    fn boot_snapshot_grants_exactly_the_settings_capability() {
        use crate::protocol::PROTOCOL_VERSION;
        assert_eq!(super::BOOT_SNAPSHOT.protocol_version, PROTOCOL_VERSION);
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"settings.get_config"));
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"settings.set_config"));
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"settings.getConfig"));
    }

    #[test]
    fn initialization_script_exposes_async_settings_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("settings_get_config"));
        assert!(script.contains("settings_config"));
        assert!(script.contains("settings.get_config"));
        assert!(script.contains("getConfig"));
        // The facade must be asynchronous: no synchronous IPC emulation.
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn localization_requests_require_an_entry_discord_origin() {
        // The origin gate runs before command dispatch, so a localization
        // request from any other origin never reaches the catalog lookup.
        assert!(is_trusted_bridge_origin(
            &"https://discord.com/app".parse().unwrap()
        ));
        assert!(!is_trusted_bridge_origin(
            &"https://discord.com.attacker.invalid/app".parse().unwrap()
        ));
        assert!(!is_trusted_bridge_origin(
            &"https://example.com/app".parse().unwrap()
        ));
    }

    #[test]
    fn boot_snapshot_grants_exactly_the_localization_capability() {
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"localization.get_lang"));
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"localization.set_lang"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"getLang"));
    }

    #[test]
    fn initialization_script_exposes_async_get_lang_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("localization_get_lang"));
        assert!(script.contains("localization_string"));
        assert!(script.contains("localization.get_lang"));
        assert!(script.contains("getLang"));
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn boot_snapshot_grants_exactly_the_mutation_capability() {
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"settings.set_block_power_saving_in_voice_chat"));
        // No generic mutation surface may exist alongside the allowlist.
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"settings.set_config"));
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"settings.setConfig"));
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"settings.write"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
    }

    #[test]
    fn boot_snapshot_grants_diagnostics_and_mod_cache_capabilities() {
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"diagnostics.get_info"));
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"graphics.get_info"));
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"mod.clear_cache"));
        // No generic surface may exist alongside them: no clipboard handle,
        // no file deletion selector, no bundle-reader, no eval, and no
        // renderer-selected CDP method (the two probed methods are fixed).
        for capability in [
            "diagnostics.set",
            "diagnostics.get_all",
            "graphics.call_method",
            "graphics.get_all",
            "mod.delete_bundle",
            "mod.get_bundle_source",
            "clipboard",
            "filesystem",
            "invoke",
            "eval",
        ] {
            assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&capability));
        }
    }

    #[test]
    fn boot_snapshot_grants_spellcheck_capability() {
        // F18: exactly the read-only state capability. No setter, no
        // language-list writer, no session handle may exist alongside it:
        // languages resolve host-side from the authoritative snapshot.
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"spellcheck.get_state"));
        for capability in [
            "spellcheck.set",
            "spellcheck.set_enabled",
            "spellcheck.set_languages",
            "spellcheck.get_all",
            "session.set_languages",
        ] {
            assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&capability));
        }
    }

    #[test]
    fn startup_channel_resolution_matches_redirect_parity() {
        // The channel setting selects the entry URL the host navigates to
        // (redirect.html parity); unknown values fail safe to stable.
        assert_eq!(
            crate::channel::resolve_channel_url("stable"),
            "https://discord.com/app"
        );
        assert_eq!(
            crate::channel::resolve_channel_url("canary"),
            "https://canary.discord.com/app"
        );
        assert_eq!(
            crate::channel::resolve_channel_url("ptb"),
            "https://ptb.discord.com/app"
        );
        assert_eq!(
            crate::channel::resolve_channel_url("beta"),
            "https://discord.com/app"
        );
        // Every resolved URL is navigable and carries a trusted bridge.
        for channel in ["stable", "canary", "ptb"] {
            let url = crate::channel::resolve_channel_url(channel);
            assert!(super::is_allowed_navigation(url), "{channel}");
            assert!(
                super::is_trusted_bridge_origin(&url.parse().unwrap()),
                "{channel}"
            );
        }
    }

    #[test]
    fn initialization_script_exposes_async_mutation_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("settings_set_block_power_saving"));
        assert!(script.contains("settings_mutation"));
        assert!(script.contains("settings.set_block_power_saving_in_voice_chat"));
        assert!(script.contains("setBlockPowerSavingInVoiceChat"));
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn mutation_smoke_template_restores_only_the_allowlisted_value() {
        // The restore script carries exactly one host-formatted boolean;
        // it cannot name a key, path, or second field.
        assert!(super::SETTINGS_MUTATION_RESTORE_SCRIPT_TEMPLATE
            .contains("__REMCORD_MUTATION_RESTORE_VALUE__"));
        assert!(super::SETTINGS_MUTATION_RESTORE_SCRIPT_TEMPLATE
            .contains("setBlockPowerSavingInVoiceChat(RESTORE_TO)"));
        assert!(!super::SETTINGS_MUTATION_RESTORE_SCRIPT_TEMPLATE.contains("setConfig("));
        let rendered = super::SETTINGS_MUTATION_RESTORE_SCRIPT_TEMPLATE
            .replace("__REMCORD_MUTATION_RESTORE_VALUE__", "false");
        assert!(!rendered.contains("__REMCORD_MUTATION_RESTORE_VALUE__"));
        assert!(rendered.contains("const RESTORE_TO = false;"));
    }

    #[test]
    fn boot_snapshot_grants_exactly_the_settings_set_capability() {
        assert!(super::BOOT_SNAPSHOT.capabilities.contains(&"settings.set"));
        // No generic settings surface may exist alongside the allowlist.
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"settings.set_config"));
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"settings.setConfig"));
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"settings.write"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
    }

    #[test]
    fn initialization_script_exposes_async_settings_set_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("settings_set"));
        assert!(script.contains("settings.set"));
        assert!(!script.contains("sendSync"));
        assert!(!script.contains("settings_set_config"));
    }

    #[test]
    fn settings_set_smoke_template_stays_within_the_allowlist() {
        // Both scripts exercise exactly one allowlisted wire key and the
        // closed `set` facade; they cannot name a path, URL, second key, or
        // generic config call.
        for template in [
            super::SETTINGS_SET_SMOKE_SCRIPT,
            super::SETTINGS_SET_RESTORE_SCRIPT_TEMPLATE,
        ] {
            assert!(template.contains("support_banner_dismissed"));
            assert!(template.contains("settings.set("));
            assert!(template.contains("settings_set_probe"));
            assert!(!template.contains("setConfig("));
            assert!(!template.contains("sendSync"));
            assert!(!template.contains("localStorage"));
        }
        assert!(super::SETTINGS_SET_RESTORE_SCRIPT_TEMPLATE
            .contains("__REMCORD_SETTINGS_SET_RESTORE_VALUE__"));
        let rendered = super::SETTINGS_SET_RESTORE_SCRIPT_TEMPLATE
            .replace("__REMCORD_SETTINGS_SET_RESTORE_VALUE__", "false");
        assert!(!rendered.contains("__REMCORD_SETTINGS_SET_RESTORE_VALUE__"));
        assert!(rendered.contains("const RESTORE_TO = false;"));
    }

    #[test]
    fn boot_snapshot_grants_exactly_the_app_version_capability() {
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"app.get_version"));
        // No generic version/channel surface may exist alongside the typed
        // read.
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"app.getVersion"));
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"get-app-version"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
    }

    #[test]
    fn initialization_script_exposes_async_app_version_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("app_get_version"));
        assert!(script.contains("app_version"));
        assert!(script.contains("app.get_version"));
        assert!(script.contains("getVersion"));
        // The facade must be asynchronous: no synchronous IPC emulation.
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn app_version_smoke_probe_uses_only_the_typed_capability() {
        assert!(super::APP_VERSION_SMOKE_SCRIPT.contains("native?.getVersion"));
        assert!(super::APP_VERSION_SMOKE_SCRIPT.contains("app_version_probe"));
        assert!(!super::APP_VERSION_SMOKE_SCRIPT.contains("sendSync"));
        assert!(!super::APP_VERSION_SMOKE_SCRIPT.contains("remcord.version"));
    }

    #[test]
    fn graphics_smoke_probe_uses_only_the_typed_capability() {
        assert!(super::GRAPHICS_SMOKE_SCRIPT.contains("graphics?.getInfo"));
        assert!(super::GRAPHICS_SMOKE_SCRIPT.contains("graphics_probe"));
        assert!(super::GRAPHICS_SMOKE_SCRIPT.contains("device_summary"));
        assert!(super::GRAPHICS_SMOKE_SCRIPT.contains("UNMASKED_RENDERER_WEBGL"));
        assert!(!super::GRAPHICS_SMOKE_SCRIPT.contains("sendSync"));
        assert!(!super::GRAPHICS_SMOKE_SCRIPT.contains("commandLine"));
        assert!(!super::GRAPHICS_SMOKE_SCRIPT.contains("user-data-dir"));
    }

    #[test]
    fn embedded_app_version_keeps_a_servable_shape() {
        // The startup gate serves this constant on every request.
        assert!(super::validate_version(super::current_app_version()).is_ok());
    }

    #[test]
    fn boot_snapshot_grants_exactly_the_plugin_storage_capabilities() {
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"plugin_storage.read"));
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"plugin_storage.write"));
        // No generic filesystem surface may exist alongside the scoped pair.
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"fs.readFile"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"fs.writeFile"));
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"plugin_storage"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
    }

    #[test]
    fn initialization_script_exposes_namespaced_plugin_storage_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("plugin_storage_read"));
        assert!(script.contains("plugin_storage_write"));
        assert!(script.contains("plugin_storage.read"));
        assert!(script.contains("plugin_storage.write"));
        assert!(script.contains("pluginStorage"));
        // The facade must be asynchronous and namespaced: no generic fs
        // object and no synchronous IPC emulation.
        assert!(!script.contains("window.remcord.fs"));
        assert!(!script.contains("remcord.fs"));
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn boot_snapshot_grants_no_theme_capability() {
        // Theme injection is a host push: the renderer never requests theme
        // CSS, so no theme-CSS pull capability may exist. `folders.open_theme`,
        // `themes.install_code`, `themes.write_css`, `themes.import_files`,
        // `themes.set_enabled`, and `themes.list` are explicitly allowed: the
        // first names the themes ROOT for folder opening, the rest write or
        // inventory theme files the host then pushes itself — none reads
        // theme CSS back to the renderer.
        for capability in super::BOOT_SNAPSHOT.capabilities {
            if *capability == "folders.open_theme"
                || *capability == "themes.install_code"
                || *capability == "themes.write_css"
                || *capability == "themes.import_files"
                || *capability == "themes.set_enabled"
                || *capability == "themes.list"
            {
                continue;
            }
            assert!(!capability.starts_with("themes."));
            assert!(!capability.contains("theme"));
        }
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
    }

    #[test]
    fn initialization_script_exposes_no_theme_pull_surface() {
        // The earlier theme READ investigation was intentionally rejected:
        // current themes are host -> renderer pushes with no native renderer
        // caller for a pull-style read. The facade must not grow one here.
        // ("themes" as a bare word now legitimately appears as the closed
        // folder-opening location for the themes ROOT — validated id,
        // host-built path, OS shell — so the pull surface is guarded by the
        // specific CSS tokens below, not by the bare word.)
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(!script.contains("getCSS"));
        assert!(!script.contains("getCss"));
        assert!(!script.contains("addTheme"));
        assert!(!script.contains("removeTheme"));
        // The theme push stays push-only, but the QuickCSS editor pair is
        // the deliberate exception (see
        // `boot_snapshot_grants_exactly_the_quickcss_editor_capabilities`):
        // exactly the fixed-file read/write, nothing else quickcss-shaped.
        assert!(script.contains("quickcss.read"));
        assert!(script.contains("quickcss.write"));
        assert!(!script.contains("quickCss.get"));
        assert!(!script.contains("quickcss.delete"));
        assert!(!script.contains("quickcss.list"));
        assert!(!script.contains("remcord-native-theme"));
    }

    #[test]
    fn themes_smoke_observation_only_reads_managed_dom_nodes() {
        assert!(super::THEMES_SMOKE_SCRIPT.contains("remcord-native-theme-"));
        assert!(super::THEMES_SMOKE_SCRIPT.contains("querySelectorAll"));
        assert!(super::THEMES_SMOKE_SCRIPT.contains("remcord-native-vencord-css"));
        assert!(super::THEMES_SMOKE_SCRIPT.contains("remcord-quick-css"));
        assert!(super::THEMES_SMOKE_SCRIPT.contains("order-probe"));
        // Null-safe in a headless early document: a missing head yields an
        // empty order instead of throwing.
        assert!(super::THEMES_SMOKE_SCRIPT.contains("document.head"));
        // Observation only: no bridge request, no event post, no eval.
        assert!(!super::THEMES_SMOKE_SCRIPT.contains("postMessage"));
        assert!(!super::THEMES_SMOKE_SCRIPT.contains("getCSS"));
        assert!(!super::THEMES_SMOKE_SCRIPT.contains("eval("));
    }

    #[test]
    fn boot_snapshot_grants_exactly_the_quickcss_editor_capabilities() {
        // Quick CSS stays a host push, but the in-Discord editor needs its
        // narrow read/write pair: exactly these two capabilities may exist,
        // and nothing else quickcss-shaped.
        assert!(super::BOOT_SNAPSHOT.capabilities.contains(&"quickcss.read"));
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"quickcss.write"));
        for capability in super::BOOT_SNAPSHOT.capabilities {
            if *capability == "quickcss.read" || *capability == "quickcss.write" {
                continue;
            }
            assert!(!capability.contains("quickcss"));
            assert!(!capability.contains("quick_css"));
            assert!(!capability.contains("quickCss"));
        }
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
    }

    #[test]
    fn initialization_script_exposes_only_the_quickcss_editor_pair() {
        // The editor pair is the only renderer-reachable Quick CSS surface:
        // no generic pull, no path, no third operation.
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("quickcss.read"));
        assert!(script.contains("quickcss.write"));
        assert!(!script.contains("quickCss.get"));
        assert!(!script.contains("quickcss.delete"));
        assert!(!script.contains("quickcss.list"));
    }

    #[test]
    fn live_css_watching_adds_no_capability_or_renderer_surface() {
        // Live watching is a host-owned lifecycle: two native watchers with
        // no bridge command, no boot capability, no facade member, and no
        // renderer-supplied path anywhere near this milestone.
        // (`themes.write_css` names CSS but is a file-writing capability
        // consumed BY the watchers, not a watching surface itself.)
        for capability in super::BOOT_SNAPSHOT.capabilities {
            assert!(!capability.contains("watch"));
            assert!(!capability.contains("live"));
            if *capability == "themes.write_css"
                || *capability == "quickcss.read"
                || *capability == "quickcss.write"
            {
                continue;
            }
            assert!(!capability.contains("css"));
        }
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(!script.contains("watch"));
        assert!(!script.contains("LiveCss"));
        assert!(!script.contains("fs.watch"));
        assert!(!script.contains("MutationObserver"));
        // Exactly one watcher set for the whole application lifetime.
        assert_eq!(crate::css_watch::LiveCssWatchers::watcher_count(), 2);
        assert_eq!(crate::css_watch::CSS_DEBOUNCE_MS, 300);
    }

    #[test]
    fn plugin_storage_smoke_probe_uses_only_the_typed_capability() {
        assert!(super::PLUGIN_STORAGE_SMOKE_SCRIPT.contains("pluginStorage"));
        assert!(super::PLUGIN_STORAGE_SMOKE_SCRIPT.contains("plugin_storage_probe"));
        assert!(super::PLUGIN_STORAGE_SMOKE_SCRIPT.contains("remcordNativeProbe"));
        assert!(super::PLUGIN_STORAGE_SMOKE_SCRIPT.contains("../escape.txt"));
        assert!(!super::PLUGIN_STORAGE_SMOKE_SCRIPT.contains("sendSync"));
        // The probe asserts the generic facade is ABSENT (`window.remcord.fs
        // !== undefined` check); what must never appear is a generic fs
        // method call.
        assert!(!super::PLUGIN_STORAGE_SMOKE_SCRIPT.contains("fs.readFile"));
        assert!(!super::PLUGIN_STORAGE_SMOKE_SCRIPT.contains("fs.writeFile"));
        assert!(!super::PLUGIN_STORAGE_SMOKE_SCRIPT.contains("C:\\Users"));
    }

    #[test]
    fn boot_snapshot_grants_discovery_save_and_restore_for_backups() {
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"backup.list_bundles"));
        assert!(super::BOOT_SNAPSHOT.capabilities.contains(&"backup.save"));
        // Restore is preview-then-commit: host-mediated open dialog plus
        // bounded validate / stage (inspect), an explicit commit, and a
        // discard — the renderer supplies nothing at any step.
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"backup.inspect"));
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"backup.commit_restore"));
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"backup.discard_restore"));
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"backup.restore"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"backup"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"dialog"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"zip"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
    }

    #[test]
    fn initialization_script_exposes_namespaced_backup_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("bundles_list"));
        // P0.4: the facade sends the exact protocol wire type. The old
        // `backup_list_bundles` string is rejected by the host and must not
        // appear anywhere in shipped script.
        assert!(!script.contains("backup_list_bundles"));
        assert!(script.contains("backup.list_bundles"));
        assert!(script.contains("listBundles"));
        // Save (D4-1): the Electron-compatible JSON-string save with the
        // long-operation timeout and the bounded save outcome.
        assert!(script.contains("backup_save"));
        assert!(script.contains("backup.save"));
        assert!(script.contains("function saveBackup("));
        assert!(script.contains("backup_saved"));
        assert!(script.contains("120_000"));
        // Restore is preview-then-commit: the parameter-free inspect with
        // the long-operation timeout and the bounded preview, the explicit
        // commit with the bounded clientMods outcome, and the discard. No
        // open-dialog send, no sync IPC, and no generic invocation may ride
        // along. The single-call restore wire type is gone.
        assert!(script.contains("backup_inspect"));
        assert!(script.contains("backup.inspect"));
        assert!(script.contains("function inspectBackup("));
        assert!(script.contains("backup_inspected"));
        assert!(script.contains("backup_commit_restore"));
        assert!(script.contains("backup.commit_restore"));
        assert!(script.contains("function commitRestore("));
        assert!(script.contains("backup_restored"));
        assert!(script.contains("backup_discard_restore"));
        assert!(script.contains("backup.discard_restore"));
        assert!(script.contains("backup_restore_discarded"));
        assert!(!script.contains("backup.restore"));
        assert!(!script.contains("function restoreBackup("));
        assert!(!script.contains("showOpenDialog"));
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn backup_save_runs_on_a_dedicated_named_worker() {
        // The modal save dialog must never run on the WebView/UI thread: the
        // save contract lives on BACKUP_SAVE_WORKER_NAME's documented worker.
        assert_eq!(super::BACKUP_SAVE_WORKER_NAME, "remcord-backup-save-sta");
        assert_ne!(
            super::BACKUP_SAVE_WORKER_NAME,
            super::THEME_IMPORT_WORKER_NAME
        );
    }

    #[test]
    fn backup_save_smoke_probe_uses_only_the_save_capability() {
        assert!(super::BACKUP_SAVE_SMOKE_SCRIPT.contains("backup.save"));
        assert!(super::BACKUP_SAVE_SMOKE_SCRIPT.contains("backup_save_probe"));
        assert!(super::BACKUP_SAVE_SMOKE_SCRIPT.contains("CANCELLED"));
        assert!(super::BACKUP_SAVE_SMOKE_SCRIPT.contains("stage"));
        // Save and restore are sibling capabilities now: the save probe never
        // drives restore (no restore wire type, open dialog, fetch, or sync
        // IPC may appear in it).
        assert!(!super::BACKUP_SAVE_SMOKE_SCRIPT.contains("backup_restore"));
        assert!(!super::BACKUP_SAVE_SMOKE_SCRIPT.contains("showOpenDialog"));
        assert!(!super::BACKUP_SAVE_SMOKE_SCRIPT.contains("fetch("));
        assert!(!super::BACKUP_SAVE_SMOKE_SCRIPT.contains("sendSync"));
    }

    #[test]
    fn backup_restore_runs_on_a_dedicated_named_worker() {
        // The modal open dialog plus validation/staging/commit must never run
        // on the WebView/UI thread, and the restore lifecycle stays isolated
        // from the save lifecycle.
        assert_eq!(
            super::BACKUP_RESTORE_WORKER_NAME,
            "remcord-backup-restore-sta"
        );
        assert_ne!(
            super::BACKUP_RESTORE_WORKER_NAME,
            super::BACKUP_SAVE_WORKER_NAME
        );
        assert_ne!(
            super::BACKUP_RESTORE_WORKER_NAME,
            super::THEME_IMPORT_WORKER_NAME
        );
    }

    #[test]
    fn backup_restore_smoke_probe_uses_only_the_restore_capability() {
        assert!(super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("backup.inspect"));
        assert!(super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("backup.commitRestore"));
        assert!(super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("backup_restore_probe"));
        assert!(super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("CANCELLED"));
        assert!(super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("stage"));
        assert!(super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("client_mods_bytes"));
        // The single-call restore is gone from the probe: preview first,
        // then an explicit commit of exactly the held staged bytes.
        assert!(!super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("backup.restore"));
        assert!(!super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("backup_restore\""));
        // Parameter-free by protocol: no archive path, filename, URL,
        // filter, or dialog option may appear. The host never applies
        // clientMods: no localStorage write, no remote-plugin fetch, no
        // script execution surface may appear either.
        assert!(!super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("localStorage"));
        assert!(!super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("addRemotePlugin"));
        assert!(!super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("fetch("));
        assert!(!super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("sendSync"));
        assert!(!super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("showOpenDialog"));
    }

    #[test]
    fn native_bundle_allowlist_matches_the_seven_electron_names() {
        // Semantic parity with Electron `BUNDLE_FILES`
        // (`src/common/backup.ts`): same 7 names in the same order.
        // `custom.js` / `custom.css` are inventoried here AND executed from
        // the same host-built profile paths by the custom mod pipeline
        // (`custom_bundle.rs` + the registered `custom` descriptor); the
        // inventory never implies a renderer-supplied path (asserted by the
        // electron-compat suite).
        assert_eq!(
            super::BUNDLE_FILES,
            &[
                "vencord.js",
                "vencord.css",
                "equicord.js",
                "equicord.css",
                "shelter.js",
                "custom.js",
                "custom.css",
            ]
        );
    }

    #[test]
    fn boot_snapshot_grants_only_the_two_folder_capabilities() {
        assert!(super::BOOT_SNAPSHOT.capabilities.contains(&"folders.open"));
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"folders.open_theme"));
        // No generic path-opening, dialog, picker, or shell surface may
        // exist alongside the two scoped capabilities.
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"folders"));
        assert!(!super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"folders.open_path"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"shell"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"dialog"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
    }

    #[test]
    fn initialization_script_exposes_namespaced_folder_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("folders_open"));
        assert!(script.contains("folders_open_theme"));
        assert!(script.contains("folders.open"));
        assert!(script.contains("folders.open_theme"));
        assert!(script.contains("openTheme"));
        // The facade carries identifiers only: no shell path opener, no
        // dialog/picker, no synchronous IPC, no OS surface leakage.
        assert!(!script.contains("openPath("));
        assert!(!script.contains("showItemInFolder"));
        assert!(!script.contains("ShellExecute"));
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn folders_smoke_probe_is_env_gated_like_other_probes() {
        // The DEV-only probe must stay behind its documented env flag and
        // must never run unconditionally.
        assert_eq!(super::FOLDERS_SMOKE_ENV, "REMCORD_NATIVE_FOLDERS_SMOKE");
    }

    #[test]
    fn boot_snapshot_grants_only_theme_install_for_themes() {
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"themes.install_code"));
        // No theme read/pull, URL install, dialog, picker, editor, or
        // generic capability may exist alongside the install command.
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"themes"));
        for capability in super::BOOT_SNAPSHOT.capabilities {
            assert_ne!(*capability, "themes.install");
            assert_ne!(*capability, "themes.get_css");
            assert_ne!(*capability, "themes.dialog");
        }
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"network"));
    }

    #[test]
    fn initialization_script_exposes_namespaced_theme_install_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("theme_install_code"));
        assert!(script.contains("themes.install_code"));
        assert!(script.contains("installFromCode"));
        // CSS text in, metadata out: no URL install, fetch, dialog,
        // picker, editor launch, or synchronous IPC anywhere near it.
        assert!(!script.contains("installBDTheme"));
        assert!(!script.contains("fetch("));
        assert!(!script.contains("showOpenDialog"));
        assert!(!script.contains("ShellExecute"));
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn theme_install_smoke_probe_reports_through_the_typed_probe_event() {
        assert_eq!(
            super::THEME_INSTALL_SMOKE_ENV,
            "REMCORD_NATIVE_THEME_INSTALL_SMOKE"
        );
        assert!(super::THEME_INSTALL_SMOKE_SCRIPT.contains("installFromCode"));
        assert!(super::THEME_INSTALL_SMOKE_SCRIPT.contains("theme_install_probe"));
        assert!(super::THEME_INSTALL_SMOKE_SCRIPT.contains("RemcordSmoke"));
        // The probe installs through the capability and observes one fetch;
        // it never fetches theme code for installation.
        assert!(super::THEME_INSTALL_SMOKE_SCRIPT.contains("fetch_observed"));
        assert!(!super::THEME_INSTALL_SMOKE_SCRIPT.contains("installBDTheme"));
        assert!(!super::THEME_INSTALL_SMOKE_SCRIPT.contains("sendSync"));
    }

    #[test]
    fn boot_snapshot_grants_only_theme_write_for_edits() {
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"themes.write_css"));
        // No editor launch, dialog, picker, URL install, or generic
        // capability may exist alongside the write command.
        for capability in super::BOOT_SNAPSHOT.capabilities {
            assert_ne!(*capability, "themes.edit");
            assert_ne!(*capability, "themes.open_editor");
            assert_ne!(*capability, "themes.install");
        }
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"network"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"shell"));
    }

    #[test]
    fn initialization_script_exposes_namespaced_theme_write_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("theme_write_css"));
        assert!(script.contains("themes.write_css"));
        assert!(script.contains("writeCss"));
        // CSS text in, metadata out: no editor surface, URL install,
        // dialog, picker, fetch, or synchronous IPC anywhere near it.
        assert!(!script.contains("openCssEditor"));
        assert!(!script.contains("installBDTheme"));
        assert!(!script.contains("showOpenDialog"));
        assert!(!script.contains("fetch("));
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn theme_write_smoke_probe_reports_reflection_through_the_typed_probe_event() {
        assert_eq!(
            super::THEME_WRITE_SMOKE_ENV,
            "REMCORD_NATIVE_THEME_WRITE_SMOKE"
        );
        assert!(super::THEME_WRITE_SMOKE_SCRIPT.contains("writeCss"));
        assert!(super::THEME_WRITE_SMOKE_SCRIPT.contains("theme_write_probe"));
        assert!(super::THEME_WRITE_SMOKE_SCRIPT.contains("RemcordWriteProbe"));
        // The probe proves in-WebView reflection and single-node updates;
        // it never launches an editor or fetches code.
        assert!(super::THEME_WRITE_SMOKE_SCRIPT.contains("getComputedStyle"));
        assert!(!super::THEME_WRITE_SMOKE_SCRIPT.contains("openCssEditor"));
        assert!(!super::THEME_WRITE_SMOKE_SCRIPT.contains("installBDTheme"));
        assert!(!super::THEME_WRITE_SMOKE_SCRIPT.contains("sendSync"));
    }

    #[test]
    fn boot_snapshot_grants_theme_import_without_generic_surface() {
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"themes.import_files"));
        // Theme-specific only: no generic dialog, filesystem, path, URL, or
        // handle capability may exist alongside the picker.
        for capability in super::BOOT_SNAPSHOT.capabilities {
            assert_ne!(*capability, "dialog.openFile");
            assert_ne!(*capability, "dialog");
            assert_ne!(*capability, "filesystem.read");
            assert_ne!(*capability, "filesystem.write");
            assert_ne!(*capability, "themes.import");
        }
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"network"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"shell"));
    }

    #[test]
    fn initialization_script_exposes_namespaced_theme_import_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("theme_import_files"));
        assert!(script.contains("themes.import_files"));
        assert!(script.contains("importFiles"));
        // Host-owned picker only: the facade sends an exact empty payload
        // with a modal-picker timeout. No path, URL, filter, dialog-option,
        // legacy dialog API, fetch, or synchronous IPC anywhere near it.
        assert!(!script.contains("dialog.openFile"));
        assert!(!script.contains("showOpenDialog"));
        assert!(!script.contains("GetOpenFileName"));
        assert!(!script.contains("installBDTheme"));
        assert!(!script.contains("fetch("));
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn theme_import_runs_on_a_dedicated_named_worker() {
        // The modal picker must never run on the WebView/UI thread: one
        // named worker owns the dialog plus the sequential ingestion. The
        // name is pinned so logs stay greppable; the single-COM-init STA
        // contract lives on THEME_IMPORT_WORKER_NAME's documented worker.
        assert_eq!(super::THEME_IMPORT_WORKER_NAME, "remcord-theme-import-sta");
    }

    #[test]
    fn theme_import_smoke_probe_opens_the_real_picker_through_the_capability() {
        assert_eq!(
            super::THEME_IMPORT_SMOKE_ENV,
            "REMCORD_NATIVE_THEME_IMPORT_SMOKE"
        );
        assert!(super::THEME_IMPORT_SMOKE_SCRIPT.contains("importFiles"));
        assert!(super::THEME_IMPORT_SMOKE_SCRIPT.contains("theme_import_probe"));
        assert!(super::THEME_IMPORT_SMOKE_SCRIPT.contains("canceled"));
        // The probe opens the picker and reports the bounded summary; it
        // never passes a path/URL/filter, fetches code, or uses sync IPC.
        // (The single `openImportPicker` mention is the must-not-exist
        // absence check, mirroring the install probe's `install` guard.)
        assert!(super::THEME_IMPORT_SMOKE_SCRIPT.contains("stage"));
        assert!(!super::THEME_IMPORT_SMOKE_SCRIPT.contains("installBDTheme"));
        assert!(!super::THEME_IMPORT_SMOKE_SCRIPT.contains("fetch("));
        assert!(!super::THEME_IMPORT_SMOKE_SCRIPT.contains("sendSync"));
    }

    #[test]
    fn boot_snapshot_grants_theme_enable_without_generic_surface() {
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"themes.set_enabled"));
        // Flip-only: no uninstall, editor, URL install, or generic
        // filesystem/settings capability may exist alongside it.
        // (`themes.list` is the separate read-only inventory slice.)
        for capability in super::BOOT_SNAPSHOT.capabilities {
            assert_ne!(*capability, "themes.set");
            assert_ne!(*capability, "themes.uninstall");
            assert_ne!(*capability, "filesystem.write");
        }
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"network"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"shell"));
    }

    #[test]
    fn initialization_script_exposes_namespaced_theme_enable_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("theme_set_enabled"));
        assert!(script.contains("themes.set_enabled"));
        assert!(script.contains("setEnabled"));
        // Id plus boolean only: no path, filename, manifest, CSS, URL,
        // untyped set surface, editor launch, fetch, or synchronous IPC.
        // ("themes.set_enabled" never contains "themes.set(", so the
        // untyped-send guard below stays exact.)
        assert!(!script.contains("themes.set("));
        assert!(!script.contains("openCssEditor"));
        assert!(!script.contains("installBDTheme"));
        assert!(!script.contains("fetch("));
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn theme_set_enabled_smoke_probe_flips_through_the_capability() {
        assert_eq!(
            super::THEME_SET_ENABLED_SMOKE_ENV,
            "REMCORD_NATIVE_THEME_SET_ENABLED_SMOKE"
        );
        assert!(super::THEME_SET_ENABLED_SMOKE_SCRIPT.contains("setEnabled"));
        assert!(super::THEME_SET_ENABLED_SMOKE_SCRIPT.contains("theme_set_enabled_probe"));
        assert!(super::THEME_SET_ENABLED_SMOKE_SCRIPT.contains("getComputedStyle"));
        assert!(super::THEME_SET_ENABLED_SMOKE_SCRIPT.contains("clearvision"));
        // The probe flips flags and observes managed nodes; it never takes
        // a path, edits CSS, launches an editor, or uses sync IPC. (The
        // single `themes.set` mention is the must-not-exist absence check.)
        assert!(!super::THEME_SET_ENABLED_SMOKE_SCRIPT.contains("installBDTheme"));
        assert!(!super::THEME_SET_ENABLED_SMOKE_SCRIPT.contains("openCssEditor"));
        assert!(!super::THEME_SET_ENABLED_SMOKE_SCRIPT.contains("fetch("));
        assert!(!super::THEME_SET_ENABLED_SMOKE_SCRIPT.contains("sendSync"));
    }

    #[test]
    fn boot_snapshot_grants_theme_list_without_generic_surface() {
        assert!(super::BOOT_SNAPSHOT.capabilities.contains(&"themes.list"));
        // Metadata-only inventory: no CSS pull, no paths, no update URLs,
        // and no generic filesystem/listing capability alongside it.
        for capability in super::BOOT_SNAPSHOT.capabilities {
            assert_ne!(*capability, "themes.get_css");
            assert_ne!(*capability, "filesystem.list");
            assert_ne!(*capability, "directory.list");
        }
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"network"));
    }

    #[test]
    fn initialization_script_exposes_namespaced_theme_list_facade() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("themes_list"));
        assert!(script.contains("themes.list"));
        assert!(script.contains("listThemes"));
        // Metadata in, nothing out to the filesystem: no CSS contents, no
        // paths, no URL/update surface, no generic listing, no sync IPC.
        // ("themes.list" never contains "themes.get", so the pull-surface
        // guards below stay exact.)
        assert!(!script.contains("getCSS"));
        assert!(!script.contains("getCss"));
        assert!(!script.contains("updateSrc"));
        assert!(!script.contains("remcord.fs"));
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn theme_list_smoke_probe_inventories_through_the_capability() {
        assert_eq!(
            super::THEME_LIST_SMOKE_ENV,
            "REMCORD_NATIVE_THEME_LIST_SMOKE"
        );
        assert!(super::THEME_LIST_SMOKE_SCRIPT.contains("listThemes"));
        assert!(super::THEME_LIST_SMOKE_SCRIPT.contains("theme_list_probe"));
        assert!(super::THEME_LIST_SMOKE_SCRIPT.contains("clearvision"));
        // The probe inventories metadata and checks the display shape; it
        // never reads CSS, paths, or the filesystem, and never uses sync
        // IPC. (The `getThemes`/`getCSS` mentions are must-not-exist
        // absence checks.)
        assert!(!super::THEME_LIST_SMOKE_SCRIPT.contains("installBDTheme"));
        assert!(!super::THEME_LIST_SMOKE_SCRIPT.contains("fetch("));
        assert!(!super::THEME_LIST_SMOKE_SCRIPT.contains("sendSync"));
    }

    #[test]
    fn shelter_loader_probe_covers_only_the_public_loader_surface() {
        assert_eq!(
            super::SHELTER_LOADER_PROBE_ENV,
            "REMCORD_NATIVE_LOADER_PROBE"
        );
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("shelter_loader_probe"));
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("addLocalPlugin"));
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("startPlugin"));
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("stopPlugin"));
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("removePlugin"));
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("remcord-loader-probe"));
        assert!(!super::SHELTER_LOADER_PROBE_SCRIPT.contains("installedPlugins"));
        assert!(!super::SHELTER_LOADER_PROBE_SCRIPT.contains("loadedPlugins"));
        // The throw-chain reads no reactive store: Shelter serves registry
        // snapshots through deferred getters whose synchronous freshness
        // inside one evaluation cannot be relied on, so the probe verifies
        // the cycle purely through upstream throw behavior (taken, missing,
        // already-running).
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("ensureLoaderPlugin"));
        // The probe asserts the URL-based loader is ABSENT (property check
        // only) and round-trips one synthetic host-fixed plugin: it never
        // fetches, never evals itself, never touches sync IPC, a custom
        // protocol, or the filesystem, and carries no source in its report.
        // (`ensureLoaderPlugin` appears only in the absence check, never as
        // a call.)
        assert!(!super::SHELTER_LOADER_PROBE_SCRIPT.contains("ensureLoaderPlugin("));
        assert!(!super::SHELTER_LOADER_PROBE_SCRIPT.contains("fetch("));
        assert!(!super::SHELTER_LOADER_PROBE_SCRIPT.contains("eval("));
        assert!(!super::SHELTER_LOADER_PROBE_SCRIPT.contains("Function("));
        assert!(!super::SHELTER_LOADER_PROBE_SCRIPT.contains("sendSync"));
        assert!(!super::SHELTER_LOADER_PROBE_SCRIPT.contains("remcord://"));
        assert!(!super::SHELTER_LOADER_PROBE_SCRIPT.contains("plugin.js"));
    }

    #[test]
    fn navigation_observer_coexists_with_wry_page_load_path() {
        // Phase 2B is a SECOND raw-COM `NavigationCompleted` subscription:
        // wry's own page-load callback must keep firing `PageLoadFinished`
        // when the observer is registered. Phase 2F factors the main
        // WebView's wry wiring plus both observers into the host-owned
        // helper; the child keeps its direct registrations. Both
        // registrations stay present in the shipped wiring, and the
        // observer adds no reload path.
        let main_source = include_str!("main.rs");
        let helper_source = include_str!("main_webview.rs");
        // The main WebView builds exactly once through the helper, which
        // owns the wry page-load subscription and both observer attach /
        // detach entry points.
        assert!(main_source.contains("main_webview::main_webview_builder"));
        assert!(main_source.contains("main_webview::attach_main_observers"));
        assert!(main_source.contains("main_webview::detach_main_observers"));
        assert!(helper_source.contains("with_on_page_load_handler"));
        assert!(main_source.contains("NativeEvent::PageLoadFinished"));
        // The raw-COM observer is additive, for main (helper) and child
        // (direct). The child path is untouched by the refactor.
        assert!(helper_source.contains("navigation_completed::register"));
        assert!(main_source.contains("navigation_completed::register"));
        assert!(main_source.contains("navigation_completed::unregister"));
        assert!(main_source.contains("NativeEvent::NavigationCompleted"));
        assert!(main_source.contains("remove_NavigationCompleted"));
        // No new reload path: the single host-side `reload()` still belongs
        // to the Phase 2A recovery decision only. (Needles are joined at
        // runtime so this assertion's own source text cannot self-count.)
        let reload_needle = ["match webview", "reload()"].join(".");
        assert_eq!(main_source.matches(&reload_needle).count(), 1);
        assert_eq!(helper_source.matches(&reload_needle).count(), 0);
        // The navigation event never reaches the recovery budget: the only
        // policy decision still feeds on the process-failure record.
        let decide_needle = ["main_recovery", "decide"].join(".");
        assert_eq!(main_source.matches(&decide_needle).count(), 1);
    }

    #[test]
    fn trusted_load_gating_wiring() {
        // Phase 2C: the trusted Discord pipeline runs only for a proven
        // successful navigation. Wry's page-load subscription and the
        // recovery path are untouched; the gate pairs finishes with raw
        // outcomes by generation; failures only skip (no reload/retry).
        // Phase 2F threads the helper-owned instance id through the gate
        // and recovery instead of a hardcoded constant (same value, 0).
        let source = include_str!("main.rs");
        let helper_source = include_str!("main_webview.rs");
        // Wry's callbacks live in the per-instance helper now and stay
        // registered and functional.
        assert!(helper_source.contains("with_on_page_load_handler"));
        assert!(source.contains("NativeEvent::PageLoadFinished"));
        // Every navigation attempt still resets the document lifecycle
        // and still releases the recovery single-flight guard.
        assert!(source.contains("document_lifecycle.begin_document()"));
        assert!(source.contains("main_recovery.on_page_load_started()"));
        // The gate exists per main WebView and degrades only when the
        // observer registration failed. The boot records exactly one
        // construction with the preserved initial identity.
        assert!(source.contains("navigation_gate::NavigationGate::new"));
        assert!(source.contains("main_nav_observer_active"));
        assert!(source.contains("MainWebviewLifecycle::new"));
        assert!(source.contains("record_initial()"));
        assert!(source.contains("main_instance_id"));
        // Finishes consult the gate; outcomes resolve through it.
        assert!(source.contains("main_trusted_gate.on_finished(generation)"));
        // Only the authoritative success flag reaches the gate — HTTP
        // and error metadata never decide.
        assert!(source.contains("main_trusted_gate.on_outcome(event.is_success)"));
        // New navigations supersede pending finishes and stored outcomes.
        assert!(source.contains("main_trusted_gate.on_started("));
        // The extracted pipeline runs from exactly the two resolution
        // sites (finish-first and outcome-first orders). The needle is
        // joined so this assertion's own text cannot self-count.
        let run_needle = ["run_trusted_load_pipeline(TrustedLoad", "Context"].join("");
        assert_eq!(source.matches(&run_needle).count(), 2);
        // Failure diagnostics are fixed-vocabulary and sanitized.
        assert!(source.contains("format_skip_log"));
        assert!(source.contains("format_awaiting_log"));
        assert!(source.contains("format_stranded_log"));
        // Still no automatic reload anywhere near the gate: the single
        // host-side reload belongs to the recovery decision only.
        let reload_needle = ["match webview", "reload()"].join(".");
        assert_eq!(source.matches(&reload_needle).count(), 1);
    }
}

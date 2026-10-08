mod app;
mod backup;
mod bundles;
mod css_watch;
mod downloads;
mod folders;
mod legcord_protocol;
mod localization;
mod mod_substrate;
mod new_window;
mod permissions;
mod plugin_storage;
mod protocol;
mod quick_css;
mod settings;
mod settings_boot;
mod settings_plugin;
mod theme_import;
mod themes;

use std::{
    collections::HashMap,
    error::Error,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use mod_substrate::{
    load_mod_bundle, managed_css_order_script, managed_css_remove_script, managed_css_script,
    mod_css_script, mod_execution_context_script, mod_marker_script, stage as mod_stage,
    validate_descriptor, DocumentModLifecycle, ModArtifactIdentity, ModCache, ModCssSource,
    ModDescriptor, ModEnabling, ModHostMetrics, PageGlobalReadiness, PendingModBootstrap,
};

use app::{current_app_version, validate_version};
use backup::{
    build_backup_manifest, build_backup_zip, collect_backup_entries, created_at_now,
    restore_profile_from_picked_file, show_open_picker, show_save_picker, validate_backup_request,
    BackupFailure, BackupSaveSummary, OpenPickerOutcome, RestoreSummary, SavePickerOutcome,
};
use bundles::{describe_bundle_files, BUNDLE_FILES};
use css_watch::{
    diff_theme_snapshots, quick_css_transition, start_live_css_watchers, LiveCssCounters,
    LiveCssFsSignal, QuickCssTransition, ResourceDebouncer, CSS_DEBOUNCE_MS,
};
use folders::{
    ensure_folder_root, open_folder, resolve_theme_folder, reveal_folder, FolderLocation,
};
use legcord_protocol::scheme::create_environment as create_legcord_webview_environment;
use legcord_protocol::{build_namespace_roots, extend_csp_for_legcord, register_legcord_protocol};
use localization::{lookup as lookup_lang_string, validate_key as validate_lang_key};
use plugin_storage::{read_plugin_file, write_plugin_file};
use protocol::{
    parse_bridge_message, validate_mod_bundle_id, window_state_event_json, AppVersionResponse,
    BackupRestoredResponse, BackupSavedResponse, BridgeCommand, BridgeError, BridgeEvent,
    BridgeEventMessage, BridgeMessage, BridgeRequest, BridgeResponse, BridgeResponsePayload,
    BundleFileEntry, BundlesListResponse, FoldersOpenedResponse, LocalizationStringResponse,
    ModBundleResponse, PingResponse, PluginStorageReadResponse, PluginStorageWriteResponse,
    RequestTracker, SettingsConfigResponse, SettingsMutationResponse, SettingsSetResponse,
    ThemeEnabledResponse, ThemeFolderOpenedResponse, ThemeImportedResponse, ThemeInstalledResponse,
    ThemeWrittenResponse, ThemesListResponse, WindowAckResponse, PROTOCOL_VERSION,
};
use quick_css::{load_quick_css, QUICK_CSS_STYLE_ID};
use serde::Serialize;
use settings::{apply_block_power_saving, apply_settings_set, load_settings};
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
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2Environment, ICoreWebView2WebResourceRequestedEventArgs,
    ICoreWebView2_2, COREWEBVIEW2_WEB_RESOURCE_CONTEXT,
};
#[cfg(target_os = "windows")]
use webview2_com::WebResourceRequestedEventHandler;
#[cfg(target_os = "windows")]
use windows::{
    core::{Interface, BOOL, HSTRING, PWSTR},
    Win32::System::Com::IStream,
};
use wry::{
    http::{Request, Uri},
    NewWindowResponse, PageLoadEvent, WebContext, WebView, WebViewBuilder,
};
#[cfg(target_os = "windows")]
use wry::{WebViewBuilderExtWindows, WebViewExtWindows};

const DISCORD_URL: &str = "https://discord.com/app";
const WEBVIEW_PROFILE: &str = r"E:\Legcord.webview-dev-user-data";
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
        style_element_id: "legcord-native-vencord-css",
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
    bootstrap_dom_event: "legcord-native:vencord-bootstrap",
    storage_context_global: "__legcordNativeVencordStorageAtExecution",
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
///   wraps the source in an IIFE that declares `SHELTER_INJECTOR_PLUGINS`;
///   natively that wrapper is deliberately NOT reproduced because the
///   `legcord://` protocol and the preload plugin loader are both unimplemented,
///   and Shelter skips the injector entirely when the global is absent.
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
        ready_global: "__legcordNativeShelterPageReady",
        // Shelter waits for `#app-mount` to render, so the global appears well
        // after bundle evaluation. The pre-bundle trap resolves the normal case
        // with no polling; the 250 ms poll is only a bounded fallback.
        timeout_ms: 120_000,
        poll_ms: 250,
    },
    enabling: ModEnabling::AlwaysEnabled,
    success_marker: "shelter-bootstrap-executed",
    settings_key: None,
    bootstrap_dom_event: "legcord-native:shelter-bootstrap",
    storage_context_global: "__legcordNativeShelterStorageAtExecution",
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
        style_element_id: "legcord-native-equicord-css",
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
    bootstrap_dom_event: "legcord-native:equicord-bootstrap",
    storage_context_global: "__legcordNativeEquicordStorageAtExecution",
    log_tag: "Mod:Equicord",
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

/// Every mod the native host serves. Each entry is a thin typed descriptor plus
/// a thin renderer adapter under `native/web/mods/<id>/`; the substrate
/// mechanics in `mod_substrate.rs` are identical for all of them.
const KNOWN_MODS: &[ModDescriptor] = &[VENCORD_MOD, SHELTER_MOD, EQUICORD_MOD];

/// DEV diagnostic: force a mod's adapter OFF regardless of `settings.mods`, so
/// a "native without <mod>" control run needs no profile edit.
const MOD_ADAPTER_OVERRIDE_ENV: &str = "LEGCORD_NATIVE_DIAGNOSTIC_MODS";
/// DEV-only: enable the post-load mod runtime observation.
const MOD_SMOKE_ENV: &str = "LEGCORD_NATIVE_MOD_SMOKE";
const SETTINGS_SMOKE_ENV: &str = "LEGCORD_NATIVE_SETTINGS_SMOKE";
const SETTINGS_MUTATION_SMOKE_ENV: &str = "LEGCORD_NATIVE_SETTINGS_MUTATION_SMOKE";
const SETTINGS_SET_SMOKE_ENV: &str = "LEGCORD_NATIVE_SETTINGS_SET_SMOKE";
const GETLANG_SMOKE_ENV: &str = "LEGCORD_NATIVE_GETLANG_SMOKE";
const APP_VERSION_SMOKE_ENV: &str = "LEGCORD_NATIVE_APP_VERSION_SMOKE";
const PLUGIN_STORAGE_SMOKE_ENV: &str = "LEGCORD_NATIVE_PLUGIN_STORAGE_SMOKE";
const THEME_INSTALL_SMOKE_ENV: &str = "LEGCORD_NATIVE_THEME_INSTALL_SMOKE";
const THEME_WRITE_SMOKE_ENV: &str = "LEGCORD_NATIVE_THEME_WRITE_SMOKE";
const THEME_IMPORT_SMOKE_ENV: &str = "LEGCORD_NATIVE_THEME_IMPORT_SMOKE";
const THEME_SET_ENABLED_SMOKE_ENV: &str = "LEGCORD_NATIVE_THEME_SET_ENABLED_SMOKE";
const THEME_LIST_SMOKE_ENV: &str = "LEGCORD_NATIVE_THEME_LIST_SMOKE";
const BACKUP_SAVE_SMOKE_ENV: &str = "LEGCORD_NATIVE_BACKUP_SAVE_SMOKE";
const BACKUP_RESTORE_SMOKE_ENV: &str = "LEGCORD_NATIVE_BACKUP_RESTORE_SMOKE";
const SHELTER_LOADER_PROBE_ENV: &str = "LEGCORD_NATIVE_LOADER_PROBE";
const SETTINGS_BOOT_SMOKE_ENV: &str = "LEGCORD_NATIVE_SETTINGS_BOOT_SMOKE";
const SETTINGS_UI_SMOKE_ENV: &str = "LEGCORD_NATIVE_SETTINGS_UI_SMOKE";
const FOLDERS_SMOKE_ENV: &str = "LEGCORD_NATIVE_FOLDERS_SMOKE";
const LEGCORD_PROTOCOL_SMOKE_ENV: &str = "LEGCORD_NATIVE_LEGCORD_PROTOCOL_SMOKE";
const PERMISSIONS_SMOKE_ENV: &str = "LEGCORD_NATIVE_PERMISSIONS_SMOKE";
const THEMES_SMOKE_ENV: &str = "LEGCORD_NATIVE_THEMES_SMOKE";
const QUICKCSS_SMOKE_ENV: &str = "LEGCORD_NATIVE_QUICKCSS_SMOKE";
const BARE_PAGE_ENV: &str = "LEGCORD_NATIVE_DIAGNOSTIC_BARE_PAGE";
const INTERACTION_DIAGNOSTIC_ENV: &str = "LEGCORD_NATIVE_DIAGNOSTIC_INTERACTION";
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
        "localization.get_lang",
        "app.get_version",
        "plugin_storage.read",
        "plugin_storage.write",
        "backup.list_bundles",
        "backup.save",
        "backup.restore",
        "folders.open",
        "folders.open_theme",
        "themes.install_code",
        "themes.write_css",
        "themes.import_files",
        "themes.set_enabled",
        "themes.list",
        "window.show",
        "window.hide",
        "window.minimize",
        "window.maximize",
        "window.unmaximize",
        "window.quit",
    ],
};
const COMPATIBILITY_SCRIPT: &str = include_str!("../web/compatibility.js");
const MOD_SUBSTRATE_SCRIPT: &str = include_str!("../web/mod-substrate/mod-bootstrap.js");
const VENCORD_ADAPTER_SCRIPT: &str = include_str!("../web/mods/vencord/adapter.js");
const SHELTER_ADAPTER_SCRIPT: &str = include_str!("../web/mods/shelter/adapter.js");
const EQUICORD_ADAPTER_SCRIPT: &str = include_str!("../web/mods/equicord/adapter.js");

/// The renderer adapter for one registered mod, or `None` for a descriptor
/// with no adapter file (a registry entry must always carry one).
fn mod_adapter_script(descriptor: &ModDescriptor) -> Option<&'static str> {
    match descriptor.id {
        "vencord" => Some(VENCORD_ADAPTER_SCRIPT),
        "shelter" => Some(SHELTER_ADAPTER_SCRIPT),
        "equicord" => Some(EQUICORD_ADAPTER_SCRIPT),
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
        ("vencord", "__legcordNativeVencord"),
        ("shelter", "__legcordNativeShelter"),
        ("equicord", "__legcordNativeEquicord"),
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
/// 2. An optional DEV diagnostic allowlist (`LEGCORD_NATIVE_DIAGNOSTIC_MODS`),
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
  const id = "legcord-native-prototype-css";
  let style = document.getElementById(id);
  if (!style) {
    style = document.createElement("style");
    style.id = id;
    (document.head || document.documentElement).appendChild(style);
  }
  style.textContent = ":root { --legcord-native-prototype-css: ready; }";
  window.legcord.native.reportCss(
    getComputedStyle(document.documentElement)
      .getPropertyValue("--legcord-native-prototype-css")
      .trim()
  );
})();
"#;

const BRIDGE_PROBE_SCRIPT: &str = r#"
(() => {
  const api = window.legcord;
  if (
    !api ||
    api.boot.runtime !== "webview2" ||
    api.boot.protocol_version !== 1 ||
    api.native.supports("bridge.ping") !== true
  ) {
    console.error("[Legcord native DEV] compatibility facade or boot snapshot is unavailable");
    return;
  }

  try {
    api.native.requireCapability("filesystem");
    console.error("[Legcord native DEV] unsupported filesystem capability was accepted");
    return;
  } catch (error) {
    if (error.code !== "UNSUPPORTED_CAPABILITY") {
      console.error("[Legcord native DEV] unsupported capability failed unexpectedly", error);
      return;
    }
  }

  document.documentElement.dataset.legcordNativeCompatibility = "ready";
  api.native.ping("discord-load")
    .then((result) => {
      document.documentElement.dataset.legcordNativeBridge = result.message;
      console.info("[Legcord native DEV] bridge round-trip succeeded");
    })
    .catch((error) => console.error("[Legcord native DEV] bridge round-trip failed", error));
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
    documentStartStorage: window.__legcordNativeStorageAtDocumentStart || null,
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
  if (Object.hasOwn(window, "__legcordNativeInteractionProbe")) return;
  Object.defineProperty(window, "__legcordNativeInteractionProbe", { value: true });
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

/// DEV-only localization smoke probe (`LEGCORD_NATIVE_GETLANG_SMOKE=1`).
/// Exercises the exact lookup the invite back-button caller performs:
/// `window.legcord.getLang("invite-goBackToApp")`, plus a missing key to
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
    const getLang = window.legcord?.getLang;
    if (typeof getLang !== "function") {
      fail("localization facade missing");
      return;
    }
    const valid = getLang.call(window.legcord, "invite-goBackToApp");
    const missing = getLang.call(window.legcord, "legcord-native-missing-key-probe");
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

/// DEV-only app-version smoke probe (`LEGCORD_NATIVE_APP_VERSION_SMOKE=1`).
/// Exercises the exact read the migrated update-checker caller performs:
/// `window.legcord.native.getVersion()` — the async replacement for the
/// synchronous `window.legcord.version` Electron read. Reported as one typed
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
    const getVersion = window.legcord?.native?.getVersion;
    if (typeof getVersion !== "function") {
      fail("app version facade missing");
      return;
    }
    const pending = getVersion.call(window.legcord.native);
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

/// DEV-only plugin-storage smoke probe
/// (`LEGCORD_NATIVE_PLUGIN_STORAGE_SMOKE=1`). Exercises the exact public
/// third-party contract (`window.legcord.pluginStorage.writeFile/readFile`,
/// the native replacement for Electron's `window.legcord.fs` over
/// `pluginWriteFile`/`pluginReadFile`): write a fixture under a temporary
/// logical plugin id, read it back, verify exact bytes, then block three
/// attack paths (traversal, absolute path, Windows drive path). Reported as
/// one typed `plugin_storage_probe` event per document with
/// `stage: "write_read"`. Never touches real user plugin namespaces.
const PLUGIN_STORAGE_SMOKE_SCRIPT: &str = r#"
(() => {
  const PLUGIN_ID = "legcordNativeProbe";
  const REL_PATH = "smoke/note.txt";
  const CONTENT = "legcord-native-plugin-storage-smoke-v1";
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
      const storage = window.legcord?.pluginStorage;
      if (typeof storage?.writeFile !== "function" || typeof storage?.readFile !== "function") {
        fail("plugin storage facade missing");
        return;
      }
      if (window.legcord.fs !== undefined) {
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

/// DEV-only theme install smoke probe (`LEGCORD_NATIVE_THEME_INSTALL_SMOKE=1`).
/// Installs one disposable fixture (`LegcordNativeSmoke` →
/// `LegcordNativeSmoke-BD`, disabled by Electron parity so the loader skips
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
  const FIXTURE_CSS = "/**\n * @name LegcordNativeSmoke\n * @author Legcord Native Probe\n * @version 0.0.0\n */\n:root {\n  --legcord-native-install-probe: 1;\n}\n";
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
      const themes = window.legcord?.themes;
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
        && installed.id === "LegcordNativeSmoke-BD"
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

/// DEV-only theme write smoke probe (`LEGCORD_NATIVE_THEME_WRITE_SMOKE=1`).
/// Replaces the CSS of a manually pre-placed ENABLED fixture
/// (`LegcordWriteProbe`, whose `manifest.theme` is deliberately NOT
/// `src.css`) through the real `themes.write_css` capability, then proves
/// the new value is reflected via `getComputedStyle` with exactly one
/// managed style node. Writes a second value to prove repeated atomic
/// replacement converges, then blocks three attack shapes (traversal id,
/// unknown theme, oversized CSS). Reported as one typed
/// `theme_write_probe` event per document with `stage: "write"`. The
/// fixture is removed by hand after validation (uninstall is deferred).
const THEME_WRITE_SMOKE_SCRIPT: &str = r#"
(() => {
  const THEME_ID = "LegcordWriteProbe";
  const FIRST_CSS = ":root{\n  --legcord-native-write-probe: first;\n}\n";
  const SECOND_CSS = ":root{\n  --legcord-native-write-probe: second;\n}\n";
  const STYLE_ID = "legcord-native-theme-LegcordWriteProbe";
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
    .getPropertyValue("--legcord-native-write-probe").trim();
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
      const themes = window.legcord?.themes;
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
      const blockedUnknown = await expectReject(themes.writeCss("legcord-native-no-such-theme", FIRST_CSS));
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

/// DEV-only theme import smoke probe (`LEGCORD_NATIVE_THEME_IMPORT_SMOKE=1`).
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
      const themes = window.legcord?.themes;
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

/// DEV-only theme enable smoke probe (`LEGCORD_NATIVE_THEME_SET_ENABLED_SMOKE=1`).
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
  const THEME_ID = "LegcordEnableProbe-BD";
  const THEME_CSS = "/**\n * @name LegcordEnableProbe\n * @version 0.0.0\n */\n:root {\n  --legcord-native-enable-probe: 1;\n}\n";
  const SECOND_ID = "LegcordSecondProbe-BD";
  const SECOND_CSS = "/**\n * @name LegcordSecondProbe\n * @version 0.0.0\n */\n:root {\n  --legcord-native-second-probe: 2;\n}\n";
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
  const styleId = (id) => "legcord-native-theme-" + id;
  const nodeBytes = (id) => {
    const node = document.getElementById(styleId(id));
    return node ? (node.textContent || "").length : -1;
  };
  const probeVar = () => getComputedStyle(document.documentElement)
    .getPropertyValue("--legcord-native-enable-probe").trim();
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
      const themes = window.legcord?.themes;
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
      const blockedUnknown = await expectReject(themes.setEnabled("legcord-native-no-such-theme", true));
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

/// DEV-only theme list smoke probe (`LEGCORD_NATIVE_THEME_LIST_SMOKE=1`).
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
      const themes = window.legcord?.themes;
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

/// DEV-only backup save probe (`LEGCORD_NATIVE_BACKUP_SAVE_SMOKE=1`).
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
      const backup = window.legcord?.backup;
      if (typeof backup?.save !== "function") {
        fail("backup save facade missing");
        return;
      }
      // Fixed probe payload: every section on, small synthetic envelope.
      // Cancellation is control flow ({ ok: false, error: "CANCELLED" }),
      // never a rejection.
      const payload = JSON.stringify({
        includes: {
          legcordConfig: true,
          legcordThemesAndQuickCss: true,
          legcordExtensionPlugins: true,
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

/// DEV-only backup restore probe (`LEGCORD_NATIVE_BACKUP_RESTORE_SMOKE=1`).
/// Opens the REAL host picker through the `backup.restore` capability and
/// reports the bounded outcome as one typed `backup_restore_probe` event per
/// document with `stage: "restore"`. Completion validates, stages, commits,
/// and reports the clientMods byte size (the host never applies clientMods —
/// no localStorage write, no remote-plugin fetch, no script execution);
/// dismissal resolves `{ canceled: true }` and the probe reports the silent
/// path. Like the save probe there is no `attacks_blocked`: the facade takes
/// no input, so there is no client-side attack shape to block. The probe
/// never carries settings, themes, paths, URLs, or client-mod contents —
/// only stage strings, flags, and byte counts.
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
      const backup = window.legcord?.backup;
      if (typeof backup?.restore !== "function") {
        fail("backup restore facade missing");
        return;
      }
      // Parameter-free by protocol: no archive path, filename, URL,
      // filter, or dialog option may exist alongside the capability.
      if (backup.restore.length !== 0) {
        fail("restore facade must take no arguments");
        return;
      }
      // Opens the real host picker; user completion or dismissal resolves.
      // Cancellation is control flow ({ ok: false, error: "CANCELLED" }),
      // never a rejection. The host never applies clientMods: the probe
      // reports the envelope size only.
      const result = await backup.restore();
      if (!result || typeof result.ok !== "boolean") {
        fail("backup restore result shape is invalid");
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

/// DEV-only Shelter plugin-loader probe (`LEGCORD_NATIVE_LOADER_PROBE=1`).
/// Verifies the exact upstream `shelter.plugins` entry points a future
/// native settings-plugin loader needs — `addLocalPlugin`, `startPlugin`,
/// `stopPlugin`, `removePlugin` — and asserts the URL-based
/// `ensureLoaderPlugin` is NOT exposed on the public plugins object (it is
/// omitted upstream, so no URL-selected loading surface is reachable).
/// It then round-trips one synthetic plugin with host-fixed id and source
/// (`legcord-loader-probe`, `({onLoad(){}})`) through add/start/stop/remove
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
  const PROBE_ID = "legcord-loader-probe";
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
        js: PROBE_JS, manifest: { name: "Legcord Loader Probe" }, update: false,
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

/// DEV-only settings boot snapshot probe (`LEGCORD_NATIVE_SETTINGS_BOOT_SMOKE=1`).
/// Observes the live `window.legcord.settingsBoot` surface in the document:
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
    const boot = window.legcord?.settingsBoot;
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

/// DEV-only settings UI workflow probe (`LEGCORD_NATIVE_SETTINGS_UI_SMOKE=1`).
/// Drives the real executing settings UI end to end in the page: opens Discord
/// user settings via the gear button, asserts the Legcord section registered
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
  const STAGE_KEY = "legcord-settings-ui-stage";
  const NONCE_KEY = "legcord-settings-ui-nonce";
  const RUN_NONCE = "__LEGCORD_SETTINGS_UI_RUN_NONCE__";
  const probe = (stage, ok, detail, error) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "settings_ui_probe", payload: { stage, ok, detail: String(detail || "").slice(0, 512), error: error ? String((error && error.message) || error).slice(0, 256) : null } } })
  );
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const visible = (el) => !!el && el.getClientRects().length > 0;
  const themeStyleCount = () => document.querySelectorAll('style[id^="legcord-native-theme-"]').length;
  const themeStylePresent = (id) => !!document.getElementById("legcord-native-theme-" + id);
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
    // showing): open = a visible dialog containing "My Account" or "Legcord".
    function settingsDialog() {
      const dialogs = [...document.querySelectorAll('[role="dialog"], [role="alertdialog"]')].filter(visible);
      return dialogs[dialogs.length - 1] || null;
    }
    const modalOpen = () => {
      const dlg = settingsDialog();
      if (!dlg) return false;
      return !!findText(dlg, "My Account") || !!findText(dlg, "Legcord");
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
    // Verify the modal genuinely opened: "Legcord" alone could match an
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
      probe("open", false, "disabledSeen=" + disabledSeen + " legcord=" + !!findText(document, "Legcord") + forensics, "settings modal did not open");
      return;
    }
    probe("open", true, "settings-ui-open via=" + openedBy);
    const legcord = await waitFor(() => findText(document, "Legcord"), 10, 500);
    if (!legcord) { probe("open", false, "via=" + openedBy, "Legcord section missing"); return; }
    const opened = await openThemesPage();
    if (!opened) {
      probe("themes", false, "navThemes=" + countText(document, "Themes") + " legcord=" + !!findText(document, "Legcord"), "Installed Themes missing");
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
      const echoed = await window.legcord.folders.openTheme("ClearVision-BD");
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
        SETTINGS_UI_PROBE_TEMPLATE.replace("__LEGCORD_SETTINGS_UI_RUN_NONCE__", &nonce.to_string());
    if script.contains("__LEGCORD_SETTINGS_UI_RUN_NONCE__") {
        return Err("settings UI probe script has an unspliced nonce".to_owned());
    }
    Ok(script)
}
/// DEV-only legcord:// protocol smoke probe
/// (`LEGCORD_NATIVE_LEGCORD_PROTOCOL_SMOKE=1`). Loads three real bundle
/// resources that exist on disk through real WebView2 subresource/fetch
/// paths and records the outcome in document dataset flags the host reads
/// back:
///
/// - a stylesheet (`legcord://assets/css/discord.css` via `<link>`,
///   exercises style-src CSP + 200 + text/css)
/// - an image (`legcord://assets/desktop.png` via `<img>`, exercises
///   img-src CSP + 200 + image/png)
/// - a plugin resource (`legcord://plugins/settings/plugin.js` via
///   `fetch`, observed only — CORS headers are deliberately NOT added, so
///   a TypeError here proves the handler served while reporting the CORS
///   posture honestly)
///
/// The host logs one `[LegcordProtocol][Smoke]` line per document; no bridge
/// event, no new capability, no script execution of the fetched bytes.
const LEGCORD_PROTOCOL_SMOKE_SCRIPT: &str = r#"
(() => {
  try {
    const root = document.documentElement;
    if (root.dataset.legcordProtocolSmoke === "started") return;
    root.dataset.legcordProtocolSmoke = "started";
    root.dataset.legcordProtocolCss = "pending";
    root.dataset.legcordProtocolImg = "pending";
    root.dataset.legcordProtocolPlugin = "pending";
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = "legcord://assets/css/discord.css";
    link.onload = () => { root.dataset.legcordProtocolCss = "loaded"; };
    link.onerror = () => { root.dataset.legcordProtocolCss = "error"; };
    (document.head || document.documentElement).appendChild(link);
    const img = new Image();
    img.onload = () => { root.dataset.legcordProtocolImg = "loaded"; };
    img.onerror = () => { root.dataset.legcordProtocolImg = "error"; };
    img.src = "legcord://assets/desktop.png";
    // Distinguish CSP blocks from scheme/network failures: a
    // securitypolicyviolation naming a legcord:// URI proves the scheme
    // resolved and CSP refused it; silence + error proves the request never
    // reached the handler.
    root.dataset.legcordProtocolCsp = "none";
    document.addEventListener("securitypolicyviolation", (event) => {
      try {
        const blocked = String(event.blockedURI || "");
        if (blocked.indexOf("legcord://") === 0) {
          root.dataset.legcordProtocolCsp =
            String(event.violatedDirective || "unknown").slice(0, 48)
            + "|" + blocked.slice(0, 80);
        }
      } catch (ignored) {}
    });
    // CORS posture observation: a no-cors fetch that resolves opaque proves
    // the handler served bytes while a cors-mode rejection stands.
    root.dataset.legcordProtocolNoCors = "pending";
    fetch("legcord://plugins/settings/plugin.js", { method: "GET", mode: "no-cors" }).then(
      () => { root.dataset.legcordProtocolNoCors = "resolved"; },
      (error) => {
        root.dataset.legcordProtocolNoCors =
          "rejected:" + String((error && error.name) || error).slice(0, 32);
      },
    );
    fetch("legcord://plugins/settings/plugin.js", { method: "GET" }).then(
      (response) => {
        root.dataset.legcordProtocolPlugin =
          "http-" + response.status + "-bytes-" + (response.headers.get("content-type") || "no-mime");
      },
      (error) => {
        root.dataset.legcordProtocolPlugin =
          "fetch-rejected:" + String((error && error.name) || error).slice(0, 64);
      },
    );
  } catch (error) {
    document.documentElement.dataset.legcordProtocolSmoke =
      "probe-failed:" + String((error && error.message) || error).slice(0, 64);
  }
})();
"#;

/// Reads back the DEV-only legcord:// probe dataset flags. Returns JSON
/// through the evaluate callback (like the renderer health probe), never
/// through the typed bridge.
const LEGCORD_PROTOCOL_SMOKE_READBACK_SCRIPT: &str = r#"
(() => {
  const root = document.documentElement;
  return JSON.stringify({
    smoke: root.dataset.legcordProtocolSmoke || "absent",
    css: root.dataset.legcordProtocolCss || "absent",
    img: root.dataset.legcordProtocolImg || "absent",
    plugin: root.dataset.legcordProtocolPlugin || "absent",
    csp: root.dataset.legcordProtocolCsp || "absent",
    nocors: root.dataset.legcordProtocolNoCors || "absent",
  });
})()
"#;

/// DEV-only permissions smoke probe (`LEGCORD_NATIVE_PERMISSIONS_SMOKE=1`).
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
    if (root.dataset.legcordPermissionsSmoke === "started") return;
    root.dataset.legcordPermissionsSmoke = "started";
    root.dataset.legcordPermissionsNotification = "pending";
    root.dataset.legcordPermissionsMic = "pending";
    root.dataset.legcordPermissionsCamera = "pending";
    root.dataset.legcordPermissionsGeo = "pending";
    try {
      const pending = Notification.requestPermission();
      if (pending && typeof pending.then === "function") {
        pending.then(
          (value) => { root.dataset.legcordPermissionsNotification = String(value); },
          (error) => { root.dataset.legcordPermissionsNotification = "error"; },
        );
      } else {
        root.dataset.legcordPermissionsNotification = String(Notification.permission);
      }
    } catch (error) {
      root.dataset.legcordPermissionsNotification = "error";
    }
    try {
      navigator.mediaDevices.getUserMedia({ audio: true }).then(
        (stream) => {
          try { stream.getTracks().forEach((track) => track.stop()); } catch (ignored) {}
          root.dataset.legcordPermissionsMic = "live-stopped";
        },
        (error) => { root.dataset.legcordPermissionsMic = "denied:" + String((error && error.name) || error).slice(0, 32); },
      );
    } catch (error) {
      root.dataset.legcordPermissionsMic = "error";
    }
    try {
      navigator.mediaDevices.getUserMedia({ video: true }).then(
        (stream) => {
          try { stream.getTracks().forEach((track) => track.stop()); } catch (ignored) {}
          root.dataset.legcordPermissionsCamera = "live-stopped";
        },
        (error) => { root.dataset.legcordPermissionsCamera = "denied:" + String((error && error.name) || error).slice(0, 32); },
      );
    } catch (error) {
      root.dataset.legcordPermissionsCamera = "error";
    }
    try {
      navigator.geolocation.getCurrentPosition(
        () => { root.dataset.legcordPermissionsGeo = "unexpected-allow"; },
        (error) => { root.dataset.legcordPermissionsGeo = "denied:" + (error && error.code); },
        { timeout: 8000 },
      );
    } catch (error) {
      root.dataset.legcordPermissionsGeo = "error";
    }
  } catch (error) {
    document.documentElement.dataset.legcordPermissionsSmoke =
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
    smoke: root.dataset.legcordPermissionsSmoke || "absent",
    notification: root.dataset.legcordPermissionsNotification || "absent",
    mic: root.dataset.legcordPermissionsMic || "absent",
    camera: root.dataset.legcordPermissionsCamera || "absent",
    geo: root.dataset.legcordPermissionsGeo || "absent",
  });
})()
"#;
/// DEV-only themes observation (`LEGCORD_NATIVE_THEMES_SMOKE=1`,
/// `LEGCORD_NATIVE_QUICKCSS_SMOKE=1`). Theme and Quick CSS injection are host
/// pushes: the renderer never requests their CSS, so this probe only observes
/// the managed `<style>` nodes the host injected at document load. It returns
/// JSON through the evaluate callback (like the renderer health probe), never
/// through the typed bridge. `order_probe` reports the computed value of the
/// harmless `--legcord-native-order-probe` custom property the DEV fixtures
/// set in both Quick CSS and themes, proving cascade order (later stylesheets
/// win on equal specificity).
const THEMES_SMOKE_SCRIPT: &str = r#"
(() => {
  const head = document.head;
  const headOrder = head ? [...head.querySelectorAll("style")].map((node) => node.id) : [];
  const nodes = [...document.querySelectorAll('style[id^="legcord-native-theme-"]')]
    .map((node) => ({ id: node.id, bytes: (node.textContent || "").length }));
  const quickCssNode = document.getElementById("legcord-quick-css");
  return JSON.stringify({
    count: nodes.length,
    themes: nodes,
    head_style_order: headOrder,
    vencord_present: headOrder.includes("legcord-native-vencord-css"),
    quickcss_present: Boolean(quickCssNode),
    quickcss_bytes: quickCssNode ? (quickCssNode.textContent || "").length : 0,
    order_probe: getComputedStyle(document.documentElement)
      .getPropertyValue("--legcord-native-order-probe")
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

/// Closed registry id for the real Legcord settings plugin. It matches the
/// `"legcord-settings"` key Electron pins in the Shelter injector map
/// (`src/discord/preload/mods/shelter.mts`), so host logs and the Electron
/// contract name the same plugin. The renderer never supplies this id: the
/// host splices it into the fixed start script below.
const SETTINGS_PLUGIN_ID: &str = "legcord-settings";

/// Fixed manifest for the real Legcord settings plugin registration. Only the
/// name is carried (the loader-probe precedent); the renderer supplies no
/// manifest, id, source, or update flag.
const SETTINGS_PLUGIN_MANIFEST_JSON: &str = r#"{"name":"Legcord Settings"}"#;

/// Host-built start script for the real Legcord settings plugin, evaluated
/// once per document after Shelter's own bootstrap marker proves
/// `window.shelter.plugins` exists. It registers the embedded artifact
/// through the same closed add/start lifecycle the loader probe proved
/// (remove-first for idempotency, host-fixed id/source/manifest, then start),
/// and returns a plain status string the host logs — never plugin source,
/// paths, or URLs.
///
/// `__LEGCORD_SETTINGS_PLUGIN_JS__` is spliced as a JSON-encoded string
/// literal (never interpolated raw); `__LEGCORD_SETTINGS_PLUGIN_MANIFEST__`
/// is spliced as fixed JSON.
const SETTINGS_PLUGIN_START_TEMPLATE: &str = r#"
(() => {
  const ID = "legcord-settings";
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
      js: __LEGCORD_SETTINGS_PLUGIN_JS__,
      manifest: __LEGCORD_SETTINGS_PLUGIN_MANIFEST__,
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
        .replace("__LEGCORD_SETTINGS_PLUGIN_JS__", &js)
        .replace(
            "__LEGCORD_SETTINGS_PLUGIN_MANIFEST__",
            SETTINGS_PLUGIN_MANIFEST_JSON,
        );
    if script.contains("__LEGCORD_SETTINGS_PLUGIN_") {
        return Err("settings plugin start script has an unspliced placeholder".to_owned());
    }
    if !script.contains(SETTINGS_PLUGIN_ID) {
        return Err("settings plugin start script lost the closed plugin id".to_owned());
    }
    Ok(script)
}

/// Per-document guard for the real Legcord settings plugin start. The plugin
/// may start at most once per document generation: the guard compares against
/// the live generation, so reloads start fresh and replays are refused.
/// Threaded as one bundle to keep the bootstrap handler readable.
struct SettingsPluginStart<'a> {
    /// Host-built evaluation script (closed id + embedded artifact).
    script: &'a str,
    /// Generation that already started the plugin (`None` = not yet started).
    started_generation: &'a mut Option<u64>,
    /// Count of documents the plugin was started in (3d-2 observability).
    start_count: &'a mut u64,
}

/// Decides whether the settings plugin may start in `current_generation`.
/// Pure so the reload-fresh / replay-refuse semantics are unit-testable
/// without a WebView.
fn settings_plugin_start_due(started_generation: Option<u64>, current_generation: u64) -> bool {
    started_generation != Some(current_generation)
}

/// DEV-only settings smoke probe (`LEGCORD_NATIVE_SETTINGS_SMOKE=1`).
/// Exercises the exact read the migrated Shelter power caller performs:
/// `window.legcord.settings.getConfig()` → `blockPowerSavingInVoiceChat`.
/// The result is reported through the typed `settings_probe` event so the
/// host log shows request/response behavior per document.
const SETTINGS_SMOKE_SCRIPT: &str = r#"
(() => {
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "settings_probe", payload } })
  );
  try {
    const getConfig = window.legcord?.settings?.getConfig;
    if (typeof getConfig !== "function") {
      probe({ ok: false, source: "unavailable", payload_bytes: 0, block_power_saving: false, error: "settings facade missing" });
      return;
    }
    const result = getConfig.call(window.legcord.settings);
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
/// (`LEGCORD_NATIVE_SETTINGS_MUTATION_SMOKE=1`). Exercises the exact
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
    const settings = window.legcord?.settings;
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
/// `__LEGCORD_MUTATION_RESTORE_VALUE__` with the pre-mutation value before
/// evaluating: read the post-reload value (proving the mutation survived),
/// write the original back, re-read, and report `stage: "restored"`.
const SETTINGS_MUTATION_RESTORE_SCRIPT_TEMPLATE: &str = r#"
(() => {
  const RESTORE_TO = __LEGCORD_MUTATION_RESTORE_VALUE__;
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "settings_mutation_probe", payload } })
  );
  const fail = (before, error) => probe({
    ok: false, stage: "restored", before, value: before, reread_matches: false,
    error: String(error?.message ?? error).slice(0, 256),
  });
  try {
    const settings = window.legcord?.settings;
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

/// DEV-only settings.set smoke probe (`LEGCORD_NATIVE_SETTINGS_SET_SMOKE=1`).
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
    const settings = window.legcord?.settings;
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
/// `__LEGCORD_SETTINGS_SET_RESTORE_VALUE__` with the pre-mutation value
/// before evaluating: read the post-reload value (proving the mutation
/// survived), write the original back, re-read, and report `stage:
/// "restored"`. The three attack shapes were proven blocked in the mutated
/// stage, so this stage reports the same count.
const SETTINGS_SET_RESTORE_SCRIPT_TEMPLATE: &str = r#"
(() => {
  const RESTORE_TO = __LEGCORD_SETTINGS_SET_RESTORE_VALUE__;
  const KEY = "support_banner_dismissed";
  const probe = (payload) => window.ipc.postMessage(
    JSON.stringify({ version: 1, event: { type: "settings_set_probe", payload } })
  );
  const fail = (error) => probe({
    ok: false, stage: "restored", key: KEY, before: false, reread_matches: false,
    attacks_blocked: 3, error: String(error?.message ?? error).slice(0, 256),
  });
  try {
    const settings = window.legcord?.settings;
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
    /// Completed backup restore from the dedicated STA restore worker. The
    /// worker owns the modal open dialog, the bounded archive read, full
    /// validation, staging, and the commit with rollback, so the WebView/UI
    /// thread never blocks; the bounded outcome (cancellation plus the
    /// clientMods envelope — never a path) is delivered here.
    BackupRestoreReady {
        request_id: String,
        received_at: Instant,
        result: Result<RestoreSummary, BackupFailure>,
    },
    /// One DEV-only window chrome smoke step (`LEGCORD_NATIVE_WINDOW_SMOKE=1`).
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
    /// DEV-only delayed readback for the legcord:// protocol smoke probe.
    /// Fired once per document ~5 s after the probe started, so async
    /// subresource loads have settled before the host reads the flags.
    LegcordProtocolSmokeReadback,
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
    is_discord_uri(uri)
        && uri
            .authority()
            .is_some_and(|authority| authority.host().eq_ignore_ascii_case("discord.com"))
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
/// absent adapter, not a disabled one.
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
        .replace("__LEGCORD_BOOT_SNAPSHOT__", &snapshot)
        .replace("__LEGCORD_SETTINGS_BOOT_SNAPSHOT__", settings_boot_json)
        .replace("__LEGCORD_HOST_PLATFORM__", host_platform_json);
    if compatibility.contains("__LEGCORD_BOOT_SNAPSHOT__")
        || compatibility.contains("__LEGCORD_SETTINGS_BOOT_SNAPSHOT__")
        || compatibility.contains("__LEGCORD_HOST_PLATFORM__")
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
/// The single managed popup child (P0.4). At most one lives at a time:
/// the approved Discord popout / `about:blank` / hidden ICS surface.
/// The `WebView` field is never read — it exists to keep the child alive
/// exactly as long as the slot; dropping the slot destroys the child.
struct PopupChild {
    window: Arc<tao::window::Window>,
    #[allow(dead_code)]
    webview: WebView,
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
/// facade, the same bridge origin gate, the same `legcord://` registration,
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
    roots: &crate::legcord_protocol::NamespaceRoots,
    downloads_dir: &std::path::Path,
    always_on_top: bool,
) -> Result<PopupChild, String> {
    let (visible, initial_url) = match kind {
        new_window::PopupKind::AboutBlank => (true, "about:blank"),
        // Hidden transient (Electron `show:false` parity). No download
        // manager exists yet: the blob load is attempted, never shown.
        new_window::PopupKind::Ics => (false, url),
        new_window::PopupKind::DiscordPopout => (true, url),
    };
    let child_window = WindowBuilder::new()
        .with_title("Legcord")
        .with_inner_size(Size::Logical(LogicalSize::new(800.0, 600.0)))
        .with_visible(false)
        .with_always_on_top(always_on_top && kind == new_window::PopupKind::DiscordPopout)
        .build(target)
        .map_err(|error| format!("popup window failed: {error}"))?;
    let child_window = Arc::new(child_window);
    let allow_blob = kind == new_window::PopupKind::Ics;
    let child_ipc_proxy = ipc_proxy.clone();
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
        if let Err(error) = register_legcord_protocol(&core_webview2, roots.clone()) {
            eprintln!("[Popup][LegcordProtocol] registration failed: {error}");
        }
        if let Err(error) = permissions::register(&core_webview2) {
            eprintln!("[Popup][Permissions] registration failed: {error}");
        }
    }
    if visible {
        child_window.set_visible(true);
    }
    Ok(PopupChild {
        window: child_window,
        webview: child_webview,
    })
}

/// Name of the dedicated STA worker thread that owns the theme import
/// picker dialog plus the sequential bounded ingestion. The modal dialog
/// must never run on the WebView/UI thread: COM is initialized exactly once
/// for this thread (STA, as the Common Item Dialog requires — never once per
/// request), and picker jobs are processed sequentially for determinism and
/// bounded memory.
const THEME_IMPORT_WORKER_NAME: &str = "legcord-theme-import-sta";

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
const BACKUP_SAVE_WORKER_NAME: &str = "legcord-backup-save-sta";

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
const BACKUP_RESTORE_WORKER_NAME: &str = "legcord-backup-restore-sta";

/// One backup-restore job for the dedicated STA worker thread. The renderer
/// supplies nothing: the dialog, the picked path, validation, staging, and
/// commit are all host-owned.
struct BackupRestoreJob {
    request_id: String,
    received_at: Instant,
    profile: PathBuf,
    proxy: EventLoopProxy<NativeEvent>,
}

/// Run one backup-restore job on the STA worker: show the host-owned open
/// dialog, read the picked archive bounded, validate the entire archive,
/// stage outside the profile, and commit with rollback. Cancellation is
/// normal control flow (`canceled: true`, zeroed envelope). Timings are
/// logged by stage (dialog wait, validation + staging, commit) so dialog
/// wait never masquerades as host work.
fn run_backup_restore_job(job: &BackupRestoreJob) -> Result<RestoreSummary, BackupFailure> {
    let dialog_started = Instant::now();
    let picked = match show_open_picker(&job.profile) {
        OpenPickerOutcome::Canceled => {
            println!("[Backup][Restore] picker dismissed by the user");
            return Ok(RestoreSummary {
                canceled: true,
                client_mods: serde_json::Value::Null,
                archive_bytes: 0,
                uncompressed_bytes: 0,
                entry_count: 0,
                client_mods_bytes: 0,
            });
        }
        OpenPickerOutcome::Selected(path) => path,
        OpenPickerOutcome::Failed(failure) => {
            eprintln!("[Backup][Restore:{}] {}", failure.code, failure.message);
            return Err(failure);
        }
    };
    let dialog_ms = dialog_started.elapsed().as_secs_f64() * 1000.0;
    let restore_started = Instant::now();
    let summary =
        restore_profile_from_picked_file(&job.profile, &picked).inspect_err(|failure| {
            eprintln!("[Backup][Restore:{}] {}", failure.code, failure.message);
        })?;
    let restore_ms = restore_started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "[Backup][Restore] entries={} uncompressed={} archive={} client_mods={} dialog_wait_ms={:.3} restore_ms={:.3}",
        summary.entry_count,
        summary.uncompressed_bytes,
        summary.archive_bytes,
        summary.client_mods_bytes,
        dialog_ms,
        restore_ms,
    );
    Ok(summary)
}

fn send_bridge_response(webview: &WebView, response: &BridgeResponse) {
    match serde_json::to_string(response) {
        Ok(serialized) => {
            let serialized =
                serde_json::to_string(&serialized).expect("a JSON string must serialize to JSON");
            let script = format!(
                "window.dispatchEvent(new CustomEvent('legcord-native:response', {{ detail: JSON.parse({serialized}) }}));"
            );
            if let Err(error) = webview.evaluate_script(&script) {
                eprintln!("[Bridge] Could not deliver response to renderer: {error}");
            }
        }
        Err(error) => eprintln!("[Bridge] Could not serialize response: {error}"),
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
                "window.dispatchEvent(new CustomEvent('legcord-native:window-state', {{ detail: JSON.parse({serialized}) }}));"
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
    settings_plugin_start: Option<SettingsPluginStart<'_>>,
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
    // Real Legcord settings plugin (3d-2): Shelter's own marker proves
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
        .with_title("Legcord")
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

    let profile_path = PathBuf::from(WEBVIEW_PROFILE);
    std::fs::create_dir_all(&profile_path)?;
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
    let mut web_context = WebContext::new(Some(profile_path.clone()));
    // WebView2 environment with the legcord custom scheme registered (P0.2).
    // Created here — before the environment exists and before navigation —
    // and adopted by wry below. Fail-fast: without registration, legcord://
    // subresources fail as an unknown scheme and no handler ever fires.
    #[cfg(target_os = "windows")]
    let webview_env = {
        let env = create_legcord_webview_environment(&profile_path)?;
        println!(
            "[LegcordProtocol] scheme registered: legcord (authority, secure, https://discord.com)"
        );
        env
    };
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
                while let Ok(job) = restore_rx.recv() {
                    let result = run_backup_restore_job(&job);
                    queue_event(
                        &job.proxy,
                        NativeEvent::BackupRestoreReady {
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
    // Adapter selection is entirely host-controlled: the authoritative settings
    // snapshot (the same file the settings capability reads) plus an optional
    // DEV diagnostic allowlist. The renderer never participates.
    let settings_mods = match load_settings(&profile_path) {
        Ok(loaded) => loaded.snapshot.mods,
        Err(error) => {
            // Corrupt/unreadable settings fail closed to "no extra mods", the
            // same parity the Quick CSS enablement already uses.
            eprintln!(
                "[Mod:Registry] mod enablement could not be resolved from settings; treating as none ({error})"
            );
            Vec::new()
        }
    };
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
        match resolve_theme_folder(&profile_path, "legcord-native-no-such-theme") {
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
    let diagnostic_legcord_protocol_smoke =
        std::env::var(LEGCORD_PROTOCOL_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_legcord_protocol_smoke {
        println!("[LegcordProtocol][Smoke] legcord:// protocol smoke probe enabled");
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
        initialization_script(&active_mods, &settings_boot_json, &host_platform_json)?
    };
    println!(
        "[Native] Document-start script: {} bytes",
        initialization_script.len()
    );
    let mut webview_builder = WebViewBuilder::new_with_web_context(&mut web_context);
    // Adopt the pre-registered environment (P0.2). wry uses a
    // caller-provided environment verbatim instead of creating its own.
    #[cfg(target_os = "windows")]
    {
        webview_builder = webview_builder.with_environment(webview_env);
    }
    // P0.5: host-owned downloads directory, no dialog, never overwrite.
    webview_builder = apply_download_handlers(webview_builder, downloads_dir.clone());
    // Two-phase boot: the initial document is the empty `about:blank`
    // (committed synchronously inside `build()`, before any
    // WebResourceRequested filter can exist), then the host navigates to
    // Discord AFTER both interception filters are registered below. Without
    // this, the first Discord document races filter registration and would
    // keep Discord's original CSP (no `legcord:` allowance) until a reload.
    let webview_builder = webview_builder
        .with_url("about:blank")
        .with_initialization_script_for_main_only(&initialization_script, true)
        .with_navigation_handler(|url| {
            let allowed = is_allowed_navigation(&url);
            println!(
                "[Navigation] {} {url}",
                if allowed { "allow" } else { "block" }
            );
            allowed
        })
        .with_new_window_req_handler(move |url, _features| {
            // Renderer window features (size/position) are deliberately
            // ignored: every child option is host-owned (see
            // `create_popup_child`). Decision order mirrors Electron
            // (`src/discord/window.ts:214-264`) with the documented
            // hardening: parsed classification, no sticky bypass.
            match new_window::decide(&url) {
                decision @ (new_window::Decision::AllowAboutBlank
                | new_window::Decision::AllowIcs
                | new_window::Decision::AllowDiscordPopout) => {
                    let kind = new_window::popup_kind_for(decision)
                        .expect("allow decision must map to a popup kind");
                    println!("[Popup] approved {kind:?}: {url}");
                    queue_event(
                        &popup_proxy,
                        NativeEvent::PopupRequested {
                            url: url.clone(),
                            kind,
                        },
                    );
                    NewWindowResponse::Deny
                }
                new_window::Decision::ExternalOpen => {
                    match new_window::open_external_url(&url) {
                        Ok(()) => println!("[Popup] external open: {url}"),
                        Err(error) => eprintln!("[Popup] external open failed: {error} url={url}"),
                    }
                    NewWindowResponse::Deny
                }
                // Unknown custom schemes: deny with a log instead of
                // Electron's modal + sticky bypass (documented P0.4
                // hardening; no new settings key). Dangerous and malformed
                // values deny the same way, never reaching the shell.
                new_window::Decision::ConfirmExternal | new_window::Decision::Deny => {
                    println!("[Popup] denied: {url}");
                    NewWindowResponse::Deny
                }
            }
        })
        .with_on_page_load_handler(move |event, url| {
            let event_name = match &event {
                PageLoadEvent::Started => "started",
                PageLoadEvent::Finished => "finished",
            };
            println!("[Navigation] {event_name}: {url}");
            match event {
                PageLoadEvent::Started => {
                    queue_event(&load_proxy, NativeEvent::PageLoadStarted);
                }
                PageLoadEvent::Finished => {
                    queue_event(&load_proxy, NativeEvent::PageLoadFinished(url));
                }
            }
        })
        .with_ipc_handler(move |request: Request<String>| {
            if !is_trusted_bridge_origin(request.uri()) {
                eprintln!(
                    "[Bridge] rejected message from untrusted origin: {}",
                    request.uri()
                );
                return;
            }

            match parse_bridge_message(request.body()) {
                Ok(BridgeMessage::Request(request)) => queue_event(
                    &ipc_proxy,
                    NativeEvent::BridgeRequest(request, Instant::now()),
                ),
                Ok(BridgeMessage::Event(event)) => {
                    queue_event(&ipc_proxy, NativeEvent::BridgeEvent(event));
                }
                Err(error) => eprintln!("[Bridge] rejected invalid message: {error}"),
            }
        });

    let webview = webview_builder.build(&window_for_webview)?;
    println!("[Native] WebView2 prototype started");
    println!("[Native] Persistent DEV profile: {WEBVIEW_PROFILE}");

    // Register legcord:// protocol handler
    // The roots are kept for the P0.4 popup child, which registers the
    // same protocol on its own core WebView (registration is per-WebView,
    // not per-environment).
    #[cfg(target_os = "windows")]
    let popup_roots = {
        let core_webview2: ICoreWebView2 = webview.webview();
        let bundle_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("ts-out");
        let user_data_dir = std::path::PathBuf::from(WEBVIEW_PROFILE);
        let roots = build_namespace_roots(&bundle_dir, &user_data_dir);
        if let Err(e) = register_legcord_protocol(&core_webview2, roots.clone()) {
            eprintln!("[LegcordProtocol] Failed to register protocol handler: {e}");
        } else {
            println!("[LegcordProtocol] Custom scheme handler registered");
        }
        // Native permission policy (P0.3): exact-origin allowlist for
        // microphone/camera/notifications, explicit deny for everything
        // else, request-scoped (no profile persistence). A registration
        // failure is logged loudly but must not block boot: permissions
        // are not boot-required, and the pre-P0.3 behavior (browser
        // default) is strictly worse but not a startup crash.
        if let Err(e) = permissions::register(&core_webview2) {
            eprintln!("[Permissions] Failed to register permission handler: {e}");
        } else {
            println!("[Permissions] Permission handler registered (discord.com: mic/camera/notifications)");
        }
        roots
    };

    // Register WebResourceRequested handler to modify CSP header for Discord main document
    // This reproduces Electron's setupNoCSP() behavior by intercepting the main document response
    // and adding badges.vencord.dev to the connect-src directive.
    #[cfg(target_os = "windows")]
    {
        let core_webview2: ICoreWebView2 = webview.webview();
        // Add filter for main frame document resources on discord.com
        let filter = HSTRING::from("https://discord.com/*");
        if let Ok(webview2) = core_webview2.cast::<ICoreWebView2_2>() {
            unsafe {
                webview2
                    .AddWebResourceRequestedFilter(
                        &filter,
                        COREWEBVIEW2_WEB_RESOURCE_CONTEXT(1), // DOCUMENT
                    )
                    .ok();
            }
        }
        // Get environment for creating WebResourceResponse
        let env: ICoreWebView2Environment = if let Ok(wv2) = core_webview2.cast::<ICoreWebView2_2>()
        {
            unsafe { wv2.Environment().unwrap() }
        } else {
            eprintln!("[CSP] Failed to get ICoreWebView2_2 for environment");
            return Ok(()); // Early return from main
        };
        // Register WebResourceRequested handler
        let handler = WebResourceRequestedEventHandler::create(Box::new(
            move |_sender: Option<ICoreWebView2>,
                  args: Option<ICoreWebView2WebResourceRequestedEventArgs>| {
                let Some(args) = args else {
                    return Ok(());
                };
                let request = match unsafe { args.Request() } {
                    Ok(r) => r,
                    Err(_) => return Ok(()),
                };
                let uri = {
                    let mut uri = PWSTR::null();
                    if unsafe { request.Uri(&mut uri) }.is_err() {
                        return Ok(());
                    }
                    unsafe { uri.to_string().unwrap_or_default() }
                };
                // Only process main document navigation to /app
                if !uri.starts_with("https://discord.com/app") {
                    return Ok(());
                }
                // Get deferral to handle asynchronously
                let deferral = match unsafe { args.GetDeferral() } {
                    Ok(d) => d,
                    Err(_) => return Ok(()),
                };
                // Extract all request data in the main thread (COM interfaces are not Send)
                let method = {
                    let mut m = PWSTR::null();
                    if unsafe { request.Method(&mut m) }.is_err() {
                        let _ = unsafe { deferral.Complete() };
                        return Ok(());
                    }
                    unsafe { m.to_string().unwrap_or_else(|_| "GET".to_string()) }
                };
                let headers = match unsafe { request.Headers() } {
                    Ok(h) => h,
                    Err(_) => {
                        let _ = unsafe { deferral.Complete() };
                        return Ok(());
                    }
                };
                let mut request_headers = Vec::new();
                let iter = match unsafe { headers.GetIterator() } {
                    Ok(i) => i,
                    Err(_) => {
                        let _ = unsafe { deferral.Complete() };
                        return Ok(());
                    }
                };
                let mut has_current = BOOL::default();
                if unsafe { iter.HasCurrentHeader(&mut has_current) }.is_err() {
                    let _ = unsafe { deferral.Complete() };
                    return Ok(());
                }
                while has_current.as_bool() {
                    let mut key = PWSTR::null();
                    let mut value = PWSTR::null();
                    if unsafe { iter.GetCurrentHeader(&mut key, &mut value) }.is_err() {
                        break;
                    }
                    let key_str = unsafe { key.to_string().unwrap_or_default() };
                    let val_str = unsafe { value.to_string().unwrap_or_default() };
                    // Skip headers that WebView2 will set automatically
                    if !matches!(
                        key_str.to_lowercase().as_str(),
                        "host" | "connection" | "accept-encoding"
                    ) {
                        request_headers.push((key_str, val_str));
                    }
                    if unsafe { iter.MoveNext(&mut has_current) }.is_err() {
                        break;
                    }
                }
                // Get request body if present
                let body = match unsafe { request.Content() } {
                    Ok(content) => {
                        let mut body_bytes = Vec::new();
                        if let Ok(stream) = content.cast::<IStream>() {
                            let mut buffer = [0u8; 8192];
                            loop {
                                let mut read = 0u32;
                                unsafe {
                                    if stream
                                        .Read(
                                            buffer.as_mut_ptr() as *mut _,
                                            buffer.len() as u32,
                                            Some(&mut read),
                                        )
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                                if read == 0 {
                                    break;
                                }
                                body_bytes.extend_from_slice(&buffer[..read as usize]);
                            }
                        }
                        if body_bytes.is_empty() {
                            None
                        } else {
                            Some(body_bytes)
                        }
                    }
                    Err(_) => None,
                };
                // Forward the request using ureq (synchronously in handler)
                let mut req = ureq::request(&method, &uri);
                // Copy request headers (pre-extracted)
                for (key_str, val_str) in request_headers {
                    req = req.set(&key_str, &val_str);
                }
                let response = if let Some(body) = body {
                    req.send_bytes(&body)
                } else {
                    req.call()
                };
                let response = match response {
                    Ok(r) => r,
                    Err(error) => {
                        // Forwarding failed: leave the original response
                        // untouched (fail-closed, never stripped) and note it.
                        // Without forwarding, neither the badges patch nor
                        // the legcord: scheme allowance can apply.
                        eprintln!("[CSP] document forward failed: {error}");
                        let _ = unsafe { deferral.Complete() };
                        return Ok(());
                    }
                };
                // Build response headers, modifying CSP
                let mut headers_map = String::new();
                let mut csp_modified = false;
                // ureq Response uses header() method, not headers()
                let mut header_names = Vec::new();
                // Collect known header names from the response
                for name in [
                    "content-security-policy",
                    "content-type",
                    "cache-control",
                    "expires",
                    "etag",
                    "last-modified",
                    "server",
                    "date",
                    "vary",
                    "x-content-type-options",
                    "x-frame-options",
                    "strict-transport-security",
                    "referrer-policy",
                    "permissions-policy",
                    "cross-origin-opener-policy",
                    "cross-origin-resource-policy",
                    "cross-origin-embedder-policy",
                ] {
                    if let Some(val) = response.header(name) {
                        header_names.push((name.to_string(), val.to_string()));
                    }
                }
                for (name_str, val_str) in header_names {
                    if name_str.to_lowercase() == "content-security-policy" {
                        let mut csp = val_str.to_string();
                        // P0.2: WebView2 custom schemes participate in CSP (no
                        // bypassCSP parity), so allow legcord: in exactly the
                        // directives traced consumers need (default/script/
                        // style/img/media/connect). Preservation + badges
                        // patch below stay unchanged in behavior.
                        csp = extend_csp_for_legcord(&csp);
                        if !csp.contains("badges.vencord.dev") {
                            csp = if let Some(pos) = csp.to_lowercase().find("connect-src") {
                                let after = &csp[pos..];
                                let end = after.find(';').unwrap_or(after.len());
                                let directive = &after[..end].trim();
                                if !directive.contains("badges.vencord.dev") {
                                    csp.replacen(
                                        directive,
                                        &format!("{} https://badges.vencord.dev", directive),
                                        1,
                                    )
                                } else {
                                    csp
                                }
                            } else {
                                format!("{}; connect-src https://badges.vencord.dev", csp)
                            };
                        }
                        // Any CSP-bearing Discord response is now modified
                        // (badges and/or legcord:), never removed.
                        csp_modified = true;
                        let legcord_sources = csp.matches("legcord:").count();
                        println!(
                            "[CSP] document policy extended badges={} legcord_sources={legcord_sources}",
                            csp.contains("badges.vencord.dev")
                        );
                        headers_map.push_str(&format!("{}: {}\r\n", name_str, csp));
                    } else {
                        headers_map.push_str(&format!("{}: {}\r\n", name_str, val_str));
                    }
                }
                // If no CSP header existed, add one
                if !csp_modified {
                    headers_map.push_str(
                        "content-security-policy: connect-src https://badges.vencord.dev\r\n",
                    );
                }
                let status = response.status();
                // ureq 2.x: read body as string then convert to bytes
                let text = response.into_string().unwrap_or_default();
                let body_bytes = text.into_bytes();
                // Create WebResourceResponse
                let status_hstring = HSTRING::from(
                    http::StatusCode::from_u16(status)
                        .unwrap()
                        .canonical_reason()
                        .unwrap_or("OK"),
                );
                let headers_hstring = HSTRING::from(headers_map);
                let stream = if !body_bytes.is_empty() {
                    // Create IStream from bytes using SHCreateMemStream
                    let mut stream: Option<IStream> = None;
                    unsafe {
                        if let Some(s) =
                            windows::Win32::UI::Shell::SHCreateMemStream(Some(&body_bytes))
                        {
                            stream = Some(s);
                        }
                    }
                    stream
                } else {
                    None
                };
                if let Ok(web_response) = unsafe {
                    env.CreateWebResourceResponse(
                        stream.as_ref(),
                        status as i32,
                        &status_hstring,
                        &headers_hstring,
                    )
                } {
                    let _ = unsafe { args.SetResponse(&web_response) };
                }
                let _ = unsafe { deferral.Complete() };
                Ok(())
            },
        ));
        let mut token = 0i64;
        unsafe {
            let _ = core_webview2.add_WebResourceRequested(&handler, &mut token);
        }
        println!("[CSP] WebResourceRequested handler registered for Discord main document (CSP modification)");
    }
    // Second boot phase (P0.2): navigate to Discord only after both
    // WebResourceRequested filters exist, so the first Discord document is
    // intercepted like every later one. `about:blank` has no CSP of its own
    // and the navigation handler allows only it plus HTTPS Discord hosts.
    if let Err(error) =
        webview.evaluate_script(&format!("window.location.href = \"{DISCORD_URL}\";"))
    {
        return Err(format!("initial Discord navigation failed: {error}").into());
    }
    println!("[Native] Initial navigation: {DISCORD_URL}");
    let mut smoke_reloads_remaining: u32 = std::env::var("LEGCORD_NATIVE_SMOKE_RELOAD")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    let mut smoke_reload_request_id = None::<String>;
    let mut request_tracker = RequestTracker::default();
    let mut document_lifecycle = DocumentModLifecycle::default();
    let mut pending_mods = HashMap::<String, PendingModBootstrap>::new();
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
    // Real Legcord settings plugin (3d-2): host-built start script plus the
    // per-document guard, both moved into the event-loop closure below.
    let settings_plugin_start_script =
        settings_plugin_start_script().expect("settings plugin start script must build");
    println!(
        "[SettingsPlugin] start script bytes={}",
        settings_plugin_start_script.len()
    );
    let mut settings_plugin_started_generation = None::<u64>;
    let mut settings_plugin_start_count = 0_u64;
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
                    println!("[Popup] child closed; clearing single slot");
                    popup_child = None;
                    return;
                }
                println!("[Native] Close requested; shutting down WebView");
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
                ) {
                    Ok(child) => {
                        println!("[Popup] child opened {kind:?}: {url}");
                        popup_child = Some(child);
                    }
                    Err(error) => eprintln!("[Popup] child failed: {error} url={url}"),
                }
            }
            Event::UserEvent(NativeEvent::PageLoadStarted) => {
                document_lifecycle.begin_document();
                request_tracker.clear();
                pending_mods.clear();
                page_load_started_at = Some(Instant::now());
                println!(
                    "[Navigation] new document generation={} (bridge state reset)",
                    document_lifecycle.generation()
                );
            }
            Event::UserEvent(NativeEvent::PageLoadFinished(url)) => {
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
                    if let Err(error) = webview.evaluate_script_with_callback(
                        RENDERER_HEALTH_SCRIPT,
                        |result| println!("[Renderer][Health] {result}"),
                    ) {
                        eprintln!("[Renderer][Health] probe failed: {error}");
                    }
                    if !diagnostic_bare_page {
                        if let Err(error) = webview.evaluate_script(CSS_INJECTION_SCRIPT) {
                            eprintln!("[Injection] CSS script failed: {error}");
                        }
                        if let Err(error) = webview.evaluate_script(BRIDGE_PROBE_SCRIPT) {
                            eprintln!("[Injection] Bridge probe failed: {error}");
                        }
                        // Host-pushed Quick CSS injection (Phase D): the renderer
                        // never requests Quick CSS, so there is no bridge
                        // round trip here. The host reads its own fixed
                        // quickCss.css when the authoritative settings flag
                        // is enabled and pushes it through the shared
                        // managed-CSS lifecycle BEFORE themes, preserving
                        // Electron's Vencord -> Quick CSS -> themes cascade.
                        let quickcss_report = load_quick_css(&profile_path);
                        let mut quickcss_injected = false;
                        match &quickcss_report.css {
                            Some(css) => {
                                match managed_css_script(QUICK_CSS_STYLE_ID, css) {
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
                                }
                            }
                            None => {
                                // Disabled (or unreadable) after a previous
                                // injection: remove the managed style so no
                                // stale Quick CSS survives the reset. On a
                                // fresh document this is a cheap no-op.
                                if last_quickcss_injected {
                                    match managed_css_remove_script(QUICK_CSS_STYLE_ID) {
                                        Ok(script) => {
                                            if let Err(error) = webview.evaluate_script(&script)
                                            {
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
                        last_quickcss_injected = quickcss_injected;
                        // Content snapshot for live diffing: `Some` exactly
                        // when the managed style is present in this document.
                        last_quickcss_css = if quickcss_injected {
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
                        // Host-pushed theme injection (Phase D): the renderer
                        // never requests theme CSS, so there is no bridge
                        // round trip here. The host loads enabled themes from
                        // its own root and pushes each through the shared
                        // managed-CSS lifecycle in sorted-id order.
                        let theme_report = load_enabled_themes(&profile_path);
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
                        for stale_id in last_theme_style_ids
                            .iter()
                            .filter(|id| !current_theme_ids.contains(id))
                        {
                            match managed_css_remove_script(stale_id) {
                                Ok(script) => {
                                    if let Err(error) = webview.evaluate_script(&script) {
                                        eprintln!(
                                            "[Themes][css_removal] {stale_id}: {error}"
                                        );
                                    }
                                }
                                Err(error) => {
                                    eprintln!("[Themes][css_removal] {stale_id}: {error}");
                                }
                            }
                        }
                        last_theme_style_ids = current_theme_ids;
                        let mut theme_injected = 0_usize;
                        let mut injected_now: Vec<LoadedTheme> = Vec::new();
                        for theme in &theme_report.injected {
                            match managed_css_script(&theme.style_id, &theme.css) {
                                Ok(script) => {
                                    if let Err(error) = webview.evaluate_script(&script) {
                                        eprintln!(
                                            "[Themes][css_injection] {}: {error}",
                                            theme.id
                                        );
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
                        last_themes = injected_now;
                        if theme_report.discovered > 0 || diagnostic_themes_smoke {
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
                                println!(
                                    "[Themes][Skip] id={} reason={}",
                                    skipped.id, skipped.reason
                                );
                            }
                        }
                        if diagnostic_themes_smoke || diagnostic_quickcss_smoke {
                            // Copied for the 'static observation callback.
                            let themes_smoke = diagnostic_themes_smoke;
                            let quickcss_smoke = diagnostic_quickcss_smoke;
                            if let Err(error) = webview.evaluate_script_with_callback(
                                THEMES_SMOKE_SCRIPT,
                                move |result| {
                                    if themes_smoke {
                                        println!("[Themes][Smoke] managed_styles={result}");
                                    }
                                    if quickcss_smoke {
                                        println!("[QuickCss][Smoke] managed_styles={result}");
                                    }
                                },
                            ) {
                                eprintln!("[Themes][Smoke] observation failed: {error}");
                            }
                        }
                        // DEV-only read-only observation of every active mod's
                        // real page-world global, managed style, and adapter
                        // state. Observation only: no request, no injection.
                        if diagnostic_mod_smoke {
                            if let Err(error) = webview.evaluate_script_with_callback(
                                &mod_smoke_script,
                                |result| println!("[Mods][Smoke] {result}"),
                            ) {
                                eprintln!("[Mods][Smoke] observation failed: {error}");
                            }
                        }
                        if diagnostic_interaction {
                            if let Err(error) = webview.evaluate_script(INTERACTION_PROBE_SCRIPT) {
                                eprintln!("[Renderer][Interaction] probe failed: {error}");
                            }
                        }
                        if diagnostic_settings_smoke {
                            if let Err(error) = webview.evaluate_script(SETTINGS_SMOKE_SCRIPT) {
                                eprintln!("[Settings][Smoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_settings_mutation_smoke {
                            if mutation_smoke_stage == 0 {
                                if let Err(error) =
                                    webview.evaluate_script(SETTINGS_MUTATION_SMOKE_SCRIPT)
                                {
                                    eprintln!("[Settings][MutationSmoke] probe failed: {error}");
                                }
                            } else if mutation_smoke_stage == 1 {
                                let restore = SETTINGS_MUTATION_RESTORE_SCRIPT_TEMPLATE.replace(
                                    "__LEGCORD_MUTATION_RESTORE_VALUE__",
                                    if mutation_smoke_before { "true" } else { "false" },
                                );
                                if let Err(error) = webview.evaluate_script(&restore) {
                                    eprintln!("[Settings][MutationSmoke] restore failed: {error}");
                                }
                            }
                        }
                        if diagnostic_settings_set_smoke {
                            if settings_set_smoke_stage == 0 {
                                if let Err(error) =
                                    webview.evaluate_script(SETTINGS_SET_SMOKE_SCRIPT)
                                {
                                    eprintln!("[Settings][SetSmoke] probe failed: {error}");
                                }
                            } else if settings_set_smoke_stage == 1 {
                                let restore = SETTINGS_SET_RESTORE_SCRIPT_TEMPLATE.replace(
                                    "__LEGCORD_SETTINGS_SET_RESTORE_VALUE__",
                                    if settings_set_smoke_before {
                                        "true"
                                    } else {
                                        "false"
                                    },
                                );
                                if let Err(error) = webview.evaluate_script(&restore) {
                                    eprintln!("[Settings][SetSmoke] restore failed: {error}");
                                }
                            }
                        }
                        if diagnostic_getlang_smoke {
                            if let Err(error) = webview.evaluate_script(GETLANG_SMOKE_SCRIPT) {
                                eprintln!("[Localization][Smoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_app_version_smoke {
                            if let Err(error) = webview.evaluate_script(APP_VERSION_SMOKE_SCRIPT) {
                                eprintln!("[App][Smoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_plugin_storage_smoke {
                            if let Err(error) = webview.evaluate_script(PLUGIN_STORAGE_SMOKE_SCRIPT) {
                                eprintln!("[PluginStorage][Smoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_theme_install_smoke {
                            if let Err(error) = webview.evaluate_script(THEME_INSTALL_SMOKE_SCRIPT) {
                                eprintln!("[Themes][InstallSmoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_theme_write_smoke {
                            if let Err(error) = webview.evaluate_script(THEME_WRITE_SMOKE_SCRIPT) {
                                eprintln!("[Themes][WriteSmoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_theme_import_smoke {
                            if let Err(error) = webview.evaluate_script(THEME_IMPORT_SMOKE_SCRIPT) {
                                eprintln!("[Themes][ImportSmoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_theme_set_enabled_smoke {
                            if let Err(error) =
                                webview.evaluate_script(THEME_SET_ENABLED_SMOKE_SCRIPT)
                            {
                                eprintln!("[Themes][SetEnabledSmoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_theme_list_smoke {
                            if let Err(error) = webview.evaluate_script(THEME_LIST_SMOKE_SCRIPT) {
                                eprintln!("[Themes][ListSmoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_backup_save_smoke {
                            if let Err(error) = webview.evaluate_script(BACKUP_SAVE_SMOKE_SCRIPT) {
                                eprintln!("[Backup][SaveSmoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_backup_restore_smoke {
                            if let Err(error) = webview.evaluate_script(BACKUP_RESTORE_SMOKE_SCRIPT)
                            {
                                eprintln!("[Backup][RestoreSmoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_shelter_loader_probe {
                            if let Err(error) = webview.evaluate_script(SHELTER_LOADER_PROBE_SCRIPT) {
                                eprintln!("[ShelterLoader][Probe] probe failed: {error}");
                            }
                        }
                        if diagnostic_settings_boot_smoke {
                            if let Err(error) = webview.evaluate_script(SETTINGS_BOOT_SMOKE_SCRIPT) {
                                eprintln!("[SettingsBoot][Smoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_legcord_protocol_smoke {
                            if let Err(error) = webview.evaluate_script(LEGCORD_PROTOCOL_SMOKE_SCRIPT)
                            {
                                eprintln!("[LegcordProtocol][Smoke] probe failed: {error}");
                            } else {
                                // One-shot delayed readback: subresource loads
                                // settle asynchronously, so the flags are read
                                // ~5 s later on the event loop (DEV only).
                                let readback_proxy = proxy.clone();
                                if let Err(error) = thread::Builder::new()
                                    .name("legcord-protocol-smoke-readback".to_owned())
                                    .spawn(move || {
                                        thread::sleep(Duration::from_millis(5000));
                                        queue_event(
                                            &readback_proxy,
                                            NativeEvent::LegcordProtocolSmokeReadback,
                                        );
                                    })
                                {
                                    eprintln!(
                                        "[LegcordProtocol][Smoke] readback timer failed: {error}"
                                    );
                                }
                            }
                        }
                        if diagnostic_settings_ui_smoke {
                            if let Err(error) = webview.evaluate_script(&settings_ui_probe_script) {
                                eprintln!("[SettingsUi][Smoke] probe failed: {error}");
                            }
                        }
                        if diagnostic_permissions_smoke {
                            if let Err(error) = webview.evaluate_script(PERMISSIONS_SMOKE_SCRIPT) {
                                eprintln!("[Permissions][Smoke] probe failed: {error}");
                            } else {
                                // One-shot delayed readback: media and
                                // geolocation settle asynchronously, so the
                                // flags are read ~8 s later (DEV only).
                                let readback_proxy = proxy.clone();
                                if let Err(error) = thread::Builder::new()
                                    .name("legcord-permissions-smoke-readback".to_owned())
                                    .spawn(move || {
                                        thread::sleep(Duration::from_millis(8000));
                                        queue_event(
                                            &readback_proxy,
                                            NativeEvent::PermissionsSmokeReadback,
                                        );
                                    })
                                {
                                    eprintln!(
                                        "[Permissions][Smoke] readback timer failed: {error}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Event::UserEvent(NativeEvent::LegcordProtocolSmokeReadback) => {
                if diagnostic_legcord_protocol_smoke {
                    if let Err(error) = webview.evaluate_script_with_callback(
                        LEGCORD_PROTOCOL_SMOKE_READBACK_SCRIPT,
                        |result| {
                            println!("[LegcordProtocol][Smoke] subresources={result}");
                        },
                    ) {
                        eprintln!("[LegcordProtocol][Smoke] readback failed: {error}");
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
                        .name(format!("legcord-livecss-{tag}"))
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
                                window.set_visible(false);
                                respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_hide", &received_at);
                                response_count += 1;
                            }
                            BridgeCommand::WindowMinimize(_) => {
                                window.set_minimized(true);
                                respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_minimize", &received_at);
                                response_count += 1;
                            }
                            BridgeCommand::WindowMaximize(_) => {
                                window.set_maximized(true);
                                respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_maximize", &received_at);
                                response_count += 1;
                            }
                            BridgeCommand::WindowUnmaximize(_) => {
                                window.set_maximized(false);
                                respond_window_ack(&webview, &mut request_tracker, &request.request_id, "window_unmaximize", &received_at);
                                response_count += 1;
                            }
                            BridgeCommand::WindowQuit(_) => {
                                *control_flow = ControlFlow::Exit;
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
                        BridgeCommand::BackupRestore(_) => {
                            // Import only: the renderer supplies NOTHING (the
                            // empty payload is enforced by
                            // `deny_unknown_fields` at parse). The modal open
                            // dialog, the bounded archive read, full
                            // validation, staging, and the commit with
                            // rollback run on the dedicated STA restore
                            // worker, and the bounded outcome comes back as
                            // BackupRestoreReady. The renderer waits with the
                            // long-operation timeout (the picker stays open
                            // as long as the user needs).
                            let restore_request_id = request.request_id.clone();
                            let job = BackupRestoreJob {
                                request_id: restore_request_id.clone(),
                                received_at,
                                profile: profile_path.clone(),
                                proxy: proxy.clone(),
                            };
                            match restore_tx.send(job) {
                                Ok(()) => {
                                    println!("[Backup][Restore] restore request accepted");
                                }
                                Err(_) => {
                                    eprintln!(
                                        "[Backup][Restore:BACKUP_UNAVAILABLE] restore worker is unavailable"
                                    );
                                    let response = BridgeResponse {
                                        version: PROTOCOL_VERSION,
                                        request_id: restore_request_id.clone(),
                                        ok: false,
                                        payload: None,
                                        error: Some(BridgeError {
                                            code: "BACKUP_UNAVAILABLE",
                                            message: "backup restore is unavailable".to_owned(),
                                        }),
                                    };
                                    send_bridge_response(&webview, &response);
                                    if let Err(error) =
                                        request_tracker.complete(&restore_request_id)
                                    {
                                        eprintln!("[Bridge] correlation failure after response: {error}");
                                    }
                                    response_count += 1;
                                }
                            }
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
                            let cached_bundle = bundle_cache
                                .lock()
                                .ok()
                                .and_then(|guard| guard.get(descriptor.id));
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
                                    .name(format!("legcord-{}-bundle", descriptor.id))
                                    .spawn(move || {
                                        let retrieval_started_at = Instant::now();
                                        let result = load_mod_bundle(&descriptor, profile);
                                        if let Ok(ref bundle) = result {
                                            if let Ok(mut guard) = cache_ref.lock() {
                                                guard.insert(descriptor.id, bundle.clone());
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
                    format!("{}\n{}", mod_execution_context_script(&descriptor), bundle.js);
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
                                Some(SettingsPluginStart {
                                    script: &settings_plugin_start_script,
                                    started_generation: &mut settings_plugin_started_generation,
                                    start_count: &mut settings_plugin_start_count,
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
                    // The historically Vencord-named variants remain accepted so
                    // the already-validated Vencord wire contract is unchanged;
                    // each is normalized into the same mod-agnostic handlers.
                    BridgeEvent::VencordCssResult(payload) => {
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
                    BridgeEvent::SettingsBootRestart(_) => {
                        // Documented boundary: Electron relaunches the
                        // application (`app.relaunch()` + `app.exit()`); the
                        // native shell has no relauncher yet, so restart
                        // currently means a document reload.
                        println!("[SettingsBoot] restart requested; reloading document");
                        if let Err(error) = webview.evaluate_script("window.location.reload();") {
                            eprintln!("[SettingsBoot] restart reload failed: {error}");
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
        initialization_script, is_allowed_navigation, is_trusted_bridge_origin,
        lookup_mod_descriptor, mod_adapter_script, resolve_active_mods, settings_plugin_start_due,
        settings_plugin_start_script, settings_ui_probe_script, startup_visibility,
        StartupVisibility, EQUICORD_MOD, KNOWN_MODS, LEGCORD_PROTOCOL_SMOKE_SCRIPT,
        PERMISSIONS_SMOKE_SCRIPT, SETTINGS_PLUGIN_ID, SETTINGS_PLUGIN_START_TEMPLATE, SHELTER_MOD,
        VENCORD_MOD,
    };
    use crate::mod_substrate::{
        is_allowed_mod_download_url, mod_css_script, mod_execution_context_script,
        mod_marker_script, validate_descriptor, validate_mod_bundle, DocumentModLifecycle,
        ModEnabling,
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
        for id in ["vencord", "shelter", "equicord"] {
            let script = initialization_script(
                &adapters_for(&[id]),
                crate::settings_boot::test_fixture_json(),
                crate::app::test_fixture_json(),
            )
            .unwrap();
            assert!(
                !script.contains("__LEGCORD_BOOT_SNAPSHOT__"),
                "boot placeholder must be spliced"
            );
            assert!(
                !script.contains("__LEGCORD_SETTINGS_BOOT_SNAPSHOT__"),
                "settings boot placeholder must be spliced"
            );
            assert!(
                !script.contains("__LEGCORD_HOST_PLATFORM__"),
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
    fn legcord_protocol_smoke_probe_uses_only_fixed_bundle_urls() {
        // The DEV probe may request exactly three host-fixed bundle URLs that
        // exist on disk; it must not accept input, execute fetched bytes, or
        // open a generic fetch/eval surface.
        for fixed in [
            "legcord://assets/css/discord.css",
            "legcord://assets/desktop.png",
            "legcord://plugins/settings/plugin.js",
        ] {
            assert!(
                LEGCORD_PROTOCOL_SMOKE_SCRIPT.contains(fixed),
                "probe must reference the fixed bundle URL {fixed}"
            );
        }
        assert!(!LEGCORD_PROTOCOL_SMOKE_SCRIPT.contains("eval("));
        assert!(!LEGCORD_PROTOCOL_SMOKE_SCRIPT.contains("new Function"));
        assert!(!LEGCORD_PROTOCOL_SMOKE_SCRIPT.contains("innerHTML"));
        // No query/fragment: the resolver hardens by rejecting them.
        assert!(!LEGCORD_PROTOCOL_SMOKE_SCRIPT.contains('?'));
        assert!(!LEGCORD_PROTOCOL_SMOKE_SCRIPT.contains('#'));
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
        // (The embedded artifact legitimately carries legcord:// asset image
        // references from the Electron UI; those load as broken images
        // natively and are a documented cosmetic gap, not a loader input.)
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
        for token in ["legcord://", "postMessage", "http://", "https://"] {
            assert!(
                !SETTINGS_PLUGIN_START_TEMPLATE.contains(token),
                "template must not carry {token}"
            );
        }
        let script = settings_plugin_start_script().expect("the start script must build");
        assert!(!script.contains("__LEGCORD_SETTINGS_PLUGIN_"));
        assert!(script.contains("legcord-settings"));
        assert!(script.contains("Legcord Settings"));
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
    fn settings_ui_probe_script_carries_a_spliced_run_nonce() {
        let first = settings_ui_probe_script(11).expect("probe script must build");
        let second = settings_ui_probe_script(22).expect("probe script must build");
        assert!(!first.contains("__LEGCORD_SETTINGS_UI_RUN_NONCE__"));
        assert!(first.contains("const RUN_NONCE = \"11\""));
        assert!(!second.contains("__LEGCORD_SETTINGS_UI_RUN_NONCE__"));
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
        assert!(with_adapter.contains("__legcordNativeVencordAdapter"));
        assert!(with_adapter.contains("__legcordNativeModSubstrate"));
        assert!(without_adapter.contains("        getModBundle,"));
        assert!(without_adapter.contains("        getVencordBundle,"));
        assert!(without_adapter.contains("__legcordNativeModSubstrate"));
        assert!(!without_adapter.contains("__legcordNativeVencordAdapter"));
    }

    #[test]
    fn every_registered_mod_gets_exactly_its_own_adapter() {
        // The adapter appended for a mod must be that mod's file and no other:
        // one mod's identity can never be substituted for another's.
        let expectations = [
            ("vencord", "__legcordNativeVencordAdapter"),
            ("shelter", "__legcordNativeShelterAdapter"),
            ("equicord", "__legcordNativeEquicordAdapter"),
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
    fn initialization_script_preserves_window_prototype_storage() {
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(script.contains("__legcordNativeStorageAtDocumentStart"));
        assert!(script.contains("Window.prototype"));
        assert!(script.contains("defineProperty(Window.prototype, \"localStorage\""));
    }

    #[test]
    fn every_registered_descriptor_declares_its_traced_contract() {
        assert_eq!(KNOWN_MODS.len(), 3);
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
        assert_eq!(vencord_css.style_element_id, "legcord-native-vencord-css");
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

        // Nothing that is not registered resolves, and the registry is
        // EXACT-match only: no prefix, case, path, or URL shape selects one.
        for unregistered in [
            "custom",
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
        // A settings entry for an unregistered name selects nothing.
        let active = resolve_active_mods(&settings(&["custom", "DoNotChange"]), None);
        assert_eq!(
            active.iter().map(|d| d.id).collect::<Vec<_>>(),
            vec!["shelter"]
        );
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
        assert!(css.contains("legcord-native-vencord-css"));
        assert!(css.contains("vencord_css_result"));
        assert!(css.contains(r#"body::after{content:\"ok\"}"#));

        let context = mod_execution_context_script(&VENCORD_MOD);
        assert!(context.contains("VencordSettings"));
        assert!(context.contains("__legcordNativeVencordStorageAtExecution"));

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
        assert!(equicord_script.contains("legcord-native-equicord-css"));
        // Equicord reports through the MOD-AGNOSTIC CSS event family and
        // echoes its own host-supplied id, so the host can refuse a result
        // claimed for another mod. Vencord's legacy event name is untouched.
        assert!(equicord_script.contains("mod_css_result"));
        assert!(equicord_script.contains("mod_id: \"equicord\""));
        assert!(!equicord_script.contains("legcord-native-vencord-css"));
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
    fn bridge_origin_is_limited_to_stable_discord() {
        assert!(is_trusted_bridge_origin(
            &"https://discord.com/app".parse().unwrap()
        ));
        assert!(!is_trusted_bridge_origin(
            &"https://canary.discord.com/app".parse().unwrap()
        ));
        assert!(!is_trusted_bridge_origin(
            &"https://discord.com:443/app".parse().unwrap()
        ));
        assert!(!is_trusted_bridge_origin(
            &"https://example.com/app".parse().unwrap()
        ));
    }

    #[test]
    fn settings_requests_require_the_stable_discord_origin() {
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
    fn localization_requests_require_the_stable_discord_origin() {
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
            .contains("__LEGCORD_MUTATION_RESTORE_VALUE__"));
        assert!(super::SETTINGS_MUTATION_RESTORE_SCRIPT_TEMPLATE
            .contains("setBlockPowerSavingInVoiceChat(RESTORE_TO)"));
        assert!(!super::SETTINGS_MUTATION_RESTORE_SCRIPT_TEMPLATE.contains("setConfig("));
        let rendered = super::SETTINGS_MUTATION_RESTORE_SCRIPT_TEMPLATE
            .replace("__LEGCORD_MUTATION_RESTORE_VALUE__", "false");
        assert!(!rendered.contains("__LEGCORD_MUTATION_RESTORE_VALUE__"));
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
            .contains("__LEGCORD_SETTINGS_SET_RESTORE_VALUE__"));
        let rendered = super::SETTINGS_SET_RESTORE_SCRIPT_TEMPLATE
            .replace("__LEGCORD_SETTINGS_SET_RESTORE_VALUE__", "false");
        assert!(!rendered.contains("__LEGCORD_SETTINGS_SET_RESTORE_VALUE__"));
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
        assert!(!super::APP_VERSION_SMOKE_SCRIPT.contains("legcord.version"));
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
        assert!(!script.contains("window.legcord.fs"));
        assert!(!script.contains("legcord.fs"));
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
        assert!(!script.contains("quickCss"));
        assert!(!script.contains("legcord-native-theme"));
    }

    #[test]
    fn themes_smoke_observation_only_reads_managed_dom_nodes() {
        assert!(super::THEMES_SMOKE_SCRIPT.contains("legcord-native-theme-"));
        assert!(super::THEMES_SMOKE_SCRIPT.contains("querySelectorAll"));
        assert!(super::THEMES_SMOKE_SCRIPT.contains("legcord-native-vencord-css"));
        assert!(super::THEMES_SMOKE_SCRIPT.contains("legcord-quick-css"));
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
    fn boot_snapshot_grants_no_quickcss_capability() {
        // Quick CSS is a host push like themes: the renderer never requests
        // it, so no Quick CSS capability may exist anywhere near the bridge.
        for capability in super::BOOT_SNAPSHOT.capabilities {
            assert!(!capability.contains("quickcss"));
            assert!(!capability.contains("quick_css"));
            assert!(!capability.contains("quickCss"));
        }
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"filesystem"));
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"invoke"));
    }

    #[test]
    fn initialization_script_exposes_no_quickcss_pull_surface() {
        // The renderer pull stays rejected: no quickCss.get() shape may be
        // introduced alongside the host push.
        let script = initialization_script(
            &[],
            crate::settings_boot::test_fixture_json(),
            crate::app::test_fixture_json(),
        )
        .unwrap();
        assert!(!script.contains("quickCss"));
        assert!(!script.contains("quickcss"));
        assert!(!script.contains("quick_css"));
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
            if *capability == "themes.write_css" {
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
        assert!(super::PLUGIN_STORAGE_SMOKE_SCRIPT.contains("legcordNativeProbe"));
        assert!(super::PLUGIN_STORAGE_SMOKE_SCRIPT.contains("../escape.txt"));
        assert!(!super::PLUGIN_STORAGE_SMOKE_SCRIPT.contains("sendSync"));
        // The probe asserts the generic facade is ABSENT (`window.legcord.fs
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
        // Restore (D4-2): host-mediated open dialog plus bounded validate /
        // stage / commit — the renderer supplies nothing.
        assert!(super::BOOT_SNAPSHOT
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
        // Restore (D4-2): the parameter-free restore with the long-operation
        // timeout and the bounded clientMods outcome. No open-dialog send,
        // no sync IPC, and no generic invocation may ride along.
        assert!(script.contains("backup_restore"));
        assert!(script.contains("backup.restore"));
        assert!(script.contains("function restoreBackup("));
        assert!(script.contains("backup_restored"));
        assert!(!script.contains("backupRestore"));
        assert!(!script.contains("showOpenDialog"));
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn backup_save_runs_on_a_dedicated_named_worker() {
        // The modal save dialog must never run on the WebView/UI thread: the
        // save contract lives on BACKUP_SAVE_WORKER_NAME's documented worker.
        assert_eq!(super::BACKUP_SAVE_WORKER_NAME, "legcord-backup-save-sta");
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
            "legcord-backup-restore-sta"
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
        assert!(super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("backup.restore"));
        assert!(super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("backup_restore_probe"));
        assert!(super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("CANCELLED"));
        assert!(super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("stage"));
        assert!(super::BACKUP_RESTORE_SMOKE_SCRIPT.contains("client_mods_bytes"));
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
        // `custom.js` / `custom.css` are inventory-only and stay
        // non-executable natively (asserted by the electron-compat suite).
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
        assert_eq!(super::FOLDERS_SMOKE_ENV, "LEGCORD_NATIVE_FOLDERS_SMOKE");
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
            "LEGCORD_NATIVE_THEME_INSTALL_SMOKE"
        );
        assert!(super::THEME_INSTALL_SMOKE_SCRIPT.contains("installFromCode"));
        assert!(super::THEME_INSTALL_SMOKE_SCRIPT.contains("theme_install_probe"));
        assert!(super::THEME_INSTALL_SMOKE_SCRIPT.contains("LegcordNativeSmoke"));
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
            "LEGCORD_NATIVE_THEME_WRITE_SMOKE"
        );
        assert!(super::THEME_WRITE_SMOKE_SCRIPT.contains("writeCss"));
        assert!(super::THEME_WRITE_SMOKE_SCRIPT.contains("theme_write_probe"));
        assert!(super::THEME_WRITE_SMOKE_SCRIPT.contains("LegcordWriteProbe"));
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
        assert_eq!(super::THEME_IMPORT_WORKER_NAME, "legcord-theme-import-sta");
    }

    #[test]
    fn theme_import_smoke_probe_opens_the_real_picker_through_the_capability() {
        assert_eq!(
            super::THEME_IMPORT_SMOKE_ENV,
            "LEGCORD_NATIVE_THEME_IMPORT_SMOKE"
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
            "LEGCORD_NATIVE_THEME_SET_ENABLED_SMOKE"
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
        assert!(!script.contains("legcord.fs"));
        assert!(!script.contains("sendSync"));
    }

    #[test]
    fn theme_list_smoke_probe_inventories_through_the_capability() {
        assert_eq!(
            super::THEME_LIST_SMOKE_ENV,
            "LEGCORD_NATIVE_THEME_LIST_SMOKE"
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
            "LEGCORD_NATIVE_LOADER_PROBE"
        );
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("shelter_loader_probe"));
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("addLocalPlugin"));
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("startPlugin"));
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("stopPlugin"));
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("removePlugin"));
        assert!(super::SHELTER_LOADER_PROBE_SCRIPT.contains("legcord-loader-probe"));
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
        assert!(!super::SHELTER_LOADER_PROBE_SCRIPT.contains("legcord://"));
        assert!(!super::SHELTER_LOADER_PROBE_SCRIPT.contains("plugin.js"));
    }
}

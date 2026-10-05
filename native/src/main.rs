mod app;
mod bundles;
mod css_watch;
mod folders;
mod localization;
mod mod_substrate;
mod plugin_storage;
mod protocol;
mod quick_css;
mod settings;
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
use bundles::{describe_bundle_files, BUNDLE_FILES};
use css_watch::{
    diff_theme_snapshots, quick_css_transition, start_live_css_watchers, LiveCssCounters,
    LiveCssFsSignal, QuickCssTransition, ResourceDebouncer, CSS_DEBOUNCE_MS,
};
use folders::{
    ensure_folder_root, open_folder, resolve_theme_folder, reveal_folder, FolderLocation,
};
use localization::{lookup as lookup_lang_string, validate_key as validate_lang_key};
use plugin_storage::{read_plugin_file, write_plugin_file};
use protocol::{
    parse_bridge_message, validate_mod_bundle_id, AppVersionResponse, BridgeCommand, BridgeError,
    BridgeEvent, BridgeEventMessage, BridgeMessage, BridgeRequest, BridgeResponse,
    BridgeResponsePayload, BundleFileEntry, BundlesListResponse, FoldersOpenedResponse,
    LocalizationStringResponse, ModBundleResponse, PingResponse, PluginStorageReadResponse,
    PluginStorageWriteResponse, RequestTracker, SettingsConfigResponse, SettingsMutationResponse,
    ThemeFolderOpenedResponse, ThemeInstalledResponse, ThemeWrittenResponse, PROTOCOL_VERSION,
};
use quick_css::{load_quick_css, QUICK_CSS_STYLE_ID};
use serde::Serialize;
use settings::{apply_block_power_saving, load_settings};
use tao::{
    dpi::{LogicalSize, Size},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};
use themes::{install_theme_from_code, load_enabled_themes, write_theme_css, LoadedTheme};
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
#[cfg(target_os = "windows")]
use wry::WebViewExtWindows;
use wry::{
    http::{Request, Uri},
    NewWindowResponse, PageLoadEvent, WebContext, WebView, WebViewBuilder,
};

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
const GETLANG_SMOKE_ENV: &str = "LEGCORD_NATIVE_GETLANG_SMOKE";
const APP_VERSION_SMOKE_ENV: &str = "LEGCORD_NATIVE_APP_VERSION_SMOKE";
const PLUGIN_STORAGE_SMOKE_ENV: &str = "LEGCORD_NATIVE_PLUGIN_STORAGE_SMOKE";
const THEME_INSTALL_SMOKE_ENV: &str = "LEGCORD_NATIVE_THEME_INSTALL_SMOKE";
const THEME_WRITE_SMOKE_ENV: &str = "LEGCORD_NATIVE_THEME_WRITE_SMOKE";
const FOLDERS_SMOKE_ENV: &str = "LEGCORD_NATIVE_FOLDERS_SMOKE";
const THEMES_SMOKE_ENV: &str = "LEGCORD_NATIVE_THEMES_SMOKE";
const QUICKCSS_SMOKE_ENV: &str = "LEGCORD_NATIVE_QUICKCSS_SMOKE";
const BARE_PAGE_ENV: &str = "LEGCORD_NATIVE_DIAGNOSTIC_BARE_PAGE";
const INTERACTION_DIAGNOSTIC_ENV: &str = "LEGCORD_NATIVE_DIAGNOSTIC_INTERACTION";
const BOOT_SNAPSHOT: BootSnapshot = BootSnapshot {
    runtime: "webview2",
    platform: "win32",
    protocol_version: PROTOCOL_VERSION,
    capabilities: &[
        "bridge.ping",
        "mod.get_bundle",
        "settings.get_config",
        "settings.set_block_power_saving_in_voice_chat",
        "localization.get_lang",
        "app.get_version",
        "plugin_storage.read",
        "plugin_storage.write",
        "backup.list_bundles",
        "folders.open",
        "folders.open_theme",
        "themes.install_code",
        "themes.write_css",
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

/// DEV-only managed-styles observation (`LEGCORD_NATIVE_THEMES_SMOKE=1` or
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
    capabilities: &'static [&'static str],
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
    if (!(result instanceof Promise)) {
      probe({ ok: false, source: "unavailable", payload_bytes: 0, block_power_saving: false, error: "settings facade is not asynchronous" });
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
    url.parse::<Uri>().is_ok_and(|uri| is_discord_uri(&uri))
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
) -> Result<String, serde_json::Error> {
    let snapshot = serde_json::to_string(&BOOT_SNAPSHOT)?;
    let compatibility = COMPATIBILITY_SCRIPT.replace("__LEGCORD_BOOT_SNAPSHOT__", &snapshot);
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
        .with_title("Legcord — WebView2 DEV Prototype")
        .with_inner_size(Size::Logical(LogicalSize::new(1280.0, 800.0)))
        .with_min_inner_size(Size::Logical(LogicalSize::new(640.0, 480.0)))
        .build(&event_loop)?;

    let profile_path = PathBuf::from(WEBVIEW_PROFILE);
    std::fs::create_dir_all(&profile_path)?;
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
    let bundle_cache = Arc::new(Mutex::new(ModCache::default()));
    let ipc_proxy = proxy.clone();
    let load_proxy = proxy.clone();
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
    let initialization_script = if diagnostic_bare_page {
        println!("[Native] Bare Discord page diagnostic enabled; bridge/mod scripts are omitted");
        STORAGE_START_DIAGNOSTIC_SCRIPT.to_owned()
    } else {
        initialization_script(&active_mods)?
    };
    println!(
        "[Native] Document-start script: {} bytes",
        initialization_script.len()
    );
    let webview_builder = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_initialization_script_for_main_only(&initialization_script, true)
        .with_url(DISCORD_URL)
        .with_navigation_handler(|url| {
            let allowed = is_allowed_navigation(&url);
            println!(
                "[Navigation] {} {url}",
                if allowed { "allow" } else { "block" }
            );
            allowed
        })
        .with_new_window_req_handler(|url, _features| {
            println!("[Navigation] blocked popup: {url}");
            NewWindowResponse::Deny
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

    let webview = webview_builder.build(&window)?;
    println!("[Native] WebView2 prototype started");
    println!("[Native] Persistent DEV profile: {WEBVIEW_PROFILE}");
    println!("[Native] Initial navigation: {DISCORD_URL}");

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
                    Err(_) => {
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
                            csp_modified = true;
                        }
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
    let mut page_load_started_at = None;
    let mut incoming_messages = 0_u64;
    let mut request_count = 0_u64;
    let mut event_count = 0_u64;
    let mut response_count = 0_u64;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
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
                        }
                    },
                }
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
        lookup_mod_descriptor, mod_adapter_script, resolve_active_mods, EQUICORD_MOD, KNOWN_MODS,
        SHELTER_MOD, VENCORD_MOD,
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
    fn diagnostic_keeps_facade_and_substrate_but_omits_unselected_adapters() {
        let with_adapter = initialization_script(&adapters_for(&["vencord"])).unwrap();
        let without_adapter = initialization_script(&[]).unwrap();
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
            let script = initialization_script(&adapters_for(&[id])).unwrap();
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
            assert!(!initialization_script(&[])
                .unwrap()
                .contains("getModBundle(\"shelter\""));
        }
    }

    #[test]
    fn initialization_script_preserves_window_prototype_storage() {
        let script = initialization_script(&[]).unwrap();
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
        let script = initialization_script(&[]).unwrap();
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
        let script = initialization_script(&[]).unwrap();
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
        let script = initialization_script(&[]).unwrap();
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
        let script = initialization_script(&[]).unwrap();
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
        let script = initialization_script(&[]).unwrap();
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
        // `themes.install_code`, and `themes.write_css` are explicitly
        // allowed: the first names the themes ROOT for folder opening, the
        // others write theme files the host then pushes itself — none reads
        // theme CSS back to the renderer.
        for capability in super::BOOT_SNAPSHOT.capabilities {
            if *capability == "folders.open_theme"
                || *capability == "themes.install_code"
                || *capability == "themes.write_css"
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
        let script = initialization_script(&[]).unwrap();
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
        let script = initialization_script(&[]).unwrap();
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
        let script = initialization_script(&[]).unwrap();
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
    fn boot_snapshot_grants_only_bundle_discovery_for_backups() {
        assert!(super::BOOT_SNAPSHOT
            .capabilities
            .contains(&"backup.list_bundles"));
        // Save/restore (host-mediated dialogs + zip handling) are deferred:
        // discovery is the only backup-family capability that may exist.
        assert!(!super::BOOT_SNAPSHOT.capabilities.contains(&"backup.save"));
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
    fn initialization_script_exposes_namespaced_backup_discovery_facade() {
        let script = initialization_script(&[]).unwrap();
        assert!(script.contains("backup_list_bundles"));
        assert!(script.contains("backup.list_bundles"));
        assert!(script.contains("listBundles"));
        // No save/restore dialog surface may ride along with discovery.
        assert!(!script.contains("backupSave"));
        assert!(!script.contains("backupRestore"));
        assert!(!script.contains("showSaveDialog"));
        assert!(!script.contains("showOpenDialog"));
        assert!(!script.contains("sendSync"));
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
        let script = initialization_script(&[]).unwrap();
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
        let script = initialization_script(&[]).unwrap();
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
        let script = initialization_script(&[]).unwrap();
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
}

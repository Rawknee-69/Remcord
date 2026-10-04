mod mod_substrate;
mod protocol;
mod settings;

use std::{
    collections::HashMap,
    error::Error,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};

use mod_substrate::{
    load_mod_bundle, mod_css_script, mod_execution_context_script, mod_marker_script,
    stage as mod_stage, DocumentModLifecycle, ModCache, ModDescriptor, ModHostMetrics,
    PendingModBootstrap,
};

use protocol::{
    parse_bridge_message, BridgeCommand, BridgeError, BridgeEvent, BridgeEventMessage,
    BridgeMessage, BridgeRequest, BridgeResponse, BridgeResponsePayload, PingResponse,
    RequestTracker, SettingsConfigResponse, VencordBundleResponse, PROTOCOL_VERSION,
};
use serde::Serialize;
use settings::load_settings;
use tao::{
    dpi::{LogicalSize, Size},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};
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
    capability: "mod.vencord.get_bundle",
    js_url: "https://github.com/Vendicated/Vencord/releases/download/devbuild/browser.js",
    css_url: "https://github.com/Vendicated/Vencord/releases/download/devbuild/browser.css",
    js_file: "vencord.js",
    css_file: "vencord.css",
    allowed_hosts: &["github.com", "release-assets.githubusercontent.com"],
    max_js_bytes: 32 * 1024 * 1024,
    max_css_bytes: 8 * 1024 * 1024,
    version_header_prefix: "// Vencord ",
    style_element_id: "legcord-native-vencord-css",
    page_global: "Vencord",
    success_marker: "vencord-bootstrap-executed",
    settings_key: "VencordSettings",
    css_result_event: "vencord_css_result",
    bootstrap_dom_event: "legcord-native:vencord-bootstrap",
    storage_context_global: "__legcordNativeVencordStorageAtExecution",
    log_tag: "Mod:Vencord",
};

fn lookup_mod_descriptor(mod_id: &str) -> Option<ModDescriptor> {
    KNOWN_MODS
        .iter()
        .find(|descriptor| descriptor.id == mod_id)
        .copied()
}

/// Every mod the native host serves. Adding Shelter or Equicord means
/// appending a descriptor here plus a thin renderer adapter; the substrate
/// mechanics do not change.
const KNOWN_MODS: &[ModDescriptor] = &[VENCORD_MOD];
const DISABLE_VENCORD_ADAPTER_ENV: &str = "LEGCORD_NATIVE_DIAGNOSTIC_NO_VENCORD";
const SETTINGS_SMOKE_ENV: &str = "LEGCORD_NATIVE_SETTINGS_SMOKE";
const BARE_PAGE_ENV: &str = "LEGCORD_NATIVE_DIAGNOSTIC_BARE_PAGE";
const INTERACTION_DIAGNOSTIC_ENV: &str = "LEGCORD_NATIVE_DIAGNOSTIC_INTERACTION";
const BOOT_SNAPSHOT: BootSnapshot = BootSnapshot {
    runtime: "webview2",
    platform: "win32",
    protocol_version: PROTOCOL_VERSION,
    capabilities: &[
        "bridge.ping",
        "mod.vencord.get_bundle",
        "settings.get_config",
    ],
};
const COMPATIBILITY_SCRIPT: &str = include_str!("../web/compatibility.js");
const MOD_SUBSTRATE_SCRIPT: &str = include_str!("../web/mod-substrate/mod-bootstrap.js");
const VENCORD_ADAPTER_SCRIPT: &str = include_str!("../web/mods/vencord/adapter.js");

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

enum NativeEvent {
    PageLoadStarted,
    PageLoadFinished(String),
    BridgeRequest(BridgeRequest, Instant),
    BridgeEvent(BridgeEventMessage),
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

fn initialization_script(include_vencord_adapter: bool) -> Result<String, serde_json::Error> {
    let snapshot = serde_json::to_string(&BOOT_SNAPSHOT)?;
    let compatibility = COMPATIBILITY_SCRIPT.replace("__LEGCORD_BOOT_SNAPSHOT__", &snapshot);
    if include_vencord_adapter {
        Ok(format!(
            "{STORAGE_START_DIAGNOSTIC_SCRIPT}\n{compatibility}\n{MOD_SUBSTRATE_SCRIPT}\n{VENCORD_ADAPTER_SCRIPT}"
        ))
    } else {
        Ok(format!(
            "{STORAGE_START_DIAGNOSTIC_SCRIPT}\n{compatibility}\n{MOD_SUBSTRATE_SCRIPT}"
        ))
    }
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
    for descriptor in KNOWN_MODS {
        if !BOOT_SNAPSHOT.capabilities.contains(&descriptor.capability) {
            return Err(format!(
                "mod '{}' requires capability '{}' missing from the boot snapshot",
                descriptor.id, descriptor.capability
            )
            .into());
        }
    }
    let mut web_context = WebContext::new(Some(profile_path.clone()));
    let bundle_cache = Arc::new(Mutex::new(ModCache::default()));
    let ipc_proxy = proxy.clone();
    let load_proxy = proxy.clone();
    let include_vencord_adapter =
        !std::env::var(DISABLE_VENCORD_ADAPTER_ENV).is_ok_and(|value| value == "1");
    if !include_vencord_adapter {
        println!("[Mod:Vencord] automatic adapter disabled for DEV diagnostic comparison");
    }
    let diagnostic_bare_page = std::env::var(BARE_PAGE_ENV).is_ok_and(|value| value == "1");
    let diagnostic_interaction =
        std::env::var(INTERACTION_DIAGNOSTIC_ENV).is_ok_and(|value| value == "1");
    let diagnostic_settings_smoke =
        std::env::var(SETTINGS_SMOKE_ENV).is_ok_and(|value| value == "1");
    if diagnostic_settings_smoke {
        println!("[Settings][Smoke] settings capability smoke probe enabled");
    }
    let initialization_script = if diagnostic_bare_page {
        println!("[Native] Bare Discord page diagnostic enabled; bridge/mod scripts are omitted");
        STORAGE_START_DIAGNOSTIC_SCRIPT.to_owned()
    } else {
        initialization_script(include_vencord_adapter)?
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
    let mut smoke_reloads_remaining: u32 = std::env::var("LEGCORD_NATIVE_SMOKE_RELOAD")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    let mut smoke_reload_request_id = None::<String>;
    let mut request_tracker = RequestTracker::default();
    let mut document_lifecycle = DocumentModLifecycle::default();
    let mut pending_mods = HashMap::<String, PendingModBootstrap>::new();
    let mut mod_metrics = ModHostMetrics::default();
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
                    }
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
                        BridgeCommand::ModVencordGetBundle(_) => {
                            let descriptor = VENCORD_MOD;
                            mod_metrics.record_request(descriptor.id);
                            let request_id = request.request_id;
                            if !document_lifecycle.begin_mod(descriptor.id) {
                                mod_metrics.record_duplicate(descriptor.id);
                                send_mod_failure(
                                    &descriptor,
                                    &webview,
                                    &mut request_tracker,
                                    request_id.clone(),
                                    mod_stage::DUPLICATE_BOOTSTRAP,
                                    "Vencord bootstrap was already requested for this document"
                                        .to_owned(),
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
                                    .name("legcord-vencord-bundle".to_owned())
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
                                        format!("Could not start Vencord bundle retrieval: {error}"),
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
                    descriptor.log_tag, bundle.version, bundle.js_bytes, bundle.css_bytes
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
                        format!("WebView2 could not execute the Vencord bundle: {error}"),
                    );
                    response_count += 1;
                    return;
                }
                let execution_ms = execution_started_at.elapsed().as_secs_f64() * 1000.0;
                let css_script = match mod_css_script(&descriptor, &request_id, &bundle.css) {
                    Ok(script) => script,
                    Err(error) => {
                        send_mod_failure(
                            &descriptor,
                            &webview,
                            &mut request_tracker,
                            request_id,
                            mod_stage::ADAPTER_INITIALIZATION,
                            format!("Could not prepare the Vencord CSS adapter: {error}"),
                        );
                        response_count += 1;
                        return;
                    }
                };
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
                        css_injection_ms: 0.0,
                    },
                );
                println!(
                    "[{}][Metrics] bundle execution call completed in {execution_ms:.3} ms",
                    descriptor.log_tag
                );
                if let Err(error) = webview.evaluate_script(&css_script) {
                    pending_mods.remove(&request_id);
                    send_mod_failure(
                        &descriptor,
                        &webview,
                        &mut request_tracker,
                        request_id,
                        mod_stage::CSS_INJECTION,
                        format!("WebView2 could not run Vencord CSS injection: {error}"),
                    );
                    response_count += 1;
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
                    BridgeEvent::VencordAdapterError(payload) => {
                        eprintln!(
                            "[Mod:Vencord][adapter_initialization] {}",
                            payload.message
                        );
                    }
                    BridgeEvent::VencordCssResult(payload) => {
                        let Some(pending) = pending_mods.get_mut(&payload.request_id) else {
                            eprintln!("[Mod:Vencord][css_injection] unmatched CSS result");
                            return;
                        };
                        let Some(descriptor) = lookup_mod_descriptor(pending.mod_id) else {
                            pending_mods.remove(&payload.request_id);
                            return;
                        };
                        if pending.generation != document_lifecycle.generation() {
                            pending_mods.remove(&payload.request_id);
                            return;
                        }
                        if !payload.ok {
                            let error = payload
                                .error
                                .unwrap_or_else(|| "CSS injection failed without details".to_owned());
                            pending_mods.remove(&payload.request_id);
                            send_mod_failure(
                                &descriptor,
                                &webview,
                                &mut request_tracker,
                                payload.request_id,
                                mod_stage::CSS_INJECTION,
                                error,
                            );
                            response_count += 1;
                            return;
                        }
                        pending.css_injection_ms =
                            pending.css_started_at.elapsed().as_secs_f64() * 1000.0;
                        let marker_script = match mod_marker_script(
                            &descriptor,
                            &payload.request_id,
                            pending.execution_ms,
                        ) {
                            Ok(script) => script,
                            Err(error) => {
                                pending_mods.remove(&payload.request_id);
                                send_mod_failure(
                                    &descriptor,
                                    &webview,
                                    &mut request_tracker,
                                    payload.request_id,
                                    mod_stage::ADAPTER_INITIALIZATION,
                                    format!("Could not prepare the Vencord execution marker: {error}"),
                                );
                                response_count += 1;
                                return;
                            }
                        };
                        if let Err(error) = webview.evaluate_script(&marker_script) {
                            pending_mods.remove(&payload.request_id);
                            send_mod_failure(
                                &descriptor,
                                &webview,
                                &mut request_tracker,
                                payload.request_id,
                                mod_stage::ADAPTER_INITIALIZATION,
                                format!("Could not verify Vencord bootstrap execution: {error}"),
                            );
                            response_count += 1;
                        }
                    }
                    BridgeEvent::VencordBootstrapResult(payload) => {
                        let Some(pending) = pending_mods.remove(&payload.request_id) else {
                            eprintln!("[Mod:Vencord][script_execution] unmatched bootstrap result");
                            return;
                        };
                        let Some(descriptor) = lookup_mod_descriptor(pending.mod_id) else {
                            return;
                        };
                        if pending.generation != document_lifecycle.generation() {
                            return;
                        }
                        if !payload.ok || payload.marker != descriptor.success_marker {
                            send_mod_failure(
                                &descriptor,
                                &webview,
                                &mut request_tracker,
                                payload.request_id,
                                mod_stage::SCRIPT_EXECUTION,
                                payload.error.unwrap_or_else(|| {
                                    "Vencord execution marker did not match".to_owned()
                                }),
                            );
                            response_count += 1;
                            return;
                        }
                        if !document_lifecycle.record_mod_execution(descriptor.id) {
                            mod_metrics.record_duplicate(descriptor.id);
                            send_mod_failure(
                                &descriptor,
                                &webview,
                                &mut request_tracker,
                                payload.request_id,
                                mod_stage::DUPLICATE_BOOTSTRAP,
                                "Vencord executed more than once in one document".to_owned(),
                            );
                            response_count += 1;
                            return;
                        }

                        mod_metrics.record_bootstrap(descriptor.id);
                        let request_latency_ms =
                            pending.received_at.elapsed().as_secs_f64() * 1000.0;
                        println!(
                            "[{}][Metrics] marker={} execution_count={} request_latency_ms={request_latency_ms:.3} css_injection_ms={css_injection_ms:.3}",
                            descriptor.log_tag,
                            payload.marker,
                            document_lifecycle.mod_execution_count(descriptor.id),
                            css_injection_ms = pending.css_injection_ms
                        );
                        println!(
                            "[{}][Context] localStorage_getItem_at_execution={} VencordSettings={}",
                            descriptor.log_tag,
                            payload.local_storage_api,
                            payload.settings_state
                        );
                        let response = BridgeResponse {
                            version: PROTOCOL_VERSION,
                            request_id: payload.request_id.clone(),
                            ok: true,
                            payload: Some(BridgeResponsePayload::VencordBundle(
                                VencordBundleResponse {
                                    bundle_version: pending.bundle_version,
                                    bundle_bytes: pending.bundle_bytes,
                                    css_bytes: pending.css_bytes,
                                    request_latency_ms,
                                    retrieval_ms: pending.retrieval_ms,
                                    execution_ms: pending.execution_ms,
                                    css_injection_ms: pending.css_injection_ms,
                                    marker: descriptor.success_marker,
                                    document_execution_count:
                                        document_lifecycle.mod_execution_count(descriptor.id),
                                },
                            )),
                            error: None,
                        };
                        send_bridge_response(&webview, &response);
                        if smoke_reloads_remaining > 0 {
                            smoke_reload_request_id = Some(payload.request_id.clone());
                        }
                        if let Err(error) = request_tracker.complete(&payload.request_id) {
                            eprintln!("[Bridge] Vencord response correlation failure: {error}");
                        }
                        response_count += 1;
                    }
                    BridgeEvent::VencordResponseAck(payload) => {
                        if smoke_reload_request_id.as_deref() == Some(payload.request_id.as_str()) {
                            println!(
                                "[Mod:Vencord] renderer acknowledged execution marker response (request_id={})",
                                payload.request_id
                            );
                        }
                        if smoke_reloads_remaining > 0
                            && smoke_reload_request_id.as_deref() == Some(payload.request_id.as_str())
                        {
                            smoke_reloads_remaining -= 1;
                            smoke_reload_request_id = None;
                            println!(
                                "[Navigation] requesting Vencord DEV smoke reload (remaining={smoke_reloads_remaining})"
                            );
                            if let Err(error) = webview.evaluate_script("window.location.reload();") {
                                eprintln!("[Navigation] Vencord smoke reload failed: {error}");
                            }
                        }
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
                }
            }
            _ => {
                let _keep_window_alive = &window;
                let _keep_web_context_alive = &web_context;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        initialization_script, is_allowed_navigation, is_trusted_bridge_origin,
        lookup_mod_descriptor, KNOWN_MODS, VENCORD_MOD,
    };
    use crate::mod_substrate::{
        is_allowed_mod_download_url, mod_css_script, mod_execution_context_script,
        mod_marker_script, validate_mod_bundle, DocumentModLifecycle,
    };

    #[test]
    fn diagnostic_keeps_facade_and_substrate_but_can_omit_the_vencord_adapter() {
        let with_adapter = initialization_script(true).unwrap();
        let without_adapter = initialization_script(false).unwrap();
        assert!(with_adapter.contains("native.getVencordBundle"));
        assert!(with_adapter.contains("__legcordNativeVencordAdapter"));
        assert!(with_adapter.contains("__legcordNativeModSubstrate"));
        assert!(without_adapter.contains("        getVencordBundle,"));
        assert!(without_adapter.contains("__legcordNativeModSubstrate"));
        assert!(!without_adapter.contains("__legcordNativeVencordAdapter"));
    }

    #[test]
    fn initialization_script_preserves_window_prototype_storage() {
        let script = initialization_script(true).unwrap();
        assert!(script.contains("__legcordNativeStorageAtDocumentStart"));
        assert!(script.contains("Window.prototype"));
        assert!(script.contains("defineProperty(Window.prototype, \"localStorage\""));
    }

    #[test]
    fn vencord_descriptor_registers_exactly_one_typed_capability() {
        assert_eq!(KNOWN_MODS.len(), 1);
        let descriptor = lookup_mod_descriptor("vencord").unwrap();
        assert_eq!(descriptor.id, VENCORD_MOD.id);
        assert_eq!(descriptor.capability, "mod.vencord.get_bundle");
        assert_eq!(
            descriptor.js_url,
            "https://github.com/Vendicated/Vencord/releases/download/devbuild/browser.js"
        );
        assert_eq!(descriptor.js_file, "vencord.js");
        assert_eq!(descriptor.css_file, "vencord.css");
        assert_eq!(descriptor.style_element_id, "legcord-native-vencord-css");
        assert_eq!(descriptor.page_global, "Vencord");
        assert_eq!(descriptor.success_marker, "vencord-bootstrap-executed");
        assert_eq!(descriptor.settings_key, "VencordSettings");
        assert!(lookup_mod_descriptor("shelter").is_none());
        assert!(lookup_mod_descriptor("arbitrary").is_none());
    }

    #[test]
    fn native_document_lifecycle_allows_one_bootstrap_and_resets_after_navigation() {
        let mut lifecycle = DocumentModLifecycle::default();
        lifecycle.begin_document();
        assert_eq!(lifecycle.generation(), 1);
        assert!(lifecycle.begin_mod(VENCORD_MOD.id));
        assert!(!lifecycle.begin_mod(VENCORD_MOD.id));
        assert!(lifecycle.record_mod_execution(VENCORD_MOD.id));
        assert!(!lifecycle.record_mod_execution(VENCORD_MOD.id));

        lifecycle.begin_document();
        assert_eq!(lifecycle.generation(), 2);
        assert!(lifecycle.begin_mod(VENCORD_MOD.id));
        assert_eq!(lifecycle.mod_execution_count(VENCORD_MOD.id), 0);
    }

    #[test]
    fn accepts_only_fixed_github_vencord_asset_hosts() {
        assert!(is_allowed_mod_download_url(
            &VENCORD_MOD,
            "https://github.com/Vendicated/Vencord/releases/download/devbuild/browser.js"
        ));
        assert!(is_allowed_mod_download_url(
            &VENCORD_MOD,
            "https://release-assets.githubusercontent.com/asset/browser.js"
        ));
        assert!(!is_allowed_mod_download_url(
            &VENCORD_MOD,
            "http://github.com/Vendicated/Vencord/browser.js"
        ));
        assert!(!is_allowed_mod_download_url(
            &VENCORD_MOD,
            "https://github.com.attacker.invalid/browser.js"
        ));
    }

    #[test]
    fn validates_bundle_header_sizes_and_execution_marker() {
        let bundle = validate_mod_bundle(
            &VENCORD_MOD,
            b"// Vencord test-build\nvar Vencord = {};\n".to_vec(),
            b"/* test CSS */".to_vec(),
        )
        .unwrap();
        assert_eq!(bundle.version, "test-build");
        assert_eq!(bundle.js_bytes, bundle.js.len());
        assert_eq!(bundle.css_bytes, bundle.css.len());
        assert_eq!(VENCORD_MOD.success_marker, "vencord-bootstrap-executed");
        assert!(validate_mod_bundle(
            &VENCORD_MOD,
            b"var notVencord = {};".to_vec(),
            b"/*css*/".to_vec()
        )
        .is_err());
    }

    #[test]
    fn generates_fixed_css_and_marker_scripts_without_an_eval_surface() {
        let css =
            mod_css_script(&VENCORD_MOD, "request-1", r#"body::after{content:"ok"}"#).unwrap();
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
        let script = initialization_script(true).unwrap();
        assert!(script.contains("settings_get_config"));
        assert!(script.contains("settings_config"));
        assert!(script.contains("settings.get_config"));
        assert!(script.contains("getConfig"));
        // The facade must be asynchronous: no synchronous IPC emulation.
        assert!(!script.contains("sendSync"));
    }
}

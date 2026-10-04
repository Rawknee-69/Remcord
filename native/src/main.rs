mod protocol;

use std::{
    collections::HashMap,
    error::Error,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use protocol::{
    parse_bridge_message, BridgeCommand, BridgeError, BridgeEvent, BridgeEventMessage,
    BridgeMessage, BridgeRequest, BridgeResponse, BridgeResponsePayload, PingResponse,
    RequestTracker, VencordBundleResponse, PROTOCOL_VERSION,
};
use serde::Serialize;
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
const VENCORD_JS_URL: &str =
    "https://github.com/Vendicated/Vencord/releases/download/devbuild/browser.js";
const VENCORD_CSS_URL: &str =
    "https://github.com/Vendicated/Vencord/releases/download/devbuild/browser.css";
const VENCORD_JS_FILE: &str = "vencord.js";
const VENCORD_CSS_FILE: &str = "vencord.css";
const VENCORD_MARKER: &str = "vencord-bootstrap-executed";
const DISABLE_VENCORD_ADAPTER_ENV: &str = "LEGCORD_NATIVE_DIAGNOSTIC_NO_VENCORD";
const BARE_PAGE_ENV: &str = "LEGCORD_NATIVE_DIAGNOSTIC_BARE_PAGE";
const INTERACTION_DIAGNOSTIC_ENV: &str = "LEGCORD_NATIVE_DIAGNOSTIC_INTERACTION";
const MAX_VENCORD_JS_BYTES: usize = 32 * 1024 * 1024;
const MAX_VENCORD_CSS_BYTES: usize = 8 * 1024 * 1024;
const BOOT_SNAPSHOT: BootSnapshot = BootSnapshot {
    runtime: "webview2",
    platform: "win32",
    protocol_version: PROTOCOL_VERSION,
    capabilities: &["bridge.ping", "mod.vencord.get_bundle"],
};
const COMPATIBILITY_SCRIPT: &str = include_str!("../web/compatibility.js");
const VENCORD_ADAPTER_SCRIPT: &str = include_str!("../web/mods/vencord/adapter.js");

const STORAGE_START_DIAGNOSTIC_SCRIPT: &str = r#"
(() => {
  if (location.origin !== "https://discord.com") return;
  const describe = (descriptor) => {
    if (!descriptor) return "missing";
    if (Object.hasOwn(descriptor, "value")) return `value:${typeof descriptor.value}`;
    return `getter:${typeof descriptor.get}`;
  };
  let localStorageApi;
  try {
    localStorageApi = typeof window.localStorage?.getItem;
  } catch (error) {
    localStorageApi = `blocked:${error?.name || "unknown"}`;
  }
  try {
    const nativeStorage = window.localStorage;
    if (nativeStorage && typeof Window !== "undefined" && Window.prototype) {
      if (!Object.getOwnPropertyDescriptor(Window.prototype, "localStorage")) {
        Object.defineProperty(Window.prototype, "localStorage", {
          configurable: true,
          enumerable: true,
          get: () => nativeStorage,
        });
      }
    }
  } catch {}
  Object.defineProperty(window, "__legcordNativeStorageAtDocumentStart", {
    configurable: false,
    enumerable: false,
    value: Object.freeze({
      localStorageApi,
      windowDescriptor: describe(Object.getOwnPropertyDescriptor(window, "localStorage")),
      prototypeDescriptor: describe(
        Object.getOwnPropertyDescriptor(Object.getPrototypeOf(window), "localStorage")
      ),
      storageConstructor: typeof Storage
    })
  });
})();
"#;

const VENCORD_EXECUTION_CONTEXT_SCRIPT: &str = r#"
(() => {
  let localStorageApi = "blocked";
  let settingsState = "storage_unavailable";
  try {
    const storage = window.localStorage;
    localStorageApi = typeof storage?.getItem === "function" ? "available" : "missing";
    if (localStorageApi === "available") {
      try {
        const settings = storage.getItem("VencordSettings");
        if (settings === null) {
          settingsState = "missing";
        } else {
          JSON.parse(settings);
          settingsState = "valid_json";
        }
      } catch (error) {
        settingsState = error instanceof SyntaxError ? "invalid_json" : "read_error";
      }
    }
  } catch {}
  Object.defineProperty(globalThis, "__legcordNativeVencordStorageAtExecution", {
    configurable: false,
    enumerable: false,
    value: Object.freeze({ localStorageApi, settingsState })
  });
})();
"#;

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

enum NativeEvent {
    PageLoadStarted,
    PageLoadFinished(String),
    BridgeRequest(BridgeRequest, Instant),
    BridgeEvent(BridgeEventMessage),
    VencordBundleReady {
        generation: u64,
        request_id: String,
        received_at: Instant,
        retrieval_ms: f64,
        result: Result<VencordBundle, String>,
    },
}

#[derive(Clone)]
struct VencordBundle {
    js: String,
    css: String,
    version: String,
    js_bytes: usize,
    css_bytes: usize,
}

struct PendingVencord {
    generation: u64,
    received_at: Instant,
    retrieval_ms: f64,
    bundle_version: String,
    bundle_bytes: usize,
    css_bytes: usize,
    execution_ms: f64,
    css_started_at: Instant,
    css_injection_ms: f64,
}

#[derive(Default)]
struct DocumentLifecycle {
    generation: u64,
    vencord_started: bool,
    vencord_execution_count: u8,
}

impl DocumentLifecycle {
    fn begin_document(&mut self) {
        self.generation += 1;
        self.vencord_started = false;
        self.vencord_execution_count = 0;
    }

    fn begin_vencord(&mut self) -> bool {
        if self.vencord_started {
            return false;
        }
        self.vencord_started = true;
        true
    }

    fn record_vencord_execution(&mut self) -> bool {
        self.vencord_execution_count += 1;
        self.vencord_execution_count == 1
    }
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
            "{STORAGE_START_DIAGNOSTIC_SCRIPT}\n{compatibility}\n{VENCORD_ADAPTER_SCRIPT}"
        ))
    } else {
        Ok(format!(
            "{STORAGE_START_DIAGNOSTIC_SCRIPT}\n{compatibility}"
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

fn is_allowed_vencord_download_url(url: &str) -> bool {
    url.parse::<Uri>().is_ok_and(|uri| {
        uri.scheme_str() == Some("https")
            && uri.authority().is_some_and(|authority| {
                matches!(
                    authority.host().to_ascii_lowercase().as_str(),
                    "github.com" | "release-assets.githubusercontent.com"
                )
            })
    })
}

fn download_vencord_asset(
    agent: &ureq::Agent,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut current_url = url.to_owned();
    for _ in 0..=3 {
        if !is_allowed_vencord_download_url(&current_url) {
            return Err("Vencord asset redirect used an unapproved HTTPS host".to_owned());
        }

        let response = match agent.get(&current_url).call() {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(error) => return Err(format!("Vencord asset request failed: {error}")),
        };
        let status = response.status();
        if (200..300).contains(&status) {
            let mut bytes = Vec::new();
            response
                .into_reader()
                .take(max_bytes as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| format!("Could not read Vencord asset: {error}"))?;
            if bytes.is_empty() || bytes.len() > max_bytes {
                return Err("Vencord asset was empty or exceeded its size limit".to_owned());
            }
            return Ok(bytes);
        }

        if (300..400).contains(&status) {
            let location = response
                .header("Location")
                .ok_or_else(|| "Vencord asset redirect had no Location header".to_owned())?;
            current_url = location.to_owned();
            continue;
        }
        return Err(format!("Vencord asset request returned HTTP {status}"));
    }
    Err("Vencord asset exceeded the redirect limit".to_owned())
}

fn cached_or_downloaded_asset(
    agent: &ureq::Agent,
    profile: &Path,
    file_name: &str,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let path = profile.join(file_name);
    if path.exists() {
        let bytes =
            fs::read(&path).map_err(|error| format!("Could not read {file_name}: {error}"))?;
        if bytes.is_empty() || bytes.len() > max_bytes {
            return Err(format!(
                "{file_name} cache was empty or exceeded its size limit"
            ));
        }
        return Ok(bytes);
    }

    let bytes = download_vencord_asset(agent, url, max_bytes)?;
    let temp_path = profile.join(format!("{file_name}.tmp"));
    fs::write(&temp_path, &bytes)
        .map_err(|error| format!("Could not cache {file_name} in the DEV profile: {error}"))?;
    fs::rename(&temp_path, &path)
        .map_err(|error| format!("Could not finalize {file_name} in the DEV profile: {error}"))?;
    Ok(bytes)
}

fn load_vencord_bundle(profile: PathBuf) -> Result<VencordBundle, String> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(30))
        .build();
    let js = cached_or_downloaded_asset(
        &agent,
        &profile,
        VENCORD_JS_FILE,
        VENCORD_JS_URL,
        MAX_VENCORD_JS_BYTES,
    )?;
    let css = cached_or_downloaded_asset(
        &agent,
        &profile,
        VENCORD_CSS_FILE,
        VENCORD_CSS_URL,
        MAX_VENCORD_CSS_BYTES,
    )?;
    validate_vencord_bundle(js, css)
}

fn validate_vencord_bundle(js: Vec<u8>, css: Vec<u8>) -> Result<VencordBundle, String> {
    if js.is_empty() || js.len() > MAX_VENCORD_JS_BYTES {
        return Err("Vencord JS bundle was empty or exceeded its size limit".to_owned());
    }
    if css.is_empty() || css.len() > MAX_VENCORD_CSS_BYTES {
        return Err("Vencord CSS bundle was empty or exceeded its size limit".to_owned());
    }
    let js = String::from_utf8(js)
        .map_err(|error| format!("Vencord JS bundle is not UTF-8: {error}"))?;
    let css = String::from_utf8(css)
        .map_err(|error| format!("Vencord CSS bundle is not UTF-8: {error}"))?;
    let version = js
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("// Vencord "))
        .filter(|version| !version.is_empty() && version.len() <= 64)
        .ok_or_else(|| "Vencord bundle did not contain the expected build header".to_owned())?
        .to_owned();

    Ok(VencordBundle {
        js_bytes: js.len(),
        css_bytes: css.len(),
        js,
        css,
        version,
    })
}

fn vencord_css_script(request_id: &str, css: &str) -> Result<String, serde_json::Error> {
    let request_id = serde_json::to_string(request_id)?;
    let css = serde_json::to_string(css)?;
    Ok(format!(
        r#"(() => {{
  const requestId = {request_id};
  const report = (ok, error = null) => window.ipc.postMessage(JSON.stringify({{
    version: 1,
    event: {{ type: "vencord_css_result", payload: {{ request_id: requestId, ok, error }} }}
  }}));
  const inject = () => {{
  try {{
    const root = document.head;
    if (!root) return false;
    let style = document.getElementById("legcord-native-vencord-css");
    if (!style) {{
      style = document.createElement("style");
      style.id = "legcord-native-vencord-css";
    }}
    style.textContent = {css};
    root.appendChild(style);
    if (!style.sheet) throw new Error("Vencord stylesheet was rejected by the document");
    report(true);
    return true;
  }} catch (error) {{
    report(false, String(error?.message || error).slice(0, 2048));
    return true;
  }}
  }};
  if (!inject()) document.addEventListener("DOMContentLoaded", inject, {{ once: true }});
}})();"#
    ))
}

fn vencord_marker_script(request_id: &str, execution_ms: f64) -> Result<String, serde_json::Error> {
    let request_id = serde_json::to_string(request_id)?;
    let marker = serde_json::to_string(VENCORD_MARKER)?;
    Ok(format!(
        r#"(() => {{
  const requestId = {request_id};
  const marker = {marker};
  const storageContext = globalThis.__legcordNativeVencordStorageAtExecution || {{}};
  const localStorageApi = storageContext.localStorageApi || "blocked";
  const settingsState = storageContext.settingsState || "storage_unavailable";
  let result;
  try {{
    if (!globalThis.Vencord || typeof globalThis.Vencord !== "object") {{
      throw new Error("The Vencord global was not initialized by the bundle");
    }}
    result = {{
      request_id: requestId,
      ok: true,
      marker,
      execution_ms: {execution_ms},
      local_storage_api: localStorageApi,
      settings_state: settingsState,
      error: null
    }};
  }} catch (error) {{
    result = {{
      request_id: requestId,
      ok: false,
      marker: "",
      execution_ms: {execution_ms},
      local_storage_api: localStorageApi,
      settings_state: settingsState,
      error: String(error?.message || error).slice(0, 2048)
    }};
  }}
  window.dispatchEvent(new CustomEvent("legcord-native:vencord-bootstrap", {{ detail: result }}));
}})();"#
    ))
}

fn send_vencord_failure(
    webview: &WebView,
    request_tracker: &mut RequestTracker,
    request_id: String,
    code: &'static str,
    message: String,
) {
    eprintln!("[Mod:Vencord][{code}] {message}");
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
    fs::create_dir_all(&profile_path)?;
    let mut web_context = WebContext::new(Some(profile_path.clone()));
    let bundle_cache = Arc::new(Mutex::new(None::<Arc<VencordBundle>>));
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
    let mut document_lifecycle = DocumentLifecycle::default();
    let mut pending_vencord = HashMap::<String, PendingVencord>::new();
    let mut vencord_capability_request_count = 0_u64;
    let mut vencord_execution_count = 0_u64;
    let mut vencord_duplicate_count = 0_u64;
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
                println!(
                    "[Mod:Vencord][Metrics] capability_requests={vencord_capability_request_count} successful_document_bootstraps={vencord_execution_count} duplicate_bootstraps={vencord_duplicate_count}"
                );
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(NativeEvent::PageLoadStarted) => {
                document_lifecycle.begin_document();
                request_tracker.clear();
                pending_vencord.clear();
                page_load_started_at = Some(Instant::now());
                println!(
                    "[Navigation] new document generation={} (bridge state reset)",
                    document_lifecycle.generation
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
                        BridgeCommand::ModVencordGetBundle(_) => {
                            vencord_capability_request_count += 1;
                            let request_id = request.request_id;
                            if !document_lifecycle.begin_vencord() {
                                vencord_duplicate_count += 1;
                                send_vencord_failure(
                                    &webview,
                                    &mut request_tracker,
                                    request_id.clone(),
                                    "duplicate_bootstrap",
                                    "Vencord bootstrap was already requested for this document"
                                        .to_owned(),
                                );
                                response_count += 1;
                                return;
                            }

                            let generation = document_lifecycle.generation;
                            let cached_bundle = bundle_cache
                                .lock()
                                .ok()
                                .and_then(|guard| guard.as_ref().cloned());
                            if let Some(bundle) = cached_bundle {
                                println!(
                                    "[Mod:Vencord][capability] request accepted from in-memory cache (document_generation={generation})"
                                );
                                queue_event(
                                    &proxy,
                                    NativeEvent::VencordBundleReady {
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
                                        let result = load_vencord_bundle(profile);
                                        if let Ok(ref bundle) = result {
                                            if let Ok(mut guard) = cache_ref.lock() {
                                                *guard = Some(Arc::new(bundle.clone()));
                                            }
                                        }
                                        let retrieval_ms =
                                            retrieval_started_at.elapsed().as_secs_f64() * 1000.0;
                                        queue_event(
                                            &worker_proxy,
                                            NativeEvent::VencordBundleReady {
                                                generation,
                                                request_id: worker_request_id,
                                                received_at,
                                                retrieval_ms,
                                                result,
                                            },
                                        );
                                    })
                                {
                                    send_vencord_failure(
                                        &webview,
                                        &mut request_tracker,
                                        request_id,
                                        "bundle_retrieval",
                                        format!("Could not start Vencord bundle retrieval: {error}"),
                                    );
                                    response_count += 1;
                                } else {
                                    println!(
                                        "[Mod:Vencord][capability] request accepted (document_generation={generation})"
                                    );
                                }
                            }
                        }
                    },
                }
            }
            Event::UserEvent(NativeEvent::VencordBundleReady {
                generation,
                request_id,
                received_at,
                retrieval_ms,
                result,
            }) => {
                if generation != document_lifecycle.generation {
                    println!(
                        "[Mod:Vencord][lifecycle] discarded stale bundle result (generation={generation})"
                    );
                    return;
                }
                let bundle = match result {
                    Ok(bundle) => bundle,
                    Err(error) => {
                        send_vencord_failure(
                            &webview,
                            &mut request_tracker,
                            request_id,
                            "bundle_retrieval",
                            error,
                        );
                        response_count += 1;
                        return;
                    }
                };
                println!(
                    "[Mod:Vencord][Metrics] bundle_version={} bundle_bytes={} css_bytes={} retrieval_ms={retrieval_ms:.3}",
                    bundle.version, bundle.js_bytes, bundle.css_bytes
                );

                let execution_started_at = Instant::now();
                let execution_script =
                    format!("{VENCORD_EXECUTION_CONTEXT_SCRIPT}\n{}", bundle.js);
                if let Err(error) = webview.evaluate_script(&execution_script) {
                    send_vencord_failure(
                        &webview,
                        &mut request_tracker,
                        request_id,
                        "script_execution",
                        format!("WebView2 could not execute the Vencord bundle: {error}"),
                    );
                    response_count += 1;
                    return;
                }
                let execution_ms = execution_started_at.elapsed().as_secs_f64() * 1000.0;
                let css_script = match vencord_css_script(&request_id, &bundle.css) {
                    Ok(script) => script,
                    Err(error) => {
                        send_vencord_failure(
                            &webview,
                            &mut request_tracker,
                            request_id,
                            "adapter_initialization",
                            format!("Could not prepare the Vencord CSS adapter: {error}"),
                        );
                        response_count += 1;
                        return;
                    }
                };
                pending_vencord.insert(
                    request_id.clone(),
                    PendingVencord {
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
                    "[Mod:Vencord][Metrics] bundle execution call completed in {execution_ms:.3} ms"
                );
                if let Err(error) = webview.evaluate_script(&css_script) {
                    pending_vencord.remove(&request_id);
                    send_vencord_failure(
                        &webview,
                        &mut request_tracker,
                        request_id,
                        "css_injection",
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
                        let Some(pending) = pending_vencord.get_mut(&payload.request_id) else {
                            eprintln!("[Mod:Vencord][css_injection] unmatched CSS result");
                            return;
                        };
                        if pending.generation != document_lifecycle.generation {
                            pending_vencord.remove(&payload.request_id);
                            return;
                        }
                        if !payload.ok {
                            let error = payload
                                .error
                                .unwrap_or_else(|| "CSS injection failed without details".to_owned());
                            pending_vencord.remove(&payload.request_id);
                            send_vencord_failure(
                                &webview,
                                &mut request_tracker,
                                payload.request_id,
                                "css_injection",
                                error,
                            );
                            response_count += 1;
                            return;
                        }
                        pending.css_injection_ms =
                            pending.css_started_at.elapsed().as_secs_f64() * 1000.0;
                        let marker_script = match vencord_marker_script(
                            &payload.request_id,
                            pending.execution_ms,
                        ) {
                            Ok(script) => script,
                            Err(error) => {
                                pending_vencord.remove(&payload.request_id);
                                send_vencord_failure(
                                    &webview,
                                    &mut request_tracker,
                                    payload.request_id,
                                    "adapter_initialization",
                                    format!("Could not prepare the Vencord execution marker: {error}"),
                                );
                                response_count += 1;
                                return;
                            }
                        };
                        if let Err(error) = webview.evaluate_script(&marker_script) {
                            pending_vencord.remove(&payload.request_id);
                            send_vencord_failure(
                                &webview,
                                &mut request_tracker,
                                payload.request_id,
                                "adapter_initialization",
                                format!("Could not verify Vencord bootstrap execution: {error}"),
                            );
                            response_count += 1;
                        }
                    }
                    BridgeEvent::VencordBootstrapResult(payload) => {
                        let Some(pending) = pending_vencord.remove(&payload.request_id) else {
                            eprintln!("[Mod:Vencord][script_execution] unmatched bootstrap result");
                            return;
                        };
                        if pending.generation != document_lifecycle.generation {
                            return;
                        }
                        if !payload.ok || payload.marker != VENCORD_MARKER {
                            send_vencord_failure(
                                &webview,
                                &mut request_tracker,
                                payload.request_id,
                                "script_execution",
                                payload.error.unwrap_or_else(|| {
                                    "Vencord execution marker did not match".to_owned()
                                }),
                            );
                            response_count += 1;
                            return;
                        }
                        if !document_lifecycle.record_vencord_execution() {
                            vencord_duplicate_count += 1;
                            send_vencord_failure(
                                &webview,
                                &mut request_tracker,
                                payload.request_id,
                                "duplicate_bootstrap",
                                "Vencord executed more than once in one document".to_owned(),
                            );
                            response_count += 1;
                            return;
                        }

                        vencord_execution_count += 1;
                        let request_latency_ms =
                            pending.received_at.elapsed().as_secs_f64() * 1000.0;
                        println!(
                            "[Mod:Vencord][Metrics] marker={} execution_count={} request_latency_ms={request_latency_ms:.3} css_injection_ms={css_injection_ms:.3}",
                            payload.marker,
                            document_lifecycle.vencord_execution_count,
                            css_injection_ms = pending.css_injection_ms
                        );
                        println!(
                            "[Mod:Vencord][Context] localStorage_getItem_at_execution={} VencordSettings={}",
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
                                    marker: VENCORD_MARKER,
                                    document_execution_count:
                                        document_lifecycle.vencord_execution_count,
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
        initialization_script, is_allowed_navigation, is_allowed_vencord_download_url,
        is_trusted_bridge_origin, validate_vencord_bundle, vencord_css_script,
        vencord_marker_script, DocumentLifecycle, VENCORD_MARKER,
    };

    #[test]
    fn diagnostic_can_omit_only_the_automatic_vencord_adapter() {
        let with_adapter = initialization_script(true).unwrap();
        let without_adapter = initialization_script(false).unwrap();
        assert!(with_adapter.contains("native.getVencordBundle"));
        assert!(with_adapter.contains("__legcordNativeVencordAdapter"));
        assert!(without_adapter.contains("        getVencordBundle,"));
        assert!(!without_adapter.contains("__legcordNativeVencordAdapter"));
    }

    #[test]
    fn native_document_lifecycle_allows_one_bootstrap_and_resets_after_navigation() {
        let mut lifecycle = DocumentLifecycle::default();
        lifecycle.begin_document();
        assert_eq!(lifecycle.generation, 1);
        assert!(lifecycle.begin_vencord());
        assert!(!lifecycle.begin_vencord());
        assert!(lifecycle.record_vencord_execution());
        assert!(!lifecycle.record_vencord_execution());

        lifecycle.begin_document();
        assert_eq!(lifecycle.generation, 2);
        assert!(lifecycle.begin_vencord());
        assert_eq!(lifecycle.vencord_execution_count, 0);
    }

    #[test]
    fn accepts_only_fixed_github_vencord_asset_hosts() {
        assert!(is_allowed_vencord_download_url(
            "https://github.com/Vendicated/Vencord/releases/download/devbuild/browser.js"
        ));
        assert!(is_allowed_vencord_download_url(
            "https://release-assets.githubusercontent.com/asset/browser.js"
        ));
        assert!(!is_allowed_vencord_download_url(
            "http://github.com/Vendicated/Vencord/browser.js"
        ));
        assert!(!is_allowed_vencord_download_url(
            "https://github.com.attacker.invalid/browser.js"
        ));
    }

    #[test]
    fn validates_bundle_header_sizes_and_execution_marker() {
        let bundle = validate_vencord_bundle(
            b"// Vencord test-build\nvar Vencord = {};\n".to_vec(),
            b"/* test CSS */".to_vec(),
        )
        .unwrap();
        assert_eq!(bundle.version, "test-build");
        assert_eq!(bundle.js_bytes, bundle.js.len());
        assert_eq!(bundle.css_bytes, bundle.css.len());
        assert_eq!(VENCORD_MARKER, "vencord-bootstrap-executed");
        assert!(
            validate_vencord_bundle(b"var notVencord = {};".to_vec(), b"/*css*/".to_vec()).is_err()
        );
    }

    #[test]
    fn generates_fixed_css_and_marker_scripts_without_an_eval_surface() {
        let css = vencord_css_script("request-1", r#"body::after{content:"ok"}"#).unwrap();
        assert!(css.contains("document.head"));
        assert!(css.contains("legcord-native-vencord-css"));
        assert!(css.contains("vencord_css_result"));
        assert!(css.contains(r#"body::after{content:\"ok\"}"#));

        let marker = vencord_marker_script("request-1", 1.5).unwrap();
        assert!(marker.contains("globalThis.Vencord"));
        assert!(marker.contains("local_storage_api"));
        assert!(marker.contains(VENCORD_MARKER));
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
}

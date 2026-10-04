use std::{error::Error, path::PathBuf};

use serde::{Deserialize, Serialize};
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
const MAX_IPC_MESSAGE_BYTES: usize = 8 * 1024;
const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_ERROR_BYTES: usize = 2 * 1024;

const INITIALIZATION_SCRIPT: &str = r#"
(() => {
  "use strict";

  let sequence = 0;
  const pending = new Map();

  function nextId() {
    sequence += 1;
    return `legcord-${Date.now()}-${sequence}`;
  }

  function post(command) {
    if (!window.ipc || typeof window.ipc.postMessage !== "function") {
      throw new Error("Native bridge transport is unavailable");
    }
    window.ipc.postMessage(JSON.stringify({
      version: 1,
      request_id: nextId(),
      command
    }));
  }

  function reportRendererError(kind, message) {
    try {
      post({
        type: "renderer_error",
        payload: { kind, message: String(message).slice(0, 2048) }
      });
    } catch {
      // Renderer error reporting must not interfere with Discord's own error handling.
    }
  }

  Object.defineProperty(window, "__LEGCORD_NATIVE_EARLY__", {
    value: true,
    configurable: false,
    enumerable: false
  });

  Object.defineProperty(window, "legcordNative", {
    configurable: false,
    enumerable: false,
    value: Object.freeze({
      ping(nonce) {
        if (typeof nonce !== "string" || nonce.length > 128) {
          return Promise.reject(new TypeError("nonce must be a string of at most 128 characters"));
        }

        const requestId = nextId();
        return new Promise((resolve, reject) => {
          const timer = setTimeout(() => {
            pending.delete(requestId);
            reject(new Error("Native bridge request timed out"));
          }, 5000);
          pending.set(requestId, { resolve, reject, timer });
          try {
            window.ipc.postMessage(JSON.stringify({
              version: 1,
              request_id: requestId,
              command: { type: "ping", payload: { nonce } }
            }));
          } catch (error) {
            clearTimeout(timer);
            pending.delete(requestId);
            reject(error);
          }
        });
      },
      reportCss(value) {
        post({
          type: "css_probe",
          payload: { value: String(value).slice(0, 128) }
        });
      }
    })
  });

  window.addEventListener("legcord-native:response", (event) => {
    const response = event.detail;
    const request = response && pending.get(response.request_id);
    if (!request) return;

    clearTimeout(request.timer);
    pending.delete(response.request_id);
    if (response.ok && response.payload) {
      request.resolve(response.payload);
      post({
        type: "round_trip_ack",
        payload: { nonce: response.payload.nonce }
      });
    } else {
      request.reject(new Error(response.error || "Native bridge request failed"));
    }
  });

  window.addEventListener("pagehide", () => {
    for (const request of pending.values()) clearTimeout(request.timer);
    pending.clear();
  });

  window.addEventListener("error", (event) => {
    reportRendererError("uncaught", event.message || "Uncaught renderer error");
  });
  window.addEventListener("unhandledrejection", (event) => {
    reportRendererError("unhandled_rejection", event.reason);
  });

  const originalConsoleError = console.error.bind(console);
  console.error = (...values) => {
    originalConsoleError(...values);
    reportRendererError("console.error", values.map((value) => {
      try {
        return String(value);
      } catch {
        return "[unprintable renderer error]";
      }
    }).join(" "));
  };

  console.info("[Legcord native DEV] document-start script executed");
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
  window.legcordNative.reportCss(
    getComputedStyle(document.documentElement)
      .getPropertyValue("--legcord-native-prototype-css")
      .trim()
  );
})();
"#;

const BRIDGE_PROBE_SCRIPT: &str = r#"
window.legcordNative.ping("discord-load")
  .then((result) => {
    document.documentElement.dataset.legcordNativeBridge = result.message;
    console.info("[Legcord native DEV] bridge round-trip succeeded");
  })
  .catch((error) => console.error("[Legcord native DEV] bridge round-trip failed", error));
"#;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BridgeRequest {
    version: u8,
    request_id: String,
    command: BridgeCommand,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum BridgeCommand {
    Ping(PingRequest),
    RoundTripAck(RoundTripAck),
    CssProbe(CssProbe),
    RendererError(RendererError),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PingRequest {
    nonce: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoundTripAck {
    nonce: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CssProbe {
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RendererError {
    kind: String,
    message: String,
}

#[derive(Serialize)]
struct BridgeResponse {
    request_id: String,
    ok: bool,
    payload: Option<PingResponse>,
    error: Option<&'static str>,
}

#[derive(Serialize)]
struct PingResponse {
    nonce: String,
    message: &'static str,
}

enum NativeEvent {
    PageLoadFinished(String),
    BridgeRequest(BridgeRequest),
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

fn parse_bridge_request(body: &str) -> Result<BridgeRequest, String> {
    if body.len() > MAX_IPC_MESSAGE_BYTES {
        return Err("message exceeds the size limit".to_owned());
    }

    let request: BridgeRequest = serde_json::from_str(body).map_err(|error| error.to_string())?;
    if request.version != 1 {
        return Err("unsupported bridge version".to_owned());
    }
    if request.request_id.is_empty()
        || request.request_id.len() > MAX_REQUEST_ID_BYTES
        || !request
            .request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("invalid request id".to_owned());
    }

    match &request.command {
        BridgeCommand::Ping(payload) => {
            if payload.nonce.is_empty() || payload.nonce.len() > 128 {
                return Err("invalid nonce".to_owned());
            }
        }
        BridgeCommand::RoundTripAck(payload) => {
            if payload.nonce.is_empty() || payload.nonce.len() > 128 {
                return Err("invalid nonce".to_owned());
            }
        }
        BridgeCommand::CssProbe(payload) => {
            if payload.value.len() > 128 {
                return Err("CSS probe result exceeds the size limit".to_owned());
            }
        }
        BridgeCommand::RendererError(payload) => {
            if payload.kind.is_empty()
                || payload.kind.len() > 64
                || payload.message.len() > MAX_ERROR_BYTES
            {
                return Err("invalid renderer error report".to_owned());
            }
        }
    }

    Ok(request)
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
            let script = format!(
                "window.dispatchEvent(new CustomEvent('legcord-native:response', {{ detail: {serialized} }}));"
            );
            if let Err(error) = webview.evaluate_script(&script) {
                eprintln!("[Bridge] Could not deliver response to renderer: {error}");
            }
        }
        Err(error) => eprintln!("[Bridge] Could not serialize response: {error}"),
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

    let mut web_context = WebContext::new(Some(PathBuf::from(WEBVIEW_PROFILE)));
    let ipc_proxy = proxy.clone();
    let load_proxy = proxy.clone();
    let webview_builder = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_initialization_script_for_main_only(INITIALIZATION_SCRIPT, true)
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
            if matches!(event, PageLoadEvent::Finished) {
                queue_event(&load_proxy, NativeEvent::PageLoadFinished(url));
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

            match parse_bridge_request(request.body()) {
                Ok(request) => queue_event(&ipc_proxy, NativeEvent::BridgeRequest(request)),
                Err(error) => eprintln!("[Bridge] rejected invalid message: {error}"),
            }
        });

    let webview = webview_builder.build(&window)?;
    println!("[Native] WebView2 prototype started");
    println!("[Native] Persistent DEV profile: {WEBVIEW_PROFILE}");
    println!("[Native] Initial navigation: {DISCORD_URL}");
    let mut smoke_reload_pending =
        std::env::var("LEGCORD_NATIVE_SMOKE_RELOAD").is_ok_and(|value| value == "1");

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                println!("[Native] Close requested; shutting down WebView");
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(NativeEvent::PageLoadFinished(url)) => {
                if is_allowed_navigation(&url) {
                    if let Err(error) = webview.evaluate_script(CSS_INJECTION_SCRIPT) {
                        eprintln!("[Injection] CSS script failed: {error}");
                    }
                    if let Err(error) = webview.evaluate_script(BRIDGE_PROBE_SCRIPT) {
                        eprintln!("[Injection] Bridge probe failed: {error}");
                    }
                }
            }
            Event::UserEvent(NativeEvent::BridgeRequest(request)) => match request.command {
                BridgeCommand::Ping(payload) => {
                    send_bridge_response(
                        &webview,
                        &BridgeResponse {
                            request_id: request.request_id,
                            ok: true,
                            payload: Some(PingResponse {
                                nonce: payload.nonce,
                                message: "pong",
                            }),
                            error: None,
                        },
                    );
                }
                BridgeCommand::RoundTripAck(payload) => {
                    println!(
                        "[Bridge] renderer received Rust reply; round-trip complete (nonce={})",
                        payload.nonce
                    );
                    if smoke_reload_pending {
                        smoke_reload_pending = false;
                        println!("[Navigation] requesting one-shot DEV smoke reload");
                        if let Err(error) = webview.evaluate_script("window.location.reload();") {
                            eprintln!("[Navigation] smoke reload failed: {error}");
                        }
                    }
                }
                BridgeCommand::CssProbe(payload) => {
                    println!("[Injection] CSS probe result: {:?}", payload.value);
                }
                BridgeCommand::RendererError(payload) => {
                    eprintln!("[Renderer][{}] {}", payload.kind, payload.message);
                }
            },
            _ => {
                let _keep_window_alive = &window;
                let _keep_web_context_alive = &web_context;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{is_allowed_navigation, is_trusted_bridge_origin, parse_bridge_request};

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
    }

    #[test]
    fn bridge_accepts_only_versioned_known_operations() {
        let valid = r#"{"version":1,"request_id":"test-1","command":{"type":"ping","payload":{"nonce":"hello"}}}"#;
        assert!(parse_bridge_request(valid).is_ok());

        let unknown = r#"{"version":1,"request_id":"test-1","command":{"type":"invoke","payload":{"method":"anything"}}}"#;
        assert!(parse_bridge_request(unknown).is_err());

        let wrong_version = r#"{"version":2,"request_id":"test-1","command":{"type":"ping","payload":{"nonce":"hello"}}}"#;
        assert!(parse_bridge_request(wrong_version).is_err());

        let extra_field = r#"{"version":1,"request_id":"test-1","command":{"type":"ping","payload":{"nonce":"hello","host_api":"filesystem"}}}"#;
        assert!(parse_bridge_request(extra_field).is_err());
    }
}

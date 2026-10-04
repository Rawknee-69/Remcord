mod protocol;

use std::{error::Error, path::PathBuf, time::Instant};

use protocol::{
    parse_bridge_message, BridgeCommand, BridgeEvent, BridgeEventMessage, BridgeMessage,
    BridgeRequest, BridgeResponse, PingResponse, RequestTracker, PROTOCOL_VERSION,
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
const BOOT_SNAPSHOT: BootSnapshot = BootSnapshot {
    runtime: "webview2",
    platform: "win32",
    protocol_version: PROTOCOL_VERSION,
    capabilities: &["bridge.ping"],
};
const COMPATIBILITY_SCRIPT: &str = include_str!("../web/compatibility.js");

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

fn initialization_script() -> Result<String, serde_json::Error> {
    let snapshot = serde_json::to_string(&BOOT_SNAPSHOT)?;
    Ok(COMPATIBILITY_SCRIPT.replace("__LEGCORD_BOOT_SNAPSHOT__", &snapshot))
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
    let initialization_script = initialization_script()?;
    println!(
        "[Bridge] document-start bootstrap: {} bytes",
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
    let mut smoke_reload_pending =
        std::env::var("LEGCORD_NATIVE_SMOKE_RELOAD").is_ok_and(|value| value == "1");
    let mut request_tracker = RequestTracker::default();
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
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(NativeEvent::PageLoadStarted) => {
                request_tracker.clear();
                println!("[Bridge] cleared request correlation state for new document");
            }
            Event::UserEvent(NativeEvent::PageLoadFinished(url)) => {
                if url
                    .parse::<Uri>()
                    .is_ok_and(|uri| is_trusted_bridge_origin(&uri))
                {
                    if let Err(error) = webview.evaluate_script(CSS_INJECTION_SCRIPT) {
                        eprintln!("[Injection] CSS script failed: {error}");
                    }
                    if let Err(error) = webview.evaluate_script(BRIDGE_PROBE_SCRIPT) {
                        eprintln!("[Injection] Bridge probe failed: {error}");
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
                                    payload: Some(PingResponse {
                                        nonce: payload.nonce,
                                        message: "pong",
                                    }),
                                    error: None,
                                };
                                send_bridge_response(&webview, &response);
                                if let Err(error) = request_tracker.complete(&request_id) {
                                    eprintln!("[Bridge] correlation failure after response: {error}");
                                }
                                response_count += 1;
                                println!(
                                    "[Bridge][Metrics] request-response latency: {:.3} ms",
                                    received_at.elapsed().as_secs_f64() * 1000.0
                                );
                            }
                        },
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
                        if smoke_reload_pending {
                            smoke_reload_pending = false;
                            println!("[Navigation] requesting one-shot DEV smoke reload");
                            if let Err(error) = webview.evaluate_script("window.location.reload();") {
                                eprintln!("[Navigation] smoke reload failed: {error}");
                            }
                        }
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
    use super::{is_allowed_navigation, is_trusted_bridge_origin};

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
}

//! Main-WebView bootstrap factored for safe recreation (Phase 2F).
//!
//! No behavior change: the initial boot still builds exactly one main
//! WebView with instance id [`INITIAL_MAIN_INSTANCE_ID`] (`0`, identical to
//! [`crate::recovery::MAIN_INSTANCE_ID`]), the same two-phase
//! `about:blank`-then-Discord startup, and the same per-instance
//! integrations. What changes is ownership:
//!
//! - **Per-WebView / per-instance** (must be re-applied to every newly
//!   constructed WebView): the wry builder options and navigation setup,
//!   download handlers, `legcord://` registration, CSP request filter,
//!   permission handler, process-failure observer, and
//!   navigation-completion observer, plus the initial Discord navigation.
//! - **Process-lifetime / shared** (must never be duplicated by a future
//!   replacement): the Tao event loop and proxies, the retained
//!   `WebContext` and WebView2 environment, the application caches and
//!   long-lived watchers (live-CSS watchers, STA picker workers, bundle
//!   cache), one-time global initialization, and shutdown infrastructure.
//!
//! A future replacement must call the per-instance helpers below with a
//! fresh id from [`MainWebviewLifecycle`]; it must not re-run any
//! process-lifetime setup. COM interfaces never cross the event loop:
//! observer callbacks forward only sanitized primitives via `NativeEvent`.
//! This module adds no recovery, no retry path, and no timer.

use std::path::{Path, PathBuf};
use std::time::Instant;

use tao::event_loop::EventLoopProxy;
#[cfg(target_os = "windows")]
use wry::WebViewBuilderExtWindows;
use wry::{http::Request, NewWindowResponse, PageLoadEvent, WebContext, WebView, WebViewBuilder};

/// Instance id the initial main WebView always carries. Pinned to the
/// recovery module's contract so a future replacement id can never alias
/// the boot identity by accident.
pub const INITIAL_MAIN_INSTANCE_ID: u64 = 0;

/// Two-phase boot document: committed synchronously inside `build()`,
/// before any `WebResourceRequested` filter can exist.
pub const MAIN_BOOT_URL: &str = "about:blank";

/// Every per-instance integration one main WebView must receive, exactly
/// once. Names are fixed vocabulary for structural tests; the builder and
/// attach helpers below apply them in this order.
pub const PER_INSTANCE_INTEGRATIONS: &[&str] = &[
    "wry_builder_with_web_context",
    "adopt_pre_registered_environment",
    "download_handlers",
    "boot_about_blank",
    "initialization_script_for_main_only",
    "navigation_handler",
    "new_window_handler",
    "page_load_handler",
    "ipc_handler",
    "legcord_protocol_registration",
    "permission_handler",
    "csp_request_filter",
    "process_failed_observer",
    "navigation_completed_observer",
    "initial_discord_navigation",
];

/// Process-lifetime owners that a future replacement must not re-create.
/// Listed here so tests can assert the per-instance list never absorbs
/// them (no duplicated watchers, subscriptions, timers, caches, or
/// global state).
pub const PROCESS_LIFETIME_OWNERS: &[&str] = &[
    "tao_event_loop_and_proxies",
    "retained_web_context",
    "retained_webview_environment",
    "bundle_cache",
    "request_tracker_and_document_lifecycle",
    "live_css_watchers_and_debouncers",
    "sta_picker_workers",
    "boot_snapshot_and_init_script_source",
    "recovery_and_gate_state_per_live_instance",
    "shutdown_unregister_infrastructure",
];

/// Host-owned lifecycle for the main WebView slot.
///
/// Tracks how many main WebViews were constructed in this process and
/// which instance id is live, so stale events from a replaced WebView can
/// be rejected by id comparison. The initial boot records exactly one
/// construction with [`INITIAL_MAIN_INSTANCE_ID`]; a future replacement
/// takes [`MainWebviewLifecycle::next_instance_id`] (never `0` again).
#[derive(Debug)]
pub struct MainWebviewLifecycle {
    constructions: u32,
    active_instance: Option<u64>,
    next_instance: u64,
}

impl MainWebviewLifecycle {
    pub fn new() -> Self {
        // The inventory lengths are part of the contract: every
        // per-instance integration below must stay listed exactly once,
        // and no process-lifetime owner may ever migrate into it.
        debug_assert_eq!(PER_INSTANCE_INTEGRATIONS.len(), 15);
        debug_assert_eq!(PROCESS_LIFETIME_OWNERS.len(), 10);
        Self {
            constructions: 0,
            active_instance: None,
            next_instance: INITIAL_MAIN_INSTANCE_ID,
        }
    }

    /// Record the initial boot construction. Fails when called twice so
    /// the process can never accidentally build a second initial WebView.
    pub fn record_initial(&mut self) -> Result<u64, &'static str> {
        if self.constructions != 0 {
            return Err("initial main WebView already constructed");
        }
        let id = INITIAL_MAIN_INSTANCE_ID;
        self.active_instance = Some(id);
        self.constructions = 1;
        self.next_instance = id + 1;
        Ok(id)
    }

    /// Fresh id reserved for a future replacement. Does not allocate: the
    /// replacement path (a later milestone) records it when it builds.
    pub fn next_instance_id(&self) -> u64 {
        self.next_instance
    }

    pub fn construction_count(&self) -> u32 {
        self.constructions
    }

    pub fn active_instance(&self) -> Option<u64> {
        self.active_instance
    }

    /// Whether an event's instance id belongs to an obsolete WebView.
    /// `None` (no live WebView) treats every event as stale.
    pub fn is_stale(&self, event_instance: u64) -> bool {
        self.active_instance != Some(event_instance)
    }
}

impl Default for MainWebviewLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds the configured wry builder for one main WebView instance.
///
/// Applies every wry-level per-instance integration: the adopted
/// pre-registered environment, download handlers, the `about:blank` boot
/// URL, the document-start script, and the navigation / new-window /
/// page-load / IPC handlers. The caller builds it on its Tao window
/// immediately; no process-lifetime state is touched.
///
/// `instance_id` is accepted so a future replacement can tag its
/// observer records with a fresh id. The current builder closures do not
/// branch on it (parity), but the parameter pins the call shape the
/// replacement will need.
#[allow(clippy::too_many_arguments)]
pub fn main_webview_builder<'a>(
    web_context: &'a mut WebContext,
    #[cfg(target_os = "windows")]
    environment: webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment,
    initialization_script: &'a str,
    downloads_dir: PathBuf,
    ipc_proxy: EventLoopProxy<super::NativeEvent>,
    load_proxy: EventLoopProxy<super::NativeEvent>,
    popup_proxy: EventLoopProxy<super::NativeEvent>,
    _instance_id: u64,
) -> WebViewBuilder<'a> {
    let mut builder = WebViewBuilder::new_with_web_context(web_context);
    // Adopt the pre-registered environment (P0.2). wry uses a
    // caller-provided environment verbatim instead of creating its own.
    #[cfg(target_os = "windows")]
    {
        builder = builder.with_environment(environment);
    }
    // P0.5: host-owned downloads directory, no dialog, never overwrite.
    builder = super::apply_download_handlers(builder, downloads_dir.clone());
    // Two-phase boot: the initial document is the empty `about:blank`
    // (committed synchronously inside `build()`, before any
    // WebResourceRequested filter can exist), then the host navigates to
    // Discord AFTER both interception filters are registered below. Without
    // this, the first Discord document races filter registration and would
    // keep Discord's original CSP (no `legcord:` allowance) until a reload.
    builder
        .with_url(MAIN_BOOT_URL)
        .with_initialization_script_for_main_only(initialization_script, true)
        .with_navigation_handler(|url| {
            let allowed = super::is_allowed_navigation(&url);
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
            match super::new_window::decide(&url) {
                decision @ (super::new_window::Decision::AllowAboutBlank
                | super::new_window::Decision::AllowIcs
                | super::new_window::Decision::AllowDiscordPopout) => {
                    let kind = super::new_window::popup_kind_for(decision)
                        .expect("allow decision must map to a popup kind");
                    println!("[Popup] approved {kind:?}: {url}");
                    super::queue_event(
                        &popup_proxy,
                        super::NativeEvent::PopupRequested {
                            url: url.clone(),
                            kind,
                        },
                    );
                    NewWindowResponse::Deny
                }
                super::new_window::Decision::ExternalOpen => {
                    match super::new_window::open_external_url(&url) {
                        Ok(()) => println!("[Popup] external open: {url}"),
                        Err(error) => eprintln!("[Popup] external open failed: {error} url={url}"),
                    }
                    NewWindowResponse::Deny
                }
                // Unknown custom schemes: deny with a log instead of
                // Electron's modal + sticky bypass (documented P0.4
                // hardening; no new settings key). Dangerous and malformed
                // values deny the same way, never reaching the shell.
                super::new_window::Decision::ConfirmExternal
                | super::new_window::Decision::Deny => {
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
                    super::queue_event(&load_proxy, super::NativeEvent::PageLoadStarted);
                }
                PageLoadEvent::Finished => {
                    super::queue_event(&load_proxy, super::NativeEvent::PageLoadFinished(url));
                }
            }
        })
        .with_ipc_handler(move |request: Request<String>| {
            if !super::is_trusted_bridge_origin(request.uri()) {
                eprintln!(
                    "[Bridge] rejected message from untrusted origin: {}",
                    request.uri()
                );
                return;
            }

            match super::protocol::parse_bridge_message(request.body()) {
                Ok(super::protocol::BridgeMessage::Request(request)) => super::queue_event(
                    &ipc_proxy,
                    super::NativeEvent::BridgeRequest(request, Instant::now()),
                ),
                Ok(super::protocol::BridgeMessage::Event(event)) => {
                    super::queue_event(&ipc_proxy, super::NativeEvent::BridgeEvent(event));
                }
                Err(error) => eprintln!("[Bridge] rejected invalid message: {error}"),
            }
        })
}

/// Registers the per-WebView content interceptions for one main WebView:
/// the `legcord://` protocol, the permission policy, and the CSP request
/// filter. Must run after `build()` (needs the core WebView) and before
/// the initial Discord navigation, so the first Discord document is
/// intercepted like every later one. Returns the namespace roots for the
/// popup child.
#[cfg(target_os = "windows")]
pub fn attach_main_content_filters(
    webview: &WebView,
    profile_path: &Path,
    webview_profile: &str,
) -> crate::legcord_protocol::NamespaceRoots {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
    use wry::WebViewExtWindows;

    let core_webview2: ICoreWebView2 = webview.webview();
    let bundle_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("ts-out");
    let user_data_dir = PathBuf::from(webview_profile);
    let _ = profile_path;
    let roots = super::build_namespace_roots(&bundle_dir, &user_data_dir);
    if let Err(e) = super::register_legcord_protocol(&core_webview2, roots.clone()) {
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
    if let Err(e) = super::permissions::register(&core_webview2) {
        eprintln!("[Permissions] Failed to register permission handler: {e}");
    } else {
        println!(
            "[Permissions] Permission handler registered (discord.com: mic/camera/notifications)"
        );
    }
    register_csp_interception(&core_webview2);
    roots
}

/// Registers the CSP request filter for one main WebView (P0.2 + Phase F).
/// Moved verbatim from the boot path: adds the `discord.com` document
/// filter, then forwards the document through `ureq` to extend the CSP
/// with the badges host and the `legcord:` scheme allowances. Fail-closed:
/// a forwarding failure leaves the original response untouched.
#[cfg(target_os = "windows")]
fn register_csp_interception(
    core_webview2: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2Environment, ICoreWebView2WebResourceRequestedEventArgs,
        ICoreWebView2_2, COREWEBVIEW2_WEB_RESOURCE_CONTEXT,
    };
    use webview2_com::WebResourceRequestedEventHandler;
    use windows::{
        core::{Interface, BOOL, HSTRING, PWSTR},
        Win32::System::Com::IStream,
    };

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
    let env: ICoreWebView2Environment = if let Ok(wv2) = core_webview2.cast::<ICoreWebView2_2>() {
        unsafe { wv2.Environment().unwrap() }
    } else {
        eprintln!("[CSP] Failed to get ICoreWebView2_2 for environment");
        return;
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
                    csp = super::extend_csp_for_legcord(&csp);
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
                    if let Some(s) = windows::Win32::UI::Shell::SHCreateMemStream(Some(&body_bytes))
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

/// One raw-COM observer subscription: the cloned core WebView plus its
/// registration token, removed before the WebView is dropped.
#[cfg(target_os = "windows")]
pub type ObserverSubscription = Option<(
    webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
    i64,
)>;

/// Both per-instance observer subscriptions for one main WebView, in
/// registration order (process-failure, then navigation-completed).
#[cfg(target_os = "windows")]
pub type MainObserverSubscriptions = (ObserverSubscription, ObserverSubscription);

/// Registers both per-instance observers for one main WebView: the Phase
/// 1 process-failure observer and the Phase 2B navigation-completed
/// observer. Each is exactly one raw-COM subscription whose callback
/// forwards sanitized primitives; registration failure only loses
/// visibility and never blocks boot.
#[cfg(target_os = "windows")]
pub fn attach_main_observers(
    webview: &WebView,
    instance_id: u64,
    observer_proxy: &EventLoopProxy<super::NativeEvent>,
) -> MainObserverSubscriptions {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
    use wry::WebViewExtWindows;

    let process_failed = {
        let core_webview2: ICoreWebView2 = webview.webview();
        match super::process_failure::register(
            &core_webview2,
            super::process_failure::WebViewRole::Main,
            instance_id,
            observer_proxy,
        ) {
            Ok(token) => {
                println!(
                    "[WebView][ProcessFailed] observer registered role=main instance={instance_id}"
                );
                Some((core_webview2, token))
            }
            Err(error) => {
                eprintln!(
                    "[WebView][ProcessFailed] observer registration failed role=main: {error}"
                );
                None
            }
        }
    };
    let navigation_completed = {
        let core_webview2: ICoreWebView2 = webview.webview();
        match super::navigation_completed::register(
            &core_webview2,
            super::process_failure::WebViewRole::Main,
            instance_id,
            observer_proxy,
        ) {
            Ok(token) => {
                println!(
                    "[WebView][NavigationCompleted] observer registered role=main instance={instance_id}"
                );
                Some((core_webview2, token))
            }
            Err(error) => {
                eprintln!(
                    "[WebView][NavigationCompleted] observer registration failed role=main: {error}"
                );
                None
            }
        }
    };
    (process_failed, navigation_completed)
}

/// Removes both per-instance observer subscriptions before the WebView is
/// dropped, so no late callback can target a dead event loop. Best-effort:
/// failures are logged inside the observer modules, never propagated.
#[cfg(target_os = "windows")]
pub fn detach_main_observers(
    process_failed: ObserverSubscription,
    navigation_completed: ObserverSubscription,
) {
    if let Some((core_webview2, token)) = process_failed {
        super::process_failure::unregister(&core_webview2, token);
    }
    if let Some((core_webview2, token)) = navigation_completed {
        super::navigation_completed::unregister(&core_webview2, token);
    }
}

/// Second boot phase (P0.2): navigate to Discord only after both
/// `WebResourceRequested` filters exist, so the first Discord document is
/// intercepted like every later one. `about:blank` has no CSP of its own
/// and the navigation handler allows only it plus HTTPS Discord hosts.
pub fn navigate_to_discord(webview: &WebView, discord_url: &str) -> Result<(), String> {
    webview
        .evaluate_script(&format!("window.location.href = \"{discord_url}\";"))
        .map_err(|error| format!("initial Discord navigation failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation_completed::NavigationCompletedEvent;
    use crate::process_failure::{ProcessFailedEvent, ProcessFailedKind, WebViewRole};
    use crate::recovery::{MainRendererRecovery, MAIN_INSTANCE_ID};

    #[test]
    fn initial_identity_matches_the_recovery_contract() {
        assert_eq!(INITIAL_MAIN_INSTANCE_ID, 0);
        assert_eq!(INITIAL_MAIN_INSTANCE_ID, MAIN_INSTANCE_ID);
        assert_eq!(MAIN_BOOT_URL, "about:blank");
    }

    #[test]
    fn initial_boot_constructs_exactly_one_main_webview() {
        let mut lifecycle = MainWebviewLifecycle::new();
        assert_eq!(lifecycle.construction_count(), 0);
        assert_eq!(lifecycle.active_instance(), None);
        let id = lifecycle.record_initial().expect("first boot records");
        assert_eq!(id, INITIAL_MAIN_INSTANCE_ID);
        assert_eq!(lifecycle.construction_count(), 1);
        assert_eq!(lifecycle.active_instance(), Some(0));
        // A second initial construction is rejected: the process can never
        // accidentally boot two main WebViews.
        assert!(lifecycle.record_initial().is_err());
        assert_eq!(lifecycle.construction_count(), 1);
        assert_eq!(lifecycle.active_instance(), Some(0));
    }

    #[test]
    fn replacement_takes_a_fresh_id_that_rejects_stale_events() {
        let mut lifecycle = MainWebviewLifecycle::new();
        lifecycle.record_initial().expect("boot");
        let fresh = lifecycle.next_instance_id();
        assert_ne!(fresh, INITIAL_MAIN_INSTANCE_ID);
        // The boot id is live: its own events pass, any other id is stale.
        assert!(!lifecycle.is_stale(INITIAL_MAIN_INSTANCE_ID));
        assert!(lifecycle.is_stale(fresh));
        assert!(lifecycle.is_stale(7));
        // A future replacement would own `fresh`; events from the retired
        // boot id must then read as stale against it.
        let retired_boot_stale_against_fresh = INITIAL_MAIN_INSTANCE_ID != fresh;
        assert!(retired_boot_stale_against_fresh);
    }

    #[test]
    fn stale_process_events_spend_no_recovery_budget() {
        let mut lifecycle = MainWebviewLifecycle::new();
        lifecycle.record_initial().expect("boot");
        let mut recovery =
            MainRendererRecovery::new(lifecycle.active_instance().expect("live instance"));
        let now = Instant::now();
        let stale = ProcessFailedEvent {
            role: WebViewRole::Main,
            instance_id: lifecycle.next_instance_id(),
            kind: ProcessFailedKind::RenderProcessExited,
            reason: None,
            exit_code: None,
        };
        assert!(lifecycle.is_stale(stale.instance_id));
        let decision = recovery.decide(&stale, now);
        assert_eq!(
            decision,
            crate::recovery::RecoveryDecision::Stale {
                expected_instance: INITIAL_MAIN_INSTANCE_ID
            }
        );
        assert!(!recovery.is_in_flight());
        assert_eq!(recovery.attempts_in_window(now), 0);
    }

    #[test]
    fn stale_navigation_events_never_resolve_the_live_gate() {
        use crate::navigation_gate::{FinishAction, NavigationGate};

        let mut lifecycle = MainWebviewLifecycle::new();
        lifecycle.record_initial().expect("boot");
        let live = lifecycle.active_instance().expect("live instance");
        let mut gate = NavigationGate::new(live, true);
        gate.on_started(1);
        assert_eq!(gate.on_finished(1), FinishAction::AwaitingOutcome);
        // A stale record from another instance never reaches the gate: the
        // event-loop id check drops it first (mirrored here).
        let stale = NavigationCompletedEvent {
            role: WebViewRole::Main,
            instance_id: lifecycle.next_instance_id(),
            is_success: true,
            web_error: None,
            http_status: None,
        };
        assert!(lifecycle.is_stale(stale.instance_id));
        assert!(crate::navigation_completed::is_stale(&stale, live));
        // The live finish is still pending: no pipeline ran for the stale
        // success, and the live gate still awaits its own outcome.
        assert_eq!(
            gate.on_outcome(false),
            crate::navigation_gate::OutcomeAction::SkipFailed { generation: 1 }
        );
    }

    #[test]
    fn per_instance_integrations_cover_every_required_observer_exactly_once() {
        // Every wiring the task lists as per-WebView must appear once.
        for required in [
            "download_handlers",
            "navigation_handler",
            "ipc_handler",
            "page_load_handler",
            "new_window_handler",
            "legcord_protocol_registration",
            "permission_handler",
            "csp_request_filter",
            "process_failed_observer",
            "navigation_completed_observer",
            "initial_discord_navigation",
        ] {
            assert!(
                PER_INSTANCE_INTEGRATIONS.contains(&required),
                "missing per-instance integration {required}"
            );
            assert_eq!(
                PER_INSTANCE_INTEGRATIONS
                    .iter()
                    .filter(|entry| **entry == required)
                    .count(),
                1,
                "{required} must be registered exactly once"
            );
        }
        // Process-lifetime owners must never leak into the per-instance
        // list (a future replacement must not duplicate them).
        for owner in PROCESS_LIFETIME_OWNERS {
            assert!(
                !PER_INSTANCE_INTEGRATIONS.contains(owner),
                "process-lifetime owner {owner} must not be per-instance"
            );
        }
        // No COM interface crosses the event loop: observer callbacks
        // forward only sanitized primitives tagged with the instance id.
        // (The `NativeEvent` enum itself lives in `main.rs` and carries no
        // COM objects; here we pin that this helper threads the id into
        // both observer registrations and detaches both subscriptions.)
        let source = include_str!("main_webview.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(shipped.contains("instance_id: u64"));
        assert!(shipped.contains("attach_main_observers"));
        assert!(shipped.contains("detach_main_observers"));
        // Both observer registrations receive the caller-supplied id, and
        // both removals run at teardown — no hardcoded second identity.
        assert!(shipped.contains("process_failure::WebViewRole::Main"));
        assert!(shipped.contains("super::process_failure::unregister"));
        assert!(shipped.contains("super::navigation_completed::unregister"));
    }

    #[test]
    fn helpers_apply_all_per_instance_wiring_without_process_lifetime_work() {
        // The shipped helpers below must exist with the per-instance call
        // shape; process-lifetime setup (watchers, workers, caches) must
        // not appear here. Only shipped code counts: the test module
        // itself names the forbidden items, so the haystack is sliced to
        // the pre-test source with comment lines removed (mirroring the
        // existing shipped-module guards), and needles that would
        // self-match are joined at runtime.
        let source = include_str!("main_webview.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for needle in [
            "with_environment(",
            "apply_download_handlers",
            "with_url(MAIN_BOOT_URL)",
            "with_initialization_script_for_main_only",
            "with_navigation_handler",
            "with_new_window_req_handler",
            "with_on_page_load_handler",
            "with_ipc_handler",
            "register_legcord_protocol",
            "permissions::register",
            "AddWebResourceRequestedFilter",
            "process_failure::register",
            "navigation_completed::register",
            "navigate_to_discord",
            "process_failure::unregister",
            "navigation_completed::unregister",
        ] {
            assert!(
                shipped.contains(needle),
                "main WebView helper must contain {needle}"
            );
        }
        // Joined so this assertion's own text cannot self-count.
        let watchers_needle = ["start_live_css", "_watchers"].join("");
        let com_needle = ["CoInitialize", "Ex"].join("");
        let cache_needle = ["ModCache::", "default"].join("");
        let tracker_needle = ["RequestTracker::", "default"].join("");
        let lifecycle_needle = ["DocumentModLifecycle::", "default"].join("");
        let reload_needle = ["Reload", "Requested"].join("");
        for forbidden in [
            &watchers_needle,
            &com_needle,
            &cache_needle,
            &tracker_needle,
            &lifecycle_needle,
            &reload_needle,
        ] {
            assert!(
                !shipped.contains(forbidden.as_str()),
                "process-lifetime or recovery work must not live in the per-instance helper: {forbidden}"
            );
        }
        // Exactly one builder entry point and one attach pair: no second
        // construction path can duplicate subscriptions.
        assert_eq!(shipped.matches("pub fn main_webview_builder").count(), 1);
        assert_eq!(
            shipped
                .matches("pub fn attach_main_content_filters")
                .count(),
            1
        );
        assert_eq!(shipped.matches("pub fn attach_main_observers").count(), 1);
    }

    #[test]
    fn recovery_and_gate_contracts_are_unchanged() {
        // The refactor must not alter the bounded-recovery or trusted-load
        // state machines: spot-check their defining constants and shapes.
        assert_eq!(crate::recovery::MAX_AUTO_ATTEMPTS, 3);
        assert_eq!(
            crate::recovery::ATTEMPT_WINDOW,
            std::time::Duration::from_secs(3600)
        );
        let mut gate = crate::navigation_gate::NavigationGate::new(INITIAL_MAIN_INSTANCE_ID, true);
        gate.on_started(1);
        assert_eq!(
            gate.on_finished(1),
            crate::navigation_gate::FinishAction::AwaitingOutcome
        );
    }
}

//! WebView2 navigation-completed observer (Phase 2B: observability only).
//!
//! Registers one raw-COM `NavigationCompleted` handler per WebView (main +
//! the single managed popup/ICS child) through the same raw-COM seam the
//! codebase already uses for permissions, request interception, and the
//! Phase 1 process-failure observer (`WebView::webview() ->
//! ICoreWebView2`). This is a SECOND subscription: wry keeps its own
//! `NavigationCompleted` handler (which forwards `PageLoadEvent::Finished`
//! unconditionally, `IsSuccess` unchecked), and this observer only reads
//! the event arguments wry deliberately drops.
//!
//! The callback extracts only primitive, sanitized diagnostics — role,
//! per-instance id, `IsSuccess`, `WebErrorStatus`, `HttpStatusCode` — and
//! forwards them to the event loop. There is deliberately NO behavior
//! change here: no navigation call, no reload, no retry, no recovery
//! decision, no renderer message, no dialog, no gating of
//! `PageLoadFinished`, no change to the trusted-load pipeline or
//! document/mod initialization.
//!
//! Privacy: the handler never reads URIs, query strings, page source,
//! cookies, tokens, message bodies, or frame info. The event-loop payload
//! carries no COM objects.

use crate::process_failure::WebViewRole;

/// Web error status, mirroring `COREWEBVIEW2_WEB_ERROR_STATUS_*`.
/// Numeric values are pinned to webview2-com 0.39 bindings (verified
/// against the vendored `webview2-com-sys` sources, not assumed from
/// documentation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebErrorStatus {
    Unknown,
    CertificateCommonNameIsIncorrect,
    CertificateExpired,
    ClientCertificateContainsErrors,
    CertificateRevoked,
    CertificateIsInvalid,
    ServerUnreachable,
    Timeout,
    ErrorHttpInvalidServerResponse,
    ConnectionAborted,
    ConnectionReset,
    Disconnected,
    CannotConnect,
    HostNameNotResolved,
    OperationCanceled,
    RedirectFailed,
    UnexpectedError,
    ValidAuthenticationCredentialsRequired,
    ValidProxyAuthenticationRequired,
    Unrecognized(i32),
}

impl WebErrorStatus {
    pub fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::Unknown,
            1 => Self::CertificateCommonNameIsIncorrect,
            2 => Self::CertificateExpired,
            3 => Self::ClientCertificateContainsErrors,
            4 => Self::CertificateRevoked,
            5 => Self::CertificateIsInvalid,
            6 => Self::ServerUnreachable,
            7 => Self::Timeout,
            8 => Self::ErrorHttpInvalidServerResponse,
            9 => Self::ConnectionAborted,
            10 => Self::ConnectionReset,
            11 => Self::Disconnected,
            12 => Self::CannotConnect,
            13 => Self::HostNameNotResolved,
            14 => Self::OperationCanceled,
            15 => Self::RedirectFailed,
            16 => Self::UnexpectedError,
            17 => Self::ValidAuthenticationCredentialsRequired,
            18 => Self::ValidProxyAuthenticationRequired,
            other => Self::Unrecognized(other),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::CertificateCommonNameIsIncorrect => "CertificateCommonNameIsIncorrect",
            Self::CertificateExpired => "CertificateExpired",
            Self::ClientCertificateContainsErrors => "ClientCertificateContainsErrors",
            Self::CertificateRevoked => "CertificateRevoked",
            Self::CertificateIsInvalid => "CertificateIsInvalid",
            Self::ServerUnreachable => "ServerUnreachable",
            Self::Timeout => "Timeout",
            Self::ErrorHttpInvalidServerResponse => "ErrorHttpInvalidServerResponse",
            Self::ConnectionAborted => "ConnectionAborted",
            Self::ConnectionReset => "ConnectionReset",
            Self::Disconnected => "Disconnected",
            Self::CannotConnect => "CannotConnect",
            Self::HostNameNotResolved => "HostNameNotResolved",
            Self::OperationCanceled => "OperationCanceled",
            Self::RedirectFailed => "RedirectFailed",
            Self::UnexpectedError => "UnexpectedError",
            Self::ValidAuthenticationCredentialsRequired => {
                "ValidAuthenticationCredentialsRequired"
            }
            Self::ValidProxyAuthenticationRequired => "ValidProxyAuthenticationRequired",
            Self::Unrecognized(_) => "Unrecognized",
        }
    }
}

/// Sanitized, COM-free navigation record forwarded to the event loop.
/// Carries no interface objects, no URLs, no page data. `web_error` is
/// `None` only when the base-args read itself failed; `http_status` is
/// `None` when the extended `Args2` interface is unavailable (older
/// runtimes) or its read failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationCompletedEvent {
    pub role: WebViewRole,
    pub instance_id: u64,
    pub is_success: bool,
    pub web_error: Option<WebErrorStatus>,
    pub http_status: Option<i32>,
}

/// Pure payload mapping from already-read primitives. The COM callback
/// reads `IsSuccess` / `WebErrorStatus` / `HttpStatusCode` and calls this;
/// unit tests cover every shape without a WebView.
pub fn map_event(
    role: WebViewRole,
    instance_id: u64,
    is_success: bool,
    web_error_raw: Option<i32>,
    http_status_raw: Option<i32>,
) -> NavigationCompletedEvent {
    NavigationCompletedEvent {
        role,
        instance_id,
        is_success,
        web_error: web_error_raw.map(WebErrorStatus::from_raw),
        http_status: http_status_raw,
    }
}

/// Per-instance counters, scoped to one WebView (the child counters reset
/// with every new child; the main counters live for the process).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NavigationCompletedCounters {
    pub total: u64,
    pub succeeded: u64,
    pub failed: u64,
}

impl NavigationCompletedCounters {
    pub fn record(&mut self, event: &NavigationCompletedEvent) {
        self.total += 1;
        if event.is_success {
            self.succeeded += 1;
        } else {
            self.failed += 1;
        }
    }
}

/// Whether a record belongs to an obsolete WebView instance. Stale child
/// records (a destroyed or replaced child whose callback fired late) must
/// never be counted against the live slot.
pub fn is_stale(event: &NavigationCompletedEvent, expected_instance: u64) -> bool {
    event.instance_id != expected_instance
}

/// Fixed-vocabulary log line. Only role, instance, success flag, error
/// name (or a numeric unknown), and HTTP status (or a fixed
/// `unavailable` token) vary; no page data can appear here by
/// construction.
pub fn format_log(event: &NavigationCompletedEvent) -> String {
    let web_error = match event.web_error {
        Some(WebErrorStatus::Unrecognized(value)) => format!("Unrecognized({value})"),
        Some(known) => known.as_str().to_owned(),
        None => "unavailable".to_owned(),
    };
    let http_status = match event.http_status {
        Some(code) => code.to_string(),
        None => "unavailable".to_owned(),
    };
    format!(
        "[WebView][NavigationCompleted] success={} role={} instance={} web_error={web_error} http_status={http_status}",
        event.is_success,
        event.role.as_str(),
        event.instance_id,
    )
}

/// Fixed-vocabulary log line for a stale record that arrived after its
/// WebView was replaced or destroyed. Carries the same sanitized fields
/// plus the live instance it was compared against.
pub fn format_stale_log(event: &NavigationCompletedEvent, expected_instance: u64) -> String {
    format!(
        "{} stale expected_instance={expected_instance}",
        format_log(event),
    )
}

/// Attach exactly one raw-COM `NavigationCompleted` observer to one
/// CoreWebView2 and return its registration token. This is ADDITIVE: wry's
/// own `NavigationCompleted` subscription is untouched, and WebView2
/// delivers completions to both handlers. The callback extracts
/// primitives only and forwards them over `proxy`; COM objects never leave
/// the closure. A registration failure is returned to the caller and must
/// not block boot — the pre-observer behavior (wry page-load events only)
/// is the fallback.
#[cfg(target_os = "windows")]
pub fn register(
    webview: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
    role: WebViewRole,
    instance_id: u64,
    proxy: &tao::event_loop::EventLoopProxy<crate::NativeEvent>,
) -> windows::core::Result<i64> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2NavigationCompletedEventArgs, ICoreWebView2NavigationCompletedEventArgs2,
        COREWEBVIEW2_WEB_ERROR_STATUS,
    };
    use windows::core::{Interface, BOOL};

    let proxy = proxy.clone();
    let handler = webview2_com::NavigationCompletedEventHandler::create(Box::new(
        move |_sender: Option<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2>,
              args: Option<ICoreWebView2NavigationCompletedEventArgs>| {
            let Some(args) = args else {
                return Ok(());
            };
            // `IsSuccess` is required: without it the record is useless.
            let mut raw_success = BOOL::default();
            if unsafe { args.IsSuccess(&mut raw_success) }.is_err() {
                return Ok(());
            }
            // `WebErrorStatus` is best-effort: an unreadable field becomes
            // `None` (logged as `unavailable`), never a guess.
            let mut raw_error = COREWEBVIEW2_WEB_ERROR_STATUS::default();
            let web_error_raw = unsafe { args.WebErrorStatus(&mut raw_error) }
                .ok()
                .map(|()| raw_error.0);
            // `HttpStatusCode` lives on the extended args: older runtimes
            // serve the base shape only, and any unreadable field is `None`.
            let http_status_raw = match args.cast::<ICoreWebView2NavigationCompletedEventArgs2>() {
                Ok(args2) => {
                    let mut raw_status = 0i32;
                    unsafe { args2.HttpStatusCode(&mut raw_status) }
                        .ok()
                        .map(|()| raw_status)
                }
                Err(_) => None,
            };
            crate::queue_event(
                &proxy,
                crate::NativeEvent::NavigationCompleted(map_event(
                    role,
                    instance_id,
                    raw_success.as_bool(),
                    web_error_raw,
                    http_status_raw,
                )),
            );
            Ok(())
        },
    ));
    let mut token = 0i64;
    unsafe {
        webview.add_NavigationCompleted(&handler, &mut token)?;
    }
    Ok(token)
}

/// Remove a previously registered observer. Best-effort at shutdown and
/// child teardown: failures are logged, never propagated.
#[cfg(target_os = "windows")]
pub fn unregister(
    webview: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
    token: i64,
) {
    if unsafe { webview.remove_NavigationCompleted(token) }.is_err() {
        eprintln!("[WebView][NavigationCompleted] subscription removal failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_failure::WebViewRole;

    #[test]
    fn success_and_failure_payloads_map_with_identity() {
        let success = map_event(WebViewRole::Main, 0, true, Some(0), Some(200));
        assert_eq!(
            success,
            NavigationCompletedEvent {
                role: WebViewRole::Main,
                instance_id: 0,
                is_success: true,
                web_error: Some(WebErrorStatus::Unknown),
                http_status: Some(200),
            }
        );
        let failure = map_event(WebViewRole::Child, 3, false, Some(12), None);
        assert_eq!(
            failure,
            NavigationCompletedEvent {
                role: WebViewRole::Child,
                instance_id: 3,
                is_success: false,
                web_error: Some(WebErrorStatus::CannotConnect),
                http_status: None,
            }
        );
    }

    #[test]
    fn unavailable_metadata_degrades_to_none() {
        // Older runtimes serve the base shape only: no extended args, and a
        // failed base-args read, both degrade to `None`, never a guess.
        let degraded = map_event(WebViewRole::Main, 0, false, None, None);
        assert_eq!(degraded.web_error, None);
        assert_eq!(degraded.http_status, None);
        assert_eq!(
            format_log(&degraded),
            "[WebView][NavigationCompleted] success=false role=main instance=0 web_error=unavailable http_status=unavailable"
        );
    }

    #[test]
    fn every_documented_web_error_maps() {
        let cases = [
            (0, WebErrorStatus::Unknown, "Unknown"),
            (
                1,
                WebErrorStatus::CertificateCommonNameIsIncorrect,
                "CertificateCommonNameIsIncorrect",
            ),
            (2, WebErrorStatus::CertificateExpired, "CertificateExpired"),
            (
                3,
                WebErrorStatus::ClientCertificateContainsErrors,
                "ClientCertificateContainsErrors",
            ),
            (4, WebErrorStatus::CertificateRevoked, "CertificateRevoked"),
            (
                5,
                WebErrorStatus::CertificateIsInvalid,
                "CertificateIsInvalid",
            ),
            (6, WebErrorStatus::ServerUnreachable, "ServerUnreachable"),
            (7, WebErrorStatus::Timeout, "Timeout"),
            (
                8,
                WebErrorStatus::ErrorHttpInvalidServerResponse,
                "ErrorHttpInvalidServerResponse",
            ),
            (9, WebErrorStatus::ConnectionAborted, "ConnectionAborted"),
            (10, WebErrorStatus::ConnectionReset, "ConnectionReset"),
            (11, WebErrorStatus::Disconnected, "Disconnected"),
            (12, WebErrorStatus::CannotConnect, "CannotConnect"),
            (
                13,
                WebErrorStatus::HostNameNotResolved,
                "HostNameNotResolved",
            ),
            (14, WebErrorStatus::OperationCanceled, "OperationCanceled"),
            (15, WebErrorStatus::RedirectFailed, "RedirectFailed"),
            (16, WebErrorStatus::UnexpectedError, "UnexpectedError"),
            (
                17,
                WebErrorStatus::ValidAuthenticationCredentialsRequired,
                "ValidAuthenticationCredentialsRequired",
            ),
            (
                18,
                WebErrorStatus::ValidProxyAuthenticationRequired,
                "ValidProxyAuthenticationRequired",
            ),
        ];
        for (raw, expected, label) in cases {
            assert_eq!(WebErrorStatus::from_raw(raw), expected, "raw {raw}");
            assert_eq!(expected.as_str(), label);
        }
    }

    #[test]
    fn unknown_web_errors_fail_open_without_panicking() {
        for raw in [19, 42, -1, i32::MIN, i32::MAX] {
            let status = WebErrorStatus::from_raw(raw);
            assert_eq!(status, WebErrorStatus::Unrecognized(raw));
            assert_eq!(status.as_str(), "Unrecognized");
        }
    }

    #[test]
    fn http_status_passes_through_verbatim_including_edge_values() {
        // HTTP codes are transport facts, not policy inputs: any i32 the
        // runtime reports is preserved exactly, including non-standard and
        // negative values a strict validator might have rejected.
        for code in [200, 404, 500, 0, -1, 999, i32::MAX, i32::MIN] {
            let event = map_event(WebViewRole::Main, 0, code == 200, Some(0), Some(code));
            assert_eq!(event.http_status, Some(code));
            assert!(format_log(&event).contains(&format!("http_status={code}")));
        }
    }

    #[test]
    fn main_and_child_records_stay_distinguishable() {
        let main = map_event(WebViewRole::Main, 0, true, Some(0), Some(200));
        let child = map_event(WebViewRole::Child, 2, true, Some(0), Some(200));
        assert_ne!(main.role, child.role);
        assert_ne!(main.instance_id, child.instance_id);
        assert!(format_log(&main).contains("role=main instance=0"));
        assert!(format_log(&child).contains("role=child instance=2"));
    }

    #[test]
    fn log_lines_distinguish_success_from_failure() {
        let success = map_event(WebViewRole::Main, 0, true, Some(0), Some(200));
        assert_eq!(
            format_log(&success),
            "[WebView][NavigationCompleted] success=true role=main instance=0 web_error=Unknown http_status=200"
        );
        let failure = map_event(WebViewRole::Main, 0, false, Some(13), Some(0));
        assert_eq!(
            format_log(&failure),
            "[WebView][NavigationCompleted] success=false role=main instance=0 web_error=HostNameNotResolved http_status=0"
        );
        let odd = map_event(WebViewRole::Child, 5, false, Some(99), Some(-1));
        assert_eq!(
            format_log(&odd),
            "[WebView][NavigationCompleted] success=false role=child instance=5 web_error=Unrecognized(99) http_status=-1"
        );
    }

    #[test]
    fn log_lines_carry_no_sensitive_or_page_data() {
        // Fixed vocabulary plus role/instance/flag/numerics only: no URLs,
        // query strings, paths, cookies, tokens, or page contents can reach
        // the log by construction (the event carries none of them).
        let event = map_event(WebViewRole::Main, 0, false, Some(12), Some(0));
        let line = format_log(&event);
        for forbidden in [
            "https://",
            "http://",
            "discord",
            "?",
            "cookie",
            "token",
            "remcord://",
            "remcord://",
            "query",
        ] {
            assert!(
                !line.to_lowercase().contains(forbidden),
                "log line must not contain {forbidden:?}: {line}"
            );
        }
        assert!(line.starts_with("[WebView][NavigationCompleted] "));
    }

    #[test]
    fn stale_records_are_detected_and_logged_without_counting() {
        let live = map_event(WebViewRole::Child, 4, true, Some(0), Some(200));
        assert!(!is_stale(&live, 4));
        // A late callback from a replaced or destroyed child carries its
        // own creation-time id, never the live slot's.
        let stale = map_event(WebViewRole::Child, 3, false, Some(11), None);
        assert!(is_stale(&stale, 4));
        assert!(!is_stale(&stale, 3));
        let line = format_stale_log(&stale, 4);
        assert_eq!(
            line,
            "[WebView][NavigationCompleted] success=false role=child instance=3 web_error=Disconnected http_status=unavailable stale expected_instance=4"
        );
        // Main records use the same gate: only instance 0 is current.
        let main_stale = map_event(WebViewRole::Main, 7, false, Some(6), None);
        assert!(is_stale(&main_stale, crate::recovery::MAIN_INSTANCE_ID));
    }

    #[test]
    fn counters_scope_success_and_failure() {
        let mut counters = NavigationCompletedCounters::default();
        counters.record(&map_event(WebViewRole::Main, 0, true, Some(0), Some(200)));
        counters.record(&map_event(WebViewRole::Main, 0, false, Some(12), None));
        counters.record(&map_event(WebViewRole::Main, 0, false, Some(7), Some(0)));
        assert_eq!(
            counters,
            NavigationCompletedCounters {
                total: 3,
                succeeded: 1,
                failed: 2,
            }
        );
    }

    #[test]
    fn shipped_module_stays_observation_only() {
        let source = include_str!("navigation_completed.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // No behavior change surface: no navigation or reload calls, no
        // recovery policy, no script execution, no messaging, no
        // page-data reads, no wry page-load coupling.
        for forbidden in [
            "Navigate(",
            "Reload(",
            "Close(",
            "ExecuteScript",
            "evaluate_script",
            "postMessage",
            "PostWebMessage",
            "ProcessFailed",
            "RecoveryDecision",
            "PageLoadFinished",
            "PageLoadStarted",
            "Uri(",
            "Source(",
            "Cookie",
            "remcord://",
            "remcord://",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
        // The additive registration pair is the only COM surface.
        assert!(shipped.contains("add_NavigationCompleted"));
        assert!(shipped.contains("remove_NavigationCompleted"));
    }
}

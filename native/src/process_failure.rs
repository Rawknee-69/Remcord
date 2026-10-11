//! WebView2 process-failure observer (Phase 1: observability only).
//!
//! Registers one `ProcessFailed` handler per WebView (main + the single
//! managed popup/ICS child) through the same raw-COM seam the codebase
//! already uses for permissions and request interception
//! (`WebView::webview() -> ICoreWebView2`). This module extracts only
//! primitive, sanitized diagnostics — role, per-instance id, failure kind,
//! reason, exit code — and forwards them to the event loop. There is
//! deliberately NO recovery here: no reload, no navigation call, no window
//! recreation, no renderer message, no dialog.
//!
//! Privacy: the handler never reads URIs, page source, cookies, tokens,
//! message bodies, process descriptions (which can embed command lines),
//! or frame info. The event-loop payload carries no COM objects.

/// Stable role of the observed WebView. Main is always instance `0`;
/// each managed child takes the next slot generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebViewRole {
    Main,
    Child,
}

impl WebViewRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            WebViewRole::Main => "main",
            WebViewRole::Child => "child",
        }
    }
}

/// Failure kind, mirroring `COREWEBVIEW2_PROCESS_FAILED_KIND_*`.
/// Numeric values are pinned to webview2-com 0.39 bindings (verified
/// against the vendored source, not assumed from documentation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessFailedKind {
    BrowserProcessExited,
    RenderProcessExited,
    RenderProcessUnresponsive,
    FrameRenderProcessExited,
    UtilityProcessExited,
    SandboxHelperProcessExited,
    GpuProcessExited,
    PpapiPluginProcessExited,
    PpapiBrokerProcessExited,
    UnknownProcessExited,
    Unrecognized(i32),
}

impl ProcessFailedKind {
    pub fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::BrowserProcessExited,
            1 => Self::RenderProcessExited,
            2 => Self::RenderProcessUnresponsive,
            3 => Self::FrameRenderProcessExited,
            4 => Self::UtilityProcessExited,
            5 => Self::SandboxHelperProcessExited,
            6 => Self::GpuProcessExited,
            7 => Self::PpapiPluginProcessExited,
            8 => Self::PpapiBrokerProcessExited,
            9 => Self::UnknownProcessExited,
            other => Self::Unrecognized(other),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BrowserProcessExited => "BrowserProcessExited",
            Self::RenderProcessExited => "RenderProcessExited",
            Self::RenderProcessUnresponsive => "RenderProcessUnresponsive",
            Self::FrameRenderProcessExited => "FrameRenderProcessExited",
            Self::UtilityProcessExited => "UtilityProcessExited",
            Self::SandboxHelperProcessExited => "SandboxHelperProcessExited",
            Self::GpuProcessExited => "GpuProcessExited",
            Self::PpapiPluginProcessExited => "PpapiPluginProcessExited",
            Self::PpapiBrokerProcessExited => "PpapiBrokerProcessExited",
            Self::UnknownProcessExited => "UnknownProcessExited",
            Self::Unrecognized(_) => "Unrecognized",
        }
    }
}

/// Failure reason, mirroring `COREWEBVIEW2_PROCESS_FAILED_REASON_*`
/// (extended args; may be unavailable on older runtimes, hence `None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessFailedReason {
    Unexpected,
    Unresponsive,
    Terminated,
    Crashed,
    LaunchFailed,
    OutOfMemory,
    ProfileDeleted,
    Unrecognized(i32),
}

impl ProcessFailedReason {
    pub fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::Unexpected,
            1 => Self::Unresponsive,
            2 => Self::Terminated,
            3 => Self::Crashed,
            4 => Self::LaunchFailed,
            5 => Self::OutOfMemory,
            6 => Self::ProfileDeleted,
            other => Self::Unrecognized(other),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unexpected => "Unexpected",
            Self::Unresponsive => "Unresponsive",
            Self::Terminated => "Terminated",
            Self::Crashed => "Crashed",
            Self::LaunchFailed => "LaunchFailed",
            Self::OutOfMemory => "OutOfMemory",
            Self::ProfileDeleted => "ProfileDeleted",
            Self::Unrecognized(_) => "Unrecognized",
        }
    }
}

/// Sanitized, COM-free failure record forwarded to the event loop.
/// Carries no interface objects, no paths, no page data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessFailedEvent {
    pub role: WebViewRole,
    pub instance_id: u64,
    pub kind: ProcessFailedKind,
    pub reason: Option<ProcessFailedReason>,
    pub exit_code: Option<i32>,
}

/// Per-instance counters, scoped to one WebView (the child counters reset
/// with every new child; the main counters live for the process).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProcessFailedCounters {
    pub total: u64,
    pub browser_exits: u64,
    pub render_exits: u64,
    pub unresponsive: u64,
    pub other: u64,
}

impl ProcessFailedCounters {
    pub fn record(&mut self, kind: ProcessFailedKind) {
        self.total += 1;
        match kind {
            ProcessFailedKind::BrowserProcessExited => self.browser_exits += 1,
            ProcessFailedKind::RenderProcessExited => self.render_exits += 1,
            ProcessFailedKind::RenderProcessUnresponsive => self.unresponsive += 1,
            _ => self.other += 1,
        }
    }
}

/// Fixed-vocabulary log line. Unknown numeric values are the only
/// variable content besides role/instance/exit code; no page data can
/// appear here by construction.
pub fn format_log(event: &ProcessFailedEvent) -> String {
    let kind = match event.kind {
        ProcessFailedKind::Unrecognized(value) => format!("Unrecognized({value})"),
        known => known.as_str().to_owned(),
    };
    let reason = match event.reason {
        Some(ProcessFailedReason::Unrecognized(value)) => format!("Unrecognized({value})"),
        Some(known) => known.as_str().to_owned(),
        None => "unavailable".to_owned(),
    };
    let exit = match event.exit_code {
        Some(code) => code.to_string(),
        None => "unavailable".to_owned(),
    };
    format!(
        "[WebView][ProcessFailed] role={} instance={} kind={kind} reason={reason} exit={exit}",
        event.role.as_str(),
        event.instance_id,
    )
}

/// Attach exactly one `ProcessFailed` observer to one CoreWebView2 and
/// return its registration token. The callback extracts primitives only
/// and forwards them over `proxy`; COM objects never leave the closure.
/// A registration failure is returned to the caller and must not block
/// boot — the pre-observer behavior (no visibility) is the fallback.
#[cfg(target_os = "windows")]
pub fn register(
    webview: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
    role: WebViewRole,
    instance_id: u64,
    proxy: &tao::event_loop::EventLoopProxy<crate::NativeEvent>,
) -> windows::core::Result<i64> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2ProcessFailedEventArgs, ICoreWebView2ProcessFailedEventArgs2,
        COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_REASON,
    };
    use windows::core::Interface;

    let proxy = proxy.clone();
    let handler = webview2_com::ProcessFailedEventHandler::create(Box::new(
        move |_sender: Option<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2>,
              args: Option<ICoreWebView2ProcessFailedEventArgs>| {
            let Some(args) = args else {
                return Ok(());
            };
            // Kind is required: without it the record is useless.
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            if unsafe { args.ProcessFailedKind(&mut kind) }.is_err() {
                return Ok(());
            };
            // Extended args are best-effort: older runtimes serve the base
            // shape only, and any unreadable field becomes `None`.
            // Descriptions and frame collections are deliberately never
            // read (command lines and frame data stay out of the logs).
            let (reason, exit_code) = match args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
                Ok(args2) => {
                    let mut raw_reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
                    let reason = unsafe { args2.Reason(&mut raw_reason) }
                        .ok()
                        .map(|()| ProcessFailedReason::from_raw(raw_reason.0));
                    let mut raw_exit = 0i32;
                    let exit_code = unsafe { args2.ExitCode(&mut raw_exit) }
                        .ok()
                        .map(|()| raw_exit);
                    (reason, exit_code)
                }
                Err(_) => (None, None),
            };
            crate::queue_event(
                &proxy,
                crate::NativeEvent::ProcessFailed(ProcessFailedEvent {
                    role,
                    instance_id,
                    kind: ProcessFailedKind::from_raw(kind.0),
                    reason,
                    exit_code,
                }),
            );
            Ok(())
        },
    ));
    let mut token = 0i64;
    unsafe {
        webview.add_ProcessFailed(&handler, &mut token)?;
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
    if unsafe { webview.remove_ProcessFailed(token) }.is_err() {
        eprintln!("[WebView][ProcessFailed] subscription removal failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_have_stable_names() {
        assert_eq!(WebViewRole::Main.as_str(), "main");
        assert_eq!(WebViewRole::Child.as_str(), "child");
    }

    #[test]
    fn every_documented_kind_maps() {
        let cases = [
            (
                0,
                ProcessFailedKind::BrowserProcessExited,
                "BrowserProcessExited",
            ),
            (
                1,
                ProcessFailedKind::RenderProcessExited,
                "RenderProcessExited",
            ),
            (
                2,
                ProcessFailedKind::RenderProcessUnresponsive,
                "RenderProcessUnresponsive",
            ),
            (
                3,
                ProcessFailedKind::FrameRenderProcessExited,
                "FrameRenderProcessExited",
            ),
            (
                4,
                ProcessFailedKind::UtilityProcessExited,
                "UtilityProcessExited",
            ),
            (
                5,
                ProcessFailedKind::SandboxHelperProcessExited,
                "SandboxHelperProcessExited",
            ),
            (6, ProcessFailedKind::GpuProcessExited, "GpuProcessExited"),
            (
                7,
                ProcessFailedKind::PpapiPluginProcessExited,
                "PpapiPluginProcessExited",
            ),
            (
                8,
                ProcessFailedKind::PpapiBrokerProcessExited,
                "PpapiBrokerProcessExited",
            ),
            (
                9,
                ProcessFailedKind::UnknownProcessExited,
                "UnknownProcessExited",
            ),
        ];
        for (raw, expected, label) in cases {
            assert_eq!(ProcessFailedKind::from_raw(raw), expected, "raw {raw}");
            assert_eq!(expected.as_str(), label);
        }
    }

    #[test]
    fn unknown_kinds_fail_open_without_panicking() {
        for raw in [10, 42, -1, i32::MIN, i32::MAX] {
            let kind = ProcessFailedKind::from_raw(raw);
            assert_eq!(kind, ProcessFailedKind::Unrecognized(raw));
            assert_eq!(kind.as_str(), "Unrecognized");
        }
    }

    #[test]
    fn every_documented_reason_maps() {
        let cases = [
            (0, ProcessFailedReason::Unexpected, "Unexpected"),
            (1, ProcessFailedReason::Unresponsive, "Unresponsive"),
            (2, ProcessFailedReason::Terminated, "Terminated"),
            (3, ProcessFailedReason::Crashed, "Crashed"),
            (4, ProcessFailedReason::LaunchFailed, "LaunchFailed"),
            (5, ProcessFailedReason::OutOfMemory, "OutOfMemory"),
            (6, ProcessFailedReason::ProfileDeleted, "ProfileDeleted"),
        ];
        for (raw, expected, label) in cases {
            assert_eq!(ProcessFailedReason::from_raw(raw), expected, "raw {raw}");
            assert_eq!(expected.as_str(), label);
        }
    }

    #[test]
    fn unknown_reasons_fail_open_without_panicking() {
        for raw in [7, 99, -1, i32::MIN, i32::MAX] {
            let reason = ProcessFailedReason::from_raw(raw);
            assert_eq!(reason, ProcessFailedReason::Unrecognized(raw));
            assert_eq!(reason.as_str(), "Unrecognized");
        }
    }

    #[test]
    fn log_line_is_exact_and_data_free() {
        let event = ProcessFailedEvent {
            role: WebViewRole::Main,
            instance_id: 0,
            kind: ProcessFailedKind::RenderProcessExited,
            reason: Some(ProcessFailedReason::Crashed),
            exit_code: Some(1),
        };
        assert_eq!(
            format_log(&event),
            "[WebView][ProcessFailed] role=main instance=0 kind=RenderProcessExited reason=Crashed exit=1"
        );
        // Missing extended args degrade to a fixed token, never a guess.
        let degraded = ProcessFailedEvent {
            role: WebViewRole::Child,
            instance_id: 3,
            kind: ProcessFailedKind::UtilityProcessExited,
            reason: None,
            exit_code: None,
        };
        assert_eq!(
            format_log(&degraded),
            "[WebView][ProcessFailed] role=child instance=3 kind=UtilityProcessExited reason=unavailable exit=unavailable"
        );
        // Unrecognized numerics stay numeric; nothing else varies.
        let odd = ProcessFailedEvent {
            role: WebViewRole::Main,
            instance_id: 0,
            kind: ProcessFailedKind::Unrecognized(12),
            reason: Some(ProcessFailedReason::Unrecognized(9)),
            exit_code: Some(-1),
        };
        assert_eq!(
            format_log(&odd),
            "[WebView][ProcessFailed] role=main instance=0 kind=Unrecognized(12) reason=Unrecognized(9) exit=-1"
        );
    }

    #[test]
    fn counters_scope_by_kind() {
        let mut counters = ProcessFailedCounters::default();
        counters.record(ProcessFailedKind::RenderProcessExited);
        counters.record(ProcessFailedKind::RenderProcessUnresponsive);
        counters.record(ProcessFailedKind::BrowserProcessExited);
        counters.record(ProcessFailedKind::GpuProcessExited);
        counters.record(ProcessFailedKind::Unrecognized(77));
        assert_eq!(
            counters,
            ProcessFailedCounters {
                total: 5,
                browser_exits: 1,
                render_exits: 1,
                unresponsive: 1,
                other: 2,
            }
        );
    }

    #[test]
    fn shipped_module_stays_observation_only() {
        let source = include_str!("process_failure.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // No recovery surface: no navigation or teardown calls, no script
        // execution, no messaging, no page-data reads.
        for forbidden in [
            "Navigate(",
            "Reload(",
            "Close(",
            "ExecuteScript",
            "evaluate_script",
            "postMessage",
            "PostWebMessage",
            "ProcessDescription",
            "FrameInfos",
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
    }
}

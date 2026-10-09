//! WebView2 environment-level browser-process exit observer (Phase 2G).
//!
//! Registers one `ICoreWebView2Environment5::BrowserProcessExited` handler for
//! the shared WebView2 environment (not per-WebView). This provides the
//! authoritative browser-process exit signal with exit kind (Normal/Failed)
//! and browser PID — information the per-WebView `ProcessFailed` observer
//! cannot provide.
//!
//! The callback forwards only sanitized primitives via `NativeEvent`.
//! COM interfaces never cross the event loop. The registration token is
//! retained for process-lifetime cleanup.

use crate::browser_recovery::{BrowserExitKind, BrowserProcessExitedEvent};

/// Registers the environment-level `BrowserProcessExited` observer exactly once
/// for the shared WebView2 environment. Returns the registration token for
/// process-lifetime cleanup.
///
/// The callback extracts `BrowserProcessExitKind` and `BrowserProcessId` and
/// forwards them as a `BrowserProcessExitedEvent`. Unavailable interfaces or
/// metadata fields degrade to `None`/sentinel values — never crash.
///
/// Registration failure is returned to the caller and must not block boot:
/// the WebView-level `ProcessFailed` observer remains as the fallback signal.
#[cfg(target_os = "windows")]
pub fn register(
    environment: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment,
    proxy: &tao::event_loop::EventLoopProxy<crate::NativeEvent>,
) -> windows::core::Result<i64> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2BrowserProcessExitedEventArgs, ICoreWebView2Environment5,
        COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND,
    };
    use windows::core::Interface;

    // Try to cast to ICoreWebView2Environment5 (requires WebView2 runtime
    // supporting the interface). If unavailable, the observer is silently
    // not registered and the caller must fall back to WebView-level signals.
    let env5: ICoreWebView2Environment5 = environment.cast()?;

    let proxy = proxy.clone();
    let handler = webview2_com::BrowserProcessExitedEventHandler::create(Box::new(
        move |_sender: Option<
            webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment,
        >,
              args: Option<ICoreWebView2BrowserProcessExitedEventArgs>| {
            let Some(args) = args else {
                return Ok(());
            };
            // Exit kind is required for classification.
            let mut raw_kind = COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND::default();
            if unsafe { args.BrowserProcessExitKind(&mut raw_kind) }.is_err() {
                return Ok(());
            }
            // Browser PID is best-effort: older runtimes may not populate it.
            let mut browser_pid = 0u32;
            let pid = unsafe { args.BrowserProcessId(&mut browser_pid) }
                .ok()
                .map(|()| browser_pid)
                .unwrap_or(0);

            let event = BrowserProcessExitedEvent {
                browser_pid: pid,
                kind: BrowserExitKind::from_raw(raw_kind.0),
            };
            crate::queue_event(&proxy, crate::NativeEvent::BrowserProcessExited(event));
            Ok(())
        },
    ));

    let mut token = 0i64;
    unsafe {
        env5.add_BrowserProcessExited(&handler, &mut token)?;
    }
    Ok(token)
}

/// Removes the environment-level observer. Best-effort at shutdown: failures
/// are logged, never propagated.
#[cfg(target_os = "windows")]
pub fn unregister(
    environment: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment,
    token: i64,
) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment5;
    use windows::core::Interface;

    if let Ok(env5) = environment.cast::<ICoreWebView2Environment5>() {
        if unsafe { env5.remove_BrowserProcessExited(token) }.is_err() {
            eprintln!("[Environment][BrowserProcessExited] subscription removal failed");
        }
    }
}

/// Whether the `ICoreWebView2Environment5` interface is available on the
/// current WebView2 runtime. Used for runtime validation logging.
#[cfg(target_os = "windows")]
pub fn is_available(
    environment: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment,
) -> bool {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment5;
    use windows::core::Interface;

    environment.cast::<ICoreWebView2Environment5>().is_ok()
}

#[cfg(test)]
mod tests {
    use crate::browser_recovery::{BrowserExitKind, BrowserProcessExitedEvent};

    #[test]
    fn exit_kind_maps_correctly() {
        let cases = [
            (0, BrowserExitKind::Normal, "Normal"),
            (1, BrowserExitKind::Failed, "Failed"),
        ];
        for (raw, expected, label) in cases {
            assert_eq!(BrowserExitKind::from_raw(raw), expected, "raw {raw}");
            assert_eq!(expected.as_str(), label);
        }
    }

    #[test]
    fn unknown_kinds_fail_open() {
        for raw in [2, 42, -1, i32::MIN, i32::MAX] {
            let kind = BrowserExitKind::from_raw(raw);
            assert_eq!(kind, BrowserExitKind::Unrecognized(raw));
            assert_eq!(kind.as_str(), "Unrecognized");
            assert!(!kind.is_failed());
        }
    }

    #[test]
    fn normal_exit_is_ineligible() {
        let event = BrowserProcessExitedEvent {
            browser_pid: 1234,
            kind: BrowserExitKind::Normal,
        };
        assert!(!crate::browser_recovery::BrowserRecovery::is_eligible(
            &event
        ));
    }

    #[test]
    fn failed_exit_is_eligible() {
        let event = BrowserProcessExitedEvent {
            browser_pid: 1234,
            kind: BrowserExitKind::Failed,
        };
        assert!(crate::browser_recovery::BrowserRecovery::is_eligible(
            &event
        ));
    }

    #[test]
    fn unrecognized_exit_is_ineligible() {
        let event = BrowserProcessExitedEvent {
            browser_pid: 1234,
            kind: BrowserExitKind::Unrecognized(99),
        };
        assert!(!crate::browser_recovery::BrowserRecovery::is_eligible(
            &event
        ));
    }
}

//! Native close-button behavior (release blocker 1).
//!
//! The titlebar close button sends `window_hide` when `minimizeToTray` is
//! true and `window_quit` when it is false (shared titlebar artifact,
//! `src/shelter/titlebar/index.ts:43-47` — Electron-owned, not touched
//! here). Hiding is only safe when a working tray icon can reopen the app;
//! the live worker (`tray_icon::TrayHandle` presence, checked by `main.rs`
//! at the close site) is what counts — a setting name alone never does.
//!
//! Policy (pure, no new mechanism): hide if and only if hide-to-tray was
//! requested AND a usable native tray is available; otherwise take the
//! existing orderly quit path (metrics, observer detach, `ControlFlow::Exit`
//! — never an abrupt termination, never a bypassed shutdown). The
//! `tray_available` input is the single flip point if a real tray with a
//! working reopen action is ever implemented; until then it is `false` and
//! every close quits, regardless of `minimizeToTray`. Minimize,
//! maximize/restore, startup visibility, and explicit quit are untouched,
//! as are downloads, recovery, security, and bridge capabilities.
//!
//! Privacy: the log line carries only fixed vocabulary plus two bounded
//! flags — no URIs, page data, or user content.

/// What the event loop does with a close request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Run the existing orderly shutdown (same path as `window_quit`).
    Quit,
    /// Hide the window to the tray (only with a working tray + reopen).
    Hide,
}

/// Whether a usable native tray icon with a working reopen action exists.
/// Compile-time floor for the close policy: `false`. Runtime availability
/// comes from the live tray worker (`tray_icon::TrayHandle` presence) —
/// a setting name alone (e.g. Electron's `minimizeToTray`/`tray`) never
/// counts, only an actual icon plus a tested reopen action flips the
/// runtime bit. Call sites pass the live presence; this function remains
/// as the documented floor plus its policy tests.
#[allow(dead_code)]
pub fn tray_available() -> bool {
    false
}

/// Feed one close request through the policy. `minimize_to_tray` is the
/// renderer's expressed intent (`window_hide` implies true, `window_quit`
/// implies false); `tray_available` is [`tray_available`]. Pure: no window,
/// setting, or shutdown state is touched.
pub fn decide(minimize_to_tray: bool, tray_available: bool) -> CloseAction {
    if minimize_to_tray && tray_available {
        CloseAction::Hide
    } else {
        CloseAction::Quit
    }
}

fn format_action(action: &CloseAction) -> &'static str {
    match action {
        CloseAction::Quit => "quit",
        CloseAction::Hide => "hide",
    }
}

/// Fixed-vocabulary `[Window][Close]` log line for one policy decision.
pub fn format_log(action: &CloseAction, minimize_to_tray: bool, tray_available: bool) -> String {
    format!(
        "[Window][Close] action={} minimize_to_tray={minimize_to_tray} tray_available={tray_available}",
        format_action(action),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hide_request_without_tray_quits() {
        // The reported blocker: minimizeToTray=true with no usable native
        // tray must take the orderly quit path, never strand the app.
        assert!(!tray_available());
        assert_eq!(decide(true, tray_available()), CloseAction::Quit);
        assert_eq!(
            format_log(&CloseAction::Quit, true, false),
            "[Window][Close] action=quit minimize_to_tray=true tray_available=false"
        );
    }

    #[test]
    fn explicit_quit_never_hides() {
        // minimizeToTray=false sends window_quit; it quits with or without
        // a tray — tray availability is irrelevant when nothing asked to
        // hide.
        assert_eq!(decide(false, false), CloseAction::Quit);
        assert_eq!(decide(false, true), CloseAction::Quit);
    }

    #[test]
    fn hide_requires_both_intent_and_a_working_tray() {
        // The only shape that may hide: requested AND a real tray exists.
        // No native tray exists today, so this arm is policy-only and
        // unreachable in production until a tray ships with a reopen action.
        assert_eq!(decide(true, true), CloseAction::Hide);
        assert_eq!(
            format_log(&CloseAction::Hide, true, true),
            "[Window][Close] action=hide minimize_to_tray=true tray_available=true"
        );
    }

    #[test]
    fn repeated_close_after_shutdown_stays_safe() {
        // The decision is shutdown-independent by design: a second close
        // arriving after shutdown started resolves the same way, and the
        // loop-level teardown is idempotent (each slot `take()`s to `None`,
        // so a repeat detaches nothing twice). Mirrored here with the same
        // take-twice shape the event loop relies on.
        assert_eq!(decide(true, tray_available()), CloseAction::Quit);
        let mut slot: Option<u64> = Some(1);
        assert_eq!(slot.take(), Some(1));
        assert_eq!(slot.take(), None);
        assert_eq!(decide(true, tray_available()), CloseAction::Quit);
    }

    #[test]
    fn shipped_module_stays_policy_only() {
        let source = include_str!("close_behavior.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // No behavior surface: no window calls, no settings I/O, no tray
        // construction, no shutdown execution, no navigation, no script.
        for forbidden in [
            "set_visible",
            "ControlFlow::Exit",
            "TrayIcon",
            "SystemTray",
            "load_settings",
            "set_minimized",
            "set_maximized",
            "Navigate(",
            "Reload(",
            "evaluate_script",
            "std::process",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
    }
}

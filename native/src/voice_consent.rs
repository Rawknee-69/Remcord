#![allow(dead_code)]
//! Voice/video consent-flow matrix (F10).
//!
//! NOTE (opencode-f19, 2026-10-14): this module is F10 work-in-progress at
//! the F19 base (`f2aeb19`): nothing in `main.rs` consumes it yet, so its
//! items read as dead code and deny-by-default lints fail the build. The
//! two `&[...]` const fixes below repair a base compile break of the same
//! origin. The module-level allow keeps the workspace verifiable until
//! F10 wires the module up; F10 should remove it with the wiring commit.
//!
//! Electron contract (`src/main.ts:357-400`, the sole
//! `session.defaultSession.setPermissionRequestHandler`): allow exactly
//! `fullscreen`, `notifications`, `clipboard-sanitized-write`, and one
//! undifferentiated `media`; deny everything else. On Windows the `media`
//! grant is unconditional (`callback(true)` — no OS prompt); only darwin
//! consults `systemPreferences.askForMediaAccess`. Display capture never
//! reaches that handler: it goes through the separate
//! `setDisplayMediaRequestHandler` (`src/discord/screenshare.ts`), which
//! enumerates sources and commits only after the user picks one in the
//! (custom or system) picker — cancel resolves to an empty grant.
//!
//! Native mapping (each enforced elsewhere; this module states the
//! flow-level expectations and pins them with tests):
//! - Mic/camera: [`crate::permissions`] answers per-request WebView2
//!   `PermissionRequested` events (MICROPHONE/CAMERA kinds) from the exact
//!   entry origins, never persisted (`SavesInProfile=false`), registered on
//!   the main WebView (`main_webview::attach_main_content_filters`), the
//!   child popup (`main.rs` popup builder), and re-attached after browser
//!   recovery. Electron's undifferentiated `media` allow splits into the
//!   two WebView2 kinds; the effect on Windows is identical (allow).
//! - Display: [`crate::screenshare_picker`] shows the Discord-styled setup
//!   popup on every `getDisplayMedia` call on the entry origins (cancel
//!   rejects with `NotAllowedError`, like a dismissed picker) and the OS
//!   system picker commits the concrete source. There is deliberately NO
//!   host-side silent-grant path (no `ScreenCaptureStarting` allow handler
//!   exists anywhere under `native/`), so display capture is ALWAYS
//!   user-mediated: success with an explicit pick, or denial by cancel.
//!
//! Deliberate, documented differences from Electron:
//! - The darwin `askForMediaAccess` round-trip has no native counterpart
//!   (native is Windows-only, where Electron also grants unconditionally).
//! - `fullscreen` / `clipboard-sanitized-write` have no WebView2 permission
//!   kind and cannot arrive as requests (see `permissions.rs` docs).
//! - Windows loopback audio and Venmic are platform boundaries, not
//!   omissions (see `screenshare_media.rs` docs).
//!
//! This module is pure policy plus tests: no window, no settings I/O, no
//! bridge, no filesystem, no fetch. The only authority is the flow id plus
//! the origin string, reusing [`crate::permissions`] as the single source
//! of truth for mic/camera so any policy drift fails these tests loudly.
//!
//! Privacy: log lines carry only fixed vocabulary plus booleans — no URIs,
//! page data, or user content.

use crate::permissions::{self, Decision};

/// One voice/video consent flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentFlow {
    /// Microphone capture (`getUserMedia({ audio })`).
    Mic,
    /// Camera capture (`getUserMedia({ video })`).
    Camera,
    /// Screen/window capture (`getDisplayMedia`).
    Display,
}

impl ConsentFlow {
    fn as_str(self) -> &'static str {
        match self {
            ConsentFlow::Mic => "mic",
            ConsentFlow::Camera => "camera",
            ConsentFlow::Display => "display",
        }
    }
}

/// Flow-level consent outcome. There is no silent-grant variant for display:
/// the host owns no path that produces one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentOutcome {
    /// Host pre-authorizes (mic/camera on trusted origins).
    Allow,
    /// Host pre-denies (mic/camera off trusted origins).
    Deny,
    /// No host pre-decision exists: consent is collected in the open —
    /// the setup popup plus the OS system picker (display). The user
    /// picks a source (grant) or cancels (`NotAllowedError`).
    UserMediated,
}

impl ConsentOutcome {
    fn as_str(self) -> &'static str {
        match self {
            ConsentOutcome::Allow => "allow",
            ConsentOutcome::Deny => "deny",
            ConsentOutcome::UserMediated => "user-mediated",
        }
    }
}

/// Expected outcome for one flow from one origin.
/// - Mic/camera delegate to [`permissions::decide`] (single source of
///   truth; never re-implemented here).
/// - Display is [`ConsentOutcome::UserMediated`] for EVERY origin: the
///   setup popup is origin-gated defense-in-depth, but the binding
///   constraint is structural — no silent-grant path exists host-side, so
///   an unattended grant is impossible and a hostile origin can at most
///   reach the OS picker, which still demands an explicit user pick.
///   (A future host-side `ScreenCaptureStarting` cancel for untrusted
///   origins would harden this further; it must never introduce an allow
///   path — the tests below forbid it.)
pub fn expected_outcome(flow: ConsentFlow, origin: &str) -> ConsentOutcome {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PERMISSION_KIND_CAMERA, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
    };
    match flow {
        ConsentFlow::Mic => {
            match permissions::decide(COREWEBVIEW2_PERMISSION_KIND_MICROPHONE.0, origin) {
                Decision::Allow => ConsentOutcome::Allow,
                Decision::Deny => ConsentOutcome::Deny,
            }
        }
        ConsentFlow::Camera => {
            match permissions::decide(COREWEBVIEW2_PERMISSION_KIND_CAMERA.0, origin) {
                Decision::Allow => ConsentOutcome::Allow,
                Decision::Deny => ConsentOutcome::Deny,
            }
        }
        ConsentFlow::Display => ConsentOutcome::UserMediated,
    }
}

/// Fixed-vocabulary `[Media][Consent]` log line for one flow expectation.
/// `origin_trusted` is the boolean classification, never the URI.
pub fn format_log(flow: ConsentFlow, origin_trusted: bool, outcome: ConsentOutcome) -> String {
    format!(
        "[Media][Consent] flow={} origin_trusted={origin_trusted} outcome={}",
        flow.as_str(),
        outcome.as_str(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY_ORIGINS: &[&str] = &[
        "https://discord.com",
        "https://discord.com/app",
        "https://canary.discord.com",
        "https://ptb.discord.com",
    ];

    const HOSTILE_ORIGINS: &[&str] = &[
        "https://evil.com",
        "https://sub.discord.com",
        "http://discord.com",
        "https://discord.com:443",
        "https://user@discord.com",
        "about:blank",
        "",
        "not a uri",
    ];

    #[test]
    fn mic_and_camera_allow_on_entry_origins() {
        // Electron `callback(true)` for `media` on Windows; native splits
        // the kind but the effect matches: allow.
        for origin in ENTRY_ORIGINS {
            assert_eq!(
                expected_outcome(ConsentFlow::Mic, origin),
                ConsentOutcome::Allow
            );
            assert_eq!(
                expected_outcome(ConsentFlow::Camera, origin),
                ConsentOutcome::Allow
            );
        }
    }

    #[test]
    fn mic_and_camera_deny_off_entry_origins() {
        for origin in HOSTILE_ORIGINS {
            assert_eq!(
                expected_outcome(ConsentFlow::Mic, origin),
                ConsentOutcome::Deny
            );
            assert_eq!(
                expected_outcome(ConsentFlow::Camera, origin),
                ConsentOutcome::Deny
            );
        }
    }

    #[test]
    fn mic_camera_match_permissions_decide_kind_for_kind() {
        // Tripwire: this module delegates, never re-implements. Any policy
        // change in permissions.rs must move these expectations with it.
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_PERMISSION_KIND_CAMERA, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
        };
        for origin in ENTRY_ORIGINS.iter().chain(HOSTILE_ORIGINS.iter()) {
            let mic = permissions::decide(COREWEBVIEW2_PERMISSION_KIND_MICROPHONE.0, origin);
            let camera = permissions::decide(COREWEBVIEW2_PERMISSION_KIND_CAMERA.0, origin);
            assert_eq!(
                expected_outcome(ConsentFlow::Mic, origin),
                if mic == Decision::Allow {
                    ConsentOutcome::Allow
                } else {
                    ConsentOutcome::Deny
                }
            );
            assert_eq!(
                expected_outcome(ConsentFlow::Camera, origin),
                if camera == Decision::Allow {
                    ConsentOutcome::Allow
                } else {
                    ConsentOutcome::Deny
                }
            );
        }
    }

    #[test]
    fn display_is_user_mediated_on_entry_origins() {
        // Setup popup (cancel -> NotAllowedError) plus the OS system picker
        // commits the source. No host pre-decision either way.
        for origin in ENTRY_ORIGINS {
            assert_eq!(
                expected_outcome(ConsentFlow::Display, origin),
                ConsentOutcome::UserMediated
            );
        }
    }

    #[test]
    fn display_is_never_silently_allowed_for_any_origin() {
        // The load-bearing invariant: no origin — trusted or hostile —
        // receives a silent display grant. A future change introducing an
        // allow path must fail here first.
        for origin in ENTRY_ORIGINS.iter().chain(HOSTILE_ORIGINS.iter()) {
            assert_ne!(
                expected_outcome(ConsentFlow::Display, origin),
                ConsentOutcome::Allow,
                "origin {origin:?} must never silently allow display capture"
            );
        }
    }

    #[test]
    fn consent_log_lines_use_fixed_vocabulary() {
        assert_eq!(
            format_log(ConsentFlow::Mic, true, ConsentOutcome::Allow),
            "[Media][Consent] flow=mic origin_trusted=true outcome=allow"
        );
        assert_eq!(
            format_log(ConsentFlow::Camera, false, ConsentOutcome::Deny),
            "[Media][Consent] flow=camera origin_trusted=false outcome=deny"
        );
        assert_eq!(
            format_log(ConsentFlow::Display, true, ConsentOutcome::UserMediated),
            "[Media][Consent] flow=display origin_trusted=true outcome=user-mediated"
        );
    }

    #[test]
    fn shipped_module_stays_policy_only() {
        let source = include_str!("voice_consent.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // No enforcement surface of its own: no window calls, no WebView2
        // state writes, no settings I/O, no bridge, no script execution,
        // no filesystem, no fetch, no capture machinery. Enforcement lives
        // in permissions.rs (mic/camera) and screenshare_picker.rs plus
        // the OS picker (display); this module only states expectations.
        for forbidden in [
            "SetState",
            "SavesInProfile",
            "add_PermissionRequested",
            "set_visible",
            "ControlFlow::Exit",
            "load_settings",
            "evaluate_script",
            "executeJavaScript",
            "postMessage",
            "getDisplayMedia",
            "getUserMedia",
            "desktopCapturer",
            "ScreenCaptureStarting",
            "venmic",
            "loopback",
            "std::fs",
            "std::process",
            "ureq",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
        // Delegation is real, not documented-only.
        assert!(shipped.contains("permissions::decide"));
    }
}

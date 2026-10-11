//! Native WebView2 permission policy (P0.3).
//!
//! Electron contract (`src/main.ts:368-411`, the sole
//! `session.defaultSession.setPermissionRequestHandler`): allow exactly
//! `fullscreen`, `notifications`, `clipboard-sanitized-write`, and one
//! undifferentiated `media`; deny everything else. Display capture never
//! reaches that handler — it is a separate `setDisplayMediaRequestHandler`
//! surface (`src/discord/screenshare.ts`) and stays out of this milestone.
//!
//! WebView2 mapping (verified against `webview2-com-sys 0.39.1`, never
//! guessed): only MICROPHONE, CAMERA, and NOTIFICATIONS are allowlisted.
//! There is NO WebView2 permission kind for fullscreen, clipboard-write, or
//! screen/display capture (including PERSISTENT_STORAGE, which the installed
//! bindings do not expose) — those requests cannot arrive here, and any
//! unknown or future kind denies by the catch-all.
//!
//! Origin policy: exact `https://discord.com` only (mirrors the bridge
//! origin gate, deliberately stricter than the navigation gate, which also
//! permits `*.discord.com`). Both the request URI and the live WebView
//! source must validate; any parse failure or disagreement denies.
//!
//! Persistence: WebView2 persists permission decisions in the profile by
//! default, so every handled request explicitly sets `SavesInProfile=false`
//! (Electron answered per-request with no persistence). No
//! `SetPermissionState` or profile permission API is used, ever.
//! `Handled` is deliberately NOT set: there is exactly one main-frame
//! handler and no competing consumer, matching wry's own precedent of
//! answering with `SetState` alone; setting it is not required to prevent
//! duplicate handling here.
//!
//! No deferral: the policy is fully synchronous. No filesystem, no fetch,
//! no paths, no renderer input of any kind — the only authority is native
//! WebView2 event data.

use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2PermissionRequestedEventArgs,
    ICoreWebView2PermissionRequestedEventArgs3, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
    COREWEBVIEW2_PERMISSION_STATE_DENY,
};
use webview2_com::PermissionRequestedEventHandler;
use windows::core::{Interface, BOOL, PWSTR};

/// The single allowed origin. Extended only with a traced consumer behind
/// the addition; never `*.discord.com` in this milestone.
pub const TRUSTED_PERMISSION_ORIGIN: &str = "https://discord.com";

/// Policy outcome. There is no "default": every handled request gets an
/// explicit ALLOW or DENY, never WebView2 DEFAULT (which may prompt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
}

impl Decision {
    fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Deny => "deny",
        }
    }
}

/// Strict origin check against [`TRUSTED_PERMISSION_ORIGIN`]: `https`
/// scheme, exact host `discord.com` (case-insensitive, hosts are
/// case-insensitive per RFC), no port (an explicitly written port — even
/// `:443` — denies), no userinfo, no opaque origin, no `about:blank`, no
/// subdomains. Anything else denies.
pub fn is_trusted_origin(uri: &str) -> bool {
    let allow = match url::Url::parse(TRUSTED_PERMISSION_ORIGIN) {
        Ok(allow) => allow,
        Err(_) => return false,
    };
    let parsed = match url::Url::parse(uri) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    if parsed.scheme() != allow.scheme() {
        return false;
    }
    let hosts_match = match (parsed.host_str(), allow.host_str()) {
        (Some(candidate), Some(allowlisted)) => candidate.eq_ignore_ascii_case(allowlisted),
        _ => false,
    };
    if !hosts_match {
        return false;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    // The url crate normalizes a default port away (even in `authority()`),
    // so an explicitly written port must be detected on the RAW authority
    // section of the input: `scheme://authority/rest`. Userinfo itself
    // already denied above; only the host part past the last `@` counts.
    let after_scheme = match uri.split_once("://") {
        Some((_, rest)) => rest,
        None => return false,
    };
    let raw_authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let raw_host_port = raw_authority.rsplit('@').next().unwrap_or("");
    if raw_host_port.contains(':') {
        return false;
    }
    true
}

/// Pure policy: allow exactly microphone/camera/notifications from the
/// trusted origin; deny every other kind and every other origin. Unknown
/// and future kind integers deny via the catch-all.
pub fn decide(kind: i32, origin: &str) -> Decision {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PERMISSION_KIND_CAMERA, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
        COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS,
    };
    if !is_trusted_origin(origin) {
        return Decision::Deny;
    }
    let allowed = kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE.0
        || kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA.0
        || kind == COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS.0;
    if allowed {
        Decision::Allow
    } else {
        Decision::Deny
    }
}

/// Stable kind name for diagnostics. Unknown integers report as unknown —
/// they still deny via [`decide`].
pub fn kind_name(kind: i32) -> &'static str {
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    if kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE.0 {
        "microphone"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA.0 {
        "camera"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION.0 {
        "geolocation"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS.0 {
        "notifications"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_OTHER_SENSORS.0 {
        "sensors"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ.0 {
        "clipboard-read"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS.0 {
        "multiple-downloads"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_FILE_READ_WRITE.0 {
        "file-read-write"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY.0 {
        "autoplay"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_LOCAL_FONTS.0 {
        "local-fonts"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_MIDI_SYSTEM_EXCLUSIVE_MESSAGES.0 {
        "midi"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_WINDOW_MANAGEMENT.0 {
        "window-management"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION.0 {
        "unknown"
    } else {
        "unlisted-future-kind"
    }
}

/// Attach exactly one `PermissionRequested` handler to the main WebView.
/// Deliberately NOT wry's `with_permission_handler`: that helper drops the
/// request URI/origin, which this exact-origin policy is built on.
pub fn register(webview: &ICoreWebView2) -> windows::core::Result<()> {
    let handler = PermissionRequestedEventHandler::create(Box::new(
        move |sender: Option<ICoreWebView2>,
              args: Option<ICoreWebView2PermissionRequestedEventArgs>| {
            let Some(args) = args else {
                return Ok(());
            };
            let request_uri = {
                let mut uri = PWSTR::null();
                if unsafe { args.Uri(&mut uri) }.is_err() {
                    String::new()
                } else {
                    unsafe { uri.to_string().unwrap_or_default() }
                }
            };
            let mut kind = webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PERMISSION_KIND::default();
            if unsafe { args.PermissionKind(&mut kind) }.is_err() {
                kind.0 = webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION.0;
            }
            let mut initiated = BOOL::default();
            if unsafe { args.IsUserInitiated(&mut initiated) }.is_err() {
                initiated = BOOL::from(false);
            }
            // Both the request URI and the live WebView source must validate;
            // any failure or disagreement denies.
            let source_trusted = sender.as_ref().is_some_and(|view| {
                let mut source = PWSTR::null();
                if unsafe { view.Source(&mut source) }.is_err() {
                    return false;
                }
                let source = unsafe { source.to_string().unwrap_or_default() };
                is_trusted_origin(&source)
            });
            let decision = if source_trusted {
                decide(kind.0, &request_uri)
            } else {
                Decision::Deny
            };
            // Request-scoped, like Electron's per-request callback: never
            // persist the grant in the profile.
            match args.cast::<ICoreWebView2PermissionRequestedEventArgs3>() {
                Ok(args3) => {
                    if unsafe { args3.SetSavesInProfile(false) }.is_err() {
                        eprintln!("[Permissions] could not unset SavesInProfile; state still set explicitly");
                    }
                }
                Err(_) => {
                    eprintln!("[Permissions] Args3 unavailable; state still set explicitly");
                }
            }
            let state = match decision {
                Decision::Allow => COREWEBVIEW2_PERMISSION_STATE_ALLOW,
                Decision::Deny => COREWEBVIEW2_PERMISSION_STATE_DENY,
            };
            unsafe { args.SetState(state)? };
            // Classification only: kind, trust outcome, gesture flag, and
            // decision. Never raw URIs, bodies, credentials, or page data.
            println!(
                "[Permissions] kind={} origin={} initiated={} decision={}",
                kind_name(kind.0),
                if is_trusted_origin(&request_uri) {
                    "trusted"
                } else {
                    "untrusted"
                },
                initiated.as_bool(),
                decision.as_str(),
            );
            Ok(())
        },
    ));
    let mut token = 0i64;
    unsafe {
        webview.add_PermissionRequested(&handler, &mut token)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlisted_kinds_from_trusted_origin_allow() {
        assert_eq!(
            decide(
                webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PERMISSION_KIND_MICROPHONE.0,
                "https://discord.com"
            ),
            Decision::Allow
        );
        assert_eq!(
            decide(
                webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PERMISSION_KIND_CAMERA
                    .0,
                "https://discord.com"
            ),
            Decision::Allow
        );
        assert_eq!(
            decide(
                webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS.0,
                "https://discord.com"
            ),
            Decision::Allow
        );
    }

    #[test]
    fn trusted_origin_with_path_still_allows_listed_kinds() {
        assert_eq!(
            decide(
                webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PERMISSION_KIND_MICROPHONE.0,
                "https://discord.com/app"
            ),
            Decision::Allow
        );
    }

    #[test]
    fn denied_kinds_from_trusted_origin_deny() {
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        for kind in [
            COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION.0,
            COREWEBVIEW2_PERMISSION_KIND_OTHER_SENSORS.0,
            COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ.0,
            COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS.0,
            COREWEBVIEW2_PERMISSION_KIND_FILE_READ_WRITE.0,
            COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY.0,
            COREWEBVIEW2_PERMISSION_KIND_LOCAL_FONTS.0,
            COREWEBVIEW2_PERMISSION_KIND_MIDI_SYSTEM_EXCLUSIVE_MESSAGES.0,
            COREWEBVIEW2_PERMISSION_KIND_WINDOW_MANAGEMENT.0,
            COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION.0,
        ] {
            assert_eq!(
                decide(kind, "https://discord.com"),
                Decision::Deny,
                "kind {kind} must deny"
            );
        }
    }

    #[test]
    fn fabricated_and_future_kind_integers_deny() {
        for kind in [-1, 13, 14, 99, 1000, i32::MAX, i32::MIN] {
            assert_eq!(
                decide(kind, "https://discord.com"),
                Decision::Deny,
                "kind {kind} must deny"
            );
            assert_eq!(
                kind_name(kind),
                if kind == 0 {
                    "unknown"
                } else {
                    "unlisted-future-kind"
                },
            );
        }
    }

    #[test]
    fn untrusted_origins_deny_every_kind() {
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        for origin in [
            "https://evil.com",
            "https://sub.discord.com",
            "https://deep.sub.discord.com",
            "http://discord.com",
            "https://discord.com:443",
            "https://discord.com:444",
            "https://discord.com.",
            "https://user@discord.com",
            "about:blank",
            "remcord://html/x",
            "remcord://html/x",
            "",
            "not a uri",
            "discord.com",
            "file:///E:/x",
        ] {
            for kind in [
                COREWEBVIEW2_PERMISSION_KIND_MICROPHONE.0,
                COREWEBVIEW2_PERMISSION_KIND_CAMERA.0,
                COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS.0,
            ] {
                assert_eq!(
                    decide(kind, origin),
                    Decision::Deny,
                    "origin {origin:?} must deny"
                );
            }
            assert!(!is_trusted_origin(origin), "origin {origin:?} untrusted");
        }
    }

    #[test]
    fn trusted_origin_accepts_exact_form_only() {
        assert!(is_trusted_origin("https://discord.com"));
        assert!(is_trusted_origin("https://discord.com/"));
        assert!(is_trusted_origin("https://discord.com/app"));
        assert!(is_trusted_origin("HTTPS://DISCORD.COM"));
    }

    #[test]
    fn kind_names_cover_the_installed_enum() {
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        assert_eq!(
            kind_name(COREWEBVIEW2_PERMISSION_KIND_MICROPHONE.0),
            "microphone"
        );
        assert_eq!(kind_name(COREWEBVIEW2_PERMISSION_KIND_CAMERA.0), "camera");
        assert_eq!(
            kind_name(COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS.0),
            "notifications"
        );
        assert_eq!(
            kind_name(COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION.0),
            "geolocation"
        );
        assert_eq!(
            kind_name(COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ.0),
            "clipboard-read"
        );
        assert_eq!(
            kind_name(COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION.0),
            "unknown"
        );
    }

    #[test]
    fn shipped_module_has_no_forbidden_surfaces() {
        let source = include_str!("permissions.rs");
        // No persistence, no generic surface, no renderer/facade/bridge,
        // no screenshare/media-capture machinery, no filesystem, no fetch.
        // Module docs name forbidden APIs for contrast, so comment lines
        // (`//`, `///`, `//!`) are stripped before asserting on code.
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "SetPermissionState",
            "with_permission_handler",
            "with_clipboard",
            "SavesInProfile(true)",
            "SetSavesInProfile(true)",
            "getDisplayMedia",
            "desktopCapturer",
            "venmic",
            "loopback",
            "executeJavaScript",
            "evaluate_script",
            "postMessage",
            "std::fs",
            "ureq",
            "SetHandled(true)",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
        // SavesInProfile is explicitly *disabled*, never enabled.
        assert!(shipped.contains("SetSavesInProfile(false)"));
    }
}

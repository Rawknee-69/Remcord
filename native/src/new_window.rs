//! Native popup classification + external navigation (P0.4).
//!
//! Electron contract (`src/discord/window.ts:214-264`, the sole
//! `setWindowOpenHandler`): exact `about:blank` allow, Discord ICS blob
//! allow (hidden), Discord stream popout allow (`alwaysOnTop` from the
//! host-owned `popoutPiP` setting), `https:`/`http:`/`mailto:` fast path to
//! `shell.openExternal`, otherwise a confirmation dialog with a sticky
//! `ignoreProtocolWarning` bypass.
//!
//! Native mapping (verified against `wry 0.57.0` + `webview2-com 0.39`):
//! the wry handler `Fn(String, NewWindowFeatures) -> NewWindowResponse`
//! decides per URL. Renderer window features (size/position) are
//! deliberately ignored — the host owns every child option. `Deny` sets
//! WebView2 `Handled=true` with no child; external URLs additionally run
//! the narrow host-owned shell open below; approved popups are denied at the
//! WebView2 layer and re-created as one managed single-slot child on the
//! event loop (wry cannot build a `Tao` window inside the handler, which has
//! no `EventLoopWindowTarget`).
//!
//! Documented hardening deltas from Electron (never silent):
//! - Popup classification parses URLs; no raw `startsWith` trust.
//! - Popout requires exact host, no explicit port (even `:443`), no
//!   userinfo. `https://evil@discord.com/popout` and
//!   `https://discord.com:443/popout` are NOT trusted popups.
//! - The sticky `ignoreProtocolWarning` bypass is NOT reproduced. Unknown
//!   non-dangerous schemes resolve to `ConfirmExternal`, which the host
//!   answers with deny + log (no modal, no new settings key, no persistent
//!   bypass). `javascript:`/`data:`/`file:`/non-ICS `blob:` never confirm.
//! - `https:`/`http:` external opens require a parsed host; `mailto:`
//!   requires only its scheme. Malformed values never reach the shell.
//!
//! No renderer surface: [`Decision`] is host-internal. No HWND/PID, no
//! command line, no `executeJavaScript`, no filesystem access here.

/// Host-internal popup outcome. Never crosses the bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    AllowAboutBlank,
    AllowIcs,
    AllowDiscordPopout,
    ExternalOpen,
    /// Unknown non-dangerous custom scheme. The host answers deny (no
    /// Electron-style modal, no sticky bypass).
    ConfirmExternal,
    Deny,
}

/// Which managed single-slot child to create for an allowed popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    AboutBlank,
    Ics,
    DiscordPopout,
}

/// Maps an allow decision to its child kind. External/confirm/deny
/// decisions create no child.
pub fn popup_kind_for(decision: Decision) -> Option<PopupKind> {
    match decision {
        Decision::AllowAboutBlank => Some(PopupKind::AboutBlank),
        Decision::AllowIcs => Some(PopupKind::Ics),
        Decision::AllowDiscordPopout => Some(PopupKind::DiscordPopout),
        Decision::ExternalOpen | Decision::ConfirmExternal | Decision::Deny => None,
    }
}

/// Exact `about:blank` only (Electron parity: string equality, and the
/// string parses as `about:` so it must precede every scheme check).
pub fn is_about_blank(url: &str) -> bool {
    url == "about:blank"
}

/// Exact Discord stream popout hosts. No wildcard: any other subdomain
/// (e.g. `evil.discord.com`) is NOT a popout.
fn is_popout_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("discord.com")
        || host.eq_ignore_ascii_case("canary.discord.com")
        || host.eq_ignore_ascii_case("ptb.discord.com")
}

/// Raw-authority check shared with the P0.3 permission policy: an
/// explicitly written port (even `:443`) or userinfo denies. The `url`
/// crate normalizes default ports away, so the check runs on the raw
/// `scheme://authority` section; only the host part past the last `@`
/// counts for the port test.
fn raw_authority_is_bare_host(url_str: &str) -> bool {
    let after_scheme = match url_str.split_once("://") {
        Some((_, rest)) => rest,
        None => return false,
    };
    let raw_authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    if raw_authority.is_empty() {
        return false;
    }
    // Any userinfo at all denies.
    if raw_authority.contains('@') {
        return false;
    }
    !raw_authority.contains(':')
}

/// Discord stream popout: `https:`, exact host allowlist, no port, no
/// userinfo, pathname exactly `/popout`. Query and fragment are allowed
/// (Discord appends stream parameters); `http:`, wildcard subdomains,
/// `discord.com.evil.com`, and malformed values are rejected.
pub fn is_discord_popout_url(url_str: &str) -> bool {
    let parsed = match url::Url::parse(url_str) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    if parsed.scheme() != "https" {
        return false;
    }
    let host = match parsed.host_str() {
        Some(host) => host,
        None => return false,
    };
    if !is_popout_host(host) {
        return false;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    if !raw_authority_is_bare_host(url_str) {
        return false;
    }
    parsed.path() == "/popout"
}

/// True when `hostname` is exactly `discord.com` or a subdomain (Electron
/// `hostnameMatches` parity for the ICS inner origin).
fn hostname_matches_discord_com(hostname: &str) -> bool {
    hostname.eq_ignore_ascii_case("discord.com")
        || hostname.to_ascii_lowercase().ends_with(".discord.com")
}

/// Discord ICS blob: outer `blob:`, inner `http:`/`https:` URL whose host is
/// `discord.com` or a proven subdomain, no inner userinfo, no inner
/// explicit port. Mirrors `isDiscordIcsBlobUrl` (`src/common/sanitization.ts`)
/// with the same userinfo/port hardening as the popout. Opaque, encoded,
/// or malformed inner values fail closed.
pub fn is_discord_ics_blob_url(url_str: &str) -> bool {
    let outer = match url::Url::parse(url_str) {
        Ok(outer) => outer,
        Err(_) => return false,
    };
    if outer.scheme() != "blob" {
        return false;
    }
    // `blob:https://discord.com/<uuid>`: the payload rides in the path.
    let inner_str = outer.path();
    let inner = match url::Url::parse(inner_str) {
        Ok(inner) => inner,
        Err(_) => return false,
    };
    if inner.scheme() != "https" && inner.scheme() != "http" {
        return false;
    }
    let host = match inner.host_str() {
        Some(host) => host,
        None => return false,
    };
    if !hostname_matches_discord_com(host) {
        return false;
    }
    if !inner.username().is_empty() || inner.password().is_some() {
        return false;
    }
    raw_authority_is_bare_host(inner_str)
}

/// Narrow external fast path: `https:`/`http:` with a parsed host, or
/// `mailto:`. This intentionally replaces Electron's raw
/// `startsWith("https:")` prefix (which also matches `https:evil` and other
/// unparseable strings): unparseable and hostless values never reach the
/// shell. Userinfo/ports are the browser's job here; the popup trust checks
/// above already ran first.
pub fn is_fast_path_external(url_str: &str) -> bool {
    let parsed = match url::Url::parse(url_str) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    match parsed.scheme() {
        // `://` authority is required in the raw text: WHATWG parsing maps
        // `https:evil` to host `evil`, so the raw form gates the oddity
        // out — only real `scheme://authority` links fast-path.
        "https" | "http" => {
            let authority = match url_str.split_once("://") {
                Some((_, rest)) => rest.split(['/', '?', '#']).next().unwrap_or(""),
                None => return false,
            };
            !authority.is_empty() && parsed.host_str().is_some_and(|host| !host.is_empty())
        }
        "mailto" => true,
        _ => false,
    }
}

/// Always-deny schemes: scriptable, file, or non-ICS blob content. `blob:`
/// is dangerous unless the ICS classifier already accepted it (checked
/// before this runs in [`decide`]). Unknown custom schemes are NOT
/// dangerous — they resolve to [`Decision::ConfirmExternal`].
fn is_dangerous_scheme(scheme: &str) -> bool {
    matches!(scheme, "javascript" | "data" | "file" | "blob")
}

/// Exact popup decision tree. Order mirrors Electron
/// (`window.ts:214-264`): `about:blank`, ICS blob, Discord popout, fast
/// external, then dangerous/malformed deny, unknown custom confirm.
pub fn decide(url_str: &str) -> Decision {
    if is_about_blank(url_str) {
        return Decision::AllowAboutBlank;
    }
    if is_discord_ics_blob_url(url_str) {
        return Decision::AllowIcs;
    }
    if is_discord_popout_url(url_str) {
        return Decision::AllowDiscordPopout;
    }
    if is_fast_path_external(url_str) {
        return Decision::ExternalOpen;
    }
    match url::Url::parse(url_str) {
        Ok(parsed) if is_dangerous_scheme(parsed.scheme()) => Decision::Deny,
        Ok(_) => Decision::ConfirmExternal,
        Err(_) => Decision::Deny,
    }
}

/// Host-owned external open for exactly the fast-path schemes. Revalidates
/// before touching the shell, so no caller can smuggle a denied scheme in.
/// Direct `ShellExecuteW("open", url)` — no command line is ever
/// constructed, no shell interpreter runs, no HWND/PID crosses to the
/// renderer. Non-Windows fails closed.
#[cfg(target_os = "windows")]
pub fn open_external_url(url_str: &str) -> Result<(), &'static str> {
    if !is_fast_path_external(url_str) {
        return Err("URL is not an allowed external scheme");
    }
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::{
        Foundation::HWND, UI::Shell::ShellExecuteW, UI::WindowsAndMessaging::SW_SHOWNORMAL,
    };
    let operation = HSTRING::from("open");
    let file = HSTRING::from(url_str);
    // `ShellExecuteW` dispatches the verb and returns immediately; it does
    // not block the calling thread on the opened application.
    let instance = unsafe {
        ShellExecuteW(
            Some(HWND::default()),
            &operation,
            &file,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (instance.0 as isize) > 32 {
        Ok(())
    } else {
        Err("shell open failed")
    }
}

/// See [`open_external_url`]: Windows-only shell surface, typed failure
/// elsewhere.
#[cfg(not(target_os = "windows"))]
pub fn open_external_url(_url_str: &str) -> Result<(), &'static str> {
    Err("external opening is not supported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_blank_matches_the_exact_string_only() {
        assert_eq!(decide("about:blank"), Decision::AllowAboutBlank);
        assert!(is_about_blank("about:blank"));
        // Near-misses are not the Electron branch.
        for other in [
            "about:blank ",
            " about:blank",
            "ABOUT:BLANK",
            "about:blank#x",
            "about:srcdoc",
        ] {
            assert!(!is_about_blank(other), "{other} must not match");
            assert_ne!(decide(other), Decision::AllowAboutBlank);
        }
    }

    #[test]
    fn popout_allows_the_three_exact_hosts_with_query_and_fragment() {
        for url in [
            "https://discord.com/popout",
            "https://discord.com/popout?server=1&channel=2",
            "https://discord.com/popout#x",
            "https://canary.discord.com/popout",
            "https://canary.discord.com/popout?x=1",
            "https://ptb.discord.com/popout",
            "https://ptb.discord.com/popout?x=1#y",
            "HTTPS://DISCORD.COM/popout",
        ] {
            assert!(is_discord_popout_url(url), "{url} must be a popout");
            assert_eq!(decide(url), Decision::AllowDiscordPopout);
        }
    }

    #[test]
    fn popout_rejects_wildcards_spoofs_ports_and_userinfo() {
        for url in [
            "https://evil.discord.com/popout",
            "https://deep.sub.discord.com/popout",
            "https://discord.com.evil.com/popout",
            "https://evil.com/popout",
            "http://discord.com/popout",
            "https://discord.com:443/popout",
            "https://discord.com:444/popout",
            "https://evil@discord.com/popout",
            "https://user:pass@discord.com/popout",
            "https://discord.com/popout/",
            "https://discord.com/popouts",
            "https://discord.com/app",
            "https://discord.com",
            "not a url",
            "",
        ] {
            assert!(!is_discord_popout_url(url), "{url} must not be a popout");
            assert_ne!(decide(url), Decision::AllowDiscordPopout);
        }
    }

    #[test]
    fn ics_accepts_only_discord_origin_blobs() {
        for url in [
            "blob:https://discord.com/12345678-1234-1234-1234-1234567890ab",
            "blob:https://canary.discord.com/12345678-1234-1234-1234-1234567890ab",
            "blob:http://discord.com/uuid",
        ] {
            assert!(is_discord_ics_blob_url(url), "{url} must be an ICS blob");
            assert_eq!(decide(url), Decision::AllowIcs);
        }
        for url in [
            "blob:https://evil.com/uuid",
            "blob:https://discord.com.evil.com/uuid",
            "blob:https://evil@discord.com/uuid",
            "blob:https://discord.com:443/uuid",
            "blob:file:///C:/evil.ics",
            "blob:data:text/plain,hi",
            "blob:opaque-string",
            "blob:",
            "https://discord.com/uuid",
        ] {
            assert!(!is_discord_ics_blob_url(url), "{url} must not be ICS");
            assert_ne!(decide(url), Decision::AllowIcs);
        }
    }

    #[test]
    fn fast_path_opens_https_http_and_mailto_only() {
        for url in [
            "https://example.com/x",
            "http://example.com/x",
            "mailto:someone@example.com",
            "https://github.com/Legcord/Legcord",
            "https://discord.gg/JatCnhKufc",
        ] {
            assert!(is_fast_path_external(url), "{url} must be fast-path");
            assert_eq!(decide(url), Decision::ExternalOpen);
        }
        // Hostless or malformed https values never reach the shell.
        for url in ["https:evil", "https:///path", "https:", ""] {
            assert!(!is_fast_path_external(url), "{url} must not fast-path");
        }
    }

    #[test]
    fn dangerous_schemes_and_malformed_values_deny_without_confirm() {
        for url in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "data:text/html,<h1>hi</h1>",
            "file:///C:/Windows/System32/drivers/etc/hosts",
            "file://server/share/evil.exe",
            "blob:https://evil.com/uuid",
            "tel:+123456789",
            "not a url [[[",
            "",
        ] {
            let decision = decide(url);
            // `tel:` is unknown (confirm); everything else here denies.
            if url.starts_with("tel:") {
                assert_eq!(decision, Decision::ConfirmExternal);
            } else {
                assert_eq!(decision, Decision::Deny, "{url}");
            }
            assert!(popup_kind_for(decision).is_none());
        }
    }

    #[test]
    fn unknown_custom_schemes_confirm_never_allow_or_open() {
        for url in [
            "discord://-/channels/@me",
            "unknown://do-thing",
            "legcord://plugins/rpc/",
        ] {
            assert_eq!(decide(url), Decision::ConfirmExternal, "{url}");
            assert!(!is_fast_path_external(url));
        }
    }

    #[test]
    fn redirect_query_never_confirms_discord_trust() {
        // Trust applies to the handled URL itself, never to a query echo.
        assert_eq!(
            decide("https://evil.com/redirect?to=https://discord.com/popout"),
            Decision::ExternalOpen
        );
        assert!(!is_discord_popout_url(
            "https://evil.com/redirect?to=https://discord.com/popout"
        ));
    }

    #[test]
    fn shell_open_rejects_everything_outside_the_fast_path() {
        for url in [
            "javascript:alert(1)",
            "data:text/html,x",
            "file:///C:/Windows/System32/x",
            "file://server/share/x",
            "blob:https://evil.com/uuid",
            "unknown://thing",
            "discord://-/x",
            "not a url",
            "",
        ] {
            assert!(open_external_url(url).is_err(), "{url} must not shell-open");
        }
        // Positive shell launch is never exercised in tests (it would open
        // a real browser); the allow-shape is covered by
        // `fast_path_opens_https_http_and_mailto_only`.
    }

    #[test]
    fn popup_kinds_cover_exactly_the_allow_decisions() {
        assert_eq!(
            popup_kind_for(Decision::AllowAboutBlank),
            Some(PopupKind::AboutBlank)
        );
        assert_eq!(popup_kind_for(Decision::AllowIcs), Some(PopupKind::Ics));
        assert_eq!(
            popup_kind_for(Decision::AllowDiscordPopout),
            Some(PopupKind::DiscordPopout)
        );
    }

    #[test]
    fn shipped_module_has_no_forbidden_surfaces() {
        let source = include_str!("new_window.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "Command::new",
            "std::process",
            "executeJavaScript",
            "evaluate_script",
            "SetNewWindow",
            "postMessage",
            "window.open",
            "with_related_view",
            "SetPermissionState",
            "showOpenDialog",
            "showSaveDialog",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
        // The one shell verb is fixed to "open"; no interpreter, no args.
        assert!(shipped.contains("HSTRING::from(\"open\")"));
    }
}

//! Native diagnostics windows (F25: WebRTC internals + GPU page).
//!
//! Electron contract (`src/discord/ipc.ts:76-104,422-427`): one
//! `chromeInternalsWindows` map keyed by the exact internal URL holds up to
//! two singleton `BrowserWindow`s (`chrome://webrtc-internals/` titled
//! "WebRTC Internals", `chrome://gpu/` titled "GPU", both 1100x800 with a
//! 640x480 minimum, `autoHideMenuBar`, sandboxed, no Node). Opening an
//! already-open page restores/focuses it instead of creating a second
//! window; closing a window deletes its map entry. The renderer triggers
//! them fire-and-forget through `window.remcord.settings.openWebRTCInternals`
//! / `openGPUInfo` (`src/discord/preload/bridge.ts:69-70`), wired to two
//! Settings-page buttons (`src/shelter/settings/pages/SettingsPage.tsx`).
//!
//! Native mapping (WebView2 is Edge under the hood, so the internal pages
//! load at the `edge://` scheme; the `chrome://` spellings are the documented
//! manual equivalent, not host-navigated): the host owns a page-keyed window
//! slot with the same singleton semantics — open focuses the live window,
//! close drops the slot entry. Geometry and titles are Electron-identical.
//! The renderer supplies nothing (both requests are parameter-free): no URL,
//! page id, size, or flag crosses the bridge, so a hostile renderer cannot
//! aim the window at an arbitrary origin.
//!
//! Documented hardening deltas from Electron (never silent):
//! - The child navigation allowlist admits ONLY the two internal schemes
//!   (`edge://`, `chrome://`) plus `about:blank`. Electron's chrome windows
//!   are full browsers that can wander anywhere; these leaf debug surfaces
//!   cannot (no Discord, no `https:`, no `file:`/`data:`/`javascript:`).
//! - No bridge, no facade script, no IPC handler, and no observers run in
//!   the diagnostics children: they render runtime-owned internal pages,
//!   never product UI. If the runtime blocks internal pages in embedded
//!   WebView2, the window shows the runtime's own error page — the host
//!   never invents substitute content.
//! - `alwaysOnTop` is never set (Electron never sets it on these windows).

use serde::Serialize;

/// Canonical page id for the WebRTC internals window. Exact string the
/// renderer sees in acknowledgements; never a URL.
pub const WEBRTC_PAGE_ID: &str = "webrtc-internals";
/// Canonical page id for the GPU info window.
pub const GPU_PAGE_ID: &str = "gpu";

/// Electron-identical initial size (`BrowserWindow` 1100x800).
pub const WINDOW_WIDTH: f64 = 1100.0;
/// Electron-identical initial size (`BrowserWindow` 1100x800).
pub const WINDOW_HEIGHT: f64 = 800.0;
/// Electron-identical minimum width (`minWidth: 640`).
pub const WINDOW_MIN_WIDTH: f64 = 640.0;
/// Electron-identical minimum height (`minHeight: 480`).
pub const WINDOW_MIN_HEIGHT: f64 = 480.0;

/// Electron-identical title for the WebRTC internals window.
pub const WEBRTC_TITLE: &str = "WebRTC Internals";
/// Electron-identical title for the GPU window.
pub const GPU_TITLE: &str = "GPU";

/// Primary internal URL for the WebRTC page (Edge-native scheme).
pub const WEBRTC_URL: &str = "edge://webrtc-internals/";
/// Primary internal URL for the GPU page (Edge-native scheme).
pub const GPU_URL: &str = "edge://gpu/";

/// Closed set of diagnostics pages. Only these two ever open: the renderer
/// cannot invent a page because the requests carry no page member at all —
/// each wire type names its page at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticPage {
    WebrtcInternals,
    Gpu,
}

impl DiagnosticPage {
    /// Canonical id used in acknowledgements and logs (never a URL).
    pub fn id(self) -> &'static str {
        match self {
            DiagnosticPage::WebrtcInternals => WEBRTC_PAGE_ID,
            DiagnosticPage::Gpu => GPU_PAGE_ID,
        }
    }

    /// Electron-identical window title.
    pub fn title(self) -> &'static str {
        match self {
            DiagnosticPage::WebrtcInternals => WEBRTC_TITLE,
            DiagnosticPage::Gpu => GPU_TITLE,
        }
    }

    /// First internal URL the child loads (Edge-native scheme).
    pub fn primary_url(self) -> &'static str {
        match self {
            DiagnosticPage::WebrtcInternals => WEBRTC_URL,
            DiagnosticPage::Gpu => GPU_URL,
        }
    }
}

/// Child navigation allowlist for diagnostics windows: exactly the two
/// internal schemes plus `about:blank`. Internal pages rewrite their own
/// location (fragments, sub-paths), so any path under those schemes is
/// allowed; every other scheme — including `https:` (no product browsing
/// from a leaf debug surface), `file:`, `data:`, `javascript:`, and `blob:`
/// — denies.
pub fn is_allowed_navigation(url: &str) -> bool {
    if url == "about:blank" {
        return true;
    }
    // The raw `scheme://authority` form is required: WHATWG parsing maps
    // `edge:gpu` to scheme `edge` with an opaque path, so the raw form gates
    // that oddity out (same discipline as `new_window::is_fast_path_external`).
    if !url.contains("://") {
        return false;
    }
    match url::Url::parse(url) {
        // Internal pages address the browser itself (`edge://gpu/`,
        // `chrome://webrtc-internals/`): a host is always present, so a
        // hostless root (`edge://`) or opaque form (`edge:gpu`, already
        // gated above) never passes.
        Ok(parsed) if matches!(parsed.scheme(), "edge" | "chrome") => {
            parsed.host_str().is_some_and(|host| !host.is_empty())
        }
        _ => false,
    }
}

/// Bridge acknowledgement for opening (or focusing) one diagnostics window:
/// the closed page id plus which path was taken. `opened` and
/// `focused_existing` are never both true and never both false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticsWindowAck {
    pub page: &'static str,
    pub opened: bool,
    pub focused_existing: bool,
}

/// Build the acknowledgement for one open request. `opened == true` means a
/// new window was created; `false` means the live singleton was focused
/// (Electron's restore-and-focus path).
pub fn ack_for(page: DiagnosticPage, opened: bool) -> DiagnosticsWindowAck {
    DiagnosticsWindowAck {
        page: page.id(),
        opened,
        focused_existing: !opened,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ids_are_closed_distinct_and_url_safe() {
        // The closed two-entry set: distinct ids with no separator,
        // whitespace, or scheme characters, so an id can never smuggle a
        // URL, path, or second value through a log or ack line.
        assert_eq!(DiagnosticPage::WebrtcInternals.id(), "webrtc-internals");
        assert_eq!(DiagnosticPage::Gpu.id(), "gpu");
        for id in [WEBRTC_PAGE_ID, GPU_PAGE_ID] {
            assert!(!id.is_empty());
            assert!(id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'-'));
            assert!(!id.contains("://"));
        }
        assert_ne!(WEBRTC_PAGE_ID, GPU_PAGE_ID);
    }

    #[test]
    fn geometry_and_titles_match_electron() {
        // `src/discord/ipc.ts openChromeInternalsPage`: 1100x800 window,
        // 640x480 minimum, "WebRTC Internals" / "GPU" titles.
        assert_eq!((WINDOW_WIDTH, WINDOW_HEIGHT), (1100.0, 800.0));
        assert_eq!((WINDOW_MIN_WIDTH, WINDOW_MIN_HEIGHT), (640.0, 480.0));
        assert_eq!(DiagnosticPage::WebrtcInternals.title(), "WebRTC Internals");
        assert_eq!(DiagnosticPage::Gpu.title(), "GPU");
        assert_eq!(DiagnosticPage::WebrtcInternals.id(), "webrtc-internals");
        assert_eq!(DiagnosticPage::Gpu.id(), "gpu");
    }

    #[test]
    fn internal_urls_use_only_internal_schemes() {
        // WebView2 is Edge: the children load the `edge://` pages. No
        // remote, file, or scriptable URL may ever be a start page.
        for url in [
            WEBRTC_URL,
            GPU_URL,
            DiagnosticPage::WebrtcInternals.primary_url(),
            DiagnosticPage::Gpu.primary_url(),
        ] {
            let parsed = url::Url::parse(url).expect("start URL must parse");
            assert!(
                matches!(parsed.scheme(), "edge" | "chrome"),
                "{url} must use an internal scheme"
            );
            // Internal pages address the browser itself: the only hosts are
            // the two page hosts, never a remote origin.
            assert!(
                matches!(parsed.host_str(), Some("gpu") | Some("webrtc-internals")),
                "{url} must carry only an internal host"
            );
        }
        assert!(WEBRTC_URL.contains("webrtc-internals"));
        assert!(GPU_URL.contains("gpu"));
        assert_ne!(WEBRTC_URL, GPU_URL);
    }

    #[test]
    fn navigation_allows_only_internal_pages_and_about_blank() {
        for allowed in [
            "about:blank",
            "edge://gpu/",
            "edge://gpu/#features",
            "chrome://gpu/",
            "edge://webrtc-internals/",
            "chrome://webrtc-internals/",
        ] {
            assert!(is_allowed_navigation(allowed), "{allowed} must be allowed");
        }
        for denied in [
            "",
            "https://discord.com",
            "https://discord.com/app",
            "http://localhost:3000",
            "file:///C:/Windows/System32/drivers/etc/hosts",
            "data:text/html,<h1>hi</h1>",
            "javascript:alert(1)",
            "blob:https://discord.com/uuid",
            "mailto:someone@example.com",
            "discord://-/channels/@me",
            "remcord://plugins/rpc/",
            "edge:gpu",
            "chrome:gpu",
            "edge://",
            "not a url [[[",
        ] {
            assert!(!is_allowed_navigation(denied), "{denied} must be denied");
        }
    }

    #[test]
    fn ack_covers_exactly_the_open_and_focus_paths() {
        let opened = ack_for(DiagnosticPage::WebrtcInternals, true);
        assert_eq!(opened.page, "webrtc-internals");
        assert!(opened.opened);
        assert!(!opened.focused_existing);
        let focused = ack_for(DiagnosticPage::Gpu, false);
        assert_eq!(focused.page, "gpu");
        assert!(!focused.opened);
        assert!(focused.focused_existing);
        // Fixed keys only: page echo plus the two path flags.
        let value = serde_json::to_value(&opened).expect("must serialize");
        assert_eq!(value.as_object().unwrap().len(), 3);
        assert_eq!(value["page"], "webrtc-internals");
    }

    #[test]
    fn shipped_module_has_no_forbidden_surfaces() {
        // The module is pure host-owned logic: no bridge strings, no shell,
        // no process spawning, no script execution, no fetch/storage, and no
        // renderer-supplied selectors. Comment lines are stripped before
        // asserting (docs name forbidden APIs for contrast).
        let shipped = include_str!("diagnostics_windows.rs");
        let shipped = shipped[..shipped.find("#[cfg(test)]").unwrap_or(shipped.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "Command::new",
            "std::process",
            "ShellExecuteW",
            "executeJavaScript",
            "evaluate_script",
            "postMessage",
            "fetch(",
            "localStorage",
            "window.open",
            "https://",
            "http://",
            "file://",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
    }
}

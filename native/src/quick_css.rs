//! Host-pushed native Quick CSS injection for Phase D.
//!
//! This module is the native counterpart of the Electron Quick CSS path in
//! `src/common/themes.ts` (`initQuickCss` / `disableQuickCss`), whose actual
//! lifecycle was traced before any native code was written:
//!
//! ```text
//! startup  -> injectThemesMain on did-finish-load ->
//!             if getConfig("quickCss") { initQuickCss } ->
//!             missing quickCss.css is CREATED empty ->
//!             webContents.send("addTheme", "legcord-quick-css", <utf-8 content,
//!             even when empty>) -> <style id="legcord-quick-css"> FIRST,
//!             before every theme style
//! toggle   -> enableQuickCss: initQuickCss again (re-read + watcher restart)
//!             disableQuickCss: watcher closed + send("removeTheme",
//!             "legcord-quick-css")
//! live     -> fs.watch + 300 ms debounce -> removeTheme + addTheme (DEFERRED
//!             natively; this milestone is static startup injection + reload)
//! reload   -> fresh document; did-finish-load re-pushes the file content
//! ```
//!
//! Like themes, Quick CSS is a **host push only**: no renderer caller reads
//! Quick CSS back, so there is deliberately NO `quickCss.get()` bridge, no
//! new protocol command, and no new boot capability. The host reads the
//! fixed file itself and injects it through the shared substrate builders
//! ([`crate::mod_substrate::managed_css_script`]).
//!
//! Electron parity decisions (traced, not inferred):
//! - File: `<profile>/quickCss.css`, UTF-8. A missing file is CREATED empty
//!   by the host when enabled (Electron's `initQuickCss` writes `""`).
//! - Empty content IS pushed as an empty managed style (Electron sends
//!   `addTheme` unconditionally). Enabled-vs-disabled is therefore observable
//!   as style-present vs style-absent.
//! - Style identity preserves Electron's `legcord-quick-css` verbatim. It
//!   collides with no native style (`legcord-native-vencord-css`,
//!   `legcord-native-theme-*`, `legcord-native-prototype-css`), so keeping the
//!   exact id is safe and maximally compatible.
//! - Cascade position: Quick CSS is pushed BEFORE themes in the same document
//!   load (Electron calls `initQuickCss` before the theme loop; Vencord CSS
//!   lands earlier via preload `insertCSS`). Target order is therefore
//!   `Vencord CSS -> Quick CSS -> theme CSS`, and the host emission order in
//!   `main.rs` (Quick CSS block before the theme block) constructs exactly
//!   that. Later stylesheets win on equal specificity, so a property set by
//!   both Quick CSS and a theme resolves to the theme value.
//! - Enablement comes from the existing authoritative settings load
//!   (`settings::load_settings`, the same source the `settings.get_config`
//!   capability serves; default `true`, matching Electron's default). No new
//!   capability is created for the boolean. A corrupt settings file fails
//!   closed to disabled — matching Electron, where a corrupt cache falls
//!   back to `{}` and `getConfig("quickCss")` reads falsy `undefined`.
//!
//! Out of scope (deferred): live file watching (300 ms debounce
//! remove/add), the CSS editor, folder opening, CSS import appending, and any
//! renderer-facing Quick CSS API.
//!
//! Logical resource owned by this module: exactly one fixed file —
//! `<profile>/quickCss.css`. The renderer never receives the path.

use std::path::{Path, PathBuf};
use std::time::Instant;

/// Electron's Quick CSS style identity, preserved verbatim. It collides with
/// no native managed style, so compatibility is safe.
pub const QUICK_CSS_STYLE_ID: &str = "legcord-quick-css";
/// Fixed Quick CSS filename inside the host profile. The renderer never sees
/// this name as a path; it only ever observes the style identity above.
pub const QUICK_CSS_FILE: &str = "quickCss.css";
/// Maximum Quick CSS payload in bytes. Matches the theme/plugin-storage file
/// bound; real Quick CSS files are small editor buffers. Electron is
/// unbounded.
pub const MAX_QUICK_CSS_BYTES: usize = 1024 * 1024;

/// Host-controlled Quick CSS file: `<profile>/quickCss.css`. The renderer
/// never receives this path.
pub fn quick_css_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join(QUICK_CSS_FILE)
}

/// One host-side Quick CSS load pass. `css` carries the text to push when it
/// is `Some` — including the empty string, which Electron pushes as an empty
/// managed style. `state` names the outcome for DEV metrics; it is never
/// exposed to the renderer.
#[derive(Debug, Clone)]
pub struct QuickCssReport {
    pub enabled: bool,
    pub css: Option<String>,
    pub css_bytes: usize,
    pub state: &'static str,
    pub read_ms: f64,
}

/// Determine the enabled state from the authoritative settings and load the
/// host-owned Quick CSS file when enabled.
///
/// Order: settings load (fail closed on corruption, matching Electron's
/// falsy fallback) → enabled flag → missing file is created empty (Electron
/// parity) → size bound → UTF-8 → content (possibly empty) or a typed state.
/// The host never crashes on any of these paths; failures surface as a state
/// the caller logs instead of injecting.
pub fn load_quick_css(profile_dir: &Path) -> QuickCssReport {
    let started_at = Instant::now();
    let read_ms = |started: Instant| started.elapsed().as_secs_f64() * 1000.0;

    let enabled = match crate::settings::load_settings(profile_dir) {
        Ok(load) => load.snapshot.quick_css,
        Err(_) => {
            return QuickCssReport {
                enabled: false,
                css: None,
                css_bytes: 0,
                state: "settings_unavailable",
                read_ms: read_ms(started_at),
            };
        }
    };
    if !enabled {
        return QuickCssReport {
            enabled: false,
            css: None,
            css_bytes: 0,
            state: "disabled",
            read_ms: read_ms(started_at),
        };
    }

    let path = quick_css_path(profile_dir);
    if !path.exists() {
        // Electron parity: initQuickCss creates the missing file empty.
        if std::fs::write(&path, "").is_err() {
            return QuickCssReport {
                enabled: true,
                css: None,
                css_bytes: 0,
                state: "create_failed",
                read_ms: read_ms(started_at),
            };
        }
        return QuickCssReport {
            enabled: true,
            css: Some(String::new()),
            css_bytes: 0,
            state: "created_empty",
            read_ms: read_ms(started_at),
        };
    }

    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return QuickCssReport {
                enabled: true,
                css: None,
                css_bytes: 0,
                state: "unreadable",
                read_ms: read_ms(started_at),
            };
        }
    };
    if bytes.len() > MAX_QUICK_CSS_BYTES {
        return QuickCssReport {
            enabled: true,
            css: None,
            css_bytes: 0,
            state: "too_large",
            read_ms: read_ms(started_at),
        };
    }
    let css = match String::from_utf8(bytes) {
        Ok(css) => css,
        Err(_) => {
            return QuickCssReport {
                enabled: true,
                css: None,
                css_bytes: 0,
                state: "invalid_utf8",
                read_ms: read_ms(started_at),
            };
        }
    };
    if css.is_empty() {
        return QuickCssReport {
            enabled: true,
            css: Some(css),
            css_bytes: 0,
            state: "injected_empty",
            read_ms: read_ms(started_at),
        };
    }
    let css_bytes = css.len();
    QuickCssReport {
        enabled: true,
        css: Some(css),
        css_bytes,
        state: "injected",
        read_ms: read_ms(started_at),
    }
}

#[cfg(test)]
mod tests {
    use super::{load_quick_css, quick_css_path, QUICK_CSS_FILE, QUICK_CSS_STYLE_ID};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_profile() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must advance")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "legcord-native-quickcss-test-{}-{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(dir.join("storage")).expect("must create temp profile");
        // Authoritative settings with Quick CSS enabled (Electron default).
        fs::write(
            dir.join("storage").join("settings.json"),
            r#"{"quickCss":true}"#,
        )
        .expect("must seed settings");
        dir
    }

    fn set_enabled(profile: &Path, enabled: bool) {
        fs::write(
            profile.join("storage").join("settings.json"),
            format!(r#"{{"quickCss":{enabled}}}"#),
        )
        .expect("must seed settings");
    }

    #[test]
    fn style_identity_preserves_the_electron_id_without_colliding() {
        assert_eq!(QUICK_CSS_STYLE_ID, "legcord-quick-css");
        assert_eq!(QUICK_CSS_FILE, "quickCss.css");
        // No native managed style uses this identity.
        assert_ne!(QUICK_CSS_STYLE_ID, "legcord-native-vencord-css");
        assert_ne!(QUICK_CSS_STYLE_ID, "legcord-native-prototype-css");
        assert!(!QUICK_CSS_STYLE_ID.starts_with("legcord-native-theme-"));
    }

    #[test]
    fn resource_path_is_host_built_and_fixed() {
        let profile = temp_profile();
        assert_eq!(quick_css_path(&profile), profile.join("quickCss.css"));
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn enabled_file_is_loaded_with_bytes() {
        let profile = temp_profile();
        fs::write(quick_css_path(&profile), ":root{--q:1;}").expect("fixture");
        let report = load_quick_css(&profile);
        assert!(report.enabled);
        assert_eq!(report.css.as_deref(), Some(":root{--q:1;}"));
        assert_eq!(report.css_bytes, ":root{--q:1;}".len());
        assert_eq!(report.state, "injected");
        assert!(report.read_ms >= 0.0);
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn disabled_flag_injects_nothing() {
        let profile = temp_profile();
        set_enabled(&profile, false);
        fs::write(quick_css_path(&profile), ":root{--q:1;}").expect("fixture");
        let report = load_quick_css(&profile);
        assert!(!report.enabled);
        assert!(report.css.is_none());
        assert_eq!(report.state, "disabled");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn missing_file_is_created_empty_like_electron() {
        let profile = temp_profile();
        assert!(!quick_css_path(&profile).exists());
        let report = load_quick_css(&profile);
        assert!(report.enabled);
        assert_eq!(report.css.as_deref(), Some(""));
        assert_eq!(report.state, "created_empty");
        // Electron parity: the host created the file.
        assert_eq!(fs::read(quick_css_path(&profile)).unwrap(), b"");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn empty_file_pushes_empty_css_like_electron() {
        let profile = temp_profile();
        fs::write(quick_css_path(&profile), "").expect("fixture");
        let report = load_quick_css(&profile);
        assert!(report.enabled);
        assert_eq!(report.css.as_deref(), Some(""));
        assert_eq!(report.state, "injected_empty");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn oversized_file_is_rejected_before_injection() {
        let profile = temp_profile();
        let big = "a".repeat(super::MAX_QUICK_CSS_BYTES + 1);
        fs::write(quick_css_path(&profile), big).expect("fixture");
        let report = load_quick_css(&profile);
        assert!(report.enabled);
        assert!(report.css.is_none());
        assert_eq!(report.state, "too_large");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let profile = temp_profile();
        fs::write(quick_css_path(&profile), [0x66, 0xFF, 0x66]).expect("fixture");
        let report = load_quick_css(&profile);
        assert!(report.enabled);
        assert!(report.css.is_none());
        assert_eq!(report.state, "invalid_utf8");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn directory_at_the_resource_path_is_unreadable() {
        let profile = temp_profile();
        fs::create_dir_all(quick_css_path(&profile)).expect("fixture dir");
        let report = load_quick_css(&profile);
        assert!(report.enabled);
        assert!(report.css.is_none());
        assert_eq!(report.state, "unreadable");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn corrupt_settings_fail_closed_to_disabled() {
        let profile = temp_profile();
        fs::write(profile.join("storage").join("settings.json"), "not json{")
            .expect("corrupt settings");
        fs::write(quick_css_path(&profile), ":root{--q:1;}").expect("fixture");
        let report = load_quick_css(&profile);
        // Electron parity: a corrupt cache reads falsy, so Quick CSS stays off.
        assert!(!report.enabled);
        assert!(report.css.is_none());
        assert_eq!(report.state, "settings_unavailable");
        fs::remove_dir_all(&profile).ok();
    }
}

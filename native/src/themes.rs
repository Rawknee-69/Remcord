//! Host-controlled native theme injection for Phase D.
//!
//! This module is the native counterpart of the Electron theme pipeline in
//! `src/common/themes.ts` (`buildThemeCssCache`, `injectThemesMain`,
//! `setThemeEnabled`), whose actual lifecycle was traced before any native
//! code was written:
//!
//! ```text
//! startup  -> discover <userData>/themes/<id>/ -> read manifest.json ->
//!             resolve manifest.theme -> cache CSS -> did-finish-load ->
//!             webContents.send("addTheme", id, css) [Quick CSS first when
//!             enabled, then themes in discovery order] -> <style id> created
//! change   -> list-cache watcher only (500 ms debounce, manifest.json only;
//!             per-theme CSS edits need an enable toggle or a reload)
//! disable  -> webContents.send("removeTheme", id)
//! reload   -> fresh document; did-finish-load re-sends the whole cache
//! ```
//!
//! The renderer side (`src/discord/preload/themes.ts` over
//! `src/common/dom.ts::addTheme`) only ever receives `(id, css)` pushes; no
//! renderer caller reads theme CSS back, so this milestone is a **host push
//! only**. There is deliberately NO `themes.getCSS()` bridge, no new protocol
//! command, and no new boot capability: the host reads manifests/CSS itself
//! and injects CSS through the shared substrate builders
//! ([`crate::mod_substrate::managed_css_script`]).
//!
//! Native hardening (deliberate, documented differences from Electron):
//! - Theme ids are VALIDATED (`^[A-Za-z0-9._-]{1,64}$`), never joined raw.
//! - The manifest-provided CSS filename is validated by an allowlist: a bare
//!   `*.css` filename with no separators, no drive/UNC/device syntax, no
//!   reserved names. Electron joins `themesFolder + id + manifest.theme` with
//!   string interpolation and no filename validation, so a manifest such as
//!   `{ "theme": "../../quickCss.css" }` escapes the theme directory there;
//!   native rejects it as `invalid_filename`.
//! - Containment is enforced twice: the validated filename cannot escape
//!   lexically (single path component), AND the resolved file is
//!   `canonicalize`d and must stay under the canonical theme root, so
//!   pre-planted symlinks/junctions cannot redirect outside the root.
//! - Size-bounded: manifests <= 64 KiB, CSS <= 1 MiB. Electron is unbounded.
//! - Injection order is deterministic (sorted by theme id). Electron uses
//!   filesystem discovery order.
//! - A missing `enabled` field defaults to enabled. Electron additionally
//!   consults a legacy `disabled.txt`; that legacy migration path is not
//!   reproduced natively.
//! - Empty CSS files are skipped (no visual effect; avoids DOM clutter).
//!   Electron would push an empty style.
//!
//! Out of scope for this milestone (deferred): live file watching (the first
//! milestone is static startup injection + reload lifecycle), theme
//! installation/editing, Quick CSS (a separate host-owned file with its own
//! lifecycle), and any renderer-facing theme API.
//!
//! Logical root owned by this module: exactly one —
//! `<profile>/themes/`. The renderer never receives the absolute path.

use std::path::{Path, PathBuf};
use std::time::Instant;

/// Maximum theme id length in UTF-8 bytes. Mirrors the plugin-storage bound;
/// real theme directory names are far shorter.
pub const MAX_THEME_ID_BYTES: usize = 64;
/// Maximum manifest file size in bytes. Manifests are small JSON documents.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Maximum manifest-provided CSS filename length in UTF-8 bytes.
pub const MAX_THEME_FILENAME_BYTES: usize = 128;
/// Maximum theme CSS payload in bytes in either direction. Matches the
/// plugin-storage file bound; comfortably covers real theme stylesheets.
pub const MAX_THEME_CSS_BYTES: usize = 1024 * 1024;
/// Byte prefix for every native-managed theme style element. The raw theme id
/// is never used as a DOM id so native styles cannot collide with Discord
/// node ids (Electron reuses the raw id; this namespacing is deliberate).
pub const THEME_STYLE_ID_PREFIX: &str = "legcord-native-theme-";

fn is_valid_theme_id_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
}

/// Validate a theme identifier. The id is used verbatim as a single directory
/// name under the host-controlled theme root, never transformed.
pub fn validate_theme_id(theme_id: &str) -> Result<(), String> {
    if theme_id.is_empty() {
        return Err("theme id must not be empty".to_owned());
    }
    if theme_id.len() > MAX_THEME_ID_BYTES {
        return Err("theme id exceeds the size limit".to_owned());
    }
    if !theme_id.bytes().all(is_valid_theme_id_char) {
        return Err("theme id uses characters outside [A-Za-z0-9._-]".to_owned());
    }
    if theme_id == "." || theme_id == ".." {
        return Err("theme id must not be '.' or '..'".to_owned());
    }
    Ok(())
}

fn is_reserved_device_stem(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    if upper.len() == 4 {
        let (head, tail) = upper.split_at(3);
        if (head == "COM" || head == "LPT")
            && matches!(tail, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        {
            return true;
        }
    }
    false
}

/// Validate the manifest-provided CSS filename.
///
/// The value must be a single bare `*.css` filename: no directory separators
/// of either flavor, no drive/UNC/device syntax, no reserved Windows names.
/// This is the allowlist Electron lacks: it joins the raw value onto the
/// theme directory, so traversal and absolute paths escape there.
pub fn validate_theme_css_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty() {
        return Err("theme CSS filename must not be empty".to_owned());
    }
    if filename.len() > MAX_THEME_FILENAME_BYTES {
        return Err("theme CSS filename exceeds the size limit".to_owned());
    }
    if filename.contains('\0') {
        return Err("theme CSS filename must not contain NUL".to_owned());
    }
    if filename.bytes().any(|byte| byte < 0x20) {
        return Err("theme CSS filename must not contain control characters".to_owned());
    }
    if filename.contains('/') || filename.contains('\\') {
        return Err("theme CSS filename must not contain a path separator".to_owned());
    }
    // `:` anywhere blocks drive letters (`C:...`), ADS streams (`a:b`), and
    // NTFS alternate syntax in one rule.
    if filename.contains(':') {
        return Err("theme CSS filename must not contain ':'".to_owned());
    }
    if filename.contains(['<', '>', '"', '|', '?', '*']) {
        return Err("theme CSS filename contains a reserved Windows character".to_owned());
    }
    if filename == "." || filename == ".." {
        return Err("theme CSS filename must not be '.' or '..'".to_owned());
    }
    // Windows strips trailing spaces/dots, which would alias distinct inputs
    // to one file; reject instead of normalizing.
    if filename.ends_with(' ') || filename.ends_with('.') {
        return Err("theme CSS filename must not end with a space or dot".to_owned());
    }
    let stem = filename.split('.').next().unwrap_or(filename);
    if is_reserved_device_stem(stem) {
        return Err("theme CSS filename uses a reserved device name".to_owned());
    }
    if !filename.to_ascii_lowercase().ends_with(".css") {
        return Err("theme CSS filename must end with '.css'".to_owned());
    }
    Ok(())
}

/// Host-controlled theme root: `<profile>/themes/`. The renderer never sees
/// this path.
pub fn themes_dir(profile_dir: &Path) -> PathBuf {
    profile_dir.join("themes")
}

/// Deterministic DOM id for one theme's managed style element.
pub fn theme_style_id(theme_id: &str) -> String {
    format!("{THEME_STYLE_ID_PREFIX}{theme_id}")
}

/// Minimal manifest shape the native host consumes. Unknown future keys are
/// ignored (forward compatibility, mirroring the settings snapshot); only
/// `theme` and `enabled` affect loading.
#[derive(Debug, Clone, serde::Deserialize)]
struct ThemeManifestFile {
    theme: String,
    #[serde(default)]
    enabled: Option<bool>,
}

/// One successfully loaded theme: host-chosen style identity plus CSS text.
/// No path crosses this boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedTheme {
    pub id: String,
    pub style_id: String,
    pub css: String,
    pub css_bytes: usize,
}

/// Per-theme skip reason for DEV metrics. One bad theme never blocks others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedTheme {
    pub id: String,
    pub reason: &'static str,
}

/// Aggregate report for one host-side theme load pass.
#[derive(Debug, Clone, Default)]
pub struct ThemeLoadReport {
    pub discovered: usize,
    pub enabled: usize,
    pub injected: Vec<LoadedTheme>,
    pub skipped: Vec<SkippedTheme>,
    pub css_bytes: usize,
    pub read_ms: f64,
}

fn canonical_contains(root_canonical: &Path, path: &Path) -> bool {
    path.starts_with(root_canonical)
}

/// Discover enabled themes under the host-controlled root and load their CSS.
///
/// Order: validate id, read + parse the bounded manifest, honor `enabled`,
/// validate the manifest-provided filename, confine the resolved file
/// (lexical by construction plus canonical containment), enforce the CSS size
/// bound, require UTF-8. Failures skip exactly one theme with a typed reason.
/// Emission order is sorted by theme id for deterministic cascade order.
pub fn load_enabled_themes(profile_dir: &Path) -> ThemeLoadReport {
    let started_at = Instant::now();
    let mut report = ThemeLoadReport::default();
    let root = themes_dir(profile_dir);
    if !root.exists() {
        report.read_ms = started_at.elapsed().as_secs_f64() * 1000.0;
        return report;
    }
    let root_canonical = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());

    let mut entries: Vec<String> = Vec::new();
    let listing = std::fs::read_dir(&root);
    let listing = match listing {
        Ok(listing) => listing,
        Err(_) => {
            report.read_ms = started_at.elapsed().as_secs_f64() * 1000.0;
            return report;
        }
    };
    for entry in listing.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        entries.push(name);
    }
    entries.sort();
    report.discovered = entries.len();

    for id in entries {
        if validate_theme_id(&id).is_err() {
            report.skipped.push(SkippedTheme {
                id,
                reason: "invalid_id",
            });
            continue;
        }
        let theme_dir = root.join(&id);
        let manifest_path = theme_dir.join("manifest.json");
        let manifest_len = std::fs::metadata(&manifest_path)
            .ok()
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len());
        let Some(len) = manifest_len else {
            report.skipped.push(SkippedTheme {
                id,
                reason: "missing_manifest",
            });
            continue;
        };
        if len > MAX_MANIFEST_BYTES as u64 {
            report.skipped.push(SkippedTheme {
                id,
                reason: "manifest_too_large",
            });
            continue;
        }
        let manifest_text = match std::fs::read_to_string(&manifest_path) {
            Ok(text) => text,
            Err(_) => {
                report.skipped.push(SkippedTheme {
                    id,
                    reason: "invalid_manifest",
                });
                continue;
            }
        };
        let manifest: ThemeManifestFile = match serde_json::from_str(&manifest_text) {
            Ok(manifest) => manifest,
            Err(_) => {
                report.skipped.push(SkippedTheme {
                    id,
                    reason: "invalid_manifest",
                });
                continue;
            }
        };
        if manifest.enabled == Some(false) {
            report.skipped.push(SkippedTheme {
                id,
                reason: "disabled",
            });
            continue;
        }
        report.enabled += 1;
        if validate_theme_css_filename(&manifest.theme).is_err() {
            report.skipped.push(SkippedTheme {
                id,
                reason: "invalid_filename",
            });
            continue;
        }
        let css_path = theme_dir.join(&manifest.theme);
        // The file must exist: a missing CSS file skips exactly this theme,
        // exactly like a missing entry in Electron's CSS cache build.
        let css_len = match std::fs::metadata(&css_path) {
            Ok(metadata) if metadata.is_file() => metadata.len(),
            Ok(_) => {
                report.skipped.push(SkippedTheme {
                    id,
                    reason: "missing_css",
                });
                continue;
            }
            Err(_) => {
                report.skipped.push(SkippedTheme {
                    id,
                    reason: "missing_css",
                });
                continue;
            }
        };
        if css_len > MAX_THEME_CSS_BYTES as u64 {
            report.skipped.push(SkippedTheme {
                id,
                reason: "css_too_large",
            });
            continue;
        }
        // Canonical containment: resolves symlinks/junctions on the host.
        // Both sides are canonicalized so the Windows `\\?\` prefix cannot
        // break the check. The read below goes through the canonical path,
        // mirroring the plugin-storage capability.
        let css_canonical = match std::fs::canonicalize(&css_path) {
            Ok(canonical) => canonical,
            Err(_) => {
                report.skipped.push(SkippedTheme {
                    id,
                    reason: "unreadable_css",
                });
                continue;
            }
        };
        if !canonical_contains(&root_canonical, &css_canonical) {
            report.skipped.push(SkippedTheme {
                id,
                reason: "outside_root",
            });
            continue;
        }
        let css = match std::fs::read_to_string(&css_canonical) {
            Ok(css) => css,
            Err(_) => {
                report.skipped.push(SkippedTheme {
                    id,
                    reason: "unreadable_css",
                });
                continue;
            }
        };
        if css.is_empty() {
            report.skipped.push(SkippedTheme {
                id,
                reason: "empty_css",
            });
            continue;
        }
        let css_bytes = css.len();
        report.css_bytes += css_bytes;
        report.injected.push(LoadedTheme {
            style_id: theme_style_id(&id),
            id,
            css,
            css_bytes,
        });
    }

    report.read_ms = started_at.elapsed().as_secs_f64() * 1000.0;
    report
}

#[cfg(test)]
mod tests {
    use super::{
        load_enabled_themes, theme_style_id, validate_theme_css_filename, validate_theme_id,
        MAX_THEME_CSS_BYTES, THEME_STYLE_ID_PREFIX,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_profile() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must advance")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "legcord-native-themes-test-{}-{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(dir.join("themes")).expect("must create temp theme root");
        dir
    }

    fn write_theme(profile: &Path, id: &str, manifest: &str, css: Option<(&str, &str)>) {
        let dir = profile.join("themes").join(id);
        fs::create_dir_all(&dir).expect("must create theme dir");
        fs::write(dir.join("manifest.json"), manifest).expect("must write manifest");
        if let Some((name, content)) = css {
            fs::write(dir.join(name), content).expect("must write css");
        }
    }

    #[test]
    fn accepts_valid_theme_ids_and_rejects_separators() {
        assert!(validate_theme_id("midnight-BD").is_ok());
        assert!(validate_theme_id("my.theme_v2").is_ok());
        assert!(validate_theme_id("").is_err());
        assert!(validate_theme_id(&"t".repeat(65)).is_err());
        assert!(validate_theme_id("../escape").is_err());
        assert!(validate_theme_id("a/b").is_err());
        assert!(validate_theme_id("a\\b").is_err());
        assert!(validate_theme_id("C:").is_err());
        assert!(validate_theme_id(".").is_err());
        assert!(validate_theme_id("..").is_err());
        assert!(validate_theme_id("has space").is_err());
    }

    #[test]
    fn manifest_filename_allowlist_blocks_escape_shapes() {
        assert!(validate_theme_css_filename("src.css").is_ok());
        assert!(validate_theme_css_filename("theme.CSS").is_ok());
        // The exact shape Electron would honor and escape with.
        assert!(validate_theme_css_filename("../../quickCss.css").is_err());
        assert!(validate_theme_css_filename("..\\..\\secret.css").is_err());
        assert!(validate_theme_css_filename("/absolute.css").is_err());
        assert!(validate_theme_css_filename("\\absolute.css").is_err());
        assert!(validate_theme_css_filename("C:\\secret.css").is_err());
        assert!(validate_theme_css_filename("C:/secret.css").is_err());
        assert!(validate_theme_css_filename("\\\\server\\share.css").is_err());
        assert!(validate_theme_css_filename("..").is_err());
        assert!(validate_theme_css_filename("style.js").is_err());
        assert!(validate_theme_css_filename("noextension").is_err());
        assert!(validate_theme_css_filename("a:b.css").is_err());
        assert!(validate_theme_css_filename("CON.css").is_err());
        assert!(validate_theme_css_filename("com1.css").is_err());
        assert!(validate_theme_css_filename("trailingdot.css.").is_err());
        assert!(validate_theme_css_filename("").is_err());
        assert!(validate_theme_css_filename(&"a".repeat(129)).is_err());
    }

    #[test]
    fn loads_enabled_themes_in_sorted_order_with_style_identity() {
        let profile = temp_profile();
        write_theme(
            &profile,
            "zulu",
            r#"{"name":"Z","theme":"src.css"}"#,
            Some(("src.css", ":root{--z:1;}")),
        );
        write_theme(
            &profile,
            "alpha",
            r#"{"name":"A","theme":"main.css","enabled":true}"#,
            Some(("main.css", ":root{--a:1;}")),
        );
        let report = load_enabled_themes(&profile);
        assert_eq!(report.discovered, 2);
        assert_eq!(report.enabled, 2);
        assert_eq!(report.injected.len(), 2);
        assert_eq!(report.injected[0].id, "alpha");
        assert_eq!(report.injected[1].id, "zulu");
        assert_eq!(
            report.injected[0].style_id,
            format!("{THEME_STYLE_ID_PREFIX}alpha")
        );
        assert_eq!(report.injected[0].css, ":root{--a:1;}");
        assert_eq!(
            report.css_bytes,
            ":root{--a:1;}".len() + ":root{--z:1;}".len()
        );
        assert!(report.skipped.is_empty());
        assert!(report.read_ms >= 0.0);
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn disabled_themes_are_skipped_without_css_access() {
        let profile = temp_profile();
        write_theme(
            &profile,
            "off",
            r#"{"name":"Off","theme":"src.css","enabled":false}"#,
            None,
        );
        let report = load_enabled_themes(&profile);
        assert_eq!(report.injected.len(), 0);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, "disabled");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn missing_css_and_malformed_manifests_skip_exactly_one_theme() {
        let profile = temp_profile();
        write_theme(
            &profile,
            "good",
            r#"{"name":"G","theme":"src.css"}"#,
            Some(("src.css", ":root{--g:1;}")),
        );
        write_theme(&profile, "nocss", r#"{"name":"N","theme":"src.css"}"#, None);
        write_theme(&profile, "broken", "not json{", Some(("src.css", "x")));
        let report = load_enabled_themes(&profile);
        assert_eq!(report.injected.len(), 1);
        assert_eq!(report.injected[0].id, "good");
        let reasons: Vec<&str> = report.skipped.iter().map(|entry| entry.reason).collect();
        assert!(reasons.contains(&"missing_css"));
        assert!(reasons.contains(&"invalid_manifest"));
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn traversal_filenames_never_reach_the_filesystem() {
        let profile = temp_profile();
        // Plant the escape target: Electron's raw join would read this file.
        fs::write(profile.join("planted.css"), ":root{--pwned:1;}").expect("plant target");
        write_theme(
            &profile,
            "evil",
            r#"{"name":"E","theme":"../planted.css"}"#,
            None,
        );
        let report = load_enabled_themes(&profile);
        assert!(report.injected.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, "invalid_filename");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn symlink_escape_is_blocked_by_canonical_containment() {
        let profile = temp_profile();
        let outside = profile.join("outside.css");
        fs::write(&outside, ":root{--o:1;}").expect("outside target");
        let link = profile.join("themes").join("linked.css");
        if std::os::windows::fs::symlink_file(&outside, &link).is_err() {
            // Hosts without link privileges cannot plant the escape; the
            // lexical allowlist plus canonical layer still hold.
            fs::remove_dir_all(&profile).ok();
            return;
        }
        write_theme(
            &profile,
            "linked",
            r#"{"name":"L","theme":"linked.css"}"#,
            None,
        );
        // Move the link into the theme dir (fixture wrote manifest already).
        let theme_link = profile.join("themes").join("linked").join("linked.css");
        fs::rename(&link, &theme_link).expect("move link into theme dir");
        let report = load_enabled_themes(&profile);
        assert!(report.injected.is_empty());
        assert!(report
            .skipped
            .iter()
            .any(|entry| entry.reason == "outside_root"));
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn oversized_css_is_rejected_before_injection() {
        let profile = temp_profile();
        let big = "a".repeat(MAX_THEME_CSS_BYTES + 1);
        write_theme(
            &profile,
            "big",
            r#"{"name":"B","theme":"src.css"}"#,
            Some(("src.css", &big)),
        );
        let report = load_enabled_themes(&profile);
        assert!(report.injected.is_empty());
        assert_eq!(report.skipped[0].reason, "css_too_large");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn empty_css_is_skipped() {
        let profile = temp_profile();
        write_theme(
            &profile,
            "empty",
            r#"{"name":"E","theme":"src.css"}"#,
            Some(("src.css", "")),
        );
        let report = load_enabled_themes(&profile);
        assert!(report.injected.is_empty());
        assert_eq!(report.skipped[0].reason, "empty_css");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn missing_theme_root_loads_nothing() {
        let profile = temp_profile();
        fs::remove_dir_all(profile.join("themes")).ok();
        let report = load_enabled_themes(&profile);
        assert_eq!(report.discovered, 0);
        assert!(report.injected.is_empty());
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn style_ids_are_namespaced_and_deterministic() {
        assert_eq!(
            theme_style_id("midnight"),
            format!("{THEME_STYLE_ID_PREFIX}midnight")
        );
        assert_ne!(theme_style_id("a"), "a");
    }
}

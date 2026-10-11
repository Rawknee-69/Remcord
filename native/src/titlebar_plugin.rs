//! Exact Remcord titlebar Shelter plugin artifact for the native shell.
//!
//! The artifact is authored in `src/shelter/titlebar/`, compiled by the
//! existing JS pipeline (`pnpm build:plugins` → `lune ci`), and embedded at
//! compile time by `native/build.rs` (which fails the build with an actionable
//! message when the artifact is missing or malformed). This module only
//! exposes the embedded text plus its content identity; it performs no
//! execution and offers no renderer-selectable path, URL, or plugin id. Only
//! the `"Remcord Titlebar Controller"` artifact may be supplied by the native
//! loader, under the `"remcord-titlebar"` id Electron pins in the Shelter
//! injector map (`src/discord/preload/mods/shelter.mts`).
//!
//! Identity is a host-computed SHA-256 over the exact embedded bytes. Any
//! byte change in the pipeline output yields a different identity.

include!(concat!(
    env!("OUT_DIR"),
    "/remcord_titlebar_plugin/titlebar_plugin.rs"
));

include!(concat!(
    env!("OUT_DIR"),
    "/remcord_titlebar_plugin/titlebar_css.rs"
));

/// Maximum accepted artifact size in bytes (mirrors the `build.rs` embed
/// limit so the runtime invariant cannot drift from the build invariant).
pub const MAX_TITLEBAR_PLUGIN_BYTES: usize = 256 * 1024;

/// Structural members the real titlebar plugin must surface. Must stay
/// identical to the `build.rs` marker set; both are asserted, so a pipeline
/// change dropping the controls fails loudly instead of shipping buttonless.
pub const TITLEBAR_PLUGIN_BOOT_MARKERS: &[&str] =
    &["remcordNavControls", "window-controls-container", "onLoad"];

/// The exact plugin name pinned by `build.rs` from `plugin.json`.
pub const TITLEBAR_PLUGIN_EXPECTED_NAME: &str = "Remcord Titlebar Controller";

/// Stylesheet keys embedded by `build.rs`, mirroring the per-OS selection in
/// `src/discord/preload/newTitlebar.mts` (`base` always, one of
/// `win`/`darwin`/`linux` by platform).
pub const TITLEBAR_CSS_KEYS: &[&str] = &["base", "win", "darwin", "linux"];

/// Returns the embedded stylesheet text for one key, or `None` for an
/// unknown key. Only the four build-pinned sheets exist; the renderer never
/// selects a path or URL.
pub fn css_for(key: &str) -> Option<&'static str> {
    match key {
        "base" => Some(TITLEBAR_CSS_BASE),
        "win" => Some(TITLEBAR_CSS_WIN),
        "darwin" => Some(TITLEBAR_CSS_DARWIN),
        "linux" => Some(TITLEBAR_CSS_LINUX),
        _ => None,
    }
}

/// Host-controlled handle to the embedded artifact. No renderer input can
/// influence which artifact this resolves to.
pub struct TitlebarPluginArtifact {
    /// Exact `plugin.js` text.
    pub js: &'static str,
    /// Lowercase hex SHA-256 over the exact embedded bytes.
    pub sha256: &'static str,
    /// Byte length of the embedded text.
    pub bytes: usize,
    /// Plugin name from `plugin.json`.
    pub name: &'static str,
}

/// Returns the embedded titlebar plugin artifact.
pub fn artifact() -> TitlebarPluginArtifact {
    TitlebarPluginArtifact {
        js: TITLEBAR_PLUGIN_JS,
        sha256: TITLEBAR_PLUGIN_SHA256,
        bytes: TITLEBAR_PLUGIN_BYTES,
        name: TITLEBAR_PLUGIN_NAME,
    }
}

/// Validates a candidate artifact with the same rules `build.rs` enforces, so
/// the runtime invariant is testable without re-running the build.
pub fn validate_artifact(js: &str, name: &str) -> Result<(), String> {
    if js.is_empty() {
        return Err("titlebar plugin artifact must not be empty".to_owned());
    }
    if js.len() > MAX_TITLEBAR_PLUGIN_BYTES {
        return Err("titlebar plugin artifact exceeds the size limit".to_owned());
    }
    for marker in TITLEBAR_PLUGIN_BOOT_MARKERS {
        if !js.contains(marker) {
            return Err(format!(
                "titlebar plugin artifact is missing the structural member `{marker}`"
            ));
        }
    }
    if name != TITLEBAR_PLUGIN_EXPECTED_NAME {
        return Err(format!(
            "unexpected titlebar plugin name {name:?}; refusing a foreign artifact"
        ));
    }
    Ok(())
}

/// Validates that `sha256` is a well-formed lowercase hex digest.
pub fn validate_sha256(sha256: &str) -> Result<(), String> {
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("titlebar plugin identity must be 64 lowercase hex characters".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{artifact, validate_artifact, validate_sha256, TITLEBAR_PLUGIN_BOOT_MARKERS};

    #[test]
    fn embedded_artifact_passes_validation() {
        let embedded = artifact();
        validate_artifact(embedded.js, embedded.name)
            .expect("the embedded titlebar plugin artifact must validate");
        validate_sha256(embedded.sha256).expect("the embedded identity must be well-formed");
        assert_eq!(
            embedded.bytes,
            embedded.js.len(),
            "the recorded byte length must match the embedded text"
        );
        assert!(
            embedded.bytes > 1_000,
            "the titlebar plugin artifact must be the full compiled bundle, not a stub"
        );
    }

    #[test]
    fn empty_artifact_is_rejected() {
        assert!(validate_artifact("", "Remcord Titlebar Controller").is_err());
    }

    #[test]
    fn oversized_artifact_is_rejected() {
        let oversized = "x".repeat(super::MAX_TITLEBAR_PLUGIN_BYTES + 1);
        assert!(validate_artifact(&oversized, "Remcord Titlebar Controller").is_err());
    }

    #[test]
    fn artifact_missing_a_boot_marker_is_rejected() {
        for marker in TITLEBAR_PLUGIN_BOOT_MARKERS {
            let mut candidate = String::from("remcordNavControls window-controls-container onLoad");
            candidate = candidate.replace(marker, "renamed");
            assert!(
                validate_artifact(&candidate, "Remcord Titlebar Controller").is_err(),
                "removing `{marker}` must be rejected"
            );
        }
    }

    #[test]
    fn foreign_plugin_name_is_rejected() {
        let embedded = artifact();
        assert!(validate_artifact(embedded.js, "Remcord Settings").is_err());
    }

    #[test]
    fn malformed_identity_is_rejected() {
        assert!(validate_sha256("").is_err());
        assert!(validate_sha256("0".repeat(64).as_str()).is_ok());
        assert!(validate_sha256("0".repeat(63).as_str()).is_err());
        assert!(validate_sha256("G".repeat(64).as_str()).is_err());
        assert!(validate_sha256("A".repeat(64).as_str()).is_err());
    }

    #[test]
    fn embedded_stylesheets_cover_every_platform_key() {
        for key in super::TITLEBAR_CSS_KEYS {
            let css = super::css_for(key).expect("every platform key must resolve");
            assert!(!css.is_empty(), "{key} stylesheet must not be empty");
            assert!(
                css.contains("window-controls-container"),
                "{key} stylesheet must carry the controls rules"
            );
        }
        assert!(super::css_for("macos").is_none());
        assert!(super::css_for("../escape").is_none());
        assert!(super::css_for("").is_none());
    }
}

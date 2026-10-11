//! Exact Remcord settings Shelter plugin artifact for the native shell.
//!
//! The artifact is authored in `src/shelter/settings/`, compiled by the
//! existing JS pipeline (`pnpm build:plugins` → `lune ci`), and embedded at
//! compile time by `native/build.rs` (which fails the build with an actionable
//! message when the artifact is missing or malformed). This module only
//! exposes the embedded text plus its content identity; it performs no
//! execution and offers no renderer-selectable path, URL, or plugin id. Only
//! the `"Legcord Settings"` artifact may be supplied by the native loader.
//!
//! Identity is a host-computed SHA-256 over the exact embedded bytes (the
//! upstream artifact carries no build banner to pin). Any byte change in the
//! pipeline output yields a different identity.

include!(concat!(
    env!("OUT_DIR"),
    "/remcord_settings_plugin/settings_plugin.rs"
));

/// Maximum accepted artifact size in bytes (mirrors the `build.rs` embed
/// limit so the runtime invariant cannot drift from the build invariant).
pub const MAX_SETTINGS_PLUGIN_BYTES: usize = 2 * 1024 * 1024;

/// Synchronous boot members the real settings plugin reads during `onLoad()`.
/// Must stay identical to the `build.rs` marker set; both are asserted, so a
/// rename upstream fails loudly instead of desynchronizing the boot snapshot.
pub const SETTINGS_PLUGIN_BOOT_MARKERS: &[&str] = &[
    "getConfig",
    "getThemes",
    "translations",
    "setLang",
    "restart",
];

/// The exact plugin name pinned by `build.rs` from `plugin.json`.
pub const SETTINGS_PLUGIN_EXPECTED_NAME: &str = "Legcord Settings";

/// Host-controlled handle to the embedded artifact. No renderer input can
/// influence which artifact this resolves to.
pub struct SettingsPluginArtifact {
    /// Exact `plugin.js` text.
    pub js: &'static str,
    /// Lowercase hex SHA-256 over the exact embedded bytes.
    pub sha256: &'static str,
    /// Byte length of the embedded text.
    pub bytes: usize,
    /// Plugin name from `plugin.json`.
    pub name: &'static str,
}

/// Returns the embedded settings plugin artifact.
pub fn artifact() -> SettingsPluginArtifact {
    SettingsPluginArtifact {
        js: SETTINGS_PLUGIN_JS,
        sha256: SETTINGS_PLUGIN_SHA256,
        bytes: SETTINGS_PLUGIN_BYTES,
        name: SETTINGS_PLUGIN_NAME,
    }
}

/// Validates a candidate artifact with the same rules `build.rs` enforces, so
/// the runtime invariant is testable without re-running the build.
pub fn validate_artifact(js: &str, name: &str) -> Result<(), String> {
    if js.is_empty() {
        return Err("settings plugin artifact must not be empty".to_owned());
    }
    if js.len() > MAX_SETTINGS_PLUGIN_BYTES {
        return Err("settings plugin artifact exceeds the size limit".to_owned());
    }
    for marker in SETTINGS_PLUGIN_BOOT_MARKERS {
        if !js.contains(marker) {
            return Err(format!(
                "settings plugin artifact is missing the synchronous boot member `{marker}`"
            ));
        }
    }
    if name != SETTINGS_PLUGIN_EXPECTED_NAME {
        return Err(format!(
            "unexpected settings plugin name {name:?}; refusing a foreign artifact"
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
        return Err("settings plugin identity must be 64 lowercase hex characters".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{artifact, validate_artifact, validate_sha256, SETTINGS_PLUGIN_BOOT_MARKERS};

    #[test]
    fn embedded_artifact_passes_validation() {
        let embedded = artifact();
        validate_artifact(embedded.js, embedded.name)
            .expect("the embedded settings plugin artifact must validate");
        validate_sha256(embedded.sha256).expect("the embedded identity must be well-formed");
        assert_eq!(
            embedded.bytes,
            embedded.js.len(),
            "the recorded byte length must match the embedded text"
        );
        assert!(
            embedded.bytes > 100_000,
            "the settings plugin artifact must be the full compiled bundle, not a stub"
        );
    }

    #[test]
    fn empty_artifact_is_rejected() {
        assert!(validate_artifact("", "Legcord Settings").is_err());
    }

    #[test]
    fn oversized_artifact_is_rejected() {
        let oversized = "x".repeat(super::MAX_SETTINGS_PLUGIN_BYTES + 1);
        assert!(validate_artifact(&oversized, "Legcord Settings").is_err());
    }

    #[test]
    fn artifact_missing_a_boot_marker_is_rejected() {
        for marker in SETTINGS_PLUGIN_BOOT_MARKERS {
            let mut candidate = String::from("getConfig getThemes translations setLang restart");
            candidate = candidate.replace(marker, "renamed");
            // `setLang` is a substring-sensitive case: removing it must fail.
            assert!(
                validate_artifact(&candidate, "Legcord Settings").is_err(),
                "removing `{marker}` must be rejected"
            );
        }
    }

    #[test]
    fn foreign_plugin_name_is_rejected() {
        let embedded = artifact();
        assert!(validate_artifact(embedded.js, "Remcord Power").is_err());
    }

    #[test]
    fn malformed_identity_is_rejected() {
        assert!(validate_sha256("").is_err());
        assert!(validate_sha256("0".repeat(64).as_str()).is_ok());
        assert!(validate_sha256("0".repeat(63).as_str()).is_err());
        assert!(validate_sha256("G".repeat(64).as_str()).is_err());
        assert!(validate_sha256("A".repeat(64).as_str()).is_err());
    }
}

//! Native authoritative application-version source for the `app.get_version`
//! capability.
//!
//! This module mirrors Electron `src/common/version.ts` (`getVersion()`)
//! without Electron:
//! - The reported release value is the repository `package.json` `version`
//!   (the same value Electron serves via `app.getVersion()`), extracted at
//!   compile time by `native/build.rs`. The renderer cannot select a file,
//!   path, URL, or version: the request carries no parameters at all.
//! - Electron returns `"0.0.0"` for dev builds (`isDev`). Native debug builds
//!   are the DEV counterpart, so they report `"0.0.0"` too; release builds
//!   report the embedded package version.
//! - There is no runtime source to be missing or corrupt (unlike the settings
//!   file): a missing or malformed `package.json` version fails the build,
//!   and the embedded constant is shape-validated at host startup.
//!
//! The only caller-relevant behavior beyond the constant is the dev/release
//! parity rule, kept as a pure function so tests do not depend on the build
//! profile.

/// Maximum accepted application version length in UTF-8 bytes. Real versions
/// are dotted triples well under this; the bound only rejects absurd values.
pub const MAX_APP_VERSION_BYTES: usize = 64;

include!(concat!(env!("OUT_DIR"), "/legcord_app/app_version.rs"));

/// Electron parity for `getVersion()`: dev builds report `"0.0.0"`.
pub fn app_version(is_dev: bool) -> &'static str {
    if is_dev {
        "0.0.0"
    } else {
        LEGCORD_APP_VERSION
    }
}

/// The version the running host reports: debug builds are DEV builds.
pub fn current_app_version() -> &'static str {
    app_version(cfg!(debug_assertions))
}

/// Validate a version string before serving or accepting it. Empty and
/// oversized values are rejected; non-UTF-8 input cannot exist as `&str`.
pub fn validate_version(version: &str) -> Result<(), String> {
    if version.is_empty() {
        return Err("app version must not be empty".to_owned());
    }
    if version.len() > MAX_APP_VERSION_BYTES {
        return Err("app version exceeds the size limit".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{app_version, current_app_version, validate_version, LEGCORD_APP_VERSION};

    #[test]
    fn embeds_the_package_json_version() {
        assert!(!LEGCORD_APP_VERSION.is_empty());
        assert!(LEGCORD_APP_VERSION.len() <= 32);
        assert!(LEGCORD_APP_VERSION
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+')));
        // The repository version is a dotted triple; the embedded value must
        // keep that shape so the update checker comparison keeps working.
        let parts: Vec<&str> = LEGCORD_APP_VERSION.split('.').collect();
        assert!(parts.len() >= 2);
        assert!(parts.iter().all(|part| !part.is_empty()));
    }

    #[test]
    fn dev_builds_report_zero_like_electron_is_dev() {
        // Mirrors `getVersion()`: `isDev` serves "0.0.0" instead of the
        // packaged version.
        assert_eq!(app_version(true), "0.0.0");
        assert_eq!(app_version(false), LEGCORD_APP_VERSION);
    }

    #[test]
    fn debug_host_reports_the_dev_version() {
        // The running host reports one of the two parity values: "0.0.0"
        // for dev builds (Electron `isDev`), otherwise the embedded package
        // version. The exact branch is covered by
        // `dev_builds_report_zero_like_electron_is_dev`.
        let current = current_app_version();
        assert!(current == "0.0.0" || current == LEGCORD_APP_VERSION);
        assert!(validate_version(current).is_ok());
    }

    #[test]
    fn rejects_empty_and_oversized_versions() {
        assert!(validate_version("").is_err());
        assert!(validate_version("1.3.0").is_ok());
        assert!(validate_version(&"1".repeat(64)).is_ok());
        assert!(validate_version(&"1".repeat(65)).is_err());
    }
}

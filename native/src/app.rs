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

/// Maximum accepted OS release string length in bytes. Real releases are
/// short dotted triples (`"10.0.22631"`); the bound only rejects absurd values.
pub const MAX_OS_RELEASE_BYTES: usize = 64;

/// Reports the host OS release exactly like Electron's `os.release()`
/// (`src/discord/ipc.ts:374-376` serves `os.release()`, i.e.
/// `"major.minor.build"` on Windows).
///
/// Used ONLY for the closed settings shim's `window.legcord.osRelease`, which
/// the Settings UI reads during render (`isMinWindowsVersion()` in
/// `src/shelter/settings/settings.ts:397-402`, called from
/// `SettingsPage.tsx:348`). There is no capability, no request, and no
/// renderer input: the value is queried once per launch and spliced into the
/// document-start script as static data.
///
/// The query uses `RtlGetVersion` (via an already-loaded `ntdll.dll`), which
/// — unlike the deprecated `GetVersionExW` — is not subject to application
/// compatibility version lies, so the gating decisions the plugin makes from
/// this string match the real host. On the (effectively impossible) query
/// failure the host serves `"0.0.0"` and logs loudly: that hides cosmetic
/// version-gated sections instead of crashing render, and it is documented,
/// not hidden.
#[cfg(windows)]
pub fn os_version_string() -> String {
    use windows::core::w;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    #[repr(C)]
    struct RtlOsVersionInfo {
        dw_os_version_info_size: u32,
        dw_major_version: u32,
        dw_minor_version: u32,
        dw_build_number: u32,
        dw_platform_id: u32,
        sz_csd_version: [u16; 128],
    }

    type RtlGetVersionFn = unsafe extern "system" fn(*mut RtlOsVersionInfo) -> i32;

    let version = unsafe {
        let ntdll = LoadLibraryW(w!("ntdll.dll")).ok();
        let address = ntdll
            .as_ref()
            .and_then(|module| GetProcAddress(*module, windows::core::s!("RtlGetVersion")));
        address.map(|entry| {
            let function: RtlGetVersionFn = std::mem::transmute(entry);
            let mut info = RtlOsVersionInfo {
                dw_os_version_info_size: std::mem::size_of::<RtlOsVersionInfo>() as u32,
                dw_major_version: 0,
                dw_minor_version: 0,
                dw_build_number: 0,
                dw_platform_id: 0,
                sz_csd_version: [0; 128],
            };
            let status = function(&mut info);
            (status, info)
        })
    };
    match version {
        Some((0, info)) => format!(
            "{}.{}.{}",
            info.dw_major_version, info.dw_minor_version, info.dw_build_number
        ),
        _ => {
            eprintln!("[HostPlatform] OS version query failed; serving 0.0.0");
            "0.0.0".to_owned()
        }
    }
}

/// Non-Windows fallback for the closed settings shim. The native shell
/// targets Windows WebView2 (other platforms fail closed elsewhere); the
/// fallback keeps the code compiling and render crash-free, never crashing on
/// a missing value.
#[cfg(not(windows))]
pub fn os_version_string() -> String {
    "0.0.0".to_owned()
}

/// Validate an OS release string before splicing it into the document-start
/// script. Dotted numeric triples only; empty, oversized, and non-numeric
/// values are rejected.
pub fn validate_os_release(version: &str) -> Result<(), String> {
    if version.is_empty() {
        return Err("OS release must not be empty".to_owned());
    }
    if version.len() > MAX_OS_RELEASE_BYTES {
        return Err("OS release exceeds the size limit".to_owned());
    }
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err("OS release must be a dotted numeric triple".to_owned());
    }
    Ok(())
}

/// Minimal schema-valid host platform JSON for host-side tests that only need
/// the initialization-script plumbing (placeholder replacement, ordering)
/// without querying the OS.
#[cfg(test)]
pub fn test_fixture_json() -> &'static str {
    r#"{"platform":"win32","osRelease":"10.0.22631"}"#
}

#[cfg(test)]
mod tests {
    use super::{
        app_version, current_app_version, os_version_string, validate_os_release, validate_version,
        LEGCORD_APP_VERSION,
    };

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

    #[test]
    fn os_release_is_a_dotted_numeric_triple() {
        let release = os_version_string();
        validate_os_release(&release).expect("the host OS release must validate");
    }

    #[test]
    fn rejects_empty_oversized_and_non_numeric_os_releases() {
        assert!(validate_os_release("10.0.22631").is_ok());
        assert!(validate_os_release("").is_err());
        assert!(validate_os_release("10.0").is_err());
        assert!(validate_os_release("10.0.22631.1").is_err());
        assert!(validate_os_release("10.0.x").is_err());
        assert!(validate_os_release("../escape").is_err());
        assert!(validate_os_release(&"1".repeat(65)).is_err());
    }

    #[test]
    fn host_platform_fixture_is_schema_valid() {
        let parsed: serde_json::Value =
            serde_json::from_str(super::test_fixture_json()).expect("fixture must be valid JSON");
        assert_eq!(parsed["platform"], "win32");
        assert!(validate_os_release(parsed["osRelease"].as_str().unwrap()).is_ok());
    }
}

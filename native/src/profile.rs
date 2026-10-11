//! Native profile-directory resolution (release blocker 2).
//!
//! Three disjoint configurations, selected by build profile plus one
//! explicit host-side override:
//!
//! - Debug/DEV: the isolated DEV profile (`DEV_PROFILE_DIR`), preserving
//!   the established development behavior. Never the Electron profile.
//! - Release: `%LOCALAPPDATA%\Remcord` — a stable, writable,
//!   per-user, machine-local directory. Machine-local (not roaming) fits
//!   the WebView2 user-data caches living beneath it, and the leaf
//!   (`Remcord`) can never alias the production Electron profile
//!   (roaming, different leaf and different root).
//! - Explicit override (`REMCORD_NATIVE_PROFILE_DIR`, absolute path only):
//!   honored in both builds for safe isolated testing (e.g. a temp
//!   directory). The renderer never supplies this: it is read once from
//!   the host process environment at startup.
//!
//! Failure policy: a missing/empty `LOCALAPPDATA` in release, a relative
//! or empty override, or an uncreatable directory is a hard startup error.
//! The resolver never falls back across products — in particular, release
//! never silently uses the DEV profile. Settings, plugin data, themes,
//! Quick CSS, bundles, backups, and downloads all live beneath the
//! resolved root; the WebView2 user-data directory is the root itself
//! (WebView2 creates its `EBWebView` child inside).
//!
//! Privacy: paths are host-side only. Resolution takes no renderer input,
//! and no error message echoes user content beyond the configured path.

use std::path::PathBuf;

/// The isolated DEV profile. Debug builds only; never a release fallback.
pub const DEV_PROFILE_DIR: &str = r"E:\Remcord.webview-dev-user-data";
/// Explicit host-side profile override (absolute path). Both builds.
pub const PROFILE_OVERRIDE_ENV: &str = "REMCORD_NATIVE_PROFILE_DIR";
/// Leaf of the release profile under `%LOCALAPPDATA%`. Distinct from
/// the production Electron profile by root and by leaf.
pub const RELEASE_PROFILE_LEAF: &str = "Remcord";

/// Which configuration a resolved profile came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileKind {
    /// Debug default: the isolated DEV profile.
    Dev,
    /// Release default: `%LOCALAPPDATA%\Remcord`.
    Release,
    /// Explicit absolute override via [`PROFILE_OVERRIDE_ENV`].
    Override,
}

impl ProfileKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProfileKind::Dev => "dev",
            ProfileKind::Release => "release",
            ProfileKind::Override => "override",
        }
    }
}

/// The resolved profile: its root directory plus how it was selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProfile {
    pub path: PathBuf,
    pub kind: ProfileKind,
}

/// Debug default: the isolated DEV profile, verbatim.
pub fn dev_default_dir() -> PathBuf {
    PathBuf::from(DEV_PROFILE_DIR)
}

/// Release default from one `%LOCALAPPDATA%` value. Pure: no environment
/// or filesystem access. Empty input is an explicit error, never a
/// fallback to another product's directory.
pub fn release_dir_from_localappdata(local_app_data: &str) -> Result<PathBuf, String> {
    if local_app_data.is_empty() {
        return Err("release profile requires LOCALAPPDATA to be set".to_owned());
    }
    Ok(PathBuf::from(local_app_data).join(RELEASE_PROFILE_LEAF))
}

/// Validate one explicit override value. Absolute paths only: a relative
/// value would resolve against an unpredictable working directory, and an
/// empty one is not a selection. Pure: no filesystem access here; the
/// caller creates the directory and fails loudly if it cannot.
pub fn validated_override(raw: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw);
    if raw.is_empty() {
        return Err(format!("{PROFILE_OVERRIDE_ENV} must not be empty"));
    }
    if !path.is_absolute() {
        return Err(format!("{PROFILE_OVERRIDE_ENV} must be an absolute path"));
    }
    Ok(path)
}

/// Resolve the profile for one launch. `is_dev` is true for debug builds;
/// `override_raw` and `local_app_data` are the already-read host
/// environment values (`None` when unset). The override wins in both
/// builds; otherwise debug takes the DEV default and release requires
/// `LOCALAPPDATA`. Every failure is an `Err` — the caller fails startup
/// instead of redirecting to another product's data.
pub fn resolve_for_build(
    is_dev: bool,
    override_raw: Option<&str>,
    local_app_data: Option<&str>,
) -> Result<ResolvedProfile, String> {
    if let Some(raw) = override_raw {
        return Ok(ResolvedProfile {
            path: validated_override(raw)?,
            kind: ProfileKind::Override,
        });
    }
    if is_dev {
        return Ok(ResolvedProfile {
            path: dev_default_dir(),
            kind: ProfileKind::Dev,
        });
    }
    let root = local_app_data.unwrap_or("");
    Ok(ResolvedProfile {
        path: release_dir_from_localappdata(root)?,
        kind: ProfileKind::Release,
    })
}

/// Create the resolved root if missing. Only the required directories are
/// created (the root itself; feature subdirectories self-create on use).
/// A creation failure is returned — never masked by falling back.
pub fn ensure_profile_dir(path: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| {
        format!(
            "profile directory is not creatable ({}): {error}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_selects_the_isolated_dev_profile() {
        let resolved = resolve_for_build(true, None, None).expect("dev must resolve");
        assert_eq!(resolved.kind, ProfileKind::Dev);
        assert_eq!(resolved.path, PathBuf::from(DEV_PROFILE_DIR));
        assert_eq!(resolved.path, dev_default_dir());
    }

    #[test]
    fn release_selects_localappdata_remcord() {
        let overridden = resolve_for_build(false, Some(r"C:\Temp\native-override"), None)
            .expect("override wins");
        assert_eq!(overridden.kind, ProfileKind::Override);
        let resolved =
            resolve_for_build(false, None, Some(r"C:\Users\Test\AppData\Local")).expect("release");
        assert_eq!(resolved.kind, ProfileKind::Release);
        assert_eq!(
            resolved.path,
            PathBuf::from(r"C:\Users\Test\AppData\Local\Remcord")
        );
    }

    #[test]
    fn dev_and_release_never_alias() {
        let dev = resolve_for_build(true, None, None).expect("dev");
        for local in [
            r"C:\Users\Test\AppData\Local",
            r"E:\",
            r"C:\Users\Test\AppData\Roaming",
        ] {
            let release =
                resolve_for_build(false, None, Some(local)).expect("release must resolve");
            assert_ne!(dev.path, release.path, "LOCALAPPDATA={local}");
        }
        // Electron's roaming profile is a different root AND a different
        // leaf; the release path touches neither.
        let release =
            resolve_for_build(false, None, Some(r"C:\Users\Test\AppData\Local")).expect("release");
        assert_eq!(
            release.path.file_name().and_then(|n| n.to_str()),
            Some("Remcord")
        );
        assert!(!release
            .path
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case("roaming")));
    }

    #[test]
    fn resolution_is_stable_across_repeated_calls() {
        for _ in 0..3 {
            assert_eq!(
                resolve_for_build(true, None, None).expect("dev"),
                resolve_for_build(true, None, None).expect("dev")
            );
            assert_eq!(
                resolve_for_build(false, None, Some(r"C:\Users\T\AppData\Local")).expect("rel"),
                resolve_for_build(false, None, Some(r"C:\Users\T\AppData\Local")).expect("rel")
            );
        }
    }

    #[test]
    fn absolute_override_wins_in_both_builds() {
        for is_dev in [true, false] {
            let resolved = resolve_for_build(is_dev, Some(r"C:\Temp\native-probe"), None)
                .expect("absolute override");
            assert_eq!(resolved.kind, ProfileKind::Override);
            assert_eq!(resolved.path, PathBuf::from(r"C:\Temp\native-probe"));
        }
    }

    #[test]
    fn relative_or_empty_override_fails_without_fallback() {
        for (is_dev, raw) in [
            (true, "relative/dir"),
            (false, "relative/dir"),
            (true, ""),
            (false, ""),
        ] {
            let error = resolve_for_build(is_dev, Some(raw), Some(r"C:\Users\T\AppData\Local"))
                .expect_err("bad override must fail");
            assert!(
                error.contains(PROFILE_OVERRIDE_ENV),
                "error must name the variable: {error}"
            );
        }
    }

    #[test]
    fn release_without_localappdata_fails_and_never_falls_back_to_dev() {
        for local in [None, Some("")] {
            let error = resolve_for_build(false, None, local).expect_err("must fail");
            assert!(
                error.contains("LOCALAPPDATA"),
                "error must name the missing input: {error}"
            );
            assert!(
                !error.contains(DEV_PROFILE_DIR),
                "failure must not redirect toward the DEV profile: {error}"
            );
        }
    }

    #[test]
    fn ensure_creates_only_the_root_and_reports_failure() {
        let root = std::env::temp_dir().join(format!(
            "remcord-profile-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        let target = root.join("profile");
        ensure_profile_dir(&target).expect("creation must succeed");
        assert!(target.is_dir());
        // A file where the directory belongs fails loudly instead of
        // redirecting anywhere.
        let blocker = root.join("blocker");
        std::fs::write(&blocker, b"x").expect("fixture");
        assert!(ensure_profile_dir(&blocker).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn product_identity_matches_project_metadata() {
        // Binary name and crate version are the release identity; both must
        // match the product metadata (package.json), enforced at build time
        // in build.rs and pinned here against drift.
        assert_eq!(env!("CARGO_PKG_NAME"), "remcord");
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../../package.json"))
            .expect("package.json must parse");
        let product_version = manifest
            .get("version")
            .and_then(|v| v.as_str())
            .expect("package.json must define version");
        assert_eq!(env!("CARGO_PKG_VERSION"), product_version);
        assert_eq!(product_version, "1.3.0");
    }

    #[test]
    fn kinds_have_stable_names() {
        assert_eq!(ProfileKind::Dev.as_str(), "dev");
        assert_eq!(ProfileKind::Release.as_str(), "release");
        assert_eq!(ProfileKind::Override.as_str(), "override");
    }
}

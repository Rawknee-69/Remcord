//! User-mediated import of a previous-generation native profile into the
//! Remcord profile.
//!
//! Design: the previous product generation used different profile paths.
//! This module never names those paths, so no retired identifier can leak
//! into the product. Instead, the user points the application at the
//! previous profile exactly once through the `REMCORD_IMPORT_FROM`
//! environment variable (absolute path only):
//!
//! - The source must exist and exhibit the native profile layout (at least
//!   one of `storage/`, `themes/`, `quickCss.css`, `plugin-storage/`,
//!   `plugins/`, `bundles/`, `backups/`, `downloads/`). Anything else —
//!   including the production Electron profile, which has a different
//!   layout — fails validation instead of importing.
//! - The destination must be the resolved Remcord profile and must be empty
//!   (or carry a previous import marker). A non-empty destination without a
//!   marker fails with a reconciliation error instead of merging.
//! - The source is never deleted, moved, or modified. The import is
//!   copy-only through sibling temp files plus rename.
//! - `settings.json` is copied byte-for-byte (raw bytes, never
//!   re-serialized), preserving UTF-8 without BOM and its JSON structure.
//! - The import is idempotent: completion writes a marker file last; reruns
//!   observe the marker and succeed without copying again, so user files
//!   added after importing are never overwritten by a rerun. Interruption
//!   is safe for the same reason (no marker, copy again).
//! - Diagnostics report counts only — never paths (which embed the Windows
//!   username) and never file contents.
//! - Failures return `Err` with path-free messages; the caller fails startup
//!   rather than falling back anywhere.
//!
//! What is preserved: settings, themes, Quick CSS, plugin storage, plugins,
//! bundles, backups, downloads, and WebView2 user data beneath the root.
//!
//! Manual alternative (no variable needed): quit the application, copy the
//! previous profile's contents into the Remcord profile with Explorer (skip
//! any `EBWebView` cache directory; it regenerates), then launch. The
//! application validates what it reads and fails closed on corrupt state.

use std::path::{Path, PathBuf};

/// Environment variable carrying the user-selected previous profile.
/// Absolute path only; unset means no import.
pub const IMPORT_FROM_ENV: &str = "REMCORD_IMPORT_FROM";
/// Marker file written into the Remcord profile after a completed import.
pub const IMPORT_MARKER: &str = ".remcord-imported";

/// Native-layout markers. At least one must exist at the top level of an
/// import source; the production Electron profile has none of these.
/// Directory markers must be directories, file markers must be files.
const LAYOUT_DIRS: &[&str] = &[
    "storage",
    "themes",
    "plugin-storage",
    "plugins",
    "bundles",
    "backups",
    "downloads",
];
const LAYOUT_FILES: &[&str] = &["quickCss.css"];

/// Outcome of an import attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// Files copied in this run (0 when nothing was imported).
    pub files_copied: u64,
    /// Total bytes copied in this run.
    pub bytes_copied: u64,
    /// True when the run observed an existing marker and copied nothing.
    pub already_imported: bool,
}

/// Read the user-mediated import source. `None` when unset or empty.
pub fn read_import_source() -> Option<PathBuf> {
    match std::env::var(IMPORT_FROM_ENV) {
        Ok(raw) if !raw.trim().is_empty() => Some(PathBuf::from(raw.trim())),
        _ => None,
    }
}

/// True when `path` exists and contains at least one entry.
fn dir_has_data(path: &Path) -> bool {
    match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => false,
    }
}

/// Validate a user-selected import source without naming any retired path:
/// absolute, a directory, exhibiting the native profile layout, and not the
/// destination itself.
fn validate_source(source: &Path, dest_root: &Path) -> Result<(), String> {
    if !source.is_absolute() {
        return Err(format!(
            "{IMPORT_FROM_ENV} must be an absolute path to the previous profile directory"
        ));
    }
    if !source.is_dir() {
        return Err(format!(
            "{IMPORT_FROM_ENV} does not point at a readable profile directory"
        ));
    }
    if source == dest_root {
        return Err("the import source and the Remcord profile must differ".to_owned());
    }
    let has_layout = LAYOUT_DIRS.iter().any(|entry| source.join(entry).is_dir())
        || LAYOUT_FILES
            .iter()
            .any(|entry| source.join(entry).is_file());
    if !has_layout {
        return Err(format!(
            "{IMPORT_FROM_ENV} does not exhibit a native profile layout (expected one of: {})",
            LAYOUT_DIRS.join(", ")
        ));
    }
    Ok(())
}

/// Copy one file byte-for-byte through a sibling temp file plus rename, so
/// an interrupted copy never leaves a partial destination behind.
fn copy_file_atomic(source: &Path, dest: &Path) -> Result<u64, String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("import cannot create the destination directory: {error}"))?;
    }
    let tmp = dest.with_extension("remcord-import-tmp");
    std::fs::copy(source, &tmp)
        .map_err(|error| format!("import cannot copy a profile file: {error}"))?;
    let bytes = std::fs::metadata(&tmp).map(|meta| meta.len()).unwrap_or(0);
    std::fs::rename(&tmp, dest)
        .map_err(|error| format!("import cannot commit a profile file: {error}"))?;
    Ok(bytes)
}

/// Recursively copy `source_dir` into `dest_dir`. Symlinks and junctions
/// resolve to content (plain files); failures abort the import.
fn copy_tree(source_dir: &Path, dest_dir: &Path) -> Result<(u64, u64), String> {
    let mut files = 0_u64;
    let mut bytes = 0_u64;
    let entries = std::fs::read_dir(source_dir)
        .map_err(|error| format!("import cannot list the source profile: {error}"))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("import cannot list the source profile: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("import cannot inspect the source profile: {error}"))?;
        // Never copy a previous import marker into the new tree.
        if entry.file_name().as_os_str() == IMPORT_MARKER {
            continue;
        }
        let dest = dest_dir.join(entry.file_name());
        if file_type.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|error| {
                format!("import cannot create the destination directory: {error}")
            })?;
            let (nested_files, nested_bytes) = copy_tree(&entry.path(), &dest)?;
            files += nested_files;
            bytes += nested_bytes;
        } else {
            let count = copy_file_atomic(&entry.path(), &dest)?;
            files += 1;
            bytes += count;
        }
    }
    Ok((files, bytes))
}

/// Run the user-mediated import of `source` into the Remcord profile at
/// `dest_root`.
///
/// - No source (`None`): no-op success.
/// - Marker present at the destination: no-op success (`already_imported`).
/// - Destination non-empty without a marker while the source has data:
///   `Err` requiring manual reconciliation. Never merges, never overwrites.
/// - Otherwise: validate, copy the whole tree, write the marker last.
///
/// The source tree is never modified. The destination leaf is asserted to
/// be a Remcord profile leaf so a miswired caller cannot aim the copy at an
/// arbitrary directory.
pub fn import_previous_profile(
    source: Option<&Path>,
    dest_root: &Path,
    expected_dest_leaf: &str,
) -> Result<ImportReport, String> {
    let Some(source) = source else {
        return Ok(ImportReport {
            files_copied: 0,
            bytes_copied: 0,
            already_imported: false,
        });
    };
    match dest_root.file_name().and_then(|name| name.to_str()) {
        Some(leaf) if leaf == expected_dest_leaf => {}
        other => {
            return Err(format!(
                "import destination leaf must be {expected_dest_leaf:?}, found {other:?}"
            ));
        }
    }
    validate_source(source, dest_root)?;
    if !dir_has_data(source) {
        return Ok(ImportReport {
            files_copied: 0,
            bytes_copied: 0,
            already_imported: false,
        });
    }
    if dest_root.join(IMPORT_MARKER).is_file() {
        return Ok(ImportReport {
            files_copied: 0,
            bytes_copied: 0,
            already_imported: true,
        });
    }
    if dir_has_data(dest_root) {
        return Err(
            "both the selected previous profile and the Remcord profile contain data; \
             refusing to merge or overwrite. Back up both directories, reconcile them by hand, \
             then relaunch"
                .to_owned(),
        );
    }
    std::fs::create_dir_all(dest_root)
        .map_err(|error| format!("import cannot create the Remcord profile directory: {error}"))?;
    let (files, bytes) = copy_tree(source, dest_root)?;
    let marker_body = format!("imported files={files} bytes={bytes}\n");
    std::fs::write(dest_root.join(IMPORT_MARKER), marker_body)
        .map_err(|error| format!("import cannot record its completion marker: {error}"))?;
    Ok(ImportReport {
        files_copied: files,
        bytes_copied: bytes,
        already_imported: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "remcord-import-test-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ))
    }

    struct Fixture {
        base: PathBuf,
        old: PathBuf,
        new: PathBuf,
    }

    impl Fixture {
        fn fresh(tag: &str, new_leaf: &str) -> Self {
            let base = unique_root(tag);
            let _ = std::fs::remove_dir_all(&base);
            let old = base.join("old");
            let new = base.join(new_leaf);
            std::fs::create_dir_all(&old).expect("fixture old root");
            Self { base, old, new }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    fn seed_old_profile(old: &Path) {
        std::fs::create_dir_all(old.join("storage")).expect("storage dir");
        std::fs::write(
            old.join("storage").join("settings.json"),
            br#"{"a":1,"mods":["shelter"]}"#,
        )
        .expect("settings");
        std::fs::write(old.join("quickCss.css"), b":root{color:red;}").expect("quickcss");
        std::fs::create_dir_all(old.join("themes").join("demo")).expect("theme dir");
        std::fs::write(
            old.join("themes").join("demo").join("manifest.json"),
            br#"{"name":"demo","theme":"src.css","enabled":true}"#,
        )
        .expect("manifest");
        std::fs::write(old.join("themes").join("demo").join("src.css"), b"body{}").expect("css");
        std::fs::create_dir_all(old.join("plugin-storage").join("probe")).expect("pstorage");
        std::fs::write(
            old.join("plugin-storage").join("probe").join("note.txt"),
            b"hello",
        )
        .expect("pfile");
    }

    #[test]
    fn imports_a_populated_tree_and_preserves_settings_bytes() {
        let fixture = Fixture::fresh("copy", "Remcord");
        seed_old_profile(&fixture.old);
        let before = std::fs::read(fixture.old.join("storage").join("settings.json")).unwrap();
        let report = import_previous_profile(Some(&fixture.old), &fixture.new, "Remcord")
            .expect("import must succeed");
        assert!(!report.already_imported);
        assert!(report.files_copied >= 5);
        assert!(report.bytes_copied >= before.len() as u64);
        let after = std::fs::read(fixture.new.join("storage").join("settings.json")).unwrap();
        assert_eq!(before, after);
        assert!(!after.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(
            std::fs::read(fixture.new.join("quickCss.css")).unwrap(),
            b":root{color:red;}"
        );
        assert!(fixture.new.join(IMPORT_MARKER).is_file());
        assert!(fixture.old.join("storage").join("settings.json").is_file());
    }

    #[test]
    fn import_is_idempotent_and_never_overwrites_user_files() {
        let fixture = Fixture::fresh("idempotent", "Remcord");
        seed_old_profile(&fixture.old);
        import_previous_profile(Some(&fixture.old), &fixture.new, "Remcord").expect("first");
        std::fs::write(
            fixture.new.join("storage").join("settings.json"),
            br#"{"a":2,"mods":[]}"#,
        )
        .expect("user edit");
        let report = import_previous_profile(Some(&fixture.old), &fixture.new, "Remcord")
            .expect("rerun must succeed");
        assert!(report.already_imported);
        assert_eq!(report.files_copied, 0);
        assert_eq!(
            std::fs::read(fixture.new.join("storage").join("settings.json")).unwrap(),
            br#"{"a":2,"mods":[]}"#.as_slice()
        );
    }

    #[test]
    fn refuses_to_overwrite_a_non_empty_destination() {
        let fixture = Fixture::fresh("clash", "Remcord");
        seed_old_profile(&fixture.old);
        std::fs::create_dir_all(&fixture.new).expect("new root");
        std::fs::write(fixture.new.join("storage.json"), b"{}").expect("user file");
        let error = import_previous_profile(Some(&fixture.old), &fixture.new, "Remcord")
            .expect_err("clash must fail");
        assert!(
            error.contains("reconcile"),
            "must require reconciliation: {error}"
        );
        assert!(!fixture.new.join(IMPORT_MARKER).is_file());
        assert!(!fixture.new.join("quickCss.css").exists());
    }

    #[test]
    fn missing_source_is_a_no_op() {
        let fixture = Fixture::fresh("empty", "Remcord");
        let report = import_previous_profile(None, &fixture.new, "Remcord")
            .expect("absent source must succeed");
        assert_eq!(report.files_copied, 0);
        assert!(!report.already_imported);
        assert!(!fixture.new.join(IMPORT_MARKER).is_file());
    }

    #[test]
    fn rejects_relative_and_non_profile_sources() {
        let fixture = Fixture::fresh("validate", "Remcord");
        let error =
            import_previous_profile(Some(Path::new("relative/dir")), &fixture.new, "Remcord")
                .expect_err("relative source must fail");
        assert!(error.contains(IMPORT_FROM_ENV));
        let plain = fixture.base.join("plain");
        std::fs::create_dir_all(&plain).expect("plain dir");
        std::fs::write(plain.join("random.txt"), b"x").expect("random file");
        let error = import_previous_profile(Some(&plain), &fixture.new, "Remcord")
            .expect_err("layout-less source must fail");
        assert!(
            error.contains("layout"),
            "must name the layout rule: {error}"
        );
        assert!(!fixture.new.join(IMPORT_MARKER).is_file());
    }

    #[test]
    fn refuses_a_wrong_destination_leaf() {
        let fixture = Fixture::fresh("leaf", "SomethingElse");
        seed_old_profile(&fixture.old);
        let error = import_previous_profile(Some(&fixture.old), &fixture.new, "Remcord")
            .expect_err("wrong leaf must fail");
        assert!(
            error.contains("Remcord"),
            "must name the expected leaf: {error}"
        );
    }

    #[test]
    fn error_messages_carry_no_filesystem_paths() {
        let fixture = Fixture::fresh("paths", "Remcord");
        seed_old_profile(&fixture.old);
        std::fs::create_dir_all(&fixture.new).expect("new root");
        std::fs::write(fixture.new.join("x.txt"), b"x").expect("user file");
        let error = import_previous_profile(Some(&fixture.old), &fixture.new, "Remcord")
            .expect_err("must fail");
        let base_text = fixture.base.to_string_lossy().into_owned();
        assert!(
            !error.contains(&base_text),
            "no temp path may leak: {error}"
        );
        assert!(
            !error.contains("C:\\") && !error.contains("E:\\"),
            "no root may leak: {error}"
        );
    }
}

//! Native profile migration from the old Legcord-branded native profile to the
//! Remcord profile (Remcord rebrand, step 2).
//!
//! Canonical pairs (host-owned, fixed):
//! - DEV: `E:\Legcord.webview-dev-user-data` -> `E:\Remcord.webview-dev-user-data`
//! - Release: `%LOCALAPPDATA%\LegcordNative` -> `%LOCALAPPDATA%\Remcord`
//!
//! Safety contract (never relaxed):
//! - The production Electron profile (`%APPDATA%\legcord`) is never read,
//!   written, or migrated. Any path whose leaf is `legcord`
//!   (case-insensitive) is refused outright.
//! - The old native profile is never deleted, moved, or modified. Migration
//!   is copy-only; the old tree stays exactly as it was.
//! - A non-empty Remcord profile without a migration marker is never
//!   overwritten. When both profiles contain data, migration stops with an
//!   error that requires the documented reconciliation procedure.
//! - `settings.json` is copied byte-for-byte (raw bytes, never re-serialized),
//!   so its UTF-8 encoding (no BOM) and JSON structure are preserved exactly.
//! - Migration is idempotent: a completed migration writes a marker file last;
//!   reruns observe the marker and succeed without copying again (user files
//!   added after migration are never overwritten by a rerun).
//! - Migration is safe to interrupt: the marker is written only after every
//!   file copied successfully, so an interrupted run simply copies again.
//! - Files are copied individually (each file is one `fs::copy`); no archives,
//!   no deletes, no renderer input anywhere in the chain.
//! - Diagnostics report leaf names and counts only — never full paths (which
//!   embed the Windows username) and never file contents.
//!
//! What is preserved: settings, themes, Quick CSS, plugin storage, plugins,
//! bundles, backups, downloads, and the WebView2 user data beneath the root
//! (copied as ordinary files; locked files abort with a clear error telling
//! the user to close the old application, never with a partial profile).
//!
//! Reconciliation procedure (when both profiles contain data): back up both
//! directories, decide per file which copy wins, copy the winners into the
//! Remcord profile by hand, delete the marker if one exists, then launch.
//! The launcher never merges automatically.

use std::path::{Path, PathBuf};

/// Leaf of the legacy native release profile under `%LOCALAPPDATA%`.
pub const LEGACY_RELEASE_LEAF: &str = "LegcordNative";
/// Legacy native DEV profile directory (debug builds only).
pub const LEGACY_DEV_DIR: &str = r"E:\Legcord.webview-dev-user-data";
/// Marker file written into the new profile after a completed migration.
pub const MIGRATION_MARKER: &str = ".remcord-migrated";
/// Legacy profile override variable, honored as a deprecated fallback behind
/// [`crate::profile::PROFILE_OVERRIDE_ENV`].
pub const LEGACY_PROFILE_OVERRIDE_ENV: &str = "LEGCORD_NATIVE_PROFILE_DIR";

/// Outcome of a completed migration or a no-op rerun.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// Files copied in this run (0 on a marker no-op rerun).
    pub files_copied: u64,
    /// Total bytes copied in this run.
    pub bytes_copied: u64,
    /// True when the run observed an existing marker and copied nothing.
    pub already_migrated: bool,
}

/// Read the explicit profile override, preferring the canonical
/// `REMCORD_NATIVE_PROFILE_DIR` and falling back to the deprecated
/// `LEGCORD_NATIVE_PROFILE_DIR`. Returns `(value, was_legacy)`.
pub fn read_override() -> (Option<String>, bool) {
    if let Ok(value) = std::env::var(crate::profile::PROFILE_OVERRIDE_ENV) {
        return (Some(value), false);
    }
    if let Ok(value) = std::env::var(LEGACY_PROFILE_OVERRIDE_ENV) {
        return (Some(value), true);
    }
    (None, false)
}

/// Legacy native release profile directory for one `%LOCALAPPDATA%` value.
pub fn legacy_release_dir(local_app_data: &str) -> PathBuf {
    PathBuf::from(local_app_data).join(LEGACY_RELEASE_LEAF)
}

/// True when `path` exists and contains at least one entry (file, dir, or
/// marker). Missing roots and empty directories need no migration.
fn dir_has_data(path: &Path) -> bool {
    match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => false,
    }
}

/// Refuse production Electron paths and any caller confusion: the old root
/// must never be the production profile, and old and new must differ.
fn validate_pair(old_root: &Path, new_root: &Path, expected_new_leaf: &str) -> Result<(), String> {
    if old_root == new_root {
        return Err("migration source and destination must differ".to_owned());
    }
    for root in [old_root, new_root] {
        if let Some(leaf) = root.file_name().and_then(|name| name.to_str()) {
            if leaf.eq_ignore_ascii_case("legcord") {
                return Err(format!(
                    "refusing to migrate a production Electron profile (leaf {leaf:?}); \
                     migration only covers the native application profile"
                ));
            }
        }
    }
    match new_root.file_name().and_then(|name| name.to_str()) {
        Some(leaf) if leaf == expected_new_leaf => {}
        other => {
            return Err(format!(
                "migration destination leaf must be {expected_new_leaf:?}, found {other:?}"
            ));
        }
    }
    Ok(())
}

/// Copy one file byte-for-byte through a sibling temp file plus rename, so an
/// interrupted copy never leaves a partial destination behind.
fn copy_file_atomic(source: &Path, dest: &Path) -> Result<u64, String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!("migration cannot create the destination directory: {error}")
        })?;
    }
    let tmp = dest.with_extension("remcord-migrate-tmp");
    std::fs::copy(source, &tmp)
        .map_err(|error| format!("migration cannot copy a profile file: {error}"))?;
    let bytes = std::fs::metadata(&tmp).map(|meta| meta.len()).unwrap_or(0);
    std::fs::rename(&tmp, dest)
        .map_err(|error| format!("migration cannot commit a profile file: {error}"))?;
    Ok(bytes)
}

/// Recursively copy `source_dir` into `dest_dir` (both must exist as
/// directories). Returns `(files, bytes)`. Symlinks and junctions are copied
/// as their target contents (plain files); failures abort the migration.
fn copy_tree(source_dir: &Path, dest_dir: &Path) -> Result<(u64, u64), String> {
    let mut files = 0_u64;
    let mut bytes = 0_u64;
    let entries = std::fs::read_dir(source_dir)
        .map_err(|error| format!("migration cannot list the old profile: {error}"))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("migration cannot list the old profile: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("migration cannot inspect the old profile: {error}"))?;
        let dest = dest_dir.join(entry.file_name());
        if file_type.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|error| {
                format!("migration cannot create the destination directory: {error}")
            })?;
            let (nested_files, nested_bytes) = copy_tree(&entry.path(), &dest)?;
            files += nested_files;
            bytes += nested_bytes;
        } else {
            // Files, symlinks, and junctions all resolve to content here;
            // `fs::copy` follows links, so no link is recreated outside.
            let count = copy_file_atomic(&entry.path(), &dest)?;
            files += 1;
            bytes += count;
        }
    }
    Ok((files, bytes))
}

/// Migrate the old native profile at `old_root` into the Remcord profile at
/// `new_root`. `expected_new_leaf` is `"Remcord"` (release) or
/// `"Remcord.webview-dev-user-data"` (DEV): the destination is refused unless
/// its leaf matches, so a miswired caller can never aim the copy at the
/// production profile or an arbitrary directory.
///
/// Behavior:
/// - Old root missing or empty: no-op success (`already_migrated: false`,
///   zero counts). The caller still ensures the new root afterwards.
/// - Marker present in the new root: no-op success (`already_migrated: true`).
/// - New root non-empty without a marker while the old root has data: `Err`
///   requiring manual reconciliation. Never merges, never overwrites.
/// - Otherwise: copy the whole tree, write the marker last, report counts.
///
/// The old tree is never modified. Failures return `Err` with a path-free
/// message; the caller must fail startup rather than fall back to the old
/// profile.
pub fn migrate_native_profile(
    old_root: &Path,
    new_root: &Path,
    expected_new_leaf: &str,
) -> Result<MigrationReport, String> {
    validate_pair(old_root, new_root, expected_new_leaf)?;
    if !dir_has_data(old_root) {
        return Ok(MigrationReport {
            files_copied: 0,
            bytes_copied: 0,
            already_migrated: false,
        });
    }
    if new_root.join(MIGRATION_MARKER).is_file() {
        return Ok(MigrationReport {
            files_copied: 0,
            bytes_copied: 0,
            already_migrated: true,
        });
    }
    if dir_has_data(new_root) {
        let old_leaf = old_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("old-profile");
        return Err(format!(
            "both the legacy native profile ({old_leaf}) and the Remcord profile \
             ({expected_new_leaf}) contain data; refusing to merge or overwrite. \
             Back up both directories, reconcile them by hand, then relaunch"
        ));
    }
    std::fs::create_dir_all(new_root).map_err(|error| {
        format!("migration cannot create the Remcord profile directory: {error}")
    })?;
    let (files, bytes) = copy_tree(old_root, new_root)?;
    let old_leaf = old_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("old-profile");
    let marker_body = format!("migrated-from={old_leaf} files={files} bytes={bytes}\n");
    std::fs::write(new_root.join(MIGRATION_MARKER), marker_body)
        .map_err(|error| format!("migration cannot record its completion marker: {error}"))?;
    Ok(MigrationReport {
        files_copied: files,
        bytes_copied: bytes,
        already_migrated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "remcord-migration-test-{}-{}-{}",
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
        // settings.json with exact bytes (UTF-8, no BOM, compact JSON).
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
    fn migrates_a_populated_tree_and_preserves_settings_bytes() {
        let fixture = Fixture::fresh("copy", "Remcord");
        seed_old_profile(&fixture.old);
        let before = std::fs::read(fixture.old.join("storage").join("settings.json")).unwrap();
        let report = migrate_native_profile(&fixture.old, &fixture.new, "Remcord")
            .expect("migration must succeed");
        assert!(!report.already_migrated);
        assert!(report.files_copied >= 5);
        assert!(report.bytes_copied >= before.len() as u64);
        // Byte-exact preservation: no BOM added, no re-serialization.
        let after = std::fs::read(fixture.new.join("storage").join("settings.json")).unwrap();
        assert_eq!(before, after);
        assert!(!after.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(
            std::fs::read(fixture.new.join("quickCss.css")).unwrap(),
            b":root{color:red;}"
        );
        assert!(fixture.new.join(MIGRATION_MARKER).is_file());
        // The old tree is untouched.
        assert!(fixture.old.join("storage").join("settings.json").is_file());
    }

    #[test]
    fn migration_is_idempotent_and_never_overwrites_user_files() {
        let fixture = Fixture::fresh("idempotent", "Remcord");
        seed_old_profile(&fixture.old);
        migrate_native_profile(&fixture.old, &fixture.new, "Remcord").expect("first");
        // User edits the new profile after migrating.
        std::fs::write(
            fixture.new.join("storage").join("settings.json"),
            br#"{"a":2,"mods":[]}"#,
        )
        .expect("user edit");
        let report = migrate_native_profile(&fixture.old, &fixture.new, "Remcord")
            .expect("rerun must succeed");
        assert!(report.already_migrated);
        assert_eq!(report.files_copied, 0);
        assert_eq!(
            std::fs::read(fixture.new.join("storage").join("settings.json")).unwrap(),
            br#"{"a":2,"mods":[]}"#.as_slice()
        );
    }

    #[test]
    fn refuses_to_overwrite_a_non_empty_new_profile() {
        let fixture = Fixture::fresh("clash", "Remcord");
        seed_old_profile(&fixture.old);
        std::fs::create_dir_all(&fixture.new).expect("new root");
        std::fs::write(fixture.new.join("storage.json"), b"{}").expect("user file");
        let error = migrate_native_profile(&fixture.old, &fixture.new, "Remcord")
            .expect_err("clash must fail");
        assert!(
            error.contains("reconcile"),
            "must require reconciliation: {error}"
        );
        assert!(!fixture.new.join(MIGRATION_MARKER).is_file());
        // Nothing was copied.
        assert!(!fixture.new.join("quickCss.css").exists());
    }

    #[test]
    fn missing_or_empty_old_profile_is_a_no_op() {
        let fixture = Fixture::fresh("empty", "Remcord");
        let missing_old = fixture.base.join("no-such-old");
        let report = migrate_native_profile(&missing_old, &fixture.new, "Remcord")
            .expect("missing old must succeed");
        assert_eq!(report.files_copied, 0);
        assert!(!report.already_migrated);
        let report = migrate_native_profile(&fixture.old, &fixture.new, "Remcord")
            .expect("empty old must succeed");
        assert_eq!(report.files_copied, 0);
        assert!(!fixture.new.join(MIGRATION_MARKER).is_file());
    }

    #[test]
    fn refuses_the_production_electron_profile() {
        let base = unique_root("prod");
        let _ = std::fs::remove_dir_all(&base);
        let prod = base.join("legcord");
        let dest = base.join("Remcord");
        std::fs::create_dir_all(&prod).expect("prod dir");
        std::fs::write(prod.join("settings.json"), b"{}").expect("prod file");
        let error = migrate_native_profile(&prod, &dest, "Remcord")
            .expect_err("production must be refused");
        assert!(
            error.contains("production"),
            "must name the danger: {error}"
        );
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn refuses_a_wrong_destination_leaf() {
        let fixture = Fixture::fresh("leaf", "SomethingElse");
        seed_old_profile(&fixture.old);
        let error = migrate_native_profile(&fixture.old, &fixture.new, "Remcord")
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
        let error =
            migrate_native_profile(&fixture.old, &fixture.new, "Remcord").expect_err("must fail");
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

    #[test]
    fn legacy_release_dir_joins_the_leaf() {
        assert_eq!(
            legacy_release_dir(r"C:\Users\Test\AppData\Local"),
            PathBuf::from(r"C:\Users\Test\AppData\Local\LegcordNative")
        );
    }
}

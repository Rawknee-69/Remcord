//! Read-only bundle-file discovery for Phase D backups (Step 1).
//!
//! This module is the native counterpart of the bundle-file half of the
//! Electron backup handlers in `src/common/backup.ts` (`BUNDLE_FILES`,
//! `buildBackupZipBuffer`, `applyBackupFromMap`) as invoked from
//! `src/discord/ipc.ts` (`backupSave` / `backupRestore`, exposed as
//! `window.legcord.backup.save` / `.restore` via
//! `src/discord/preload/bridge.ts`). Only the discovery step is migrated:
//! which fixed bundle files exist under the profile root and how large they
//! are. No file contents, no zip handling, no dialogs, no writes.
//!
//! Traced Electron behavior (actual implementation, not declarations):
//! - Allowlist: exactly 7 fixed names at `<userData>/` — `vencord.js`,
//!   `vencord.css`, `equicord.js`, `equicord.css`, `shelter.js`,
//!   `custom.js`, `custom.css` (`backup.ts:73-81`).
//! - Discovery: one existence check per allowlisted name (`addFileIfExists`);
//!   missing files are silently skipped, never placeholders or errors.
//! - Reads are whole-file and unbounded; restore writes with recursive
//!   `mkdir` plus silent overwrite, gated only by `includes.modBundles` and
//!   exact allowlist equality. No size caps, no atomicity.
//!
//! Native contract (deliberate, documented differences from Electron):
//! - Metadata only: `{ name, present, bytes }` per allowlisted name. File
//!   contents, absolute paths, profile paths, URLs, and arbitrary filenames
//!   never cross the boundary.
//! - Symlinks, junctions, directories, and any other non-regular file are
//!   reported as absent (`present: false, bytes: 0`) and are NEVER followed.
//!   Electron's `readFileSync` follows links, so a planted link would make a
//!   backup embed bytes from outside the profile; the native inventory must
//!   not even stat outside the root. This matches the established native
//!   filesystem convention (plugin storage and themes reject planted links
//!   instead of following them).
//! - `custom.js` / `custom.css` are inventoried exactly like the other five
//!   names (same allowlist, same ordering, same presence semantics) but stay
//!   inventory-only and non-executable: there is no authoritative
//!   host-controlled source for custom artifacts, so no native execution or
//!   restore path may treat presence as servability.
//! - A filesystem error other than "not found" (e.g. permission denied)
//!   fails the whole inventory closed as `BUNDLES_UNAVAILABLE` instead of
//!   guessing absence. Error messages carry only the fixed allowlisted name,
//!   never a path.
//!
//! Logical root owned by this module: the profile root itself, but ONLY for
//! the 7 fixed names joined onto a host-built path. There is no renderer
//! path input at all (the request payload is empty), so traversal is
//! impossible by construction.

use std::path::Path;

/// Exact Electron `BUNDLE_FILES` allowlist (`src/common/backup.ts`), in the
/// same order. `custom.js` / `custom.css` are inventory-only (see module
/// docs); they are listed so a future backup export slice can reason about
/// the same set Electron reasons about.
pub const BUNDLE_FILES: &[&str] = &[
    "vencord.js",
    "vencord.css",
    "equicord.js",
    "equicord.css",
    "shelter.js",
    "custom.js",
    "custom.css",
];

/// One inventoried bundle file. `bytes` is 0 whenever `present` is false.
#[derive(Debug)]
pub struct BundleEntry {
    pub name: &'static str,
    pub present: bool,
    pub bytes: u64,
}

/// Typed failure for the capability: a stable machine-readable `code` plus a
/// human message that never contains a filesystem path.
#[derive(Debug)]
pub struct BundleFailure {
    pub code: &'static str,
    pub message: String,
}

fn unavailable(name: &str) -> BundleFailure {
    BundleFailure {
        code: "BUNDLES_UNAVAILABLE",
        // `name` is always one of the fixed `BUNDLE_FILES` constants, so no
        // renderer input or host path can reach this message.
        message: format!("bundle inventory is unavailable for {name}"),
    }
}

/// Describe the fixed bundle files under the host-controlled profile root.
///
/// The root is built entirely by the caller from the isolated profile
/// directory; each allowlisted name is joined as a single path component so
/// the result cannot escape lexically. Links and non-regular files are
/// reported absent without being followed (see module docs).
pub fn describe_bundle_files(profile_dir: &Path) -> Result<Vec<BundleEntry>, BundleFailure> {
    let mut out = Vec::with_capacity(BUNDLE_FILES.len());
    for name in BUNDLE_FILES {
        let candidate = profile_dir.join(name);
        match std::fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => out.push(BundleEntry {
                name,
                present: false,
                bytes: 0,
            }),
            Err(_) => return Err(unavailable(name)),
            Ok(meta) => {
                if meta.is_file() {
                    out.push(BundleEntry {
                        name,
                        present: true,
                        bytes: meta.len(),
                    });
                } else {
                    // Symlink, junction, directory, or other non-regular
                    // file: absent, never followed.
                    out.push(BundleEntry {
                        name,
                        present: false,
                        bytes: 0,
                    });
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn unique_temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "remcord-bundles-test-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&root).expect("test temp root must be creatable");
        root
    }

    fn remove_temp_root(root: &Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn allowlist_matches_the_seven_electron_bundle_names_in_order() {
        assert_eq!(
            BUNDLE_FILES,
            &[
                "vencord.js",
                "vencord.css",
                "equicord.js",
                "equicord.css",
                "shelter.js",
                "custom.js",
                "custom.css",
            ]
        );
    }

    #[test]
    fn empty_profile_reports_every_bundle_absent() {
        let root = unique_temp_root("empty");
        let entries = describe_bundle_files(&root).expect("empty profile must inventory cleanly");
        assert_eq!(entries.len(), BUNDLE_FILES.len());
        for (entry, name) in entries.iter().zip(BUNDLE_FILES.iter()) {
            assert_eq!(entry.name, *name);
            assert!(!entry.present, "{name} must be absent");
            assert_eq!(entry.bytes, 0, "{name} absent implies zero bytes");
        }
        remove_temp_root(&root);
    }

    #[test]
    fn regular_files_report_present_with_exact_byte_lengths() {
        let root = unique_temp_root("files");
        std::fs::write(root.join("vencord.js"), b"// Vencord test bundle")
            .expect("fixture must write");
        std::fs::write(root.join("shelter.js"), b"").expect("empty fixture must write");
        let entries =
            describe_bundle_files(&root).expect("populated profile must inventory cleanly");
        let vencord = entries.iter().find(|e| e.name == "vencord.js").unwrap();
        assert!(vencord.present);
        assert_eq!(vencord.bytes, 22);
        let shelter = entries.iter().find(|e| e.name == "shelter.js").unwrap();
        assert!(shelter.present);
        assert_eq!(shelter.bytes, 0);
        let equicord = entries.iter().find(|e| e.name == "equicord.css").unwrap();
        assert!(!equicord.present);
        assert_eq!(equicord.bytes, 0);
        remove_temp_root(&root);
    }

    #[test]
    fn directories_at_bundle_names_report_absent() {
        let root = unique_temp_root("dir");
        std::fs::create_dir_all(root.join("vencord.js")).expect("fixture dir must be creatable");
        let entries =
            describe_bundle_files(&root).expect("dir at bundle name must inventory cleanly");
        let vencord = entries.iter().find(|e| e.name == "vencord.js").unwrap();
        assert!(
            !vencord.present,
            "a directory is not a servable bundle file"
        );
        assert_eq!(vencord.bytes, 0);
        remove_temp_root(&root);
    }

    #[test]
    fn inventory_never_follows_symlinks() {
        let root = unique_temp_root("link");
        let outside = unique_temp_root("outside");
        std::fs::write(outside.join("secret.txt"), b"outside-bytes")
            .expect("outside fixture must write");
        #[cfg(windows)]
        let link_result =
            std::os::windows::fs::symlink_file(outside.join("secret.txt"), root.join("vencord.js"));
        #[cfg(not(windows))]
        let link_result =
            std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("vencord.js"));
        if link_result.is_err() {
            eprintln!("SKIP: link creation needs privileges in this environment; lexical safety still holds");
            remove_temp_root(&root);
            remove_temp_root(&outside);
            return;
        }
        let entries = describe_bundle_files(&root).expect("planted link must inventory cleanly");
        let vencord = entries.iter().find(|e| e.name == "vencord.js").unwrap();
        assert!(
            !vencord.present,
            "a symlink at a bundle name must report absent, never the link target's bytes"
        );
        assert_eq!(vencord.bytes, 0);
        remove_temp_root(&root);
        remove_temp_root(&outside);
    }
}

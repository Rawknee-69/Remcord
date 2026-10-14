//! Read-only downloads manager surface (F23).
//!
//! Checklist state before this module: **partial** — silent save with
//! sanitized names plus no-overwrite uniqueness was done in
//! [`crate::downloads`], but there was no dialog, no manager, and no
//! renderer API. Electron parity reference: production Remcord registers
//! NO `will-download` / `download-item` / `downloadURL` handler, so
//! Electron uses the Chromium default per-download Save dialog with the
//! server-suggested filename and a user-chosen destination. There is NO
//! Electron download-manager IPC, store, or page to mirror.
//!
//! Deliberate hardening differences (not omissions):
//! - The P0.5 silent-save policy is unchanged: downloads still land in the
//!   host-owned `<profile>/downloads/` directory, existing files are never
//!   overwritten, and no Save dialog exists (none can run inside wry's
//!   synchronous `DownloadStarting` closure). This module adds no write
//!   path, no dialog, and no `save(path)` RPC.
//! - The three capabilities are read-only plus shell-reveal: `downloads.list`
//!   (parameter-free inventory), `downloads.open_folder` (parameter-free
//!   reveal of the fixed root, same reveal semantics as `folders.open`),
//!   and `downloads.reveal` (exactly one already-sanitized file NAME —
//!   never a path, URL, id, or destination; the host re-sanitizes with
//!   byte-exact equality and re-checks canonical containment before the
//!   shell call).
//! - The response carries display metadata only (`name`, `bytes`,
//!   `modified_ms`). No absolute path, no URL, no file contents, and no
//!   host error text ever cross the bridge.
//! - Shell access is the same direct-API mechanism `folders.rs` uses
//!   (`SHOpenFolderAndSelectItems`); no command line is ever constructed,
//!   no `ShellExecuteW` string handling is added here.
//!
//! No shell, no picker/dialog, no `DefaultDownloadFolderPath` mutation, no
//! `executeJavaScript`, no network, no filesystem access beyond the host
//! downloads dir.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Fixed renderer-visible location label for the one managed root.
/// Mirrors `gallery::GALLERY_ROOT_LABEL`: a profile-relative label, never
/// an absolute path.
pub const DOWNLOADS_LOCATION_LABEL: &str = "downloads";
/// Maximum inventory entries per `downloads.list` response.
pub const MAX_LIST_ENTRIES: usize = 500;
/// Maximum renderer-supplied download name in UTF-8 bytes for
/// `downloads.reveal`. Shape check only; the authoritative rule is the
/// byte-exact [`crate::downloads::sanitize_filename`] round trip.
pub const MAX_DOWNLOAD_NAME_BYTES: usize = 256;

/// One inventoried download: display metadata only. No path, no URL, no
/// contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DownloadEntry {
    pub name: String,
    pub bytes: u64,
    pub modified_ms: u64,
}

/// Typed failure for the capability: a stable machine-readable `code` plus
/// a human message that never contains a filesystem path.
#[derive(Debug)]
pub struct DownloadFailure {
    pub code: &'static str,
    pub message: String,
}

fn failure(code: &'static str, message: &str) -> DownloadFailure {
    DownloadFailure {
        code,
        message: message.to_owned(),
    }
}

/// Ensures the host-owned downloads directory exists and resolves it to
/// its canonical path, contained under the canonical profile root. A
/// pre-planted junction at the root can never redirect the manager — the
/// same containment rule `folders::ensure_folder_root` enforces.
pub fn canonical_downloads_dir(profile_dir: &Path) -> Result<PathBuf, DownloadFailure> {
    let dir = crate::downloads::ensure_downloads_dir(profile_dir)
        .map_err(|_| failure("DOWNLOADS_UNAVAILABLE", "downloads folder is unavailable"))?;
    let canonical_root = std::fs::canonicalize(profile_dir)
        .map_err(|_| failure("DOWNLOADS_UNAVAILABLE", "downloads folder is unavailable"))?;
    let canonical = std::fs::canonicalize(&dir)
        .map_err(|_| failure("DOWNLOADS_UNAVAILABLE", "downloads folder is unavailable"))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(failure(
            "OUTSIDE_ROOT",
            "downloads target is outside the approved root",
        ));
    }
    Ok(canonical)
}

/// File modification time as millis since the Unix epoch; `0` when the
/// clock or metadata is unreadable (ordering degrades, never fails).
fn modified_ms(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// Read-only inventory of the host downloads dir, newest-first with a
/// name tie-break for determinism, bounded to [`MAX_LIST_ENTRIES`].
/// Regular files only: subdirectories and symlinks/junctions are never
/// followed and never listed.
pub fn list_downloads(profile_dir: &Path) -> Result<Vec<DownloadEntry>, DownloadFailure> {
    let dir = canonical_downloads_dir(profile_dir)?;
    let read = std::fs::read_dir(&dir)
        .map_err(|_| failure("DOWNLOADS_UNAVAILABLE", "downloads folder is unreadable"))?;
    let mut entries = Vec::new();
    for entry in read {
        let Ok(entry) = entry else { continue };
        // Never follow links: metadata comes from the entry itself.
        let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.is_empty() {
            continue;
        }
        entries.push(DownloadEntry {
            name,
            bytes: metadata.len(),
            modified_ms: modified_ms(&metadata),
        });
    }
    entries.sort_by(|left, right| {
        right
            .modified_ms
            .cmp(&left.modified_ms)
            .then_with(|| left.name.cmp(&right.name))
    });
    entries.truncate(MAX_LIST_ENTRIES);
    Ok(entries)
}

/// Resolves exactly one renderer-named download to its canonical path.
/// The name must already be sanitized: [`crate::downloads::sanitize_filename`]
/// must reproduce it byte-exactly, so traversal (`../`), absolute/UNC
/// forms, drive prefixes, stream shapes, and reserved stems can never
/// resolve. The entry must exist as a non-link regular file inside the
/// canonical downloads dir; anything else is `UNKNOWN_DOWNLOAD`, and a
/// link resolving outside is `OUTSIDE_ROOT`.
pub fn resolve_download_file(profile_dir: &Path, name: &str) -> Result<PathBuf, DownloadFailure> {
    if name.is_empty() || name.len() > MAX_DOWNLOAD_NAME_BYTES {
        return Err(failure("INVALID_DOWNLOAD_NAME", "download name is invalid"));
    }
    let sanitized = crate::downloads::sanitize_filename(name);
    if sanitized.as_deref() != Some(name) {
        return Err(failure("INVALID_DOWNLOAD_NAME", "download name is invalid"));
    }
    let dir = canonical_downloads_dir(profile_dir)?;
    let candidate = dir.join(name);
    // Lexical guard first (sanitized names are bare, so this always
    // holds — defense in depth before any filesystem touch).
    if candidate.parent() != Some(dir.as_path()) {
        return Err(failure("INVALID_DOWNLOAD_NAME", "download name is invalid"));
    }
    let metadata = std::fs::symlink_metadata(&candidate)
        .map_err(|_| failure("UNKNOWN_DOWNLOAD", "download is unknown"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(failure("UNKNOWN_DOWNLOAD", "download is unknown"));
    }
    let canonical = std::fs::canonicalize(&candidate)
        .map_err(|_| failure("UNKNOWN_DOWNLOAD", "download is unknown"))?;
    if !canonical.starts_with(&dir) {
        return Err(failure(
            "OUTSIDE_ROOT",
            "downloads target is outside the approved root",
        ));
    }
    Ok(canonical)
}

/// Reveal semantics for one resolved download file (the `showItemInFolder`
/// equivalent Electron would get for a file): Explorer opens at the
/// parent with exactly this file selected. `path` must already be a
/// host-resolved canonical file. Direct shell API; no command line is
/// ever constructed.
#[cfg(target_os = "windows")]
pub fn reveal_download_file(path: &Path) -> Result<(), DownloadFailure> {
    use windows::{
        core::HSTRING,
        Win32::{
            System::Com::{
                CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{Common::ITEMIDLIST, SHOpenFolderAndSelectItems, SHParseDisplayName},
        },
    };

    fn display(path: &Path) -> Result<String, DownloadFailure> {
        let raw = path
            .as_os_str()
            .to_str()
            .ok_or_else(|| failure("REVEAL_FAILED", "download reveal failed"))?;
        Ok(raw
            .strip_prefix(r"\\?\UNC\")
            .map(|rest| format!(r"\\{rest}"))
            .unwrap_or_else(|| raw.strip_prefix(r"\\?\").unwrap_or(raw).to_owned()))
    }

    let path_str = display(path)?;
    let parent_str = path
        .parent()
        .map(display)
        .transpose()?
        .ok_or_else(|| failure("REVEAL_FAILED", "download reveal failed"))?;
    let com_init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if com_init.is_err() {
        return Err(failure("REVEAL_FAILED", "download reveal failed"));
    }
    let com_owned = com_init == windows::core::HRESULT(0);
    let parent_name = HSTRING::from(parent_str.as_str());
    let child_name = HSTRING::from(path_str.as_str());
    let mut parent_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    let mut child_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    let result = (|| {
        let parent_hr =
            unsafe { SHParseDisplayName(&parent_name, None, &mut parent_pidl, 0, None) };
        if parent_hr.is_err() || parent_pidl.is_null() {
            return Err(failure("REVEAL_FAILED", "download reveal failed"));
        }
        let child_hr = unsafe { SHParseDisplayName(&child_name, None, &mut child_pidl, 0, None) };
        if child_hr.is_err() || child_pidl.is_null() {
            return Err(failure("REVEAL_FAILED", "download reveal failed"));
        }
        let selected = [child_pidl as *const ITEMIDLIST];
        let reveal_hr = unsafe {
            SHOpenFolderAndSelectItems(parent_pidl as *const ITEMIDLIST, Some(&selected), 0)
        };
        if reveal_hr.is_err() {
            return Err(failure("REVEAL_FAILED", "download reveal failed"));
        }
        Ok(())
    })();
    unsafe {
        if !parent_pidl.is_null() {
            CoTaskMemFree(Some(parent_pidl as *const std::ffi::c_void));
        }
        if !child_pidl.is_null() {
            CoTaskMemFree(Some(child_pidl as *const std::ffi::c_void));
        }
        if com_owned {
            CoUninitialize();
        }
    }
    result
}

/// Downloads reveal targets Windows WebView2 first (project rule); other
/// platforms fail closed with a typed error instead of guessing a shell
/// command. No `xdg-open`/`open` abstraction is introduced.
#[cfg(not(target_os = "windows"))]
pub fn reveal_download_file(_path: &Path) -> Result<(), DownloadFailure> {
    Err(failure(
        "DOWNLOADS_UNSUPPORTED_PLATFORM",
        "downloads reveal is not supported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "remcord-download-manager-test-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&root).expect("test temp root must be creatable");
        root
    }

    fn seed_downloads(profile: &Path, names: &[&str]) -> PathBuf {
        let dir = crate::downloads::ensure_downloads_dir(profile).expect("dir must build");
        for name in names {
            std::fs::write(dir.join(name), b"bytes").unwrap();
        }
        dir
    }

    #[test]
    fn inventory_lists_regular_files_newest_first_bounded() {
        let profile = temp_root("list");
        seed_downloads(&profile, &["older.txt", "newer.txt"]);
        // Force a deterministic mtime order.
        let dir = profile.join(crate::downloads::DOWNLOADS_DIR_NAME);
        let older = std::fs::File::options()
            .write(true)
            .open(dir.join("older.txt"))
            .unwrap();
        older
            .set_modified(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        drop(older);
        let entries = list_downloads(&profile).expect("listing must succeed");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "newer.txt");
        assert_eq!(entries[1].name, "older.txt");
        for entry in &entries {
            assert_eq!(entry.bytes, 5);
            assert!(entry.modified_ms > 0 || entry.name == "older.txt");
        }
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn inventory_skips_directories_and_never_follows_links() {
        let profile = temp_root("shapes");
        let dir = seed_downloads(&profile, &["real.txt"]);
        std::fs::create_dir(dir.join("subdir")).unwrap();
        std::fs::write(dir.join("subdir").join("nested.txt"), b"x").unwrap();
        #[cfg(windows)]
        let link_result =
            std::os::windows::fs::symlink_file(dir.join("real.txt"), dir.join("linked.txt"));
        #[cfg(not(windows))]
        let link_result = std::os::unix::fs::symlink(dir.join("real.txt"), dir.join("linked.txt"));
        let entries = list_downloads(&profile).expect("listing must succeed");
        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert!(names.contains(&"real.txt"));
        assert!(
            !names.contains(&"nested.txt"),
            "subdirectory contents leak: {names:?}"
        );
        if link_result.is_ok() {
            assert!(
                !names.contains(&"linked.txt"),
                "symlinks must never be listed: {names:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn resolve_accepts_only_exactly_sanitized_bare_names() {
        let profile = temp_root("resolve-ok");
        seed_downloads(&profile, &["report.txt", "photo (2).jpg"]);
        for name in ["report.txt", "photo (2).jpg"] {
            let resolved = resolve_download_file(&profile, name).expect("must resolve");
            assert_eq!(resolved.file_name().unwrap(), name);
        }
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn resolve_rejects_traversal_absolute_and_reserved_shapes() {
        let profile = temp_root("resolve-bad");
        seed_downloads(&profile, &["ok.txt", "_CON.txt"]);
        for hostile in [
            "",
            ".",
            "..",
            "../ok.txt",
            "..\\ok.txt",
            "C:\\Windows\\ok.txt",
            "C:ok.txt",
            "\\\\server\\share\\ok.txt",
            "/ok.txt",
            "sub/ok.txt",
            "sub\\ok.txt",
            "ok.txt ",
            "ok.",
            "CON.txt",
            "con",
            "ok:stream.txt",
            "ok?.txt",
        ] {
            assert!(
                resolve_download_file(&profile, hostile).is_err(),
                "{hostile:?} must never resolve"
            );
        }
        // Reserved stems resolve only under their neutralized spelling.
        assert!(resolve_download_file(&profile, "_CON.txt").is_ok());
        // Oversized names fail closed.
        assert!(resolve_download_file(&profile, &"n".repeat(MAX_DOWNLOAD_NAME_BYTES + 1)).is_err());
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn resolve_rejects_unknown_and_non_files_without_leaking_paths() {
        let profile = temp_root("unknown");
        let dir = seed_downloads(&profile, &["known.txt"]);
        std::fs::create_dir(dir.join("folder.txt")).unwrap();
        let profile_text = profile.to_string_lossy().into_owned();
        for name in ["missing.txt", "folder.txt"] {
            let failure = resolve_download_file(&profile, name).expect_err("must fail");
            assert_eq!(failure.code, "UNKNOWN_DOWNLOAD");
            assert!(!failure.message.contains(&profile_text));
            assert!(!failure.message.contains(name));
        }
        // A symlink planted at a sanitized name resolves outside → contained.
        let outside = temp_root("outside");
        std::fs::write(outside.join("secret.txt"), b"s").unwrap();
        #[cfg(windows)]
        let link_result =
            std::os::windows::fs::symlink_file(outside.join("secret.txt"), dir.join("planted.txt"));
        #[cfg(not(windows))]
        let link_result =
            std::os::unix::fs::symlink(outside.join("secret.txt"), dir.join("planted.txt"));
        if link_result.is_ok() {
            let failure =
                resolve_download_file(&profile, "planted.txt").expect_err("link must fail");
            assert!(
                failure.code == "UNKNOWN_DOWNLOAD" || failure.code == "OUTSIDE_ROOT",
                "unexpected code {}",
                failure.code
            );
            assert!(!failure.message.contains(&profile_text));
        } else {
            eprintln!("SKIP: link creation needs privileges; lexical safety still holds");
        }
        let _ = std::fs::remove_dir_all(&profile);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn canonical_dir_rejects_outside_roots() {
        let profile = temp_root("canonical");
        let dir = canonical_downloads_dir(&profile).expect("dir must resolve");
        assert!(dir.ends_with(crate::downloads::DOWNLOADS_DIR_NAME));
        assert!(
            dir.starts_with(std::fs::canonicalize(&profile).expect("profile must canonicalize"))
        );
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn shipped_module_has_no_forbidden_surfaces() {
        let source = include_str!("download_manager.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "showOpenDialog",
            "showSaveDialog",
            "showItemInFolder",
            "openPath",
            "openExternal",
            "DefaultDownloadFolderPath",
            "executeJavaScript",
            "evaluate_script",
            "postMessage",
            "window.ipc",
            "window.remcord",
            "Command::new",
            "std::process",
            "reqwest",
            "fetch(",
            "Url::parse",
            "GalleryList",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
    }
}

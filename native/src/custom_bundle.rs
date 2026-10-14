//! Host-controlled retrieval for user-owned custom client mods (F1).
//!
//! This module is the native counterpart of Electron's `getCustomBundle`
//! handler (`src/discord/ipc.ts:130-137`) as consumed by
//! `src/discord/preload/mods/custom.mts`: when `settings.mods` includes
//! `"custom"`, the host serves `custom.js` (executed) plus `custom.css`
//! (inserted when present) from the profile root.
//!
//! Traced Electron behavior (actual implementation, not declarations):
//! - Source: `<userData>/custom.js` + `<userData>/custom.css`, read with
//!   `ifExistsRead` (missing file yields `undefined`, never an error).
//! - Gate: `getConfig("mods").includes("custom")`; when disabled both halves
//!   are `undefined` regardless of what sits on disk.
//! - Execution: `webFrame.executeJavaScript(bundle.js)` then
//!   `webFrame.insertCSS(bundle.css)` when CSS is present. No identity
//!   check, no marker verification: evaluation without a throw IS success.
//!
//! Native contract (deliberate, documented differences from Electron):
//! - The renderer supplies ONLY the logical id `"custom"` through the
//!   shared `mod.get_bundle` capability. Both file paths are built by the
//!   host from the isolated profile directory plus the two fixed names
//!   below; traversal is impossible by construction (no renderer path, URL,
//!   or filename input at any layer).
//! - Symlinks, junctions, directories, and any other non-regular file are
//!   treated as ABSENT (never followed), matching the bundle-inventory
//!   convention (`bundles.rs`): a planted link must never make the host
//!   execute bytes from outside the profile.
//! - Size caps come from the single source of truth: [`MAX_CUSTOM_JS_BYTES`]
//!   / [`MAX_CUSTOM_CSS_BYTES`] here, referenced by the `CUSTOM_MOD`
//!   descriptor in `main.rs`, so the caps cannot drift. An over-limit file
//!   fails closed (`CUSTOM_TOO_LARGE`) instead of truncating.
//! - Empty files are treated as absent (a zero-byte `custom.js` carries no
//!   mod; Electron would execute `""` as a no-op). Both halves absent is a
//!   typed `CUSTOM_NOT_INSTALLED` failure, not silent success: the settings
//!   toggle names a mod whose files are missing, and the adapter must hear
//!   that instead of hanging on a bundle that never arrives.
//! - UTF-8 and envelope validation stay owned by the shared substrate
//!   (`validate_mod_bundle` with the `UserOwned` identity): this module
//!   returns RAW bytes, never reinterpreted text.
//! - No network, no download, no cache file distinct from the user's own
//!   files, no in-memory cache: the files are re-read per request so a user
//!   edit applies on the next document (reload) without a restart. The
//!   `mod.clear_cache` capability never touches these files (they are
//!   user-owned, not host-managed caches).
//!
//! Logical root owned by this module: the profile root itself, but ONLY for
//! the two fixed names joined onto a host-built path.

use std::path::Path;

/// Fixed custom JS filename inside the host profile. The renderer never sees
/// this name as a path; it only ever observes the `custom` mod id.
pub const CUSTOM_JS_FILE: &str = "custom.js";
/// Fixed custom CSS filename inside the host profile. Same visibility rule.
pub const CUSTOM_CSS_FILE: &str = "custom.css";
/// Maximum accepted `custom.js` size. Generous for hand-written user mods;
/// far below the 32 MiB upstream-bundle bound because no release build
/// needs to fit. Electron is unbounded.
pub const MAX_CUSTOM_JS_BYTES: usize = 8 * 1024 * 1024;
/// Maximum accepted `custom.css` size. Matches the Quick CSS bound; real
/// custom stylesheets are small editor buffers. Electron is unbounded.
pub const MAX_CUSTOM_CSS_BYTES: usize = 1024 * 1024;

/// Raw user-owned bundle halves as read from the profile root. `None` means
/// the file is absent (missing, empty, or a non-regular file that was
/// refused without following). At least one half is always `Some`: both
/// absent is [`CustomFailure`] `CUSTOM_NOT_INSTALLED`, never an empty
/// success.
#[derive(Debug)]
pub struct RawCustomBundle {
    pub js: Option<Vec<u8>>,
    pub css: Option<Vec<u8>>,
}

/// Typed failure for custom retrieval: a stable machine-readable `code`
/// plus a human message that never contains a filesystem path.
#[derive(Debug)]
pub struct CustomFailure {
    pub code: &'static str,
    pub message: String,
}

fn failure(code: &'static str, message: &str) -> CustomFailure {
    CustomFailure {
        code,
        message: message.to_owned(),
    }
}

/// Read one fixed custom file from the host-controlled profile root.
///
/// Returns `None` when the file is absent (missing, empty, or a
/// non-regular file such as a symlink, junction, or directory — links are
/// NEVER followed). Any other filesystem error fails closed; an over-limit
/// file fails closed instead of truncating.
fn read_custom_half(
    profile_dir: &Path,
    file_name: &'static str,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>, CustomFailure> {
    let candidate = profile_dir.join(file_name);
    let meta = match std::fs::symlink_metadata(&candidate) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(failure(
                "CUSTOM_UNAVAILABLE",
                &format!("custom bundle file {file_name} is unreadable"),
            ));
        }
        Ok(meta) => meta,
    };
    if !meta.is_file() {
        // Symlink, junction, directory, or other non-regular file: absent,
        // never followed (same convention as the bundle inventory).
        return Ok(None);
    }
    if meta.len() > max_bytes as u64 {
        return Err(failure(
            "CUSTOM_TOO_LARGE",
            &format!("custom bundle file {file_name} exceeds the size limit"),
        ));
    }
    let bytes = std::fs::read(&candidate).map_err(|_| {
        failure(
            "CUSTOM_UNAVAILABLE",
            &format!("custom bundle file {file_name} is unreadable"),
        )
    })?;
    if bytes.len() > max_bytes {
        return Err(failure(
            "CUSTOM_TOO_LARGE",
            &format!("custom bundle file {file_name} exceeds the size limit"),
        ));
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(bytes))
}

/// Retrieve the raw custom halves from the host-controlled profile root.
///
/// Both paths are host-built (`profile_dir` + fixed name); the caller
/// supplies no filename, path, or URL. Both halves absent is
/// `CUSTOM_NOT_INSTALLED`.
pub fn load_raw_custom_bundle(profile_dir: &Path) -> Result<RawCustomBundle, CustomFailure> {
    let js = read_custom_half(profile_dir, CUSTOM_JS_FILE, MAX_CUSTOM_JS_BYTES)?;
    let css = read_custom_half(profile_dir, CUSTOM_CSS_FILE, MAX_CUSTOM_CSS_BYTES)?;
    if js.is_none() && css.is_none() {
        return Err(failure(
            "CUSTOM_NOT_INSTALLED",
            "custom bundles are not installed",
        ));
    }
    Ok(RawCustomBundle { js, css })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn unique_temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "remcord-custom-test-{}-{}-{tag}",
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
    fn fixed_names_match_the_electron_custom_filenames() {
        assert_eq!(CUSTOM_JS_FILE, "custom.js");
        assert_eq!(CUSTOM_CSS_FILE, "custom.css");
    }

    #[test]
    fn empty_profile_reports_not_installed() {
        let root = unique_temp_root("empty");
        let error = load_raw_custom_bundle(&root).expect_err("no custom files must fail");
        assert_eq!(error.code, "CUSTOM_NOT_INSTALLED");
        // No filesystem path may reach the renderer-facing message.
        assert!(!error.message.contains(&root.to_string_lossy().into_owned()));
        remove_temp_root(&root);
    }

    #[test]
    fn js_only_and_css_only_bundles_load() {
        let root = unique_temp_root("halves");
        std::fs::write(root.join("custom.js"), b"console.log('hi');").unwrap();
        let bundle = load_raw_custom_bundle(&root).expect("js-only must load");
        assert_eq!(bundle.js.unwrap(), b"console.log('hi');");
        assert!(bundle.css.is_none());
        remove_temp_root(&root);

        let root = unique_temp_root("css-only");
        std::fs::write(root.join("custom.css"), b".x{color:red}").unwrap();
        let bundle = load_raw_custom_bundle(&root).expect("css-only must load");
        assert!(bundle.js.is_none());
        assert_eq!(bundle.css.unwrap(), b".x{color:red}");
        remove_temp_root(&root);
    }

    #[test]
    fn empty_files_count_as_absent() {
        let root = unique_temp_root("empty-files");
        std::fs::write(root.join("custom.js"), b"").unwrap();
        let error = load_raw_custom_bundle(&root).expect_err("empty js alone must fail");
        assert_eq!(error.code, "CUSTOM_NOT_INSTALLED");
        std::fs::write(root.join("custom.css"), b".x{}").unwrap();
        let bundle = load_raw_custom_bundle(&root).expect("css carries the bundle");
        assert!(bundle.js.is_none());
        assert!(bundle.css.is_some());
        remove_temp_root(&root);
    }

    #[test]
    fn directories_at_custom_names_count_as_absent() {
        let root = unique_temp_root("dir");
        std::fs::create_dir_all(root.join("custom.js")).unwrap();
        let error = load_raw_custom_bundle(&root).expect_err("a directory is not a bundle");
        assert_eq!(error.code, "CUSTOM_NOT_INSTALLED");
        remove_temp_root(&root);
    }

    #[test]
    fn retrieval_never_follows_symlinks() {
        let root = unique_temp_root("link");
        let outside = unique_temp_root("outside");
        std::fs::write(outside.join("secret.txt"), b"outside-bytes").unwrap();
        #[cfg(windows)]
        let link_result =
            std::os::windows::fs::symlink_file(outside.join("secret.txt"), root.join("custom.js"));
        #[cfg(not(windows))]
        let link_result =
            std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("custom.js"));
        if link_result.is_err() {
            eprintln!("SKIP: link creation needs privileges in this environment");
            remove_temp_root(&root);
            remove_temp_root(&outside);
            return;
        }
        let error = load_raw_custom_bundle(&root).expect_err("a planted link must not execute");
        assert_eq!(error.code, "CUSTOM_NOT_INSTALLED");
        remove_temp_root(&root);
        remove_temp_root(&outside);
    }

    #[test]
    fn over_limit_files_fail_closed() {
        let root = unique_temp_root("too-large");
        let big = vec![b'x'; MAX_CUSTOM_CSS_BYTES + 1];
        std::fs::write(root.join("custom.css"), &big).unwrap();
        let error = load_raw_custom_bundle(&root).expect_err("over-limit css must fail");
        assert_eq!(error.code, "CUSTOM_TOO_LARGE");
        assert!(!error.message.contains(&root.to_string_lossy().into_owned()));
        remove_temp_root(&root);
    }

    #[test]
    fn failure_messages_never_carry_filesystem_paths() {
        let root = unique_temp_root("no-paths");
        std::fs::write(
            root.join("custom.css"),
            vec![b'x'; MAX_CUSTOM_CSS_BYTES + 1],
        )
        .unwrap();
        let error = load_raw_custom_bundle(&root).expect_err("must fail");
        let root_str = root.to_string_lossy().into_owned();
        assert!(!error.message.contains(&root_str));
        assert!(!error.message.contains('\\') || error.message.contains("custom.css"));
        remove_temp_root(&root);
    }
}

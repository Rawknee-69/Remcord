//! Scoped plugin-storage filesystem capability for Phase D.
//!
//! This module is the native counterpart of the Electron plugin-storage
//! handlers in `src/discord/ipc.ts` (`pluginWriteFile` / `pluginReadFile`,
//! exposed to third-party plugins as `window.legcord.fs.writeFile/readFile`
//! via `src/discord/preload/bridge.ts`). The Electron contract, traced from
//! the actual implementation rather than declarations:
//!
//! - Logical resource: `(pluginId, relativePath)` plus UTF-8 text `data`
//!   (writes only). No absolute paths, no file picker, no dialogs.
//! - Root: `<userData>/plugin-storage/<sanitizedId>/`.
//! - Electron sanitizes the id (`[^a-zA-Z0-9_-]` stripped, truncated to 64,
//!   falling back to `"default"`) and confines the path lexically
//!   (`path.resolve(base, normalize(rel))`, reject when
//!   `relative(base, resolved)` starts with `..` or is absolute).
//! - Electron gates BOTH operations on the user setting
//!   `extendedPluginAbilities` (`EXTENSION_DISABLED` otherwise), returns
//!   `{ ok: false, error: "NOT_FOUND" }` for missing files, creates parent
//!   directories recursively on write, and performs unbounded UTF-8
//!   reads/writes with no size limit and no symlink handling.
//! - There are NO in-repository callers (`grep \.fs\.` finds only the
//!   declaration): this is a public third-party plugin API, so the native
//!   milestone has no Electron caller to branch. A DEV-only smoke probe
//!   exercises the exact public contract instead.
//!
//! Native hardening (deliberate, documented differences from Electron):
//! - Plugin ids are VALIDATED, never sanitized: `^[A-Za-z0-9_-]{1,64}$`.
//!   Electron maps `"a/b"` to `"ab"`; native rejects it as
//!   `INVALID_PLUGIN_ID`. Sanitizing would turn an invalid id into an
//!   accepted one and could alias distinct inputs to one directory.
//! - Relative paths are validated component-wise (both `/` and `\` are
//!   separators): no absolute paths, no drive letters, no UNC/device
//!   prefixes, no `..`/`.` components, no empty components, no NUL, no
//!   Windows-illegal `< > : " | ? *` or control characters, no trailing
//!   space/dot components, and no reserved device names (`CON`, `PRN`,
//!   `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9`). Electron accepts several
//!   of these; native rejects them as `INVALID_PATH`.
//! - No percent-decoding is performed: `%2e%2e` is a literal filename, so
//!   encoded traversal is inert by construction.
//! - Containment is enforced twice: lexically (components cannot escape by
//!   construction) AND canonically (the resolved parent/file is
//!   `canonicalize`d and must start with the canonical plugin root, which
//!   resolves symlinks/junctions on the host).
//! - Size-bounded: ids <= 64 bytes, relative paths <= 256 bytes, file
//!   contents <= 1 MiB in both directions. Electron is unbounded.
//! - Writes are atomic (sibling temp file + same-directory rename), mirroring
//!   the Phase C-3 settings mutation. Electron writes in place.
//! - The `extendedPluginAbilities` gate is enforced from the authoritative
//!   settings load. A corrupt settings file surfaces `SETTINGS_UNAVAILABLE`
//!   (Electron would read a fallback empty cache and report disabled).
//!
//! Logical roots owned by this module: exactly one —
//! `<profile>/plugin-storage/<plugin_id>/`. No other root (themes, Quick
//! CSS, bundles, backups, userData) is exposed by this milestone.

use std::path::{Path, PathBuf};

/// Maximum plugin id length in UTF-8 bytes. Mirrors the Electron 64-char
/// truncation and the runtime plugin manifest id bound.
pub const MAX_PLUGIN_ID_BYTES: usize = 64;
/// Maximum relative path length in UTF-8 bytes. Real plugin paths such as
/// `cache/deleted-messages.json` are far shorter; this is a safety rail.
pub const MAX_PLUGIN_REL_PATH_BYTES: usize = 256;
/// Maximum file payload in bytes in either direction. Electron is unbounded;
/// 1 MiB comfortably covers JSON-ish plugin state without letting the
/// renderer make the host buffer arbitrarily large files.
pub const MAX_PLUGIN_FILE_BYTES: usize = 1024 * 1024;

fn is_valid_plugin_id_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

/// Validate a renderer-supplied plugin id. Strict accept only: the id is used
/// verbatim as a single directory name, never transformed.
pub fn validate_plugin_id(plugin_id: &str) -> Result<(), String> {
    if plugin_id.is_empty() {
        return Err("plugin id must not be empty".to_owned());
    }
    if plugin_id.len() > MAX_PLUGIN_ID_BYTES {
        return Err("plugin id exceeds the size limit".to_owned());
    }
    if !plugin_id.bytes().all(is_valid_plugin_id_char) {
        return Err("plugin id uses characters outside [A-Za-z0-9_-]".to_owned());
    }
    Ok(())
}

fn is_reserved_device_stem(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    if upper.len() == 4 {
        let (head, tail) = upper.split_at(3);
        if (head == "COM" || head == "LPT")
            && matches!(tail, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        {
            return true;
        }
    }
    false
}

/// Validate a renderer-supplied relative path and split it into components.
///
/// Both `/` and `\` separate components (Windows semantics). The returned
/// components are safe to `PathBuf::push` one at a time: they contain no
/// separators, no drive syntax, and no `.`/`..`, so the joined path cannot
/// escape lexically. Canonical containment is still enforced by the caller.
pub fn validate_relative_path(relative_path: &str) -> Result<Vec<String>, String> {
    if relative_path.is_empty() {
        return Err("relative path must not be empty".to_owned());
    }
    if relative_path.len() > MAX_PLUGIN_REL_PATH_BYTES {
        return Err("relative path exceeds the size limit".to_owned());
    }
    if relative_path.contains('\0') {
        return Err("relative path must not contain NUL".to_owned());
    }
    // Absolute paths of every Windows/POSIX form are rejected before
    // component analysis so no normalization can turn them acceptable.
    let first = relative_path.as_bytes()[0];
    if first == b'/' || first == b'\\' {
        return Err("relative path must not be absolute".to_owned());
    }
    // `:` anywhere blocks drive letters (`C:...`), ADS streams (`a:b`), and
    // NTFS alternate syntax in one rule; components below re-check per part.
    if relative_path.contains(':') {
        return Err("relative path must not contain ':'".to_owned());
    }
    if relative_path.bytes().any(|byte| byte < 0x20) {
        return Err("relative path must not contain control characters".to_owned());
    }
    // Device/UNC prefixes (`\\?\`, `\\.\`, `\??\`, `//?/`) all start with a
    // separator (rejected above) or contain ':' (rejected above); this is a
    // defense-in-depth explicit rejection.
    let lowered = relative_path.to_ascii_lowercase();
    if lowered.starts_with("\\\\?\\")
        || lowered.starts_with("\\\\.\\")
        || lowered.starts_with("\\??\\")
        || lowered.starts_with("//?/")
    {
        return Err("relative path must not use a device prefix".to_owned());
    }

    let mut components = Vec::new();
    for component in relative_path.split(['/', '\\']) {
        if component.is_empty() {
            return Err("relative path must not contain empty components".to_owned());
        }
        if component == "." || component == ".." {
            return Err("relative path must not contain '.' or '..'".to_owned());
        }
        if component.contains('\0') {
            return Err("relative path must not contain NUL".to_owned());
        }
        if component.bytes().any(|byte| byte < 0x20) {
            return Err("relative path must not contain control characters".to_owned());
        }
        if component.contains(['<', '>', '"', '|', '?', '*', ':']) {
            return Err("relative path contains a reserved Windows character".to_owned());
        }
        // Windows strips trailing spaces/dots, which would alias distinct
        // inputs to one file; reject instead of normalizing.
        if component.ends_with(' ') || component.ends_with('.') {
            return Err("relative path components must not end with a space or dot".to_owned());
        }
        // Reserved device names (`CON`, `COM1`, ...) address devices rather
        // than the plugin directory on Windows.
        let stem = component.split('.').next().unwrap_or(component);
        if is_reserved_device_stem(stem) {
            return Err("relative path uses a reserved device name".to_owned());
        }
        components.push(component.to_owned());
    }
    if components.is_empty() {
        return Err("relative path must name a file".to_owned());
    }
    Ok(components)
}

/// Typed failure for the capability: a stable machine-readable `code`
/// (mirroring the Electron error strings where they exist) plus a
/// human-readable message for the host log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStorageFailure {
    pub code: &'static str,
    pub message: String,
}

impl PluginStorageFailure {
    fn new(code: &'static str, message: String) -> Self {
        Self { code, message }
    }
}

/// Successful read: the file text plus its byte length for DEV metrics.
/// Absolute paths and filesystem metadata are never returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStorageRead {
    pub data: String,
    pub payload_bytes: usize,
}

/// Successful write: the persisted byte length for DEV metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStorageWrite {
    pub payload_bytes: usize,
}

/// Host-constructed plugin root. The renderer influences only the validated
/// `plugin_id`, which is a single validated directory name.
pub fn plugin_dir(profile_dir: &Path, plugin_id: &str) -> PathBuf {
    profile_dir.join("plugin-storage").join(plugin_id)
}

fn check_extended_abilities(profile_dir: &Path) -> Result<(), PluginStorageFailure> {
    match crate::settings::load_settings(profile_dir) {
        Ok(load) => {
            if load.snapshot.extended_plugin_abilities {
                Ok(())
            } else {
                Err(PluginStorageFailure::new(
                    "EXTENSION_DISABLED",
                    "extended plugin abilities are disabled".to_owned(),
                ))
            }
        }
        Err(error) => Err(PluginStorageFailure::new("SETTINGS_UNAVAILABLE", error)),
    }
}

fn join_components(root: &Path, components: &[String]) -> PathBuf {
    let mut path = root.to_path_buf();
    for component in components {
        path.push(component);
    }
    path
}

/// Read one scoped plugin-storage file.
///
/// Order: validate id, validate path, enforce the abilities gate, confine to
/// the host root (lexical + canonical), enforce the size bound, return text.
pub fn read_plugin_file(
    profile_dir: &Path,
    plugin_id: &str,
    relative_path: &str,
) -> Result<PluginStorageRead, PluginStorageFailure> {
    validate_plugin_id(plugin_id)
        .map_err(|message| PluginStorageFailure::new("INVALID_PLUGIN_ID", message))?;
    let components = validate_relative_path(relative_path)
        .map_err(|message| PluginStorageFailure::new("INVALID_PATH", message))?;
    check_extended_abilities(profile_dir)?;

    let root = plugin_dir(profile_dir, plugin_id);
    let target = join_components(&root, &components);

    // The file must exist: a missing plugin directory or file is NOT_FOUND,
    // exactly like Electron.
    let metadata = std::fs::metadata(&target).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            PluginStorageFailure::new("NOT_FOUND", "plugin storage file does not exist".to_owned())
        } else {
            PluginStorageFailure::new(
                "READ_FAILED",
                format!("plugin storage file cannot be read: {error}"),
            )
        }
    })?;
    if !metadata.is_file() {
        return Err(PluginStorageFailure::new(
            "NOT_FOUND",
            "plugin storage file does not exist".to_owned(),
        ));
    }
    if metadata.len() > MAX_PLUGIN_FILE_BYTES as u64 {
        return Err(PluginStorageFailure::new(
            "TOO_LARGE",
            "plugin storage file exceeds the size limit".to_owned(),
        ));
    }

    // Canonical containment: resolves symlinks/junctions. Both sides are
    // canonicalized so the Windows `\\?\` prefix cannot break the check.
    let canonical_root = std::fs::canonicalize(&root).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            PluginStorageFailure::new("NOT_FOUND", "plugin storage file does not exist".to_owned())
        } else {
            PluginStorageFailure::new(
                "READ_FAILED",
                format!("plugin storage root cannot be resolved: {error}"),
            )
        }
    })?;
    let canonical_target = std::fs::canonicalize(&target).map_err(|error| {
        PluginStorageFailure::new(
            "READ_FAILED",
            format!("plugin storage file cannot be resolved: {error}"),
        )
    })?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(PluginStorageFailure::new(
            "INVALID_PATH",
            "plugin storage path escapes its root".to_owned(),
        ));
    }

    let bytes = std::fs::read(&canonical_target).map_err(|error| {
        PluginStorageFailure::new(
            "READ_FAILED",
            format!("plugin storage file cannot be read: {error}"),
        )
    })?;
    if bytes.len() > MAX_PLUGIN_FILE_BYTES {
        return Err(PluginStorageFailure::new(
            "TOO_LARGE",
            "plugin storage file exceeds the size limit".to_owned(),
        ));
    }
    let data = String::from_utf8(bytes).map_err(|_| {
        PluginStorageFailure::new(
            "READ_FAILED",
            "plugin storage file is not valid UTF-8 text".to_owned(),
        )
    })?;
    let payload_bytes = data.len();
    Ok(PluginStorageRead {
        data,
        payload_bytes,
    })
}

/// Write one scoped plugin-storage file.
///
/// Order: validate id, validate path, validate size, enforce the abilities
/// gate, create ONLY the host-controlled plugin namespace, enforce canonical
/// containment of the parent, persist atomically (sibling temp + rename).
pub fn write_plugin_file(
    profile_dir: &Path,
    plugin_id: &str,
    relative_path: &str,
    data: &str,
) -> Result<PluginStorageWrite, PluginStorageFailure> {
    validate_plugin_id(plugin_id)
        .map_err(|message| PluginStorageFailure::new("INVALID_PLUGIN_ID", message))?;
    let components = validate_relative_path(relative_path)
        .map_err(|message| PluginStorageFailure::new("INVALID_PATH", message))?;
    if data.len() > MAX_PLUGIN_FILE_BYTES {
        return Err(PluginStorageFailure::new(
            "TOO_LARGE",
            "plugin storage data exceeds the size limit".to_owned(),
        ));
    }
    check_extended_abilities(profile_dir)?;

    let root = plugin_dir(profile_dir, plugin_id);
    std::fs::create_dir_all(&root).map_err(|error| {
        PluginStorageFailure::new(
            "WRITE_FAILED",
            format!("plugin storage directory cannot be created: {error}"),
        )
    })?;
    let canonical_root = std::fs::canonicalize(&root).map_err(|error| {
        PluginStorageFailure::new(
            "WRITE_FAILED",
            format!("plugin storage root cannot be resolved: {error}"),
        )
    })?;

    let target = join_components(&root, &components);
    let parent = target.parent().ok_or_else(|| {
        PluginStorageFailure::new("INVALID_PATH", "relative path must name a file".to_owned())
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        PluginStorageFailure::new(
            "WRITE_FAILED",
            format!("plugin storage directory cannot be created: {error}"),
        )
    })?;
    // The parent is canonicalized AFTER creation so a pre-planted
    // symlink/junction inside the namespace cannot redirect the write: the
    // resolved parent must still sit under the resolved root.
    let canonical_parent = std::fs::canonicalize(parent).map_err(|error| {
        PluginStorageFailure::new(
            "WRITE_FAILED",
            format!("plugin storage directory cannot be resolved: {error}"),
        )
    })?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(PluginStorageFailure::new(
            "INVALID_PATH",
            "plugin storage path escapes its root".to_owned(),
        ));
    }
    let target = canonical_parent.join(components.last().expect("a validated path names a file"));

    // Same-directory temp file + rename: readers never observe a truncated
    // file, mirroring the Phase C-3 settings mutation.
    let temp_name = format!(
        ".{}.tmp-{}-{}",
        components.last().expect("a validated path names a file"),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0),
    );
    let temp_path = canonical_parent.join(temp_name);
    if let Err(error) = std::fs::write(&temp_path, data.as_bytes()) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(PluginStorageFailure::new(
            "WRITE_FAILED",
            format!("plugin storage file cannot be written: {error}"),
        ));
    }
    if let Err(error) = std::fs::rename(&temp_path, &target) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(PluginStorageFailure::new(
            "WRITE_FAILED",
            format!("plugin storage file cannot be written: {error}"),
        ));
    }

    Ok(PluginStorageWrite {
        payload_bytes: data.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        read_plugin_file, validate_plugin_id, validate_relative_path, write_plugin_file,
        MAX_PLUGIN_FILE_BYTES,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_profile_dir(tag: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be available")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "remcord-plugin-storage-{tag}-{nanos}-{}",
            std::process::id()
        ))
    }

    fn seed_settings(profile: &std::path::Path, enabled: bool) {
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(
            profile.join("storage").join("settings.json"),
            format!("{{\"extendedPluginAbilities\":{enabled}}}"),
        )
        .unwrap();
    }

    #[test]
    fn accepts_normal_plugin_ids() {
        assert!(validate_plugin_id("my-plugin_01").is_ok());
        assert!(validate_plugin_id("a").is_ok());
        assert!(validate_plugin_id(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn rejects_invalid_plugin_ids() {
        // Empty, oversized, and anything outside [A-Za-z0-9_-].
        assert!(validate_plugin_id("").is_err());
        assert!(validate_plugin_id(&"a".repeat(65)).is_err());
        for bad in [
            "../escape",
            "a/b",
            "a\\b",
            "a.b",
            "a:b",
            "a b",
            "a+b",
            "ünicode",
            "a/b/../c",
        ] {
            assert!(validate_plugin_id(bad).is_err(), "must reject {bad:?}");
        }
        // Electron would sanitize "a/b" to "ab"; native must NOT.
        assert!(validate_plugin_id("a/b").is_err());
    }

    #[test]
    fn accepts_a_normal_relative_path() {
        let components = validate_relative_path("cache/deleted-messages.json").unwrap();
        assert_eq!(components, vec!["cache", "deleted-messages.json"]);
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        for bad in [
            "../escape.txt",
            "a/../../escape.txt",
            "a/../b",
            "..",
            ".",
            "",
            "/absolute/path.txt",
            "\\absolute\\path.txt",
            "C:\\secret.txt",
            "c:/secret.txt",
            "C:secret.txt",
            "\\\\server\\share\\file.txt",
            "//server/share/file.txt",
            "\\\\?\\C:\\secret.txt",
            "a//b.txt",
            "a\\b/../c.txt",
            "a/./b.txt",
            "trailing/",
            "/leading",
        ] {
            assert!(validate_relative_path(bad).is_err(), "must reject {bad:?}");
        }
    }

    #[test]
    fn rejects_mixed_separators_that_escape() {
        // Backslash is a separator on Windows: `a\..\..\x` escapes.
        assert!(validate_relative_path("a\\..\\..\\escape.txt").is_err());
        assert!(validate_relative_path("a/b\\..\\..\\escape.txt").is_err());
    }

    #[test]
    fn rejects_windows_quirks_without_normalizing_them_acceptable() {
        // ADS/drive syntax, illegal characters, trailing space/dot (which
        // Windows would strip, aliasing distinct inputs), device names.
        for bad in [
            "a:b.txt",
            "a/b:c",
            "file?.txt",
            "file*.txt",
            "file|.txt",
            "file<.txt",
            "file\".txt",
            "name ",
            "name.",
            "dir/CON",
            "CON.txt",
            "com1.json",
            "AUX/data.txt",
            "nul",
            "LPT9.txt",
            "a/CON.txt",
        ] {
            assert!(validate_relative_path(bad).is_err(), "must reject {bad:?}");
        }
    }

    #[test]
    fn rejects_the_root_path_itself_and_empty_names() {
        assert!(validate_relative_path("").is_err());
        assert!(validate_relative_path("/").is_err());
    }

    #[test]
    fn treats_encoded_traversal_as_inert_literal_names() {
        // No percent-decoding happens: these are literal file names inside
        // the namespace, never traversal. They must either validate as
        // ordinary names (containing '%') or fail closed — never escape.
        let components = validate_relative_path("%2e%2e/escape.txt").unwrap();
        assert_eq!(components, vec!["%2e%2e", "escape.txt"]);
        let profile = unique_profile_dir("encoded");
        seed_settings(&profile, true);
        write_plugin_file(&profile, "probe", "%2e%2e/escape.txt", "x").unwrap();
        assert!(!profile.join("plugin-storage").join("escape.txt").exists());
    }

    #[test]
    fn rejects_oversized_ids_paths_and_payloads() {
        assert!(validate_plugin_id(&"a".repeat(65)).is_err());
        assert!(validate_relative_path(&"a".repeat(257)).is_err());
        let profile = unique_profile_dir("oversized");
        seed_settings(&profile, true);
        let big = "x".repeat(MAX_PLUGIN_FILE_BYTES + 1);
        let failure = write_plugin_file(&profile, "probe", "big.txt", &big).unwrap_err();
        assert_eq!(failure.code, "TOO_LARGE");
    }

    #[test]
    fn round_trips_a_valid_write_and_read() {
        let profile = unique_profile_dir("roundtrip");
        seed_settings(&profile, true);
        let written = write_plugin_file(&profile, "probe-1", "cache/note.txt", "hello remcord")
            .expect("a valid write must succeed");
        assert_eq!(written.payload_bytes, "hello remcord".len());
        // No temp file may be left behind.
        let dir = profile.join("plugin-storage").join("probe-1").join("cache");
        assert_eq!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
                .filter(|name| name.to_string_lossy().contains(".tmp-"))
                .count(),
            0
        );
        let read = read_plugin_file(&profile, "probe-1", "cache/note.txt").unwrap();
        assert_eq!(read.data, "hello remcord");
        assert_eq!(read.payload_bytes, "hello remcord".len());
    }

    #[test]
    fn missing_files_report_not_found_like_electron() {
        let profile = unique_profile_dir("missing");
        seed_settings(&profile, true);
        let failure = read_plugin_file(&profile, "probe", "nope.txt").unwrap_err();
        assert_eq!(failure.code, "NOT_FOUND");
        // Unknown plugin namespace is also NOT_FOUND, not an error leak.
        let failure = read_plugin_file(&profile, "unknown-plugin", "nope.txt").unwrap_err();
        assert_eq!(failure.code, "NOT_FOUND");
    }

    #[test]
    fn disabled_abilities_gate_both_directions() {
        let profile = unique_profile_dir("disabled");
        seed_settings(&profile, false);
        assert_eq!(
            write_plugin_file(&profile, "probe", "a.txt", "x")
                .unwrap_err()
                .code,
            "EXTENSION_DISABLED"
        );
        assert_eq!(
            read_plugin_file(&profile, "probe", "a.txt")
                .unwrap_err()
                .code,
            "EXTENSION_DISABLED"
        );
    }

    #[test]
    fn corrupt_settings_surface_unavailable_instead_of_guessing() {
        let profile = unique_profile_dir("corrupt");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(profile.join("storage").join("settings.json"), b"{not json").unwrap();
        assert_eq!(
            read_plugin_file(&profile, "probe", "a.txt")
                .unwrap_err()
                .code,
            "SETTINGS_UNAVAILABLE"
        );
    }

    #[test]
    fn oversized_stored_files_are_not_buffered() {
        let profile = unique_profile_dir("toolarge");
        seed_settings(&profile, true);
        let root = profile.join("plugin-storage").join("probe");
        std::fs::create_dir_all(&root).unwrap();
        let big = vec![b'x'; MAX_PLUGIN_FILE_BYTES + 16];
        std::fs::write(root.join("big.txt"), big).unwrap();
        assert_eq!(
            read_plugin_file(&profile, "probe", "big.txt")
                .unwrap_err()
                .code,
            "TOO_LARGE"
        );
    }

    #[test]
    fn symlink_escape_is_blocked_or_environment_limited() {
        let profile = unique_profile_dir("symlink");
        seed_settings(&profile, true);
        write_plugin_file(&profile, "victim", "note.txt", "inside").unwrap();
        let outside = unique_profile_dir("symlink-outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "outside").unwrap();
        let link = profile
            .join("plugin-storage")
            .join("victim")
            .join("link.txt");
        // Point a namespace entry at a file outside the root.
        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_file(outside.join("secret.txt"), &link).is_ok();
        #[cfg(not(windows))]
        let created = std::os::unix::fs::symlink(outside.join("secret.txt"), &link).is_ok();
        if !created {
            // Creating symlinks needs privileges on some Windows hosts; the
            // lexical + canonical layers below still hold, and this
            // environment simply cannot plant the escape.
            return;
        }
        let failure = read_plugin_file(&profile, "victim", "link.txt").unwrap_err();
        assert_eq!(failure.code, "INVALID_PATH");
    }

    #[test]
    fn hostile_parent_symlink_cannot_redirect_writes() {
        let profile = unique_profile_dir("symlink-write");
        seed_settings(&profile, true);
        let outside = unique_profile_dir("symlink-write-outside");
        std::fs::create_dir_all(&outside).unwrap();
        let cache_link = profile
            .join("plugin-storage")
            .join("attacker")
            .join("cache");
        std::fs::create_dir_all(cache_link.parent().unwrap()).unwrap();
        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_dir(&outside, &cache_link).is_ok();
        #[cfg(not(windows))]
        let created = std::os::unix::fs::symlink(&outside, &cache_link).is_ok();
        if !created {
            return;
        }
        let failure = write_plugin_file(&profile, "attacker", "cache/evil.txt", "x").unwrap_err();
        assert_eq!(failure.code, "INVALID_PATH");
        assert!(!outside.join("evil.txt").exists());
    }

    #[test]
    fn unknown_plugin_ids_stay_namespaced() {
        let profile = unique_profile_dir("namespace");
        seed_settings(&profile, true);
        write_plugin_file(&profile, "plugin-a", "note.txt", "A").unwrap();
        // A second namespace cannot observe the first through any valid path.
        let failure = read_plugin_file(&profile, "plugin-b", "note.txt").unwrap_err();
        assert_eq!(failure.code, "NOT_FOUND");
        assert_eq!(
            read_plugin_file(&profile, "plugin-a", "note.txt")
                .unwrap()
                .data,
            "A"
        );
    }
}

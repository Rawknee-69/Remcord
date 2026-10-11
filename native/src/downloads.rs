//! Native download filename/destination policy (P0.5).
//!
//! Electron contract: active source has NO `will-download` /
//! `download-item` / `downloadURL` handler, so production Legcord uses the
//! Chromium/Electron default — a per-download Save dialog with the
//! server-suggested filename and a user-chosen destination.
//!
//! Native P0.5 intentionally differs (documented hardening, not parity):
//! no dialog can run inside wry's synchronous `DownloadStarting` closure,
//! so downloads land silently in the host-owned `<profile>/downloads/`
//! directory. The renderer never selects a destination, existing files are
//! never overwritten, and unsafe names are sanitized. There is deliberately
//! NO bridge/facade capability, NO `save(path)` RPC, and NO COM dependency
//! here — this module is pure filename/destination logic plus tests. The
//! wry handler wiring lives in `main.rs` (one shared helper for the main
//! and P0.4 child WebViews).
//!
//! No shell, no picker/dialog, no `DefaultDownloadFolderPath` mutation, no
//! `executeJavaScript`, no filesystem access beyond the host downloads dir.

use std::path::{Path, PathBuf};

/// Maximum final filename length in characters.
pub const MAX_FILENAME_LEN: usize = 128;
/// Maximum duplicate alternatives (`foo (2).txt` …) before denying.
pub const MAX_DUPLICATE_ATTEMPTS: u32 = 100;
/// Fallback when sanitization empties the name.
pub const FALLBACK_FILENAME: &str = "download";
/// Maximum preserved extension length in characters (alphanumeric only).
pub const MAX_EXTENSION_LEN: usize = 16;
/// Fixed host-owned downloads directory name under the profile root.
pub const DOWNLOADS_DIR_NAME: &str = "downloads";

/// Host-internal download outcome. Never crosses the bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadDecision {
    Allow(PathBuf),
    Deny,
}

/// Creates (if missing) and validates the host-owned downloads directory
/// under the profile root. Fails closed when the path exists as a
/// non-directory or a symlink/junction (never followed into).
pub fn ensure_downloads_dir(profile_dir: &Path) -> Result<PathBuf, String> {
    let dir = profile_dir.join(DOWNLOADS_DIR_NAME);
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("downloads directory is not creatable: {error}"))?;
    let meta = std::fs::symlink_metadata(&dir)
        .map_err(|error| format!("downloads directory is not readable: {error}"))?;
    if meta.file_type().is_symlink() {
        return Err("downloads directory must not be a symlink".to_owned());
    }
    if !meta.is_dir() {
        return Err("downloads directory is not a directory".to_owned());
    }
    Ok(dir)
}

/// True for Windows reserved device stems (case-insensitive, extension
/// ignored — `CON.txt` is reserved exactly like `CON`).
fn is_reserved_stem(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Sanitizes one suggested filename so the result is always a bare file
/// name (no separators, no traversal, no absolute/UNC form) safe to join
/// directly beneath the host downloads dir. Returns `None` when no safe
/// name can be produced (caller falls back to [`FALLBACK_FILENAME`]).
pub fn sanitize_filename(suggested: &str) -> Option<String> {
    // Directory discard first: everything through the last `/` or `\`
    // (covers `C:\…`, `\\server\…`, `\\?\…`, `../`, mixed separators).
    let mut name = suggested.rsplit(['/', '\\']).next().unwrap_or("");
    // `\\?\C:\foo` style leftovers and drive prefixes without separators.
    if let Some((_drive, rest)) = name.split_once(':') {
        // A colon only survives inside the final component for NTFS
        // alternate-data-stream shapes (`foo:bar`); drop the stream part.
        // (`C:foo` without a separator also collapses to its tail.)
        name = rest.rsplit(['/', '\\']).next().unwrap_or("");
        if name.is_empty() {
            return None;
        }
    }
    // Strip control characters and the Windows-invalid set.
    let mut cleaned: String = name
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
        .collect();
    // Windows ignores trailing spaces and dots; strip them explicitly so
    // `foo... ` cannot smuggle an alternate spelling.
    while cleaned.ends_with(' ') || cleaned.ends_with('.') {
        cleaned.pop();
    }
    cleaned = cleaned.trim().to_owned();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return None;
    }
    // Split a trailing extension; keep it only when alphanumeric and
    // bounded, otherwise drop it (the stem below still carries the name).
    let (mut stem, extension) = match cleaned.rfind('.') {
        Some(dot) if dot > 0 => {
            let (stem, ext) = cleaned.split_at(dot);
            let ext = &ext[1..];
            if !ext.is_empty()
                && ext.len() <= MAX_EXTENSION_LEN
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
            {
                (stem.to_owned(), Some(ext.to_owned()))
            } else {
                (stem.to_owned(), None)
            }
        }
        _ => (cleaned, None),
    };
    if stem.is_empty() {
        return None;
    }
    if is_reserved_stem(&stem) {
        stem = format!("_{stem}");
    }
    let mut result = match &extension {
        Some(ext) => format!("{stem}.{ext}"),
        None => stem.clone(),
    };
    // Enforce the length cap while preserving a kept extension.
    if result.chars().count() > MAX_FILENAME_LEN {
        match &extension {
            Some(ext) => {
                let keep = MAX_FILENAME_LEN.saturating_sub(ext.len() + 1);
                if keep == 0 {
                    return None;
                }
                let truncated: String = stem.chars().take(keep).collect();
                if truncated.is_empty() {
                    return None;
                }
                result = format!("{truncated}.{ext}");
            }
            None => {
                result = result.chars().take(MAX_FILENAME_LEN).collect();
            }
        }
    }
    if result.is_empty() {
        return None;
    }
    Some(result)
}

/// Sanitizes with the [`FALLBACK_FILENAME`] guarantee: never `None`.
pub fn sanitize_filename_or_fallback(suggested: &str) -> String {
    sanitize_filename(suggested).unwrap_or_else(|| FALLBACK_FILENAME.to_owned())
}

/// Resolves a unique destination directly beneath `dir` without ever
/// overwriting: `foo.txt`, `foo (2).txt`, … up to
/// [`MAX_DUPLICATE_ATTEMPTS`]. Returns `None` when exhausted or when the
/// containment assert (`parent == dir`) fails. Single-process
/// check-then-use: no locking ceremony (see module docs).
pub fn resolve_unique_destination(dir: &Path, filename: &str) -> Option<PathBuf> {
    let candidate = dir.join(filename);
    if candidate.parent() != Some(dir) {
        return None;
    }
    if !candidate.exists() {
        return Some(candidate);
    }
    let (stem, extension) = match filename.rfind('.') {
        Some(dot) if dot > 0 => (&filename[..dot], Some(&filename[dot + 1..])),
        _ => (filename, None),
    };
    for index in 2..=MAX_DUPLICATE_ATTEMPTS + 1 {
        let numbered = match extension {
            Some(ext) => format!("{stem} ({index}).{ext}"),
            None => format!("{filename} ({index})"),
        };
        let candidate = dir.join(&numbered);
        if candidate.parent() != Some(dir) {
            return None;
        }
        if !candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Full start-of-download decision: take ONLY the file name from WebView2's
/// suggested path (its directory is discarded — never trusted), sanitize,
/// and resolve a unique host-owned destination. Any failure denies.
pub fn decide_download(downloads_dir: &Path, suggested_path: &Path) -> DownloadDecision {
    let suggested = suggested_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let filename = sanitize_filename_or_fallback(&suggested);
    match resolve_unique_destination(downloads_dir, &filename) {
        Some(destination) => DownloadDecision::Allow(destination),
        None => DownloadDecision::Deny,
    }
}

/// Redacts a download URL for host logs: keeps `scheme://host/path`
/// (truncated), drops query/fragment/userinfo where tokens live. Never
/// fails — unparseable input yields a static placeholder.
pub fn redact_url(url_str: &str) -> String {
    let parsed = match url::Url::parse(url_str) {
        Ok(parsed) => parsed,
        Err(_) => return "unparseable-url".to_owned(),
    };
    if parsed.cannot_be_a_base() {
        return format!("{}:redacted", parsed.scheme());
    }
    let host = parsed.host_str().unwrap_or("no-host");
    let mut path = parsed.path().to_owned();
    if path.chars().count() > 96 {
        path = format!("{}…", path.chars().take(96).collect::<String>());
    }
    format!("{}://{host}{path}", parsed.scheme())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "remcord-downloads-test-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&root).expect("test temp root must be creatable");
        root
    }

    #[test]
    fn valid_names_pass_through_unchanged() {
        for name in ["test.txt", "test.png", "photo (1).jpg", "a.b.c.tar.gz"] {
            assert_eq!(sanitize_filename(name).as_deref(), Some(name));
        }
    }

    #[test]
    fn unicode_and_long_legal_names_survive() {
        assert!(sanitize_filename("фото 测试 🎉.png").is_some());
        let stem = "a".repeat(120);
        let name = format!("{stem}.txt");
        assert_eq!(sanitize_filename(&name).as_deref(), Some(name.as_str()));
    }

    #[test]
    fn empty_and_blank_names_fail() {
        for name in ["", "   ", ".", "..", "...", "   ...   "] {
            assert_eq!(sanitize_filename(name), None, "{name:?}");
            assert_eq!(sanitize_filename_or_fallback(name), FALLBACK_FILENAME);
        }
    }

    #[test]
    fn path_attacks_collapse_to_their_tail_or_fallback() {
        // Tail extraction keeps the innocent leaf; absolute/UNC forms must
        // never yield separators or parent escapes.
        let cases = [
            ("C:\\Windows\\System32\\foo.txt", Some("foo.txt")),
            ("C:/Windows/System32/foo.txt", Some("foo.txt")),
            ("\\foo.txt", Some("foo.txt")),
            ("/foo.txt", Some("foo.txt")),
            ("\\\\server\\share\\foo.txt", Some("foo.txt")),
            ("\\\\?\\C:\\foo.txt", Some("foo.txt")),
            ("../foo.txt", Some("foo.txt")),
            ("..\\foo.txt", Some("foo.txt")),
            ("a/../../foo.txt", Some("foo.txt")),
            ("a\\..\\..\\foo.txt", Some("foo.txt")),
            ("C:foo.txt", Some("foo.txt")),
            ("foo:bar.txt", Some("bar.txt")),
        ];
        for (input, expected) in cases {
            let sanitized = sanitize_filename_or_fallback(input);
            assert_eq!(sanitized, expected.unwrap(), "{input:?}");
            assert!(!sanitized.contains(['/', '\\']), "{input:?}");
            assert!(!sanitized.contains(".."), "{input:?}");
        }
        // Pure evil with no leaf falls back.
        assert_eq!(sanitize_filename_or_fallback("C:"), FALLBACK_FILENAME);
    }

    #[test]
    fn invalid_windows_characters_are_stripped() {
        assert_eq!(sanitize_filename("a<b>c.txt").as_deref(), Some("abc.txt"));
        assert_eq!(sanitize_filename("a|b?.txt").as_deref(), Some("ab.txt"));
        assert_eq!(sanitize_filename("a\"b*.txt").as_deref(), Some("ab.txt"));
        assert_eq!(
            sanitize_filename("a\u{0}b\u{1f}c.txt").as_deref(),
            Some("abc.txt")
        );
    }

    #[test]
    fn reserved_names_are_neutralized_case_insensitively() {
        for name in [
            "CON", "con", "Con.txt", "PRN", "aux", "NUL.log", "COM1", "com9.dat", "LPT1", "lpt9",
        ] {
            let sanitized = sanitize_filename_or_fallback(name);
            assert!(sanitized.starts_with('_'), "{name:?} -> {sanitized:?}");
        }
        // Near-misses are NOT reserved.
        assert_eq!(sanitize_filename("COM10").as_deref(), Some("COM10"));
        assert_eq!(
            sanitize_filename("console.txt").as_deref(),
            Some("console.txt")
        );
    }

    #[test]
    fn trailing_dots_and_spaces_are_stripped() {
        assert_eq!(sanitize_filename("foo.").as_deref(), Some("foo"));
        assert_eq!(sanitize_filename("foo...").as_deref(), Some("foo"));
        assert_eq!(sanitize_filename("foo ").as_deref(), Some("foo"));
        assert_eq!(sanitize_filename("foo... ").as_deref(), Some("foo"));
        assert_eq!(sanitize_filename("foo. . .").as_deref(), Some("foo"));
    }

    #[test]
    fn length_cap_preserves_safe_extensions() {
        let long = format!("{}.txt", "b".repeat(200));
        let sanitized = sanitize_filename_or_fallback(&long);
        assert_eq!(sanitized.chars().count(), MAX_FILENAME_LEN);
        assert!(sanitized.ends_with(".txt"));
        // Unsafe (non-alphanumeric) extensions are dropped, not kept.
        assert_eq!(sanitize_filename("foo.ex$e").as_deref(), Some("foo"));
        // Over-long extensions are dropped with the stem kept.
        let bad_ext = format!("foo.{}", "e".repeat(32));
        assert_eq!(sanitize_filename(&bad_ext).as_deref(), Some("foo"));
        // Leading-dot names are legal bare filenames and pass through.
        assert_eq!(sanitize_filename(".txt").as_deref(), Some(".txt"));
    }

    #[test]
    fn duplicates_uniquify_without_overwriting() {
        let root = temp_root("dupes");
        let dir = ensure_downloads_dir(&root).expect("dir must build");
        std::fs::write(dir.join("foo.txt"), b"original").unwrap();
        let second = resolve_unique_destination(&dir, "foo.txt").expect("second must resolve");
        assert_eq!(second.file_name().unwrap(), "foo (2).txt");
        std::fs::write(&second, b"copy").unwrap();
        let third = resolve_unique_destination(&dir, "foo.txt").expect("third must resolve");
        assert_eq!(third.file_name().unwrap(), "foo (3).txt");
        // Extensionless names uniquify too.
        std::fs::write(dir.join("LICENSE"), b"x").unwrap();
        let second_plain = resolve_unique_destination(&dir, "LICENSE").expect("must resolve");
        assert_eq!(second_plain.file_name().unwrap(), "LICENSE (2)");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn duplicate_exhaustion_denies() {
        let root = temp_root("exhaust");
        let dir = ensure_downloads_dir(&root).expect("dir must build");
        std::fs::write(dir.join("x.bin"), b"x").unwrap();
        for index in 2..=MAX_DUPLICATE_ATTEMPTS + 1 {
            std::fs::write(dir.join(format!("x ({index}).bin")), b"x").unwrap();
        }
        assert_eq!(resolve_unique_destination(&dir, "x.bin"), None);
        assert_eq!(
            decide_download(&dir, Path::new("C:\\Users\\victim\\x.bin")),
            DownloadDecision::Deny
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn containment_holds_for_every_resolution() {
        let root = temp_root("contain");
        let dir = ensure_downloads_dir(&root).expect("dir must build");
        for hostile in [
            "C:\\Windows\\foo.txt",
            "..\\foo.txt",
            "\\\\server\\share\\foo.txt",
            "test.txt",
            "foo:bar.txt",
        ] {
            match decide_download(&dir, Path::new(hostile)) {
                DownloadDecision::Allow(destination) => {
                    assert_eq!(destination.parent(), Some(dir.as_path()));
                }
                DownloadDecision::Deny => {}
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn downloads_dir_rejects_symlinks_and_files() {
        let root = temp_root("dirshape");
        let dir = ensure_downloads_dir(&root).expect("dir must build");
        assert!(dir.ends_with(DOWNLOADS_DIR_NAME));
        // A regular file at the downloads path fails closed.
        let file_root = temp_root("dirfile");
        std::fs::write(file_root.join(DOWNLOADS_DIR_NAME), b"x").unwrap();
        assert!(ensure_downloads_dir(&file_root).is_err());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&file_root);
    }

    #[test]
    fn url_redaction_drops_query_fragment_and_userinfo() {
        let redacted = redact_url(
            "https://user:pass@cdn.discordapp.com/attachments/1/2/test.png?token=secret#frag",
        );
        assert_eq!(
            redacted,
            "https://cdn.discordapp.com/attachments/1/2/test.png"
        );
        assert_eq!(redact_url("blob:https://discord.com/uuid"), "blob:redacted");
        assert_eq!(
            redact_url("not a url [[[{{{ James Bond }}}}"),
            "unparseable-url"
        );
        assert!(!redact_url("https://x.test/f?token=abc").contains("token"));
    }

    #[test]
    fn shipped_module_has_no_forbidden_surfaces() {
        let source = include_str!("downloads.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "ShellExecuteW",
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
            "legcord",
            "Command::new",
            "std::process",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
    }
}

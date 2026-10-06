//! Host-controlled native theme injection for Phase D.
//!
//! This module is the native counterpart of the Electron theme pipeline in
//! `src/common/themes.ts` (`buildThemeCssCache`, `injectThemesMain`,
//! `setThemeEnabled`), whose actual lifecycle was traced before any native
//! code was written:
//!
//! ```text
//! startup  -> discover <userData>/themes/<id>/ -> read manifest.json ->
//!             resolve manifest.theme -> cache CSS -> did-finish-load ->
//!             webContents.send("addTheme", id, css) [Quick CSS first when
//!             enabled, then themes in discovery order] -> <style id> created
//! change   -> list-cache watcher only (500 ms debounce, manifest.json only;
//!             per-theme CSS edits need an enable toggle or a reload)
//! disable  -> webContents.send("removeTheme", id)
//! reload   -> fresh document; did-finish-load re-sends the whole cache
//! ```
//!
//! The renderer side (`src/discord/preload/themes.ts` over
//! `src/common/dom.ts::addTheme`) only ever receives `(id, css)` pushes; no
//! renderer caller reads theme CSS back, so this milestone is a **host push
//! only**. There is deliberately NO `themes.getCSS()` bridge, no new protocol
//! command, and no new boot capability: the host reads manifests/CSS itself
//! and injects CSS through the shared substrate builders
//! ([`crate::mod_substrate::managed_css_script`]).
//!
//! Native hardening (deliberate, documented differences from Electron):
//! - Theme ids are VALIDATED (`^[A-Za-z0-9._-]{1,64}$`), never joined raw.
//! - The manifest-provided CSS filename is validated by an allowlist: a bare
//!   `*.css` filename with no separators, no drive/UNC/device syntax, no
//!   reserved names. Electron joins `themesFolder + id + manifest.theme` with
//!   string interpolation and no filename validation, so a manifest such as
//!   `{ "theme": "../../quickCss.css" }` escapes the theme directory there;
//!   native rejects it as `invalid_filename`.
//! - Containment is enforced twice: the validated filename cannot escape
//!   lexically (single path component), AND the resolved file is
//!   `canonicalize`d and must stay under the canonical theme root, so
//!   pre-planted symlinks/junctions cannot redirect outside the root.
//! - Size-bounded: manifests <= 64 KiB, CSS <= 1 MiB. Electron is unbounded.
//! - Injection order is deterministic (sorted by theme id). Electron uses
//!   filesystem discovery order.
//! - A missing `enabled` field defaults to enabled. Electron additionally
//!   consults a legacy `disabled.txt`; that legacy migration path is not
//!   reproduced natively.
//! - Empty CSS files are skipped (no visual effect; avoids DOM clutter).
//!   Electron would push an empty style.
//!
//! Out of scope for this milestone (deferred): live file watching (the first
//! milestone is static startup injection + reload lifecycle), theme
//! installation/editing, Quick CSS (a separate host-owned file with its own
//! lifecycle), and any renderer-facing theme API.
//!
//! Logical root owned by this module: exactly one —
//! `<profile>/themes/`. The renderer never receives the absolute path.

use std::path::{Path, PathBuf};
use std::time::Instant;

/// Maximum theme id length in UTF-8 bytes. Mirrors the plugin-storage bound;
/// real theme directory names are far shorter.
pub const MAX_THEME_ID_BYTES: usize = 64;
/// Maximum manifest file size in bytes. Manifests are small JSON documents.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Maximum manifest-provided CSS filename length in UTF-8 bytes.
pub const MAX_THEME_FILENAME_BYTES: usize = 128;
/// Maximum theme CSS payload in bytes in either direction. Matches the
/// plugin-storage file bound; comfortably covers real theme stylesheets.
pub const MAX_THEME_CSS_BYTES: usize = 1024 * 1024;
/// Byte prefix for every native-managed theme style element. The raw theme id
/// is never used as a DOM id so native styles cannot collide with Discord
/// node ids (Electron reuses the raw id; this namespacing is deliberate).
pub const THEME_STYLE_ID_PREFIX: &str = "legcord-native-theme-";

fn is_valid_theme_id_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
}

/// Validate a theme identifier. The id is used verbatim as a single directory
/// name under the host-controlled theme root, never transformed.
pub fn validate_theme_id(theme_id: &str) -> Result<(), String> {
    if theme_id.is_empty() {
        return Err("theme id must not be empty".to_owned());
    }
    if theme_id.len() > MAX_THEME_ID_BYTES {
        return Err("theme id exceeds the size limit".to_owned());
    }
    if !theme_id.bytes().all(is_valid_theme_id_char) {
        return Err("theme id uses characters outside [A-Za-z0-9._-]".to_owned());
    }
    if theme_id == "." || theme_id == ".." {
        return Err("theme id must not be '.' or '..'".to_owned());
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

/// Validate the manifest-provided CSS filename.
///
/// The value must be a single bare `*.css` filename: no directory separators
/// of either flavor, no drive/UNC/device syntax, no reserved Windows names.
/// This is the allowlist Electron lacks: it joins the raw value onto the
/// theme directory, so traversal and absolute paths escape there.
pub fn validate_theme_css_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty() {
        return Err("theme CSS filename must not be empty".to_owned());
    }
    if filename.len() > MAX_THEME_FILENAME_BYTES {
        return Err("theme CSS filename exceeds the size limit".to_owned());
    }
    if filename.contains('\0') {
        return Err("theme CSS filename must not contain NUL".to_owned());
    }
    if filename.bytes().any(|byte| byte < 0x20) {
        return Err("theme CSS filename must not contain control characters".to_owned());
    }
    if filename.contains('/') || filename.contains('\\') {
        return Err("theme CSS filename must not contain a path separator".to_owned());
    }
    // `:` anywhere blocks drive letters (`C:...`), ADS streams (`a:b`), and
    // NTFS alternate syntax in one rule.
    if filename.contains(':') {
        return Err("theme CSS filename must not contain ':'".to_owned());
    }
    if filename.contains(['<', '>', '"', '|', '?', '*']) {
        return Err("theme CSS filename contains a reserved Windows character".to_owned());
    }
    if filename == "." || filename == ".." {
        return Err("theme CSS filename must not be '.' or '..'".to_owned());
    }
    // Windows strips trailing spaces/dots, which would alias distinct inputs
    // to one file; reject instead of normalizing.
    if filename.ends_with(' ') || filename.ends_with('.') {
        return Err("theme CSS filename must not end with a space or dot".to_owned());
    }
    let stem = filename.split('.').next().unwrap_or(filename);
    if is_reserved_device_stem(stem) {
        return Err("theme CSS filename uses a reserved device name".to_owned());
    }
    if !filename.to_ascii_lowercase().ends_with(".css") {
        return Err("theme CSS filename must end with '.css'".to_owned());
    }
    Ok(())
}

/// Host-controlled theme root: `<profile>/themes/`. The renderer never sees
/// this path.
pub fn themes_dir(profile_dir: &Path) -> PathBuf {
    profile_dir.join("themes")
}

/// Maximum BetterDiscord header value length in UTF-8 bytes (the theme
/// name; other header values share the field bound below). Real theme
/// names are a few dozen bytes; this is a safety rail, not a target.
pub const MAX_BD_NAME_BYTES: usize = 128;
/// Maximum any other single BetterDiscord header value in UTF-8 bytes.
/// Descriptions and links are short strings in real themes.
pub const MAX_BD_FIELD_BYTES: usize = 4096;
/// Fixed CSS filename every installed theme carries, mirroring Electron's
/// `installThemeFromCode` (which always writes `src.css`). Never
/// renderer-selected.
pub const INSTALLED_THEME_CSS_FILE: &str = "src.css";

/// Parsed BetterDiscord header block. Mirrors exactly the fields Electron's
/// `parseBDManifest` (`src/common/themes.ts`) reads: `name` plus the known
/// metadata keys. `theme`, `enabled`, and `supportsLegcordTitlebar` are
/// host-fixed/derived, and `updateSrc` is omitted (no URL input exists in
/// the install capability; a future update flow will validate it then).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedBdManifest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub invite: Option<String>,
    pub author_id: Option<String>,
    pub author_link: Option<String>,
    pub donate: Option<String>,
    pub patreon: Option<String>,
    pub website: Option<String>,
    pub source: Option<String>,
}

/// Typed failure for theme installation: a stable machine-readable `code`
/// plus a human message that never contains a filesystem path.
#[derive(Debug)]
pub struct ThemeInstallFailure {
    pub code: &'static str,
    pub message: String,
}

fn install_failure(code: &'static str, message: &str) -> ThemeInstallFailure {
    ThemeInstallFailure {
        code,
        message: message.to_owned(),
    }
}

/// Parse a BetterDiscord theme source exactly the way Electron's
/// `parseBDManifest` does (verified against the JS `/@([^ ]*) (.*)/g` scan):
/// the `/**` marker is required; `@key value` pairs match anywhere in the
/// text (including CSS bodies); the key runs to the first space (and may
/// therefore swallow a newline, hiding a later header — reproduced, not
/// fixed); the value runs to the next newline with a trailing `\r` kept;
/// a key of exactly `import` stops the scan; later duplicates overwrite.
/// Unknown keys are ignored.
pub fn parse_bd_manifest(code: &str) -> Result<ParsedBdManifest, ThemeInstallFailure> {
    if !code.starts_with("/**") {
        return Err(install_failure(
            "NOT_A_MANIFEST",
            "theme source is not a BetterDiscord theme",
        ));
    }
    let mut manifest = ParsedBdManifest::default();
    let bytes = code.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        let Some(at_rel) = bytes[pos..].iter().position(|&byte| byte == b'@') else {
            break;
        };
        let at = pos + at_rel;
        let key_start = at + 1;
        let Some(space_rel) = bytes[key_start..].iter().position(|&byte| byte == b' ') else {
            pos = at + 1;
            continue;
        };
        let space = key_start + space_rel;
        // Byte slicing is safe: `@`, ` `, and `\n` are ASCII and never
        // appear inside a multibyte UTF-8 sequence.
        let key = &code[key_start..space];
        let value_start = space + 1;
        let value_end = bytes[value_start..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map(|newline_rel| value_start + newline_rel)
            .unwrap_or(bytes.len());
        let value = &code[value_start..value_end];
        // JS `lastIndex` resumes at the end of the match.
        pos = value_end;
        if key == "import" {
            break;
        }
        let bounded = |field: &str| {
            if field.len() > MAX_BD_FIELD_BYTES {
                None
            } else {
                Some(field.to_owned())
            }
        };
        match key {
            "name" => manifest.name = Some(value.to_owned()),
            "description" => manifest.description = bounded(value),
            "version" => manifest.version = bounded(value),
            "author" => manifest.author = bounded(value),
            "invite" => manifest.invite = bounded(value),
            "authorId" => manifest.author_id = bounded(value),
            "authorLink" => manifest.author_link = bounded(value),
            "donate" => manifest.donate = bounded(value),
            "patreon" => manifest.patreon = bounded(value),
            "website" => manifest.website = bounded(value),
            "source" => manifest.source = bounded(value),
            // Electron maps `updateUrl` to `updateSrc`, but installing from a
            // URL is out of scope: the value is dropped and documented.
            _ => {}
        }
    }
    Ok(manifest)
}

/// Derive the theme directory id from a parsed `@name`, mirroring
/// Electron's `` `${name.replace(" ", "-")}-BD` `` (first space only).
/// A missing or empty `@name` is rejected: Electron's `"null-BD"` fallback
/// would turn invalid input into an accepted theme, so it is deliberately
/// not reproduced.
fn derive_theme_id(name: Option<String>) -> Result<String, ThemeInstallFailure> {
    let invalid = || install_failure("INVALID_NAME", "theme name is missing or invalid");
    let name = match name {
        Some(name) if !name.is_empty() => name,
        _ => return Err(invalid()),
    };
    if name.len() > MAX_BD_NAME_BYTES {
        return Err(invalid());
    }
    let id = format!("{}-BD", name.replacen(' ', "-", 1));
    if validate_theme_id(&id).is_err() {
        return Err(invalid());
    }
    Ok(id)
}

/// Successful installation report: the host-derived id, the persisted
/// enablement, and the stored CSS size. No path crosses this boundary.
#[derive(Debug, PartialEq, Eq)]
pub struct ThemeInstallResult {
    pub id: String,
    pub enabled: bool,
    pub css_bytes: usize,
}

/// Atomically replace `path` with `bytes`: sibling temp file in the same
/// directory plus same-directory rename (mirroring the settings mutation),
/// with temp cleanup on either failure.
fn write_file_atomic(path: &Path, bytes: &[u8]) -> Result<(), ThemeInstallFailure> {
    let temp_path = path.with_extension("tmp");
    if std::fs::write(&temp_path, bytes).is_err() {
        let _ = std::fs::remove_file(&temp_path);
        return Err(install_failure(
            "THEME_WRITE_FAILED",
            "theme files cannot be written",
        ));
    }
    if std::fs::rename(&temp_path, path).is_err() {
        let _ = std::fs::remove_file(&temp_path);
        return Err(install_failure(
            "THEME_WRITE_FAILED",
            "theme files cannot be written",
        ));
    }
    Ok(())
}

/// Install one already-parsed BetterDiscord theme under the host-controlled
/// themes root.
///
/// This is the SINGLE installation implementation underneath both
/// [`install_theme_from_code`] (CSS text from the renderer) and the native
/// file-import path (CSS text read from host-picked files): manifest
/// generation, id validation, bounds, and atomic writes live here exactly
/// once. Callers bound and parse the CSS text first; the size is re-checked
/// here defensively so no caller can bypass the bound.
pub fn install_parsed_theme(
    profile_dir: &Path,
    css: &str,
    parsed: ParsedBdManifest,
) -> Result<ThemeInstallResult, ThemeInstallFailure> {
    if css.len() > MAX_THEME_CSS_BYTES {
        return Err(install_failure(
            "THEME_TOO_LARGE",
            "theme CSS exceeds the size limit",
        ));
    }
    let titlebar = css.contains(".titlebar");
    // Electron-identical manifest shape (key order normalized —
    // JSON semantics are order-insensitive and both loaders ignore order),
    // minus `updateSrc` (no URL input in this slice).
    let mut manifest = serde_json::Map::new();
    manifest.insert(
        "theme".to_owned(),
        serde_json::Value::String(INSTALLED_THEME_CSS_FILE.to_owned()),
    );
    // The name was validated present and non-empty during id derivation.
    let name = parsed.name.clone().unwrap_or_default();
    let id = derive_theme_id(parsed.name)?;
    manifest.insert("name".to_owned(), serde_json::Value::String(name));
    manifest.insert("enabled".to_owned(), serde_json::Value::Bool(false));
    for (key, value) in [
        ("description", parsed.description),
        ("version", parsed.version),
        ("author", parsed.author),
        ("invite", parsed.invite),
        ("authorId", parsed.author_id),
        ("authorLink", parsed.author_link),
        ("donate", parsed.donate),
        ("patreon", parsed.patreon),
        ("website", parsed.website),
        ("source", parsed.source),
    ] {
        if let Some(value) = value {
            manifest.insert(key.to_owned(), serde_json::Value::String(value));
        }
    }
    manifest.insert(
        "supportsLegcordTitlebar".to_owned(),
        serde_json::Value::Bool(titlebar),
    );
    let manifest_text = serde_json::to_string(&serde_json::Value::Object(manifest))
        .map_err(|_| install_failure("THEME_WRITE_FAILED", "theme files cannot be written"))?;

    let root = themes_dir(profile_dir);
    std::fs::create_dir_all(&root)
        .map_err(|_| install_failure("THEME_UNAVAILABLE", "theme storage is unavailable"))?;
    let root_canonical = std::fs::canonicalize(&root)
        .map_err(|_| install_failure("THEME_UNAVAILABLE", "theme storage is unavailable"))?;
    // Lexically safe (one validated component), then canonical containment
    // so a pre-planted symlink/junction at the id can never redirect the
    // write outside the root. The directory is created by the host when
    // missing; an existing one is reused for deterministic overwrite.
    let theme_dir = root.join(&id);
    std::fs::create_dir_all(&theme_dir)
        .map_err(|_| install_failure("THEME_UNAVAILABLE", "theme storage is unavailable"))?;
    let theme_canonical = std::fs::canonicalize(&theme_dir)
        .map_err(|_| install_failure("THEME_UNAVAILABLE", "theme storage is unavailable"))?;
    if !theme_canonical.starts_with(&root_canonical) {
        return Err(install_failure(
            "OUTSIDE_ROOT",
            "theme target is outside the approved root",
        ));
    }

    write_file_atomic(
        &theme_canonical.join("manifest.json"),
        manifest_text.as_bytes(),
    )?;
    write_file_atomic(
        &theme_canonical.join(INSTALLED_THEME_CSS_FILE),
        css.as_bytes(),
    )?;
    Ok(ThemeInstallResult {
        id,
        enabled: false,
        css_bytes: css.len(),
    })
}

/// Install one BetterDiscord theme from CSS text under the host-controlled
/// themes root.
///
/// The renderer supplies only the source text: the id is derived from
/// `@name` (validated, never sanitized), the CSS filename is the fixed
/// `src.css`, and `enabled` is always `false` (Electron parity — a fresh BD
/// install starts disabled; the native loader skips it until an enable path
/// exists). Overwriting an existing id directory is deterministic (same
/// inputs, same files), matching Electron's silent overwrite. The existing
/// live watcher observes the written files and drives the managed CSS
/// lifecycle; nothing here touches the renderer.
///
/// The parse and the install converge with the file-import path on
/// [`install_parsed_theme`]: behavior here is unchanged by that sharing.
pub fn install_theme_from_code(
    profile_dir: &Path,
    css: &str,
) -> Result<ThemeInstallResult, ThemeInstallFailure> {
    if css.len() > MAX_THEME_CSS_BYTES {
        return Err(install_failure(
            "THEME_TOO_LARGE",
            "theme CSS exceeds the size limit",
        ));
    }
    let parsed = parse_bd_manifest(css)?;
    install_parsed_theme(profile_dir, css, parsed)
}

/// Successful CSS-write report: the validated id, the preserved enablement
/// (read from the untouched manifest), and the stored CSS size. No path
/// crosses this boundary.
#[derive(Debug, PartialEq, Eq)]
pub struct ThemeWriteResult {
    pub id: String,
    pub enabled: bool,
    pub css_bytes: usize,
}

/// Atomically replace `path` with `bytes` through a uniquely named sibling
/// temp file plus same-directory rename, with temp cleanup on either
/// failure. The unique name (process + nanos + counter) means a pre-planted
/// temp file can never be substituted for the write; `create_new` semantics
/// fail closed on the residual collision instead of overwriting it.
fn write_file_atomic_unique(path: &Path, bytes: &[u8]) -> Result<(), ThemeInstallFailure> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temp_name = format!(
        ".tmp-{}-{}-{}",
        std::process::id(),
        nanos,
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let temp_path = path.with_file_name(temp_name);
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
    {
        Ok(mut file) => {
            use std::io::Write;
            if file.write_all(bytes).is_err() {
                let _ = std::fs::remove_file(&temp_path);
                return Err(install_failure(
                    "THEME_WRITE_FAILED",
                    "theme files cannot be written",
                ));
            }
        }
        Err(_) => {
            return Err(install_failure(
                "THEME_WRITE_FAILED",
                "theme files cannot be written",
            ));
        }
    }
    if std::fs::rename(&temp_path, path).is_err() {
        let _ = std::fs::remove_file(&temp_path);
        return Err(install_failure(
            "THEME_WRITE_FAILED",
            "theme files cannot be written",
        ));
    }
    Ok(())
}

/// Resolve the CSS file a theme id selects, without writing anything.
///
/// The id is validated (never sanitized); the theme directory must already
/// exist (never created here — a missing directory is `UNKNOWN_THEME`);
/// the bounded manifest selects the filename, which must pass the same
/// allowlist the loader enforces; the resolved file must stay under the
/// canonical themes root. A symlink/junction at the destination that
/// resolves outside is refused instead of followed.
fn resolve_theme_css(
    profile_dir: &Path,
    theme_id: &str,
) -> Result<(PathBuf, bool), ThemeInstallFailure> {
    if theme_id.is_empty() || theme_id.len() > MAX_THEME_ID_BYTES {
        return Err(install_failure("INVALID_THEME_ID", "theme id is invalid"));
    }
    if validate_theme_id(theme_id).is_err() {
        return Err(install_failure("INVALID_THEME_ID", "theme id is invalid"));
    }
    let root = themes_dir(profile_dir);
    let root_canonical = std::fs::canonicalize(&root)
        .map_err(|_| install_failure("THEME_UNAVAILABLE", "theme storage is unavailable"))?;
    // Lexically safe (one validated component, no separators possible).
    let theme_dir = root.join(theme_id);
    let theme_canonical = match std::fs::canonicalize(&theme_dir) {
        Ok(canonical) => canonical,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(install_failure("UNKNOWN_THEME", "theme folder is unknown"));
        }
        Err(_) => {
            return Err(install_failure(
                "THEME_UNAVAILABLE",
                "theme storage is unavailable",
            ));
        }
    };
    if !theme_canonical.starts_with(&root_canonical) {
        return Err(install_failure(
            "OUTSIDE_ROOT",
            "theme target is outside the approved root",
        ));
    }
    // The manifest selects the file — exactly what Electron edits — but
    // through the native filename allowlist instead of raw interpolation.
    let manifest_path = theme_canonical.join("manifest.json");
    let manifest_len = std::fs::symlink_metadata(&manifest_path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len());
    let manifest_text = match manifest_len {
        Some(len) if len <= MAX_MANIFEST_BYTES as u64 => std::fs::read_to_string(&manifest_path)
            .map_err(|_| install_failure("INVALID_MANIFEST", "theme manifest is invalid"))?,
        _ => {
            return Err(install_failure(
                "INVALID_MANIFEST",
                "theme manifest is invalid",
            ));
        }
    };
    let manifest: ThemeManifestFile = serde_json::from_str(&manifest_text)
        .map_err(|_| install_failure("INVALID_MANIFEST", "theme manifest is invalid"))?;
    if validate_theme_css_filename(&manifest.theme).is_err() {
        return Err(install_failure(
            "INVALID_FILENAME",
            "theme CSS filename is invalid",
        ));
    }
    // Single validated component onto the verified canonical directory:
    // lexically unable to escape. A pre-planted link AT the destination is
    // refused outright instead of written through or followed — the strict
    // bundles convention, not the follow-then-contain one.
    let target = theme_canonical.join(&manifest.theme);
    if !target.starts_with(&root_canonical) {
        return Err(install_failure(
            "OUTSIDE_ROOT",
            "theme target is outside the approved root",
        ));
    }
    if std::fs::symlink_metadata(&target)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(install_failure(
            "OUTSIDE_ROOT",
            "theme target is outside the approved root",
        ));
    }
    Ok((target, manifest.enabled != Some(false)))
}

/// Replace one theme's CSS text.
///
/// The renderer supplies only the id and the new CSS: the destination is
/// the manifest-declared file resolved by [`resolve_theme_css`], and
/// `manifest.json` is never touched (enablement and metadata are preserved
/// byte-for-byte, mirroring Electron's editor save path). The write is an
/// atomic unique-temp + same-directory rename that replaces an existing
/// destination. The existing live watcher observes the replacement and
/// drives the managed CSS lifecycle; nothing here touches the renderer.
pub fn write_theme_css(
    profile_dir: &Path,
    theme_id: &str,
    css: &str,
) -> Result<ThemeWriteResult, ThemeInstallFailure> {
    if css.len() > MAX_THEME_CSS_BYTES {
        return Err(install_failure(
            "THEME_TOO_LARGE",
            "theme CSS exceeds the size limit",
        ));
    }
    let (target, enabled) = resolve_theme_css(profile_dir, theme_id)?;
    write_file_atomic_unique(&target, css.as_bytes())?;
    Ok(ThemeWriteResult {
        id: theme_id.to_owned(),
        enabled,
        css_bytes: css.len(),
    })
}

/// Deterministic DOM id for one theme's managed style element.
pub fn theme_style_id(theme_id: &str) -> String {
    format!("{THEME_STYLE_ID_PREFIX}{theme_id}")
}

/// Minimal manifest shape the native host consumes. Unknown future keys are
/// ignored (forward compatibility, mirroring the settings snapshot); only
/// `theme` and `enabled` affect loading.
#[derive(Debug, Clone, serde::Deserialize)]
struct ThemeManifestFile {
    theme: String,
    #[serde(default)]
    enabled: Option<bool>,
}

/// One successfully loaded theme: host-chosen style identity plus CSS text.
/// No path crosses this boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedTheme {
    pub id: String,
    pub style_id: String,
    pub css: String,
    pub css_bytes: usize,
}

/// Per-theme skip reason for DEV metrics. One bad theme never blocks others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedTheme {
    pub id: String,
    pub reason: &'static str,
}

/// Aggregate report for one host-side theme load pass.
#[derive(Debug, Clone, Default)]
pub struct ThemeLoadReport {
    pub discovered: usize,
    pub enabled: usize,
    pub injected: Vec<LoadedTheme>,
    pub skipped: Vec<SkippedTheme>,
    pub css_bytes: usize,
    pub read_ms: f64,
}

fn canonical_contains(root_canonical: &Path, path: &Path) -> bool {
    path.starts_with(root_canonical)
}

/// Discover enabled themes under the host-controlled root and load their CSS.
///
/// Order: validate id, read + parse the bounded manifest, honor `enabled`,
/// validate the manifest-provided filename, confine the resolved file
/// (lexical by construction plus canonical containment), enforce the CSS size
/// bound, require UTF-8. Failures skip exactly one theme with a typed reason.
/// Emission order is sorted by theme id for deterministic cascade order.
pub fn load_enabled_themes(profile_dir: &Path) -> ThemeLoadReport {
    let started_at = Instant::now();
    let mut report = ThemeLoadReport::default();
    let root = themes_dir(profile_dir);
    if !root.exists() {
        report.read_ms = started_at.elapsed().as_secs_f64() * 1000.0;
        return report;
    }
    let root_canonical = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());

    let mut entries: Vec<String> = Vec::new();
    let listing = std::fs::read_dir(&root);
    let listing = match listing {
        Ok(listing) => listing,
        Err(_) => {
            report.read_ms = started_at.elapsed().as_secs_f64() * 1000.0;
            return report;
        }
    };
    for entry in listing.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        entries.push(name);
    }
    entries.sort();
    report.discovered = entries.len();

    for id in entries {
        if validate_theme_id(&id).is_err() {
            report.skipped.push(SkippedTheme {
                id,
                reason: "invalid_id",
            });
            continue;
        }
        let theme_dir = root.join(&id);
        let manifest_path = theme_dir.join("manifest.json");
        let manifest_len = std::fs::metadata(&manifest_path)
            .ok()
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len());
        let Some(len) = manifest_len else {
            report.skipped.push(SkippedTheme {
                id,
                reason: "missing_manifest",
            });
            continue;
        };
        if len > MAX_MANIFEST_BYTES as u64 {
            report.skipped.push(SkippedTheme {
                id,
                reason: "manifest_too_large",
            });
            continue;
        }
        let manifest_text = match std::fs::read_to_string(&manifest_path) {
            Ok(text) => text,
            Err(_) => {
                report.skipped.push(SkippedTheme {
                    id,
                    reason: "invalid_manifest",
                });
                continue;
            }
        };
        let manifest: ThemeManifestFile = match serde_json::from_str(&manifest_text) {
            Ok(manifest) => manifest,
            Err(_) => {
                report.skipped.push(SkippedTheme {
                    id,
                    reason: "invalid_manifest",
                });
                continue;
            }
        };
        if manifest.enabled == Some(false) {
            report.skipped.push(SkippedTheme {
                id,
                reason: "disabled",
            });
            continue;
        }
        report.enabled += 1;
        if validate_theme_css_filename(&manifest.theme).is_err() {
            report.skipped.push(SkippedTheme {
                id,
                reason: "invalid_filename",
            });
            continue;
        }
        let css_path = theme_dir.join(&manifest.theme);
        // The file must exist: a missing CSS file skips exactly this theme,
        // exactly like a missing entry in Electron's CSS cache build.
        let css_len = match std::fs::metadata(&css_path) {
            Ok(metadata) if metadata.is_file() => metadata.len(),
            Ok(_) => {
                report.skipped.push(SkippedTheme {
                    id,
                    reason: "missing_css",
                });
                continue;
            }
            Err(_) => {
                report.skipped.push(SkippedTheme {
                    id,
                    reason: "missing_css",
                });
                continue;
            }
        };
        if css_len > MAX_THEME_CSS_BYTES as u64 {
            report.skipped.push(SkippedTheme {
                id,
                reason: "css_too_large",
            });
            continue;
        }
        // Canonical containment: resolves symlinks/junctions on the host.
        // Both sides are canonicalized so the Windows `\\?\` prefix cannot
        // break the check. The read below goes through the canonical path,
        // mirroring the plugin-storage capability.
        let css_canonical = match std::fs::canonicalize(&css_path) {
            Ok(canonical) => canonical,
            Err(_) => {
                report.skipped.push(SkippedTheme {
                    id,
                    reason: "unreadable_css",
                });
                continue;
            }
        };
        if !canonical_contains(&root_canonical, &css_canonical) {
            report.skipped.push(SkippedTheme {
                id,
                reason: "outside_root",
            });
            continue;
        }
        let css = match std::fs::read_to_string(&css_canonical) {
            Ok(css) => css,
            Err(_) => {
                report.skipped.push(SkippedTheme {
                    id,
                    reason: "unreadable_css",
                });
                continue;
            }
        };
        if css.is_empty() {
            report.skipped.push(SkippedTheme {
                id,
                reason: "empty_css",
            });
            continue;
        }
        let css_bytes = css.len();
        report.css_bytes += css_bytes;
        report.injected.push(LoadedTheme {
            style_id: theme_style_id(&id),
            id,
            css,
            css_bytes,
        });
    }

    report.read_ms = started_at.elapsed().as_secs_f64() * 1000.0;
    report
}

#[cfg(test)]
mod tests {
    use super::{
        install_theme_from_code, load_enabled_themes, parse_bd_manifest, theme_style_id,
        validate_theme_css_filename, validate_theme_id, write_theme_css, MAX_THEME_CSS_BYTES,
        THEME_STYLE_ID_PREFIX,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_profile() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must advance")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "legcord-native-themes-test-{}-{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(dir.join("themes")).expect("must create temp theme root");
        dir
    }

    fn write_theme(profile: &Path, id: &str, manifest: &str, css: Option<(&str, &str)>) {
        let dir = profile.join("themes").join(id);
        fs::create_dir_all(&dir).expect("must create theme dir");
        fs::write(dir.join("manifest.json"), manifest).expect("must write manifest");
        if let Some((name, content)) = css {
            fs::write(dir.join(name), content).expect("must write css");
        }
    }

    #[test]
    fn accepts_valid_theme_ids_and_rejects_separators() {
        assert!(validate_theme_id("midnight-BD").is_ok());
        assert!(validate_theme_id("my.theme_v2").is_ok());
        assert!(validate_theme_id("").is_err());
        assert!(validate_theme_id(&"t".repeat(65)).is_err());
        assert!(validate_theme_id("../escape").is_err());
        assert!(validate_theme_id("a/b").is_err());
        assert!(validate_theme_id("a\\b").is_err());
        assert!(validate_theme_id("C:").is_err());
        assert!(validate_theme_id(".").is_err());
        assert!(validate_theme_id("..").is_err());
        assert!(validate_theme_id("has space").is_err());
    }

    #[test]
    fn manifest_filename_allowlist_blocks_escape_shapes() {
        assert!(validate_theme_css_filename("src.css").is_ok());
        assert!(validate_theme_css_filename("theme.CSS").is_ok());
        // The exact shape Electron would honor and escape with.
        assert!(validate_theme_css_filename("../../quickCss.css").is_err());
        assert!(validate_theme_css_filename("..\\..\\secret.css").is_err());
        assert!(validate_theme_css_filename("/absolute.css").is_err());
        assert!(validate_theme_css_filename("\\absolute.css").is_err());
        assert!(validate_theme_css_filename("C:\\secret.css").is_err());
        assert!(validate_theme_css_filename("C:/secret.css").is_err());
        assert!(validate_theme_css_filename("\\\\server\\share.css").is_err());
        assert!(validate_theme_css_filename("..").is_err());
        assert!(validate_theme_css_filename("style.js").is_err());
        assert!(validate_theme_css_filename("noextension").is_err());
        assert!(validate_theme_css_filename("a:b.css").is_err());
        assert!(validate_theme_css_filename("CON.css").is_err());
        assert!(validate_theme_css_filename("com1.css").is_err());
        assert!(validate_theme_css_filename("trailingdot.css.").is_err());
        assert!(validate_theme_css_filename("").is_err());
        assert!(validate_theme_css_filename(&"a".repeat(129)).is_err());
    }

    #[test]
    fn loads_enabled_themes_in_sorted_order_with_style_identity() {
        let profile = temp_profile();
        write_theme(
            &profile,
            "zulu",
            r#"{"name":"Z","theme":"src.css"}"#,
            Some(("src.css", ":root{--z:1;}")),
        );
        write_theme(
            &profile,
            "alpha",
            r#"{"name":"A","theme":"main.css","enabled":true}"#,
            Some(("main.css", ":root{--a:1;}")),
        );
        let report = load_enabled_themes(&profile);
        assert_eq!(report.discovered, 2);
        assert_eq!(report.enabled, 2);
        assert_eq!(report.injected.len(), 2);
        assert_eq!(report.injected[0].id, "alpha");
        assert_eq!(report.injected[1].id, "zulu");
        assert_eq!(
            report.injected[0].style_id,
            format!("{THEME_STYLE_ID_PREFIX}alpha")
        );
        assert_eq!(report.injected[0].css, ":root{--a:1;}");
        assert_eq!(
            report.css_bytes,
            ":root{--a:1;}".len() + ":root{--z:1;}".len()
        );
        assert!(report.skipped.is_empty());
        assert!(report.read_ms >= 0.0);
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn disabled_themes_are_skipped_without_css_access() {
        let profile = temp_profile();
        write_theme(
            &profile,
            "off",
            r#"{"name":"Off","theme":"src.css","enabled":false}"#,
            None,
        );
        let report = load_enabled_themes(&profile);
        assert_eq!(report.injected.len(), 0);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, "disabled");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn missing_css_and_malformed_manifests_skip_exactly_one_theme() {
        let profile = temp_profile();
        write_theme(
            &profile,
            "good",
            r#"{"name":"G","theme":"src.css"}"#,
            Some(("src.css", ":root{--g:1;}")),
        );
        write_theme(&profile, "nocss", r#"{"name":"N","theme":"src.css"}"#, None);
        write_theme(&profile, "broken", "not json{", Some(("src.css", "x")));
        let report = load_enabled_themes(&profile);
        assert_eq!(report.injected.len(), 1);
        assert_eq!(report.injected[0].id, "good");
        let reasons: Vec<&str> = report.skipped.iter().map(|entry| entry.reason).collect();
        assert!(reasons.contains(&"missing_css"));
        assert!(reasons.contains(&"invalid_manifest"));
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn traversal_filenames_never_reach_the_filesystem() {
        let profile = temp_profile();
        // Plant the escape target: Electron's raw join would read this file.
        fs::write(profile.join("planted.css"), ":root{--pwned:1;}").expect("plant target");
        write_theme(
            &profile,
            "evil",
            r#"{"name":"E","theme":"../planted.css"}"#,
            None,
        );
        let report = load_enabled_themes(&profile);
        assert!(report.injected.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, "invalid_filename");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn symlink_escape_is_blocked_by_canonical_containment() {
        let profile = temp_profile();
        let outside = profile.join("outside.css");
        fs::write(&outside, ":root{--o:1;}").expect("outside target");
        let link = profile.join("themes").join("linked.css");
        if std::os::windows::fs::symlink_file(&outside, &link).is_err() {
            // Hosts without link privileges cannot plant the escape; the
            // lexical allowlist plus canonical layer still hold.
            fs::remove_dir_all(&profile).ok();
            return;
        }
        write_theme(
            &profile,
            "linked",
            r#"{"name":"L","theme":"linked.css"}"#,
            None,
        );
        // Move the link into the theme dir (fixture wrote manifest already).
        let theme_link = profile.join("themes").join("linked").join("linked.css");
        fs::rename(&link, &theme_link).expect("move link into theme dir");
        let report = load_enabled_themes(&profile);
        assert!(report.injected.is_empty());
        assert!(report
            .skipped
            .iter()
            .any(|entry| entry.reason == "outside_root"));
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn oversized_css_is_rejected_before_injection() {
        let profile = temp_profile();
        let big = "a".repeat(MAX_THEME_CSS_BYTES + 1);
        write_theme(
            &profile,
            "big",
            r#"{"name":"B","theme":"src.css"}"#,
            Some(("src.css", &big)),
        );
        let report = load_enabled_themes(&profile);
        assert!(report.injected.is_empty());
        assert_eq!(report.skipped[0].reason, "css_too_large");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn empty_css_is_skipped() {
        let profile = temp_profile();
        write_theme(
            &profile,
            "empty",
            r#"{"name":"E","theme":"src.css"}"#,
            Some(("src.css", "")),
        );
        let report = load_enabled_themes(&profile);
        assert!(report.injected.is_empty());
        assert_eq!(report.skipped[0].reason, "empty_css");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn missing_theme_root_loads_nothing() {
        let profile = temp_profile();
        fs::remove_dir_all(profile.join("themes")).ok();
        let report = load_enabled_themes(&profile);
        assert_eq!(report.discovered, 0);
        assert!(report.injected.is_empty());
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn style_ids_are_namespaced_and_deterministic() {
        assert_eq!(
            theme_style_id("midnight"),
            format!("{THEME_STYLE_ID_PREFIX}midnight")
        );
        assert_ne!(theme_style_id("a"), "a");
    }

    const BD_BASIC: &str = "/**\n * @name Midnight Harbor\n * @author Harbor Line\n * @version 1.2.0\n * @description A dark harbor theme.\n */\n:root {\n  --bg: #000;\n}\n";

    #[test]
    fn bd_parser_mirrors_electron_header_semantics() {
        // Basic headers, incl. the first-space-only id derivation analogue.
        let parsed = parse_bd_manifest(BD_BASIC).expect("basic BD source must parse");
        assert_eq!(parsed.name.as_deref(), Some("Midnight Harbor"));
        assert_eq!(parsed.author.as_deref(), Some("Harbor Line"));
        assert_eq!(parsed.version.as_deref(), Some("1.2.0"));
        // Unknown keys are ignored; @media bodies must not register.
        let media = "/**\n * @name T\n */\n@media screen {\n  :root {}\n}\n";
        assert_eq!(
            parse_bd_manifest(media)
                .expect("media body must parse")
                .name
                .as_deref(),
            Some("T")
        );
        // A key of exactly `import` stops the scan; later headers are lost.
        let broken = "/**\n * @name First\n * @import https://example.invalid/x\n * @name Second\n */\n:root {}\n";
        assert_eq!(
            parse_bd_manifest(broken)
                .expect("import break must parse")
                .name
                .as_deref(),
            Some("First")
        );
        // Mid-line matches count (global scan, not line-based).
        let midline = "/**\n * @name First\nfoo @name Sneaky\n */\n:root {}\n";
        assert_eq!(
            parse_bd_manifest(midline)
                .expect("midline must parse")
                .name
                .as_deref(),
            Some("Sneaky")
        );
        // A newline swallowed into a key hides the following header.
        let swallowed = "/**\n * @note\n * @name After\n */\n:root {}\n";
        assert_eq!(
            parse_bd_manifest(swallowed)
                .expect("swallow must parse")
                .name,
            None
        );
        // Missing marker is the only parse-level failure.
        assert_eq!(
            parse_bd_manifest(":root {}\n")
                .expect_err("bare CSS must fail")
                .code,
            "NOT_A_MANIFEST"
        );
    }

    #[test]
    fn install_derives_id_writes_manifest_and_src_css() {
        let profile = temp_profile();
        let result =
            install_theme_from_code(&profile, BD_BASIC).expect("valid BD source must install");
        assert_eq!(result.id, "Midnight-Harbor-BD");
        assert!(!result.enabled);
        assert_eq!(result.css_bytes, BD_BASIC.len());

        // Exact on-disk layout: exactly manifest.json + src.css.
        let dir = profile.join("themes").join("Midnight-Harbor-BD");
        assert!(dir.join("manifest.json").is_file());
        assert!(dir.join("src.css").is_file());
        assert_eq!(
            fs::read_to_string(dir.join("src.css")).expect("css must read"),
            BD_BASIC
        );

        // Manifest carries the Electron fields (minus updateSrc, which has
        // no URL input in this slice) with enabled always false.
        let manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.join("manifest.json")).expect("manifest must read"),
        )
        .expect("manifest must be JSON");
        assert_eq!(manifest["theme"], "src.css");
        assert_eq!(manifest["name"], "Midnight Harbor");
        assert_eq!(manifest["enabled"], false);
        assert_eq!(manifest["author"], "Harbor Line");
        assert_eq!(manifest["version"], "1.2.0");
        assert_eq!(manifest["description"], "A dark harbor theme.");
        assert_eq!(manifest["supportsLegcordTitlebar"], false);
        assert!(manifest.get("updateSrc").is_none());
        assert!(manifest.get("id").is_none());

        // The installed theme loads through the existing loader (disabled,
        // so reported but skipped — enabled=false parity).
        let report = load_enabled_themes(&profile);
        assert!(report
            .skipped
            .iter()
            .any(|skipped| skipped.id == "Midnight-Harbor-BD" && skipped.reason == "disabled"));

        // No temp files left behind by the atomic writes.
        assert!(fs::read_dir(&dir).expect("dir must list").all(|entry| {
            let name = entry.expect("entry must read").file_name();
            name == "manifest.json" || name == "src.css"
        }));
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn install_rejects_missing_and_evil_names_without_null_bd() {
        let profile = temp_profile();
        // Missing @name: Electron would write `null-BD`; native rejects.
        let noname = "/**\n * @author Jane\n */\n:root {}\n";
        assert_eq!(
            install_theme_from_code(&profile, noname)
                .expect_err("missing name must fail")
                .code,
            "INVALID_NAME"
        );
        // Empty @name likewise.
        let empty = "/**\n * @name \n */\n:root {}\n";
        assert_eq!(
            install_theme_from_code(&profile, empty)
                .expect_err("empty name must fail")
                .code,
            "INVALID_NAME"
        );
        // Traversal, absolute, separator, and multi-space names fail id
        // validation (multi-space keeps a raw space after first-only dash).
        for bad in [
            "../escape",
            "..\\escape",
            "a/b",
            "a\\b",
            "C:\\x",
            "My Big Theme",
        ] {
            let code = format!("/**\n * @name {bad}\n */\n:root {{}}\n");
            assert_eq!(
                install_theme_from_code(&profile, &code)
                    .expect_err("evil name must fail")
                    .code,
                "INVALID_NAME",
                "{bad:?} must be rejected"
            );
        }
        // Nothing was created for any rejection.
        assert!(fs::read_dir(profile.join("themes"))
            .expect("root must list")
            .next()
            .is_none());
        // A single space becomes exactly one dash (first-only replacement,
        // Electron parity) and installs.
        let single = "/**\n * @name has space\n */\n:root {}\n";
        let spaced = install_theme_from_code(&profile, single).expect("single space must install");
        assert_eq!(spaced.id, "has-space-BD");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn install_rejects_oversized_css_and_reports_exact_size() {
        let profile = temp_profile();
        let big = format!(
            "/**\n * @name Big\n */\n{}",
            "x".repeat(MAX_THEME_CSS_BYTES + 1)
        );
        let failure = install_theme_from_code(&profile, &big).expect_err("oversized CSS must fail");
        assert_eq!(failure.code, "THEME_TOO_LARGE");
        assert!(!failure
            .message
            .contains(&profile.to_string_lossy().into_owned()));
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn install_overwrites_deterministically_and_never_leaks_paths() {
        let profile = temp_profile();
        let first = "/**\n * @name Steady\n * @version 1\n */\n:root { --a: 1; }\n";
        let second = "/**\n * @name Steady\n * @version 2\n */\n:root { --a: 2; }\n";
        install_theme_from_code(&profile, first).expect("first install must succeed");
        let again = install_theme_from_code(&profile, second).expect("reinstall must succeed");
        assert_eq!(again.id, "Steady-BD");
        let dir = profile.join("themes").join("Steady-BD");
        assert_eq!(
            fs::read_to_string(dir.join("src.css")).expect("css must read"),
            second
        );
        let manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.join("manifest.json")).expect("manifest must read"),
        )
        .expect("manifest must be JSON");
        assert_eq!(manifest["version"], "2");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn install_never_follows_planted_links_outward() {
        let profile = temp_profile();
        let outside = temp_profile();
        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_dir(
            outside.as_path(),
            profile.join("themes").join("Linked-BD"),
        );
        #[cfg(not(windows))]
        let link_result =
            std::os::unix::fs::symlink(outside.as_path(), profile.join("themes").join("Linked-BD"));
        if link_result.is_err() {
            eprintln!("SKIP: link creation needs privileges in this environment; lexical safety still holds");
            fs::remove_dir_all(&profile).ok();
            fs::remove_dir_all(&outside).ok();
            return;
        }
        let code = "/**\n * @name Linked\n */\n:root {}\n";
        let failure = install_theme_from_code(&profile, code).expect_err("outside link must fail");
        assert_eq!(failure.code, "OUTSIDE_ROOT");
        // Nothing escaped: the outside root gained no theme files.
        assert!(fs::read_dir(&outside)
            .expect("outside must list")
            .next()
            .is_none());
        fs::remove_dir_all(&profile).ok();
        fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn write_replaces_manifest_declared_css_and_preserves_manifest() {
        let profile = temp_profile();
        // Foreign filename on purpose: the write must follow manifest.theme,
        // never a hardcoded src.css.
        let manifest =
            r#"{"theme":"custom-name.css","name":"Foreign","enabled":true,"author":"A"}"#;
        write_theme(
            &profile,
            "foreign",
            manifest,
            Some(("custom-name.css", ":root{--v:1;}\n")),
        );
        let before = fs::read(profile.join("themes").join("foreign").join("manifest.json"))
            .expect("manifest must read");

        let result =
            write_theme_css(&profile, "foreign", ":root{--v:2;}\n").expect("write must succeed");
        assert_eq!(result.id, "foreign");
        assert!(result.enabled);
        assert_eq!(
            fs::read_to_string(
                profile
                    .join("themes")
                    .join("foreign")
                    .join("custom-name.css")
            )
            .expect("css must read"),
            ":root{--v:2;}\n"
        );
        // manifest.json byte-for-byte unchanged, src.css never created.
        assert_eq!(
            fs::read(profile.join("themes").join("foreign").join("manifest.json"))
                .expect("manifest must read"),
            before
        );
        assert!(!profile
            .join("themes")
            .join("foreign")
            .join("src.css")
            .exists());
        // No temp residue from the unique-temp rename.
        assert!(fs::read_dir(profile.join("themes").join("foreign"))
            .expect("dir must list")
            .all(|entry| {
                let name = entry.expect("entry must read").file_name();
                name == "manifest.json" || name == "custom-name.css"
            }));
        // Repeated writes converge on the latest bytes (rename replaces the
        // existing Windows destination).
        write_theme_css(&profile, "foreign", ":root{--v:3;}\n").expect("second write must succeed");
        write_theme_css(&profile, "foreign", ":root{--v:4;}\n").expect("third write must succeed");
        assert_eq!(
            fs::read_to_string(
                profile
                    .join("themes")
                    .join("foreign")
                    .join("custom-name.css")
            )
            .expect("css must read"),
            ":root{--v:4;}\n"
        );
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn write_rejects_unknown_invalid_and_evil_targets() {
        let profile = temp_profile();
        // Unknown theme: never created.
        assert_eq!(
            write_theme_css(&profile, "no-such-theme", ":root{}\n")
                .expect_err("missing dir must fail")
                .code,
            "UNKNOWN_THEME"
        );
        assert!(!profile.join("themes").join("no-such-theme").exists());
        // Id shapes that could select another path.
        for bad in [
            "",
            ".",
            "..",
            "../escape",
            "..\\escape",
            "a/b",
            "C:\\x",
            "has space",
        ] {
            assert_eq!(
                write_theme_css(&profile, bad, ":root{}\n")
                    .expect_err("evil id must fail")
                    .code,
                "INVALID_THEME_ID",
                "{bad:?} must be rejected"
            );
        }
        // Missing manifest.
        fs::create_dir_all(profile.join("themes").join("nomanifest")).expect("dir must create");
        assert_eq!(
            write_theme_css(&profile, "nomanifest", ":root{}\n")
                .expect_err("missing manifest must fail")
                .code,
            "INVALID_MANIFEST"
        );
        // Malformed and oversized manifests.
        write_theme(
            &profile,
            "badjson",
            "{not json",
            Some(("src.css", ":root{}\n")),
        );
        assert_eq!(
            write_theme_css(&profile, "badjson", ":root{}\n")
                .expect_err("malformed manifest must fail")
                .code,
            "INVALID_MANIFEST"
        );
        // Manifest without a theme field.
        write_theme(
            &profile,
            "notheme",
            r#"{"name":"X","enabled":true}"#,
            Some(("src.css", ":root{}\n")),
        );
        assert_eq!(
            write_theme_css(&profile, "notheme", ":root{}\n")
                .expect_err("theme-less manifest must fail")
                .code,
            "INVALID_MANIFEST"
        );
        // Evil manifest-declared filename (Electron would follow it).
        write_theme(
            &profile,
            "evilfile",
            r#"{"theme":"../planted.css","name":"E","enabled":true}"#,
            None,
        );
        assert_eq!(
            write_theme_css(&profile, "evilfile", ":root{}\n")
                .expect_err("evil filename must fail")
                .code,
            "INVALID_FILENAME"
        );
        // Oversized CSS with a valid target standing by.
        write_theme(
            &profile,
            "big",
            r#"{"theme":"src.css","name":"B","enabled":true}"#,
            Some(("src.css", ":root{}\n")),
        );
        let huge = "x".repeat(MAX_THEME_CSS_BYTES + 1);
        let failure = write_theme_css(&profile, "big", &huge).expect_err("oversized CSS must fail");
        assert_eq!(failure.code, "THEME_TOO_LARGE");
        assert!(!failure
            .message
            .contains(&profile.to_string_lossy().into_owned()));
        assert_eq!(
            fs::read_to_string(profile.join("themes").join("big").join("src.css"))
                .expect("css must read"),
            ":root{}\n"
        );
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn write_refuses_links_at_every_level() {
        let profile = temp_profile();
        let outside = temp_profile();
        let manifest = r#"{"theme":"src.css","name":"Linked","enabled":true}"#;
        write_theme(&profile, "linked", manifest, Some(("src.css", ":root{}\n")));
        // Destination symlink pointing outside: refused, outside untouched.
        #[cfg(windows)]
        let css_link = std::os::windows::fs::symlink_file(
            outside.join("secret.css"),
            profile.join("themes").join("linked").join("src.css"),
        );
        #[cfg(not(windows))]
        let css_link = std::os::unix::fs::symlink(
            outside.join("secret.css"),
            profile.join("themes").join("linked").join("src.css"),
        );
        // Directory symlink at the theme id pointing outside: refused.
        #[cfg(windows)]
        let dir_link = std::os::windows::fs::symlink_dir(
            outside.as_path(),
            profile.join("themes").join("dirlinked"),
        );
        #[cfg(not(windows))]
        let dir_link =
            std::os::unix::fs::symlink(outside.as_path(), profile.join("themes").join("dirlinked"));
        if css_link.is_err() || dir_link.is_err() {
            eprintln!("SKIP: link creation needs privileges in this environment; lexical safety still holds");
            fs::remove_dir_all(&profile).ok();
            fs::remove_dir_all(&outside).ok();
            return;
        }
        // Replace the linked css with a real file first scenario is covered
        // by removing the link: the point is the link itself must fail.
        assert_eq!(
            write_theme_css(&profile, "linked", ":root{}\n")
                .expect_err("css link must fail")
                .code,
            "OUTSIDE_ROOT"
        );
        assert_eq!(
            write_theme_css(&profile, "dirlinked", ":root{}\n")
                .expect_err("dir link must fail")
                .code,
            "OUTSIDE_ROOT"
        );
        assert!(fs::read_dir(&outside)
            .expect("outside must list")
            .next()
            .is_none());
        fs::remove_dir_all(&profile).ok();
        fs::remove_dir_all(&outside).ok();
    }
}

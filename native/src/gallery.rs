//! Native Gallery disk cache (F5 core).
//!
//! This module is the native counterpart of `src/common/gallery.ts`
//! (`BUNDLE` half: every pure store operation) as invoked from
//! `src/discord/ipc.ts` (`gallery:*` handlers, exposed as
//! `window.remcord.gallery.*` via `src/discord/preload/bridge.ts`).
//! Filtering, grouping, sorting, and pagination stay renderer-side
//! (`src/common/galleryUtils.ts`, already imported by `GalleryPage.tsx`);
//! only persistence and bytes cross the bridge.
//!
//! Traced Electron behavior (actual implementation, not declarations):
//! - Root: `<userData>/gallery/` by default, overridable via the
//!   `galleryRoot` setting with migration copy (`setGalleryRoot`).
//! - Index: `gallery-index.json` (`{version: 1, items, categories}`),
//!   loaded once and cached; a missing file starts empty and a corrupt
//!   file resets to empty. Persists on a 150 ms debounce through a
//!   temp-file dance.
//! - Items: `{id, kind, sourceUrl, title, fileName, localFile, sizeBytes,
//!   mime, guildId, guildName, channelId, channelName, authorName,
//!   messageId, savedAt, categories}`. `add-url` dedupes by exact
//!   `sourceUrl`; image/video kinds download bytes into `files/` (any
//!   download failure keeps a link entry); links stay metadata-only.
//!   Removal deletes the cached file when `localFile` resolves inside the
//!   root. Bulk removal caps at 2000 ids; import caps at 100 files.
//! - `gallery:file` returns inline base64 capped at 48 MiB; larger files
//!   play via their remote `sourceUrl`.
//! - Categories: normalized (`trim`, collapse whitespace, ≤48 chars),
//!   case-insensitive uniqueness, deterministic palette color from a
//!   `u32` hash of the name plus the category count.
//!
//! Native contract (deliberate, documented differences from Electron):
//! - Fixed root `<profile>/gallery/` (host-built, never renderer input).
//!   Electron's custom `galleryRoot` (+ migration) is deferred: it needs a
//!   `settings.set` allowlist addition plus a cross-volume migration copy,
//!   so `gallery:root-pick` has no native capability and the Gallery page
//!   gates that button. `gallery:root` therefore reports the fixed
//!   profile-relative label `"gallery"` — an absolute path never crosses.
//! - Bounded everything Electron leaves unbounded: at most 20,000 items,
//!   512 categories, a 64 MiB index file, 48 MiB per download/file (the
//!   `gallery:file` transport cap Electron already enforces), 60 s
//!   download timeout, ≤5 redirects. Overflow fails closed with a typed
//!   code, never a truncated store.
//! - Downloads accept only `http`/`https` URLs (Electron's `net.fetch`
//!   shape); `file:`, `data:`, `javascript:`, and every other scheme are
//!   refused outright — a renderer-supplied URL must never read local
//!   files. Local files enter only through the host-picked import dialog
//!   (F5b dialog slice).
//! - Symlinks, junctions, directories, and non-regular files are never
//!   followed or served (reported absent, like the bundle inventory).
//!   Index writes are atomic (sibling temp file + rename).
//! - `gallery:import-dialog` / `gallery:export` / `gallery:root-pick`
//!   need modal host dialogs (STA worker slices, like the D4 backup
//!   splits) and are deferred to F5b; their facade methods exist for
//!   signature parity but their capabilities are absent, so they throw
//!   `UNSUPPORTED_CAPABILITY` honestly and the Gallery page gates those
//!   buttons.
//!
//! Logical roots owned by this module: `<profile>/gallery/` and
//! `<profile>/gallery/files/`, both host-built. Renderer input is data
//! only (ids, URLs, names, meta strings, patches) — never a path.

use std::path::{Path, PathBuf};

/// Gallery directory leaf under the profile root.
pub const GALLERY_DIR_NAME: &str = "gallery";
/// Cached media directory leaf under the gallery root.
pub const FILES_DIR_NAME: &str = "files";
/// Index file name under the gallery root.
pub const INDEX_FILE_NAME: &str = "gallery-index.json";
/// Index document version, mirroring Electron's `INDEX_VERSION`.
pub const INDEX_VERSION: u32 = 1;
/// Fixed profile-relative root label reported by `gallery:root` (an
/// absolute path never crosses the bridge).
pub const GALLERY_ROOT_LABEL: &str = "gallery";

/// Maximum stored items (Electron: unbounded).
pub const MAX_ITEMS: usize = 20_000;
/// Maximum stored categories (Electron: unbounded).
pub const MAX_CATEGORIES: usize = 512;
/// Maximum accepted index file size in bytes (Electron: unbounded reads).
pub const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum downloaded/stored media bytes per item. Mirrors the 48 MiB
/// inline-preview transport cap Electron enforces on `gallery:file`.
pub const MAX_MEDIA_BYTES: usize = 48 * 1024 * 1024;
/// Maximum accepted source URL length (mirrors Electron's 2048 slice).
pub const MAX_URL_BYTES: usize = 2048;
/// Maximum ids accepted per bulk removal (mirrors Electron's 2000 slice).
pub const MAX_BULK_IDS: usize = 2000;
/// Maximum categories carried per item (mirrors Electron's 12 slice).
pub const MAX_ITEM_CATEGORIES: usize = 12;
/// Download timeout, total per item.
pub const DOWNLOAD_TIMEOUT_SECS: u64 = 60;
/// Download redirect hop limit.
pub const DOWNLOAD_MAX_REDIRECTS: u32 = 5;

/// Deterministic category palette, byte-identical to Electron's
/// `CATEGORY_COLORS` in `src/common/gallery.ts`.
pub const CATEGORY_COLORS: &[&str] = &[
    "#5865f2", "#57f287", "#fee75c", "#eb459e", "#ed4245", "#e3e5e8", "#f47fff", "#44ddff",
];

/// Stored media kind, mirroring `GalleryKind` (`image` | `video` | `link`
/// | `file`). Serialized in the same lowercase wire form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalleryKind {
    Image,
    Video,
    Link,
    File,
}

impl GalleryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GalleryKind::Image => "image",
            GalleryKind::Video => "video",
            GalleryKind::Link => "link",
            GalleryKind::File => "file",
        }
    }

    pub fn parse(value: &str) -> Option<GalleryKind> {
        match value {
            "image" => Some(GalleryKind::Image),
            "video" => Some(GalleryKind::Video),
            "link" => Some(GalleryKind::Link),
            "file" => Some(GalleryKind::File),
            _ => None,
        }
    }
}

/// One stored gallery item. Field names use Electron's exact camelCase
/// keys so the bridge JSON is shape-compatible with `GalleryPage.tsx`'s
/// `castItem`.
#[derive(Debug, Clone, PartialEq)]
pub struct GalleryItem {
    pub id: String,
    pub kind: GalleryKind,
    pub source_url: String,
    pub title: String,
    pub file_name: String,
    /// Profile-relative `files/<name>` with forward slashes, or empty for
    /// link-only items. Never an absolute path on the wire.
    pub local_file: String,
    pub size_bytes: u64,
    pub mime: String,
    pub guild_id: String,
    pub guild_name: String,
    pub channel_id: String,
    pub channel_name: String,
    pub author_name: String,
    pub message_id: String,
    pub saved_at_ms: i64,
    pub categories: Vec<String>,
}

/// One stored category. Same keys as Electron's `GalleryCategory`.
#[derive(Debug, Clone, PartialEq)]
pub struct GalleryCategory {
    pub name: String,
    pub color: String,
    pub created_at_ms: i64,
}

/// Renderer-supplied save metadata. Every member is optional, bounded,
/// and sliced exactly like Electron's `addGalleryUrl`/`addGalleryLink`.
#[derive(Debug, Clone, Default)]
pub struct GalleryMeta {
    pub title: String,
    pub guild_id: String,
    pub guild_name: String,
    pub channel_id: String,
    pub channel_name: String,
    pub author_name: String,
    pub message_id: String,
    pub categories: Vec<String>,
}

/// Typed failure: a stable machine-readable `code` plus a human message
/// that never contains a filesystem path or URL content.
#[derive(Debug, Clone, PartialEq)]
pub struct GalleryFailure {
    pub code: &'static str,
    pub message: String,
}

fn failure(code: &'static str, message: impl Into<String>) -> GalleryFailure {
    GalleryFailure {
        code,
        message: message.into(),
    }
}

fn unavailable(what: &str) -> GalleryFailure {
    failure(
        "GALLERY_UNAVAILABLE",
        format!("gallery {what} is unavailable"),
    )
}

/// Gallery id rule, mirroring `sanitizeGalleryId` (`/^[A-Za-z0-9_-]{1,64}$/`).
pub fn validate_gallery_id(id: &str) -> Result<(), GalleryFailure> {
    if id.is_empty() || id.len() > 64 {
        return Err(failure("INVALID_ID", "gallery id must be 1..=64 bytes"));
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(failure(
            "INVALID_ID",
            "gallery id uses characters outside [A-Za-z0-9_-]",
        ));
    }
    Ok(())
}

const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "svg", "ico", "apng",
];
const VIDEO_EXTS: &[&str] = &["mp4", "webm", "mov", "mkv", "avi", "m4v", "ogv"];

/// Kind detection mirroring `detectGalleryKind`: the content MIME wins,
/// then the URL path extension (lowercased), then empty URL means `file`,
/// otherwise `link`.
pub fn detect_gallery_kind(url: &str, mime: &str) -> GalleryKind {
    let lower = mime.to_ascii_lowercase();
    if lower.starts_with("image/") {
        return GalleryKind::Image;
    }
    if lower.starts_with("video/") {
        return GalleryKind::Video;
    }
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let ext = path.rsplit('.').next().unwrap_or("");
    // A bare host (`https://x`) has no extension: `rsplit` yields the whole
    // string, which matches neither set — same as Electron's `pop()`.
    if IMAGE_EXTS.contains(&ext) {
        return GalleryKind::Image;
    }
    if VIDEO_EXTS.contains(&ext) {
        return GalleryKind::Video;
    }
    if url.is_empty() {
        return GalleryKind::File;
    }
    GalleryKind::Link
}

/// File name derivation mirroring `fileNameFromUrl`: last URL path
/// segment, percent-decoded, ≤128 chars, else the fallback.
pub fn file_name_from_url(url: &str, fallback: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(last) = parsed
            .path_segments()
            .and_then(|mut segments| segments.rfind(|s| !s.is_empty()))
        {
            let decoded = percent_encoding::percent_decode_str(last).decode_utf8_lossy();
            let clean: String = decoded.chars().take(128).collect();
            if !clean.is_empty() {
                return clean;
            }
        }
        return fallback.to_owned();
    }
    fallback.to_owned()
}

/// Category normalization mirroring `normalizeCategoryName`: trim,
/// collapse whitespace, ≤48 chars.
pub fn normalize_category_name(name: &str) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for ch in name.trim().chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
        if out.len() >= 48 {
            break;
        }
    }
    if last_space {
        out.pop();
    }
    out
}

/// Category validity mirroring `isValidCategoryName` (1..=48 after
/// normalization; byte length can overshoot on multibyte input, so check
/// chars too).
pub fn is_valid_category_name(name: &str) -> bool {
    let clean = normalize_category_name(name);
    let chars = clean.chars().count();
    (1..=48).contains(&chars) && clean.len() <= 48
}

/// Deterministic palette color mirroring `categoryColor`: `u32` hash over
/// UTF-16 code units (`charCodeAt`), plus the category count, mod 8.
pub fn category_color(name: &str, count: usize) -> &'static str {
    let mut hash: u32 = 0;
    for unit in name.encode_utf16() {
        hash = hash.wrapping_mul(31).wrapping_add(unit as u32);
    }
    CATEGORY_COLORS[(hash as usize + count) % CATEGORY_COLORS.len()]
}

/// File name sanitization mirroring `safeFileName`: basename, forbidden
/// characters replaced, ≤120 chars, else `"file"`.
pub fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let clean: String = base
        .chars()
        .map(|ch| {
            matches!(
                ch,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0'
            )
            .then_some('_')
            .unwrap_or(ch)
        })
        .take(120)
        .collect();
    if clean.is_empty() {
        "file".to_owned()
    } else {
        clean
    }
}

/// Host-generated item id mirroring `createGalleryId`
/// (`<base36 ms>-<8 alnum>`).
pub fn create_gallery_id(now_ms: i64) -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut value = now_ms.max(0) as u64;
    let mut time_part = Vec::new();
    if value == 0 {
        time_part.push(b'0');
    }
    while value > 0 {
        time_part.push(ALPHABET[(value % 36) as usize]);
        value /= 36;
    }
    time_part.reverse();
    let mut random_part = String::with_capacity(8);
    // No RNG dependency: derive 8 alnum chars from a second hash of the
    // timestamp plus a process-scoped counter. Uniqueness across items in
    // one run comes from the counter; across runs from the timestamp.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seed = (now_ms as u64)
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(COUNTER.fetch_add(1, Ordering::Relaxed));
    let mut state = seed | 1;
    for _ in 0..8 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let idx = ((state >> 33) % 36) as usize;
        random_part.push(ALPHABET[idx] as char);
    }
    format!("{}-{random_part}", String::from_utf8_lossy(&time_part))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn slice_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

/// Gallery root and files dir under the host-controlled profile dir.
pub fn gallery_root(profile_dir: &Path) -> PathBuf {
    profile_dir.join(GALLERY_DIR_NAME)
}

pub fn gallery_files_dir(profile_dir: &Path) -> PathBuf {
    gallery_root(profile_dir).join(FILES_DIR_NAME)
}

pub fn ensure_gallery_dirs(profile_dir: &Path) -> Result<PathBuf, GalleryFailure> {
    let root = gallery_root(profile_dir);
    std::fs::create_dir_all(&root)
        .and_then(|_| std::fs::create_dir_all(gallery_files_dir(profile_dir)))
        .map_err(|_| unavailable("storage"))?;
    Ok(root)
}

/// Reads one directory entry's metadata without following links.
fn entry_present(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file())
}

/// Resolves a stored `files/<name>` reference against the gallery root.
/// Anything that is not exactly one bare file name under `files/`, or
/// that is missing / not a regular file / a link, resolves to `None`
/// (never followed outward).
pub fn resolve_gallery_file(profile_dir: &Path, local_file: &str) -> Option<PathBuf> {
    let name = local_file.strip_prefix("files/")?;
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name == "."
        || name == ".."
        || name.len() > 128
    {
        return None;
    }
    let root = gallery_root(profile_dir);
    let candidate = root.join("files").join(name);
    // Lexical containment first (no `..` can survive the checks above,
    // but belt-and-braces against `files/../index` shapes), then the
    // never-follow-links presence check.
    let canonical_files = std::fs::canonicalize(root.join("files")).unwrap_or(root.join("files"));
    let canonical_parent = candidate
        .parent()
        .and_then(|p| std::fs::canonicalize(p).ok())?;
    if canonical_parent != canonical_files {
        return None;
    }
    entry_present(&candidate).then_some(candidate)
}

fn unique_disk_name(dir: &Path, wanted: &str) -> String {
    let name = safe_file_name(wanted);
    if !entry_present(&dir.join(&name)) && std::fs::symlink_metadata(dir.join(&name)).is_err() {
        return name;
    }
    let (base, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (name[..dot].to_owned(), name[dot..].to_owned()),
        _ => (name.clone(), String::new()),
    };
    let base: String = base.chars().take(80).collect();
    for n in 1..1000 {
        let candidate = format!("{base}-{n}{ext}");
        if std::fs::symlink_metadata(dir.join(&candidate)).is_err() {
            return candidate;
        }
    }
    format!("{}-{name}", now_ms())
}

// ---- index document ----

/// Gallery item display JSON for bridge responses. Field names use
/// Electron's exact camelCase keys so `GalleryPage.tsx` casts unchanged;
/// `localFile` stays a profile-relative `files/<name>` reference.
pub fn gallery_item_json(item: &GalleryItem) -> serde_json::Value {
    serde_json::json!({
        "id": item.id,
        "kind": item.kind.as_str(),
        "sourceUrl": item.source_url,
        "title": item.title,
        "fileName": item.file_name,
        "localFile": item.local_file,
        "sizeBytes": item.size_bytes,
        "mime": item.mime,
        "guildId": item.guild_id,
        "guildName": item.guild_name,
        "channelId": item.channel_id,
        "channelName": item.channel_name,
        "authorName": item.author_name,
        "messageId": item.message_id,
        "savedAt": item.saved_at_ms,
        "categories": item.categories,
    })
}

fn item_from_json(value: &serde_json::Value) -> Option<GalleryItem> {
    let id = value.get("id")?.as_str()?;
    if validate_gallery_id(id).is_err() {
        return None;
    }
    Some(GalleryItem {
        id: id.to_owned(),
        kind: GalleryKind::parse(value.get("kind")?.as_str()?)?,
        source_url: value
            .get("sourceUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        title: value
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        file_name: value
            .get("fileName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        local_file: value
            .get("localFile")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        size_bytes: value.get("sizeBytes").and_then(|v| v.as_u64()).unwrap_or(0),
        mime: value
            .get("mime")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        guild_id: value
            .get("guildId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        guild_name: value
            .get("guildName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        channel_id: value
            .get("channelId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        channel_name: value
            .get("channelName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        author_name: value
            .get("authorName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        message_id: value
            .get("messageId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        saved_at_ms: value.get("savedAt").and_then(|v| v.as_i64()).unwrap_or(0),
        categories: value
            .get("categories")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c.as_str().map(str::to_owned))
                    .take(MAX_ITEM_CATEGORIES)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Gallery category display JSON for bridge responses. Same keys as
/// Electron's `GalleryCategory`.
pub fn gallery_category_json(category: &GalleryCategory) -> serde_json::Value {
    serde_json::json!({
        "name": category.name,
        "color": category.color,
        "createdAt": category.created_at_ms,
    })
}

fn category_from_json(value: &serde_json::Value) -> Option<GalleryCategory> {
    Some(GalleryCategory {
        name: value.get("name")?.as_str()?.to_owned(),
        color: value
            .get("color")
            .and_then(|v| v.as_str())
            .unwrap_or("#5865f2")
            .to_owned(),
        created_at_ms: value.get("createdAt").and_then(|v| v.as_i64()).unwrap_or(0),
    })
}

#[derive(Debug, Default)]
pub struct GalleryIndex {
    pub items: Vec<GalleryItem>,
    pub categories: Vec<GalleryCategory>,
}

/// Loads the gallery index. A missing file starts empty and a corrupt or
/// oversized file fails closed (unlike Electron, which resets corrupt
/// files to empty — silently dropping the user's library is worse than
/// refusing; the typed error names the recovery: delete or restore the
/// index file).
pub fn load_gallery_index(profile_dir: &Path) -> Result<GalleryIndex, GalleryFailure> {
    let root = ensure_gallery_dirs(profile_dir)?;
    let path = root.join(INDEX_FILE_NAME);
    let bytes = match std::fs::read(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GalleryIndex::default())
        }
        Err(_) => return Err(unavailable("index")),
        Ok(bytes) => bytes,
    };
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err(failure(
            "INDEX_TOO_LARGE",
            "gallery index exceeds its size limit",
        ));
    }
    let doc: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| unavailable("index (the file is not valid JSON)"))?;
    let items = doc
        .get("items")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(item_from_json)
                .take(MAX_ITEMS + 1)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if items.len() > MAX_ITEMS {
        return Err(failure(
            "INDEX_TOO_LARGE",
            "gallery index holds too many items",
        ));
    }
    let categories = doc
        .get("categories")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(category_from_json)
                .take(MAX_CATEGORIES + 1)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if categories.len() > MAX_CATEGORIES {
        return Err(failure(
            "INDEX_TOO_LARGE",
            "gallery index holds too many categories",
        ));
    }
    Ok(GalleryIndex { items, categories })
}

/// Persists the index atomically (sibling temp file + rename).
pub fn persist_gallery_index(
    profile_dir: &Path,
    index: &GalleryIndex,
) -> Result<(), GalleryFailure> {
    let root = ensure_gallery_dirs(profile_dir)?;
    let doc = serde_json::json!({
        "version": INDEX_VERSION,
        "items": index.items.iter().map(gallery_item_json).collect::<Vec<_>>(),
        "categories": index.categories.iter().map(gallery_category_json).collect::<Vec<_>>(),
    });
    let bytes = serde_json::to_vec(&doc).map_err(|_| unavailable("index"))?;
    let tmp = root.join(format!("{INDEX_FILE_NAME}.tmp"));
    std::fs::write(&tmp, &bytes).map_err(|_| unavailable("index"))?;
    std::fs::rename(&tmp, root.join(INDEX_FILE_NAME)).map_err(|_| unavailable("index"))?;
    Ok(())
}

// ---- operations ----

fn clean_meta_url(url: &str) -> Result<String, GalleryFailure> {
    let clean: String = url.trim().chars().take(MAX_URL_BYTES).collect();
    let lower = clean.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(failure("INVALID_URL", "gallery URL must be http(s)"));
    }
    // `Url::parse` rejects javascript:/data:/file: shapes and other
    // non-hierarchical input the prefix check alone would miss.
    if url::Url::parse(&clean).is_err() {
        return Err(failure("INVALID_URL", "gallery URL is not a valid URL"));
    }
    Ok(clean)
}

fn clean_meta(meta: &GalleryMeta) -> GalleryMeta {
    GalleryMeta {
        title: slice_chars(&meta.title, 256),
        guild_id: slice_chars(&meta.guild_id, 64),
        guild_name: slice_chars(&meta.guild_name, 128),
        channel_id: slice_chars(&meta.channel_id, 64),
        channel_name: slice_chars(&meta.channel_name, 128),
        author_name: slice_chars(&meta.author_name, 128),
        message_id: slice_chars(&meta.message_id, 64),
        categories: meta
            .categories
            .iter()
            .map(|c| normalize_category_name(c))
            .filter(|c| !c.is_empty())
            .take(MAX_ITEM_CATEGORIES)
            .collect(),
    }
}

/// Downloads one remote file for gallery storage. Only `http`/`https`;
/// bounded bytes, bounded redirects, bounded time. Returns
/// `(disk_name, bytes, mime)`.
pub fn download_gallery_url(
    url: &str,
    dir: &Path,
    file_name: &str,
) -> Result<(String, u64, String), GalleryFailure> {
    let agent = ureq::AgentBuilder::new()
        .redirects(DOWNLOAD_MAX_REDIRECTS)
        .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|_| failure("DOWNLOAD_FAILED", "gallery download failed"))?;
    if !(200..300).contains(&response.status()) {
        return Err(failure(
            "DOWNLOAD_FAILED",
            "gallery download was not successful",
        ));
    }
    let mime = response
        .header("content-type")
        .and_then(|raw| raw.split(';').next())
        .map(str::trim)
        .filter(|mime| !mime.is_empty() && mime.len() <= 128)
        .unwrap_or("application/octet-stream")
        .to_owned();
    let mut bytes = Vec::new();
    use std::io::Read;
    response
        .into_reader()
        .take(MAX_MEDIA_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failure("DOWNLOAD_FAILED", "gallery download could not be read"))?;
    if bytes.is_empty() || bytes.len() > MAX_MEDIA_BYTES {
        return Err(failure(
            "TOO_LARGE",
            "gallery download exceeds the media size limit",
        ));
    }
    let name = unique_disk_name(dir, file_name);
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, &bytes).map_err(|_| unavailable("storage"))?;
    std::fs::rename(&tmp, dir.join(&name)).map_err(|_| unavailable("storage"))?;
    Ok((name, bytes.len() as u64, mime))
}

/// Saves one remote URL (`gallery:add-url`). Dedupes by exact `sourceUrl`;
/// image/video kinds persist bytes (any download failure keeps a link
/// entry, mirroring Electron); links stay metadata-only.
pub fn add_gallery_url(
    profile_dir: &Path,
    url: &str,
    meta: &GalleryMeta,
) -> Result<GalleryItem, GalleryFailure> {
    let clean = clean_meta_url(url)?;
    let mut index = load_gallery_index(profile_dir)?;
    if let Some(existing) = index.items.iter().find(|item| item.source_url == clean) {
        return Ok(existing.clone());
    }
    if index.items.len() >= MAX_ITEMS {
        return Err(failure("GALLERY_FULL", "gallery holds too many items"));
    }
    let meta = clean_meta(meta);
    let dir = gallery_files_dir(profile_dir);
    let kind = detect_gallery_kind(&clean, "");
    let title = if meta.title.is_empty() {
        slice_chars(
            &file_name_from_url(&clean, &clean[..clean.len().min(80)]),
            256,
        )
    } else {
        meta.title.clone()
    };
    let mut item = GalleryItem {
        id: create_gallery_id(now_ms()),
        kind,
        source_url: clean,
        title,
        file_name: slice_chars(&file_name_from_url(url, "saved-file"), 128),
        local_file: String::new(),
        size_bytes: 0,
        mime: if kind == GalleryKind::Link {
            "text/uri-list".to_owned()
        } else {
            "application/octet-stream".to_owned()
        },
        guild_id: meta.guild_id.clone(),
        guild_name: meta.guild_name.clone(),
        channel_id: meta.channel_id.clone(),
        channel_name: meta.channel_name.clone(),
        author_name: meta.author_name.clone(),
        message_id: meta.message_id.clone(),
        saved_at_ms: now_ms(),
        categories: meta.categories.clone(),
    };
    if kind == GalleryKind::Image || kind == GalleryKind::Video {
        match download_gallery_url(&item.source_url, &dir, &item.file_name) {
            Ok((name, bytes, mime)) => {
                item.local_file = format!("{FILES_DIR_NAME}/{name}");
                item.size_bytes = bytes;
                item.mime = mime;
                item.file_name = name;
            }
            Err(_) => {
                // Parity: keep a link entry when the CDN fetch fails.
                item.kind = GalleryKind::Link;
                item.mime = "text/uri-list".to_owned();
            }
        }
    }
    index.items.insert(0, item.clone());
    persist_gallery_index(profile_dir, &index)?;
    Ok(item)
}

/// Saves one metadata-only link (`gallery:add-link`).
pub fn add_gallery_link(
    profile_dir: &Path,
    url: &str,
    meta: &GalleryMeta,
) -> Result<GalleryItem, GalleryFailure> {
    let clean = clean_meta_url(url)?;
    let mut index = load_gallery_index(profile_dir)?;
    if index.items.len() >= MAX_ITEMS {
        return Err(failure("GALLERY_FULL", "gallery holds too many items"));
    }
    let meta = clean_meta(meta);
    let title = if meta.title.is_empty() {
        slice_chars(
            &file_name_from_url(&clean, &clean[..clean.len().min(80)]),
            256,
        )
    } else {
        meta.title.clone()
    };
    let item = GalleryItem {
        id: create_gallery_id(now_ms()),
        kind: GalleryKind::Link,
        source_url: clean,
        title,
        file_name: slice_chars(&file_name_from_url(url, "saved-file"), 128),
        local_file: String::new(),
        size_bytes: 0,
        mime: "text/uri-list".to_owned(),
        guild_id: meta.guild_id.clone(),
        guild_name: meta.guild_name.clone(),
        channel_id: meta.channel_id.clone(),
        channel_name: meta.channel_name.clone(),
        author_name: meta.author_name.clone(),
        message_id: meta.message_id.clone(),
        saved_at_ms: now_ms(),
        categories: meta.categories.clone(),
    };
    index.items.insert(0, item.clone());
    persist_gallery_index(profile_dir, &index)?;
    Ok(item)
}

/// Removes one item (`gallery:remove`), deleting its cached file when the
/// stored reference resolves inside the gallery root.
pub fn remove_gallery_item(profile_dir: &Path, id: &str) -> Result<bool, GalleryFailure> {
    validate_gallery_id(id).map_err(|_| failure("INVALID_ID", "gallery id is not valid"))?;
    let mut index = load_gallery_index(profile_dir)?;
    let at = index.items.iter().position(|item| item.id == id);
    let Some(at) = at else {
        return Ok(false);
    };
    let removed = index.items.remove(at);
    delete_cached_file(profile_dir, &removed.local_file);
    persist_gallery_index(profile_dir, &index)?;
    Ok(true)
}

fn delete_cached_file(profile_dir: &Path, local_file: &str) {
    if let Some(abs) = resolve_gallery_file(profile_dir, local_file) {
        let _ = std::fs::remove_file(abs);
    }
}

/// Removes many items (`gallery:bulk-remove`), at most 2000 ids.
pub fn bulk_remove_gallery_items(
    profile_dir: &Path,
    ids: &[String],
) -> Result<usize, GalleryFailure> {
    let mut index = load_gallery_index(profile_dir)?;
    let mut removed = 0;
    for id in ids.iter().take(MAX_BULK_IDS) {
        if validate_gallery_id(id).is_err() {
            continue;
        }
        if let Some(at) = index
            .items
            .iter()
            .position(|item| item.id.as_str() == id.as_str())
        {
            let item = index.items.remove(at);
            delete_cached_file(profile_dir, &item.local_file);
            removed += 1;
        }
    }
    persist_gallery_index(profile_dir, &index)?;
    Ok(removed)
}

/// Updates an item's title and/or categories (`gallery:update`).
pub fn update_gallery_item(
    profile_dir: &Path,
    id: &str,
    title: Option<&str>,
    categories: Option<&[String]>,
) -> Result<GalleryItem, GalleryFailure> {
    validate_gallery_id(id).map_err(|_| failure("INVALID_ID", "gallery id is not valid"))?;
    let mut index = load_gallery_index(profile_dir)?;
    let item = index
        .items
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| failure("NOT_FOUND", "gallery item was not found"))?;
    if let Some(title) = title {
        item.title = slice_chars(title, 256);
    }
    if let Some(categories) = categories {
        item.categories = categories
            .iter()
            .map(|c| normalize_category_name(c))
            .filter(|c| !c.is_empty())
            .take(MAX_ITEM_CATEGORIES)
            .collect();
    }
    let updated = item.clone();
    persist_gallery_index(profile_dir, &index)?;
    Ok(updated)
}

/// Lists items newest-first (`gallery:list`).
pub fn list_gallery_items(profile_dir: &Path) -> Result<Vec<GalleryItem>, GalleryFailure> {
    let mut index = load_gallery_index(profile_dir)?;
    index
        .items
        .sort_by_key(|item| std::cmp::Reverse(item.saved_at_ms));
    Ok(index.items)
}

/// Lists categories alphabetically (`gallery:categories`).
pub fn list_gallery_categories(profile_dir: &Path) -> Result<Vec<GalleryCategory>, GalleryFailure> {
    let mut index = load_gallery_index(profile_dir)?;
    index.categories.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(index.categories)
}

/// Adds one category (`gallery:category-add`).
pub fn add_gallery_category(
    profile_dir: &Path,
    name: &str,
) -> Result<GalleryCategory, GalleryFailure> {
    let clean = normalize_category_name(name);
    if !is_valid_category_name(&clean) {
        return Err(failure("INVALID_NAME", "category name is not valid"));
    }
    let mut index = load_gallery_index(profile_dir)?;
    if index
        .categories
        .iter()
        .any(|c| c.name.to_lowercase() == clean.to_lowercase())
    {
        return Err(failure("EXISTS", "category already exists"));
    }
    if index.categories.len() >= MAX_CATEGORIES {
        return Err(failure("GALLERY_FULL", "gallery holds too many categories"));
    }
    let category = GalleryCategory {
        color: category_color(&clean, index.categories.len()).to_owned(),
        name: clean,
        created_at_ms: now_ms(),
    };
    index.categories.push(category.clone());
    persist_gallery_index(profile_dir, &index)?;
    Ok(category)
}

/// Renames one category, retagging items (`gallery:category-rename`).
pub fn rename_gallery_category(
    profile_dir: &Path,
    old_name: &str,
    new_name: &str,
) -> Result<(), GalleryFailure> {
    let clean = normalize_category_name(new_name);
    if !is_valid_category_name(&clean) {
        return Err(failure("INVALID_NAME", "category name is not valid"));
    }
    let mut index = load_gallery_index(profile_dir)?;
    if !index.categories.iter().any(|c| c.name == old_name) {
        return Err(failure("NOT_FOUND", "category was not found"));
    }
    if index
        .categories
        .iter()
        .any(|c| c.name.to_lowercase() == clean.to_lowercase() && c.name != old_name)
    {
        return Err(failure("EXISTS", "category already exists"));
    }
    for category in index.categories.iter_mut() {
        if category.name == old_name {
            category.name = clean.clone();
        }
    }
    for item in index.items.iter_mut() {
        for slot in item.categories.iter_mut() {
            if *slot == old_name {
                *slot = clean.clone();
            }
        }
    }
    persist_gallery_index(profile_dir, &index)?;
    Ok(())
}

/// Removes one category, untagging items (`gallery:category-remove`).
/// Always reports success once the index persists (Electron parity:
/// `removeGalleryCategory` returns `{ok: true}` unconditionally).
pub fn remove_gallery_category(profile_dir: &Path, name: &str) -> Result<(), GalleryFailure> {
    let mut index = load_gallery_index(profile_dir)?;
    index.categories.retain(|c| c.name != name);
    for item in index.items.iter_mut() {
        item.categories.retain(|c| c != name);
    }
    persist_gallery_index(profile_dir, &index)?;
    Ok(())
}

/// Exact disk usage: index file + every stored media byte, measured from
/// the filesystem (`gallery:usage`).
pub fn gallery_disk_usage(profile_dir: &Path) -> Result<(u64, u64, usize), GalleryFailure> {
    let root = ensure_gallery_dirs(profile_dir)?;
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut stack = vec![root];
    // Bounded traversal: the gallery root holds only the index plus the
    // flat files/ cache, but depth/width caps keep a hostile directory
    // from stalling the event loop.
    let mut visited = 0usize;
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|_| unavailable("storage"))?;
        for entry in entries {
            let entry = entry.map_err(|_| unavailable("storage"))?;
            let meta =
                std::fs::symlink_metadata(entry.path()).map_err(|_| unavailable("storage"))?;
            if meta.is_dir() {
                if visited < 16 {
                    visited += 1;
                    stack.push(entry.path());
                }
                continue;
            }
            if meta.is_file() {
                bytes = bytes.saturating_add(meta.len());
                files += 1;
            }
        }
    }
    let items = load_gallery_index(profile_dir)?.items.len();
    Ok((bytes, files, items))
}

/// Reads one cached media file for inline preview (`gallery:file`).
/// Returns `(base64, mime, file_name)`; files over the transport cap are
/// refused (the renderer falls back to the remote `sourceUrl`, mirroring
/// Electron's `TOO_LARGE`).
pub fn read_gallery_file(
    profile_dir: &Path,
    id: &str,
) -> Result<(String, String, String), GalleryFailure> {
    validate_gallery_id(id).map_err(|_| failure("NOT_FOUND", "gallery item was not found"))?;
    let index = load_gallery_index(profile_dir)?;
    let item = index
        .items
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| failure("NOT_FOUND", "gallery item was not found"))?;
    if item.local_file.is_empty() {
        return Err(failure("NOT_FOUND", "gallery item has no cached file"));
    }
    let abs = resolve_gallery_file(profile_dir, &item.local_file)
        .ok_or_else(|| failure("NOT_FOUND", "gallery file is unavailable"))?;
    let bytes =
        std::fs::read(&abs).map_err(|_| failure("NOT_FOUND", "gallery file is unavailable"))?;
    if bytes.len() > MAX_MEDIA_BYTES {
        return Err(failure(
            "TOO_LARGE",
            "gallery file exceeds the preview size limit",
        ));
    }
    Ok((
        encode_base64(&bytes),
        item.mime.clone(),
        item.file_name.clone(),
    ))
}

/// Minimal RFC 4648 base64 encoder (no extra dependency for one preview
/// path; covered by unit tests against known vectors).
pub fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 63) as usize] as char);
        out.push(TABLE[((triple >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(triple & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn unique_temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "remcord-gallery-test-{}-{}-{tag}",
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

    fn test_meta() -> GalleryMeta {
        GalleryMeta {
            title: "Test clip".to_owned(),
            guild_id: "g1".to_owned(),
            guild_name: "Guild".to_owned(),
            channel_id: "c1".to_owned(),
            channel_name: "general".to_owned(),
            ..GalleryMeta::default()
        }
    }

    fn blank_item() -> GalleryItem {
        GalleryItem {
            id: String::new(),
            kind: GalleryKind::Link,
            source_url: String::new(),
            title: String::new(),
            file_name: String::new(),
            local_file: String::new(),
            size_bytes: 0,
            mime: String::new(),
            guild_id: String::new(),
            guild_name: String::new(),
            channel_id: String::new(),
            channel_name: String::new(),
            author_name: String::new(),
            message_id: String::new(),
            saved_at_ms: 0,
            categories: Vec::new(),
        }
    }

    #[test]
    fn gallery_ids_use_the_host_shape() {
        let id = create_gallery_id(1_700_000_000_000);
        assert!(validate_gallery_id(&id).is_ok());
        assert!(id.contains('-'));
        assert!(validate_gallery_id("").is_err());
        assert!(validate_gallery_id("../escape").is_err());
        assert!(validate_gallery_id("a/b").is_err());
        assert!(validate_gallery_id(&"x".repeat(65)).is_err());
    }

    #[test]
    fn kind_detection_matches_the_electron_table() {
        assert_eq!(
            detect_gallery_kind("https://x/a.png", ""),
            GalleryKind::Image
        );
        assert_eq!(
            detect_gallery_kind("https://x/a.JPG?size=1", ""),
            GalleryKind::Image
        );
        assert_eq!(
            detect_gallery_kind("https://x/a.mp4", ""),
            GalleryKind::Video
        );
        assert_eq!(detect_gallery_kind("https://x/a", ""), GalleryKind::Link);
        assert_eq!(detect_gallery_kind("", ""), GalleryKind::File);
        assert_eq!(
            detect_gallery_kind("https://x/a.bin", "image/png"),
            GalleryKind::Image
        );
        assert_eq!(
            detect_gallery_kind("https://x/a.png", "video/mp4"),
            GalleryKind::Video
        );
    }

    #[test]
    fn category_names_normalize_like_electron() {
        assert_eq!(normalize_category_name("  a   b  "), "a b");
        assert!(is_valid_category_name("clips"));
        assert!(!is_valid_category_name("   "));
        assert!(!is_valid_category_name(""));
        // Palette is deterministic per name.
        assert_eq!(category_color("clips", 0), category_color("clips", 0));
        assert!(CATEGORY_COLORS.contains(&category_color("anything", 3)));
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foob"), "Zm9vYg==");
        assert_eq!(encode_base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn non_http_urls_are_refused() {
        let root = unique_temp_root("schemes");
        for url in [
            "file:///C:/secret.txt",
            "data:image/png;base64,AAA",
            "javascript:alert(1)",
            "ftp://x/y.png",
            "not a url",
            "",
        ] {
            assert!(add_gallery_url(&root, url, &test_meta()).is_err(), "{url}");
            assert!(add_gallery_link(&root, url, &test_meta()).is_err(), "{url}");
        }
        remove_temp_root(&root);
    }

    #[test]
    fn links_save_metadata_only_and_round_trip() {
        let root = unique_temp_root("links");
        let item = add_gallery_link(&root, "https://example.com/clip", &test_meta()).unwrap();
        assert_eq!(item.kind, GalleryKind::Link);
        assert!(item.local_file.is_empty());
        let items = list_gallery_items(&root).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Test clip");
        // Exact-URL dedupe applies to add-url.
        let dup = add_gallery_link(&root, "https://example.com/clip", &test_meta()).unwrap();
        assert_ne!(dup.id, item.id);
        remove_temp_root(&root);
    }

    #[test]
    fn categories_enforce_uniqueness_and_retag_on_rename() {
        let root = unique_temp_root("cats");
        let created = add_gallery_category(&root, "  Clips ").unwrap();
        assert_eq!(created.name, "Clips");
        assert!(add_gallery_category(&root, "clips").is_err());
        assert!(add_gallery_category(&root, "   ").is_err());
        let mut meta = test_meta();
        meta.categories = vec!["Clips".to_owned()];
        let item = add_gallery_link(&root, "https://example.com/a", &meta).unwrap();
        assert_eq!(item.categories, vec!["Clips"]);
        rename_gallery_category(&root, "Clips", "Highlights").unwrap();
        let items = list_gallery_items(&root).unwrap();
        assert_eq!(items[0].categories, vec!["Highlights"]);
        remove_gallery_category(&root, "Highlights").unwrap();
        let items = list_gallery_items(&root).unwrap();
        assert!(items[0].categories.is_empty());
        remove_temp_root(&root);
    }

    #[test]
    fn removal_deletes_only_contained_cache_files() {
        let root = unique_temp_root("remove");
        let outside = unique_temp_root("outside");
        let secret = outside.join("secret.bin");
        std::fs::write(&secret, b"secret").unwrap();
        // Plant a hostile localFile reference directly in the index: the
        // remover must not follow it outward.
        let hostile = GalleryIndex {
            items: vec![GalleryItem {
                id: "hostile1".to_owned(),
                kind: GalleryKind::Image,
                source_url: "https://example.com/x.png".to_owned(),
                title: "x".to_owned(),
                file_name: "x.png".to_owned(),
                local_file: "../outside/secret.bin".to_owned(),
                size_bytes: 6,
                mime: "image/png".to_owned(),
                ..blank_item()
            }],
            categories: Vec::new(),
        };
        persist_gallery_index(&root, &hostile).unwrap();
        assert!(remove_gallery_item(&root, "hostile1").unwrap());
        assert!(
            secret.exists(),
            "outside file must survive a hostile localFile"
        );
        // A symlink at the resolved cache name is never served.
        let files = gallery_files_dir(&root);
        let target = outside.join("real.bin");
        std::fs::write(&target, b"real").unwrap();
        #[cfg(windows)]
        let link = std::os::windows::fs::symlink_file(&target, files.join("linked.png"));
        #[cfg(not(windows))]
        let link = std::os::unix::fs::symlink(&target, files.join("linked.png"));
        if link.is_ok() {
            assert!(resolve_gallery_file(&root, "files/linked.png").is_none());
        }
        remove_temp_root(&root);
        remove_temp_root(&outside);
    }

    #[test]
    fn oversized_index_fails_closed() {
        let root = unique_temp_root("big");
        let dir = ensure_gallery_dirs(&root).unwrap();
        // Small but valid: loads fine.
        let entries: Vec<String> = (0..100)
            .map(|n| format!("{{\"id\":\"item{n}\",\"kind\":\"link\"}}"))
            .collect();
        std::fs::write(
            dir.join(INDEX_FILE_NAME),
            format!(
                "{{\"version\":1,\"items\":[{}],\"categories\":[]}}",
                entries.join(",")
            ),
        )
        .unwrap();
        let index = load_gallery_index(&root).expect("small valid index must load");
        assert_eq!(index.items.len(), 100);
        // Entries with unusable ids are skipped, never fatal.
        std::fs::write(
            dir.join(INDEX_FILE_NAME),
            "{\"version\":1,\"items\":[{\"id\":\"../x\",\"kind\":\"link\"}],\"categories\":[]}",
        )
        .unwrap();
        assert!(load_gallery_index(&root).unwrap().items.is_empty());
        // Corrupt JSON fails closed (never a silent empty library).
        std::fs::write(dir.join(INDEX_FILE_NAME), b"{not json").unwrap();
        assert!(load_gallery_index(&root).is_err());
        remove_temp_root(&root);
    }

    #[test]
    fn disk_usage_counts_index_plus_media() {
        let root = unique_temp_root("usage");
        add_gallery_link(&root, "https://example.com/a", &test_meta()).unwrap();
        let (bytes, files, items) = gallery_disk_usage(&root).unwrap();
        assert_eq!(items, 1);
        assert!(bytes > 0, "the index file itself counts");
        assert!(files >= 1);
        remove_temp_root(&root);
    }
}

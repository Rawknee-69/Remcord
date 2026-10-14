//! Host-controlled runtime plugin discovery for the `plugins.*` capabilities (F2).
//!
//! This module is the native counterpart of the Electron plugin backend in
//! `src/discord/plugins/manager.ts` (`discoverPlugins`, `parseManifest`,
//! `getCompatibility`, `resolvePluginEntry`, `listPlugins`,
//! `getRuntimeEntries`, `getRuntimeScript`, `setPluginEnabled`,
//! `reloadPlugin`), as invoked from `src/discord/ipc.ts` (`plugins:list` /
//! `set-enabled` / `reload` / `get-runtime-entries` / `get-runtime-script`)
//! and consumed by `src/discord/preload/plugins.mts`.
//!
//! Traced Electron behavior (actual implementation, not declarations):
//! - Root: `<userData>/plugins/`, created on first discovery. One-level
//!   scan of child directories; each child needs a `manifest.json` with
//!   truthy `id`/`name`/`version`, `id` matching
//!   `/^[a-zA-Z0-9._-]{1,64}$/`, `name` a string of at most 128 bytes,
//!   `version` a string of at most 64 bytes, at least one of
//!   `main`/`preload`/`renderer` defined, each defined entry matching
//!   `/^[^<>:"|?*\0]+$/`, and `compatibleVersions` (when present) an array
//!   of strings. Anything else is skipped with a console error.
//! - Enablement: `pluginStates[manifest.id] ?? false` (opt-in default).
//! - Compatibility: no `compatibleVersions` (or an empty array) means
//!   compatible; otherwise some pattern must match the running app version
//!   (`"*"` matches all, exact equality matches, `"prefix.x"` matches
//!   `prefix` itself and `prefix.*`). Mismatches report
//!   `Incompatible with Remcord <version> (supports: <patterns>)`.
//! - Entries resolve with `path.resolve(dir, entry)` jailed to the plugin
//!   directory and must exist; scripts are read whole and unbounded.
//! - `setEnabled(true)` is refused for incompatible plugins; `reload`
//!   re-runs the main entry (preload/renderer re-run on renderer reload).
//! - `main` entries execute with full Node/Electron capabilities
//!   (`app`, `BrowserWindow`, `ipcMain`, `dialog`, `shell`).
//!
//! Native contract (deliberate, documented differences from Electron):
//! - Discovery is STATELESS: every capability re-scans the plugins root and
//!   re-reads the authoritative settings, so manifest edits, new plugins,
//!   and enablement flips apply without a restart and no stale registry can
//!   exist. `plugins.reload` re-validates one plugin's manifest and files
//!   and reports `{ok}`; the renderer loader picks the result up on the
//!   next document (no live in-place re-execution is manufactured).
//! - `main` entries are NEVER executed: there is no Node/Electron runtime
//!   natively. `hasMain` is still reported honestly in listings so authors
//!   see their manifest was read; a main-only plugin loads no code.
//! - Preload AND renderer entries both execute in the page world at
//!   document start through the native plugin loader
//!   (`native/web/plugins/loader.js`), which provides one uniform api
//!   (`{id, name, logger, patcher, onCleanup}`) for both targets. Electron
//!   gives preload entries an isolated world with `require("electron")` +
//!   spitroast and renderer entries a different `(name, parent, callback)`
//!   patcher shape; neither exists natively. Plugin sources using
//!   `require("electron")` throw inside the loader's per-plugin guard and
//!   are skipped with a diagnostic — the failure is contained, never fatal.
//! - No host path or URL crosses the bridge: runtime entries carry
//!   `{id, name}` only (Electron's absolute `path` member is dropped), and
//!   scripts carry bounded source text. The renderer supplies only logical
//!   ids plus the closed `preload`/`renderer` target enum.
//! - Symlinks, junctions, directories, and non-regular files at any entry
//!   path are treated as ABSENT (never followed), matching the bundle
//!   inventory convention. Manifests are capped at 64 KiB, sources at 1 MiB
//!   (the plugin-storage bound); Electron is unbounded for both.
//! - Display strings are bounded for bridge safety (`name` ≤ 128 and
//!   `version` ≤ 64 per Electron; `description`/`author` ≤ 4096,
//!   `compatibleVersions` ≤ 32 patterns of ≤ 64 bytes each). Over-long
//!   display fields fail the manifest (fail-closed, logged); the plugin is
//!   skipped, never truncated.
//! - Compatibility is checked against the running host version
//!   (`current_app_version()`). Debug builds report `"0.0.0"` (Electron
//!   `isDev` parity), so a `compatibleVersions` pin like `"1.3.x"` reads
//!   incompatible in DEV while matching in release. Unpinned plugins are
//!   unaffected.
//!
//! Logical root owned by this module: `<profile>/plugins/`, created on
//! demand like the other host-owned well-known roots. Every deeper path is
//! a host-built join of a discovered child name plus a validated manifest
//! entry, lexically jailed to the plugin directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Fixed plugins directory name inside the host profile. The renderer never
/// sees this name as a path; it only ever observes logical plugin ids.
pub const PLUGINS_DIR_NAME: &str = "plugins";
/// Maximum plugin id length in bytes. Mirrors the Electron `VALID_PLUGIN_ID`
/// `{1,64}` bound and the mod bundle id cap.
pub const MAX_PLUGIN_ID_BYTES: usize = 64;
/// Fixed manifest filename inside each plugin directory.
pub const MANIFEST_FILE: &str = "manifest.json";
/// Maximum accepted `manifest.json` size. Real manifests are under a
/// kilobyte; this only rails against runaway files. Electron is unbounded.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// Maximum accepted plugin entry source size. Matches the plugin-storage
/// file bound; real entries are small scripts. Electron is unbounded.
pub const MAX_PLUGIN_SOURCE_BYTES: u64 = 1024 * 1024;
/// Maximum plugin directories inventoried per discovery pass. Matches the
/// theme list bound; extras past the cap are skipped with a loud log.
pub const MAX_PLUGIN_LIST_ENTRIES: usize = 256;
/// Maximum accepted display string (`description`/`author`) length.
/// Electron caps only `name` (128) and `version` (64); the extra fields are
/// unbounded there and bounded here for bridge safety.
pub const MAX_PLUGIN_LIST_FIELD_BYTES: usize = 4096;
/// Maximum `compatibleVersions` patterns per manifest. Electron is
/// unbounded; patterns are matched one by one.
pub const MAX_COMPAT_PATTERNS: usize = 32;
/// Maximum length of one compatibility pattern. Real patterns are dotted
/// triples well under this.
pub const MAX_COMPAT_PATTERN_BYTES: usize = 64;
/// Maximum manifest entry path length. Matches the plugin-storage relative
/// path bound. Electron does not bound entry paths.
pub const MAX_ENTRY_PATH_BYTES: usize = 256;

/// Closed runtime target enum. `main` is deliberately absent: main entries
/// execute with Node/Electron capabilities that do not exist natively, so
/// no capability accepts `"main"` (it fails closed at parse time, exactly
/// like any other unknown target).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginTarget {
    Preload,
    Renderer,
}

impl PluginTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            PluginTarget::Preload => "preload",
            PluginTarget::Renderer => "renderer",
        }
    }
}

/// Validated plugin manifest: the exact Electron schema
/// (`src/discord/plugins/manager.ts`), with native display/compat bounds.
/// Unknown extra keys are ignored so a newer Electron manifest cannot break
/// the native reader.
#[derive(Debug, Clone)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub main: Option<String>,
    pub preload: Option<String>,
    pub renderer: Option<String>,
    pub compatible_versions: Vec<String>,
}

impl PluginManifest {
    /// The manifest's entry for one runtime target, if the author declared it.
    pub fn entry_for(&self, target: PluginTarget) -> Option<&str> {
        match target {
            PluginTarget::Preload => self.preload.as_deref(),
            PluginTarget::Renderer => self.renderer.as_deref(),
        }
    }
}

/// One discovered plugin: its validated manifest plus the compatibility
/// verdict against the running host. Enablement is resolved per request
/// from the authoritative settings, so it is NOT stored here.
#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    pub manifest: PluginManifest,
    pub compatible: bool,
    pub compatibility_message: Option<String>,
    /// Host-built plugin directory. Never crosses the bridge.
    pub directory: PathBuf,
}

/// Typed failure for plugin discovery/retrieval: a stable machine-readable
/// `code` plus a human message that never contains a filesystem path.
#[derive(Debug)]
pub struct PluginFailure {
    pub code: &'static str,
    pub message: String,
}

fn failure(code: &'static str, message: &str) -> PluginFailure {
    PluginFailure {
        code,
        message: message.to_owned(),
    }
}

/// Shape validation for a logical plugin id (manifest ids, request ids):
/// non-empty, bounded, confined to `[A-Za-z0-9._-]` (the exact Electron
/// `VALID_PLUGIN_ID` charset), and never `.` or `..`. This rejects path
/// separators, drive/UNC syntax, URLs, and `javascript:` shapes; manifest
/// membership (exact match against discovery) is enforced at dispatch.
pub fn validate_plugin_id(plugin_id: &str) -> Result<(), String> {
    if plugin_id.is_empty() {
        return Err("plugin id must not be empty".to_owned());
    }
    if plugin_id.len() > MAX_PLUGIN_ID_BYTES {
        return Err("plugin id exceeds the size limit".to_owned());
    }
    if plugin_id == "." || plugin_id == ".." {
        return Err("plugin id must not be a relative path segment".to_owned());
    }
    if !plugin_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("plugin id uses characters outside [A-Za-z0-9._-]".to_owned());
    }
    Ok(())
}

/// Entry-path validation: the exact Electron `VALID_ENTRY_PATH` denylist
/// (`<>:"|?*` and NUL are forbidden; separators are allowed because entries
/// may live in subdirectories), plus the native length bound. Traversal is
/// handled separately by the lexical jail in [`resolve_plugin_entry`].
fn validate_entry_path(entry: &str) -> Result<(), String> {
    if entry.is_empty() {
        return Err("plugin entry path must not be empty".to_owned());
    }
    if entry.len() > MAX_ENTRY_PATH_BYTES {
        return Err("plugin entry path exceeds the size limit".to_owned());
    }
    if entry.bytes().any(|byte| {
        byte == 0 || byte < 0x20 || matches!(byte, b'<' | b'>' | b':' | b'"' | b'|' | b'?' | b'*')
    }) {
        return Err("plugin entry path uses a forbidden character".to_owned());
    }
    Ok(())
}

fn manifest_string(value: &serde_json::Value, field: &str) -> Option<String> {
    value.get(field)?.as_str().map(str::to_owned)
}

/// Parse and validate one manifest document. Mirrors Electron's
/// `parseManifest` field by field (required truthy `id`/`name`/`version`,
/// id charset, name/version caps, at least one entry, entry denylist,
/// `compatibleVersions` string-array shape), plus the native display/compat
/// bounds. Returns `None` for anything Electron would skip (logged by the
/// caller with the child directory name only, never a path).
pub fn parse_manifest_document(raw: &[u8]) -> Option<PluginManifest> {
    if raw.len() as u64 > MAX_MANIFEST_BYTES {
        return None;
    }
    let document: serde_json::Value = serde_json::from_slice(raw).ok()?;
    if !document.is_object() {
        return None;
    }
    let id = manifest_string(&document, "id").filter(|id| !id.is_empty())?;
    if validate_plugin_id(&id).is_err() {
        return None;
    }
    let name = manifest_string(&document, "name").filter(|name| !name.is_empty())?;
    if name.len() > 128 {
        return None;
    }
    let version = manifest_string(&document, "version").filter(|version| !version.is_empty())?;
    if version.len() > 64 {
        return None;
    }
    // A JSON null entry is absent (Electron would count it as "defined" but
    // could never resolve it; treating it as absent is strictly fail-closed).
    let entry = |field: &str| match document.get(field) {
        Some(serde_json::Value::String(entry)) => Some(entry.clone()),
        _ => None,
    };
    let main = entry("main");
    let preload = entry("preload");
    let renderer = entry("renderer");
    if main.is_none() && preload.is_none() && renderer.is_none() {
        return None;
    }
    for candidate in [&main, &preload, &renderer].into_iter().flatten() {
        if validate_entry_path(candidate).is_err() {
            return None;
        }
    }
    let description = manifest_string(&document, "description").filter(|text| !text.is_empty());
    if description
        .as_ref()
        .is_some_and(|text| text.len() > MAX_PLUGIN_LIST_FIELD_BYTES)
    {
        return None;
    }
    let author = manifest_string(&document, "author").filter(|text| !text.is_empty());
    if author
        .as_ref()
        .is_some_and(|text| text.len() > MAX_PLUGIN_LIST_FIELD_BYTES)
    {
        return None;
    }
    let compatible_versions = match document.get("compatibleVersions") {
        None => Vec::new(),
        Some(serde_json::Value::Array(patterns)) => {
            if patterns.len() > MAX_COMPAT_PATTERNS {
                return None;
            }
            let mut out = Vec::with_capacity(patterns.len());
            for pattern in patterns {
                let pattern = pattern.as_str()?;
                if pattern.len() > MAX_COMPAT_PATTERN_BYTES {
                    return None;
                }
                out.push(pattern.to_owned());
            }
            out
        }
        Some(_) => return None,
    };
    Some(PluginManifest {
        id,
        name,
        version,
        description,
        author,
        main,
        preload,
        renderer,
        compatible_versions,
    })
}

/// Exact port of Electron's `isCompatibleVersion`: `"*"` matches all,
/// exact equality matches, and `"prefix.x"` matches `prefix` itself plus
/// `prefix.*`. Anything else (including caret/tilde ranges, which Electron
/// does not implement either) never matches.
pub fn is_compatible_version(pattern: &str, version: &str) -> bool {
    if pattern == "*" || pattern == version {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(".x") {
        return version == prefix || version.starts_with(&format!("{prefix}."));
    }
    false
}

/// Exact port of Electron's `getCompatibility`: no pins (or an empty pin
/// list) means compatible; otherwise some pattern must match the running
/// host version, else the fixed incompatibility message.
pub fn check_compatibility(manifest: &PluginManifest, app_version: &str) -> (bool, Option<String>) {
    if manifest.compatible_versions.is_empty() {
        return (true, None);
    }
    let compatible = manifest
        .compatible_versions
        .iter()
        .any(|pattern| is_compatible_version(pattern, app_version));
    if compatible {
        (true, None)
    } else {
        (
            false,
            Some(format!(
                "Incompatible with Remcord {app_version} (supports: {})",
                manifest.compatible_versions.join(", ")
            )),
        )
    }
}

/// Exact-match lookup of one discovered plugin by id. Discovery dedupes
/// duplicate manifest ids last-wins, so at most one entry matches.
pub fn find_plugin<'a>(
    discovered: &'a [DiscoveredPlugin],
    plugin_id: &str,
) -> Option<&'a DiscoveredPlugin> {
    discovered
        .iter()
        .find(|plugin| plugin.manifest.id == plugin_id)
}

/// Project fresh discovery plus authoritative enablement states onto the
/// bridge display shape. `states` is the `pluginStates` settings map;
/// missing entries default to disabled (opt-in, Electron parity).
pub fn plugin_infos(
    discovered: &[DiscoveredPlugin],
    states: &std::collections::HashMap<String, bool>,
) -> Vec<PluginInfo> {
    discovered
        .iter()
        .map(|plugin| {
            let manifest = &plugin.manifest;
            PluginInfo {
                id: manifest.id.clone(),
                name: manifest.name.clone(),
                version: manifest.version.clone(),
                description: manifest.description.clone(),
                author: manifest.author.clone(),
                enabled: states.get(&manifest.id).copied().unwrap_or(false),
                compatible: plugin.compatible,
                compatibility_message: plugin.compatibility_message.clone(),
                compatible_versions: manifest.compatible_versions.clone(),
                has_main: manifest.main.is_some(),
                has_preload: manifest.preload.is_some(),
                has_renderer: manifest.renderer.is_some(),
            }
        })
        .collect()
}

/// Host-built plugins root. The renderer never receives this path.
pub fn plugins_dir(profile_dir: &Path) -> PathBuf {
    profile_dir.join(PLUGINS_DIR_NAME)
}

/// Discover every valid plugin under the host-built plugins root.
///
/// A missing root inventories as empty (created at startup by the host, not
/// here). Invalid manifests are skipped with a loud log naming only the
/// child directory name. Discovery stops at [`MAX_PLUGIN_LIST_ENTRIES`]
/// with a loud log; the first N valid plugins win. Directory children sort
/// for a deterministic order across runs and platforms.
pub fn discover_plugins(profile_dir: &Path, app_version: &str) -> Vec<DiscoveredPlugin> {
    let root = plugins_dir(profile_dir);
    let mut children: Vec<String> = match std::fs::read_dir(&root) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            eprintln!("[Plugins][discovery] plugin root is unreadable: {error}");
            return Vec::new();
        }
    };
    children.sort();
    let mut out: Vec<DiscoveredPlugin> = Vec::new();
    for child in children {
        if out.len() >= MAX_PLUGIN_LIST_ENTRIES {
            eprintln!(
                "[Plugins][discovery] plugin inventory exceeds {MAX_PLUGIN_LIST_ENTRIES} entries; skipping the rest"
            );
            break;
        }
        let manifest_path = root.join(&child).join(MANIFEST_FILE);
        let raw = match std::fs::read(&manifest_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let Some(manifest) = parse_manifest_document(&raw) else {
            eprintln!("[Plugins][discovery] skipping invalid manifest in {child}");
            continue;
        };
        let (compatible, compatibility_message) = check_compatibility(&manifest, app_version);
        let discovered = DiscoveredPlugin {
            directory: root.join(&child),
            manifest,
            compatible,
            compatibility_message,
        };
        // Duplicate manifest ids resolve last-wins (Electron's `Map.set`
        // overwrites in place); the loud log keeps the collision visible.
        if let Some(existing) = out
            .iter_mut()
            .find(|plugin| plugin.manifest.id == discovered.manifest.id)
        {
            eprintln!(
                "[Plugins][discovery] duplicate plugin id '{}' in {child}; keeping the last manifest",
                discovered.manifest.id
            );
            *existing = discovered;
        } else {
            out.push(discovered);
        }
    }
    out
}

/// Lexically normalize a joined path without touching the filesystem
/// (never follows links): resolves `.`, refuses `..` above the root.
/// Returns `None` when the entry escapes lexically.
fn jail_join(root: &Path, plugin_dir: &Path, entry: &str) -> Option<PathBuf> {
    // Absolute entries (either separator flavor, either platform) never
    // resolve inside the plugin directory.
    if entry.starts_with('/') || entry.starts_with('\\') {
        return None;
    }
    if entry.len() >= 2 && entry.as_bytes()[0].is_ascii_alphabetic() && entry.as_bytes()[1] == b':'
    {
        return None;
    }
    let mut normalized = PathBuf::new();
    for raw in entry.split(['/', '\\']) {
        match raw {
            "" | "." => {}
            ".." => {
                if !normalized.pop() {
                    return None;
                }
            }
            _ => normalized.push(raw),
        }
    }
    let candidate = plugin_dir.join(normalized);
    // The lexical result must stay under the plugin directory, which itself
    // must stay under the plugins root (belt and suspenders: discovery
    // built both, but the check is cheap).
    if !candidate.starts_with(plugin_dir) || !plugin_dir.starts_with(root) {
        return None;
    }
    Some(candidate)
}

/// Resolve one manifest entry to a host-built path, or `None` when the
/// entry is undeclared, escapes the plugin directory, or is not a regular
/// file. Symlinks, junctions, and directories resolve to `None` and are
/// NEVER followed. Mirrors Electron's `resolvePluginEntry` jail (resolve +
/// containment + existence) with the native never-follow hardening.
pub fn resolve_plugin_entry(
    root: &Path,
    plugin: &DiscoveredPlugin,
    target: PluginTarget,
) -> Option<PathBuf> {
    let entry = plugin.manifest.entry_for(target)?;
    let candidate = jail_join(root, &plugin.directory, entry)?;
    match std::fs::symlink_metadata(&candidate) {
        Ok(meta) if meta.is_file() => Some(candidate),
        _ => None,
    }
}

/// Read one runtime entry source for the bridge. Returns `None` when the
/// entry is undeclared, unresolvable, missing, or empty (the loader skips
/// falsy sources, mirroring Electron's `if (!source) continue`). Over-limit
/// sources, non-UTF-8 bytes, and unreadable files are typed errors, never
/// silent truncation or substitution. No path crosses in either direction.
pub fn read_plugin_source(
    root: &Path,
    plugin: &DiscoveredPlugin,
    target: PluginTarget,
) -> Result<Option<String>, PluginFailure> {
    let Some(path) = resolve_plugin_entry(root, plugin, target) else {
        return Ok(None);
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(failure("PLUGIN_UNAVAILABLE", "plugin entry is unreadable"));
        }
    };
    if bytes.len() as u64 > MAX_PLUGIN_SOURCE_BYTES {
        return Err(failure(
            "PLUGIN_TOO_LARGE",
            "plugin entry exceeds the size limit",
        ));
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    match String::from_utf8(bytes) {
        Ok(source) => Ok(Some(source)),
        Err(_) => Err(failure("PLUGIN_INVALID", "plugin entry is not valid UTF-8")),
    }
}

/// Bridge display metadata for one plugin: the exact Electron
/// `listPlugins()` shape (`src/discord/plugins/manager.ts`), minus nothing.
/// `description`, `author`, and `compatibilityMessage` are omitted when
/// unset (Electron's `undefined` members never reach JSON either).
/// No path, URL, or source ever crosses with an entry.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub enabled: bool,
    pub compatible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_message: Option<String>,
    pub compatible_versions: Vec<String>,
    pub has_main: bool,
    pub has_preload: bool,
    pub has_renderer: bool,
}

/// One runtime entry for the loader: logical id plus display name only.
/// Electron's absolute `path` member is deliberately dropped: the loader
/// requests sources by id+target and the host re-resolves them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRuntimeEntry {
    pub id: String,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_profile_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time must be available")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "remcord-plugins-{tag}-{nanos}-{}",
            std::process::id()
        ))
    }

    fn remove_profile_dir(root: &Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    fn install_plugin(root: &Path, child: &str, manifest: &str, files: &[(&str, &[u8])]) {
        let dir = root.join(PLUGINS_DIR_NAME).join(child);
        std::fs::create_dir_all(&dir).expect("plugin dir must be creatable");
        std::fs::write(dir.join(MANIFEST_FILE), manifest).expect("manifest must write");
        for (name, bytes) in files {
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("entry parent must be creatable");
            }
            std::fs::write(path, bytes).expect("entry must write");
        }
    }

    #[test]
    fn plugin_ids_reject_paths_urls_and_segments() {
        assert!(validate_plugin_id("my.plugin_2").is_ok());
        assert!(validate_plugin_id("a").is_ok());
        for hostile in [
            "",
            ".",
            "..",
            "../x",
            "x/y",
            "x\\y",
            "C:\\x",
            "https://a.invalid/x",
            "javascript:alert(1)",
            "ven ",
            "x|y",
            &"a".repeat(65),
        ] {
            assert!(
                validate_plugin_id(hostile).is_err(),
                "{hostile:?} must be refused"
            );
        }
    }

    #[test]
    fn version_patterns_match_exactly_like_electron() {
        assert!(is_compatible_version("*", "1.3.0"));
        assert!(is_compatible_version("1.3.0", "1.3.0"));
        assert!(!is_compatible_version("1.3.0", "1.3.1"));
        assert!(is_compatible_version("1.3.x", "1.3.0"));
        assert!(is_compatible_version("1.3.x", "1.3.99"));
        assert!(is_compatible_version("1.3.x", "1.3"));
        assert!(!is_compatible_version("1.3.x", "1.30.0"));
        assert!(!is_compatible_version("1.3.x", "1.4.0"));
        assert!(!is_compatible_version("^1.3.0", "1.3.5"));
        assert!(!is_compatible_version("", "1.3.0"));
    }

    #[test]
    fn unpinned_plugins_are_compatible_and_pins_report_the_fixed_message() {
        let manifest = |pins: Option<Vec<&str>>| PluginManifest {
            id: "demo".to_owned(),
            name: "Demo".to_owned(),
            version: "1.0.0".to_owned(),
            description: None,
            author: None,
            main: None,
            preload: Some("preload.js".to_owned()),
            renderer: None,
            compatible_versions: pins
                .unwrap_or_default()
                .iter()
                .map(|pin| pin.to_string())
                .collect(),
        };
        assert_eq!(check_compatibility(&manifest(None), "1.3.0"), (true, None));
        assert_eq!(
            check_compatibility(&manifest(Some(vec![])), "1.3.0"),
            (true, None)
        );
        assert_eq!(
            check_compatibility(&manifest(Some(vec!["1.3.x"])), "1.3.0"),
            (true, None)
        );
        assert_eq!(
            check_compatibility(&manifest(Some(vec!["9.9.x"])), "1.3.0"),
            (
                false,
                Some("Incompatible with Remcord 1.3.0 (supports: 9.9.x)".to_owned())
            )
        );
    }

    #[test]
    fn manifests_validate_exactly_like_electron_plus_native_bounds() {
        let base = r#"{"id":"demo","name":"Demo","version":"1.0.0","preload":"preload.js"}"#;
        let parsed = parse_manifest_document(base.as_bytes()).expect("valid manifest must parse");
        assert_eq!(parsed.id, "demo");
        assert!(parsed.compatible_versions.is_empty());
        // Required fields.
        assert!(
            parse_manifest_document(br#"{"name":"Demo","version":"1.0.0","preload":"a.js"}"#)
                .is_none()
        );
        assert!(
            parse_manifest_document(br#"{"id":"demo","version":"1.0.0","preload":"a.js"}"#)
                .is_none()
        );
        assert!(
            parse_manifest_document(br#"{"id":"demo","name":"Demo","preload":"a.js"}"#).is_none()
        );
        assert!(parse_manifest_document(
            br#"{"id":"","name":"Demo","version":"1.0.0","preload":"a.js"}"#
        )
        .is_none());
        // At least one entry.
        assert!(
            parse_manifest_document(br#"{"id":"demo","name":"Demo","version":"1.0.0"}"#).is_none()
        );
        // Null entries count as absent (stricter than Electron, fail-closed).
        assert!(parse_manifest_document(
            br#"{"id":"demo","name":"Demo","version":"1.0.0","main":null}"#
        )
        .is_none());
        // Id charset, name/version caps.
        assert!(parse_manifest_document(
            br#"{"id":"../x","name":"Demo","version":"1.0.0","preload":"a.js"}"#
        )
        .is_none());
        assert!(parse_manifest_document(
            format!(
                r#"{{"id":"demo","name":"{}","version":"1.0.0","preload":"a.js"}}"#,
                "n".repeat(129)
            )
            .as_bytes()
        )
        .is_none());
        // Entry denylist (Electron's `< > : " | ? *` plus NUL/controls).
        // Relative escapes like `../x` PASS parsing (Electron accepts them
        // too) and die at the resolve jail instead (covered below).
        for hostile in ["a<b", "a:b", "a|b", "a?b", "a*b", "a\"b", ""] {
            let raw =
                format!(r#"{{"id":"demo","name":"Demo","version":"1.0.0","preload":{hostile:?}}}"#);
            assert!(
                parse_manifest_document(raw.as_bytes()).is_none(),
                "{hostile:?} must be refused"
            );
        }
        // compatibleVersions shape.
        assert!(
            parse_manifest_document(
                br#"{"id":"demo","name":"Demo","version":"1.0.0","preload":"a.js","compatibleVersions":["1.3.x"]}"#
            )
            .is_some()
        );
        assert!(
            parse_manifest_document(
                br#"{"id":"demo","name":"Demo","version":"1.0.0","preload":"a.js","compatibleVersions":"*"}"#
            )
            .is_none()
        );
        // Unknown future keys are ignored.
        assert!(parse_manifest_document(
            br#"{"id":"demo","name":"Demo","version":"1.0.0","preload":"a.js","someFutureKey":1}"#
        )
        .is_some());
        // Corrupt JSON and oversized documents are refused.
        assert!(parse_manifest_document(b"{not json").is_none());
        assert!(parse_manifest_document(br#"[1,2]"#).is_none());
    }

    #[test]
    fn discovery_skips_invalid_manifests_and_reports_compatible_flags() {
        let root = unique_profile_dir("discover");
        install_plugin(
            &root,
            "good",
            r#"{"id":"good","name":"Good","version":"1.0.0","preload":"preload.js","compatibleVersions":["*"]}"#,
            &[("preload.js", b"module.exports.activate = () => {};")],
        );
        install_plugin(
            &root,
            "pinned-old",
            r#"{"id":"pinned-old","name":"Old","version":"2.0.0","renderer":"renderer.js","compatibleVersions":["9.9.x"]}"#,
            &[("renderer.js", b"void 0;")],
        );
        install_plugin(&root, "broken", r#"{"id":"broken"}"#, &[]);
        std::fs::create_dir_all(root.join(PLUGINS_DIR_NAME).join("empty")).unwrap();
        let found = discover_plugins(&root, "1.3.0");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].manifest.id, "good");
        assert!(found[0].compatible);
        assert!(found[0].compatibility_message.is_none());
        assert_eq!(found[1].manifest.id, "pinned-old");
        assert!(!found[1].compatible);
        assert_eq!(
            found[1].compatibility_message.as_deref(),
            Some("Incompatible with Remcord 1.3.0 (supports: 9.9.x)")
        );
        remove_profile_dir(&root);
    }

    #[test]
    fn missing_root_discovers_empty_and_entry_jail_holds() {
        let root = unique_profile_dir("jail");
        assert!(discover_plugins(&root, "1.3.0").is_empty());
        install_plugin(
            &root,
            "escape",
            r#"{"id":"escape","name":"Escape","version":"1.0.0","preload":"../../outside.js"}"#,
            &[("inner.js", b"void 0;")],
        );
        install_plugin(
            &root,
            "sub",
            r#"{"id":"sub","name":"Sub","version":"1.0.0","preload":"nested/entry.js"}"#,
            &[("nested/entry.js", b"window.__sub = 1;")],
        );
        let found = discover_plugins(&root, "1.3.0");
        assert_eq!(found.len(), 2);
        let plugins_root = plugins_dir(&root);
        let escape = found
            .iter()
            .find(|plugin| plugin.manifest.id == "escape")
            .unwrap();
        assert!(resolve_plugin_entry(&plugins_root, escape, PluginTarget::Preload).is_none());
        let sub = found
            .iter()
            .find(|plugin| plugin.manifest.id == "sub")
            .unwrap();
        let resolved = resolve_plugin_entry(&plugins_root, sub, PluginTarget::Preload)
            .expect("nested entries inside the plugin dir must resolve");
        assert!(resolved.ends_with(Path::new("nested").join("entry.js")));
        // Undeclared targets resolve to nothing.
        assert!(resolve_plugin_entry(&plugins_root, sub, PluginTarget::Renderer).is_none());
        remove_profile_dir(&root);
    }

    #[test]
    fn entry_reads_reject_links_oversize_and_non_utf8() {
        let root = unique_profile_dir("reads");
        install_plugin(
            &root,
            "ok",
            r#"{"id":"ok","name":"Ok","version":"1.0.0","preload":"preload.js"}"#,
            &[(
                "preload.js",
                "module.exports.activate = () => {};".as_bytes(),
            )],
        );
        let found = discover_plugins(&root, "1.3.0");
        let ok = found
            .iter()
            .find(|plugin| plugin.manifest.id == "ok")
            .unwrap();
        let plugins_root = plugins_dir(&root);
        let source = read_plugin_source(&plugins_root, ok, PluginTarget::Preload)
            .expect("readable entry must load")
            .expect("non-empty entry must yield source");
        assert!(source.contains("activate"));
        // A planted symlink at the entry path must not be followed.
        let outside = unique_profile_dir("reads-outside");
        std::fs::create_dir_all(&outside).expect("outside dir must be creatable");
        std::fs::write(outside.join("secret.txt"), b"outside-bytes").unwrap();
        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_file(
            outside.join("secret.txt"),
            root.join(PLUGINS_DIR_NAME).join("ok").join("linked.js"),
        );
        #[cfg(not(windows))]
        let link_result = std::os::unix::fs::symlink(
            outside.join("secret.txt"),
            root.join(PLUGINS_DIR_NAME).join("ok").join("linked.js"),
        );
        if link_result.is_ok() {
            std::fs::write(
                root.join(PLUGINS_DIR_NAME).join("ok").join(MANIFEST_FILE),
                r#"{"id":"ok","name":"Ok","version":"1.0.0","preload":"linked.js"}"#,
            )
            .unwrap();
            let relinked = discover_plugins(&root, "1.3.0");
            let ok = relinked
                .iter()
                .find(|plugin| plugin.manifest.id == "ok")
                .unwrap();
            assert!(
                read_plugin_source(&plugins_root, ok, PluginTarget::Preload)
                    .expect("a planted link must read cleanly")
                    .is_none(),
                "a symlink entry must resolve to absent, never outside bytes"
            );
        } else {
            eprintln!("SKIP: link creation needs privileges in this environment");
        }
        // Over-limit sources fail closed with a typed code.
        std::fs::write(
            root.join(PLUGINS_DIR_NAME).join("ok").join("big.js"),
            vec![b'x'; MAX_PLUGIN_SOURCE_BYTES as usize + 1],
        )
        .unwrap();
        std::fs::write(
            root.join(PLUGINS_DIR_NAME).join("ok").join(MANIFEST_FILE),
            r#"{"id":"ok","name":"Ok","version":"1.0.0","preload":"big.js"}"#,
        )
        .unwrap();
        let error = read_plugin_source(
            &plugins_root,
            &discover_plugins(&root, "1.3.0")
                .into_iter()
                .find(|plugin| plugin.manifest.id == "ok")
                .unwrap(),
            PluginTarget::Preload,
        )
        .expect_err("over-limit sources must fail");
        assert_eq!(error.code, "PLUGIN_TOO_LARGE");
        // Non-UTF-8 sources fail closed.
        std::fs::write(
            root.join(PLUGINS_DIR_NAME).join("ok").join("evil.js"),
            vec![0xff, 0xfe],
        )
        .unwrap();
        std::fs::write(
            root.join(PLUGINS_DIR_NAME).join("ok").join(MANIFEST_FILE),
            r#"{"id":"ok","name":"Ok","version":"1.0.0","preload":"evil.js"}"#,
        )
        .unwrap();
        let error = read_plugin_source(
            &plugins_root,
            &discover_plugins(&root, "1.3.0")
                .into_iter()
                .find(|plugin| plugin.manifest.id == "ok")
                .unwrap(),
            PluginTarget::Preload,
        )
        .expect_err("non-UTF-8 sources must fail");
        assert_eq!(error.code, "PLUGIN_INVALID");
        remove_profile_dir(&root);
        remove_profile_dir(&outside);
    }
}

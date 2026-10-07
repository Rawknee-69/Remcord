//! Native backup-save (export) for Phase D (D4-1): `backup.save` ONLY.
//!
//! This module is the native counterpart of the save half of the Electron
//! backup handlers in `src/common/backup.ts` (`buildBackupZipBuffer` as
//! invoked from `src/discord/ipc.ts` `backupSave`, exposed as
//! `window.legcord.backup.save` via `src/discord/preload/bridge.ts`).
//! Restore (`backup.restore` / `applyBackupFromMap`) is NOT migrated here.
//!
//! Traced Electron behavior (actual implementation, not declarations):
//! - Request: one JSON string `{ includes, clientMods }` — no path, no
//!   filename, no dialog options. `clientMods` is renderer-built (localStorage
//!   snapshots + Shelter plugin maps) and never touches host files.
//! - Manifest: `{ version: 1, createdAt: new Date().toISOString(),
//!   appVersion: app.getVersion(), includes, clientMods }`, where `clientMods`
//!   carries ONLY the flagged keys (absent flags stay absent; present-but-null
//!   values serialize as `null`). Written as the FIRST zip entry, 2-space
//!   pretty JSON.
//! - Entries: `data/storage/settings.json` (config file, only when present),
//!   `data/quickCss.css` + `data/themes/**` (themes flag), `data/plugins/**` +
//!   `data/runtime-plugins/**` + `data/plugin-storage/**` (extensions flag),
//!   `bundles/<name>` for the 7 fixed `BUNDLE_FILES` (bundles flag). Missing
//!   files/directories are silently skipped; filesystem order is used.
//! - Dialog: `showSaveDialog` with translated title, no `defaultPath` beyond
//!   `Documents/legcord-backup-YYYY-MM-DD.zip`, one `ZIP/.zip` filter;
//!   cancellation returns `{ ok: false, error: "CANCELLED" }`; `.zip` is
//!   appended when missing; the buffer is written with `writeFileSync`.
//!
//! Native contract (deliberate, documented differences from Electron):
//! - Typed request: `BackupIncludes` (exactly the 7 known booleans,
//!   `deny_unknown_fields`) plus a bounded `clientMods` envelope. The renderer
//!   never supplies a path, filename, archive path, source path, or root.
//! - Bounds: each client-mod string <= 1 MiB, total clientMods <= 4 MiB,
//!   shelter payload <= 2 MiB with bounded nesting; each data file <= 1 MiB
//!   (bundles <= 32 MiB — real build artifacts), total uncompressed payload
//!   <= 64 MiB, entry count <= 4096. Overflow fails the whole save as
//!   `BACKUP_TOO_LARGE` — never a truncated archive.
//! - Links: `symlink_metadata` (never followed); symlinks, junctions, and any
//!   non-regular file are SKIPPED, never embedded. Electron's `readFileSync`
//!   / `statSync` follow a planted link, so a backup could embed outside
//!   bytes; native refuses that class entirely.
//! - Determinism: recursive trees are traversed in sorted lexical order.
//!   Electron uses filesystem order, so byte order may differ for the same
//!   input; the entry SET for the same input matches.
//! - Nothing is created for backup purposes: missing files/dirs are skipped,
//!   and (unlike the Quick CSS loader) a missing `quickCss.css` is NOT
//!   created by the export path.
//! - `custom.js` / `custom.css` are DATA ONLY (same as every other bundle
//!   name here): presence is inventoried into the archive, never executed.
//! - The renderer receives only `{ saved/canceled, bytes }` semantics — never
//!   the output path, directory, filename, or path-bearing errors.
//!
//! Logical roots owned by this module (all host-built from the profile dir):
//! `<profile>/storage/settings.json`, `<profile>/quickCss.css`,
//! `<profile>/themes/`, `<profile>/extensions/` (zip: `data/plugins`),
//! `<profile>/plugins/` (zip: `data/runtime-plugins`),
//! `<profile>/plugin-storage/`, and the 7 fixed bundle names at
//! `<profile>/`. There is no renderer path input at all.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Exact Electron `LEGCORD_BACKUP_VERSION` (`src/common/backup.ts`).
pub const BACKUP_VERSION: u8 = 1;
/// Exact Electron `MANIFEST_ENTRY`: the first archive entry.
pub const MANIFEST_ENTRY: &str = "manifest.json";

/// Maximum single collected file in bytes. Real settings/CSS/theme/plugin
/// files are far smaller; Electron is unbounded.
pub const MAX_BACKUP_FILE_BYTES: usize = 1024 * 1024;
/// Maximum single collected BUNDLE file in bytes. Real client-mod bundles
/// are multi-megabyte build artifacts (`equicord.js` is ~2.5 MiB in the DEV
/// profile), so the bundle allowlist carries the same 32 MiB ceiling the mod
/// substrate enforces on bundle retrieval — while every other section keeps
/// the 1 MiB bound. The total-payload cap below still applies to bundles.
pub const MAX_BACKUP_BUNDLE_BYTES: usize = 32 * 1024 * 1024;
/// Maximum total uncompressed payload across all collected entries.
pub const MAX_BACKUP_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// Maximum archive entry count (files plus the manifest).
pub const MAX_BACKUP_ENTRIES: usize = 4096;

/// Maximum single client-mod string in bytes (Vencord/Equicord snapshots).
pub const MAX_CLIENT_MOD_STRING_BYTES: usize = 1024 * 1024;
/// Maximum serialized `clientMods` envelope in bytes.
pub const MAX_CLIENT_MODS_BYTES: usize = 4 * 1024 * 1024;
/// Maximum serialized `shelter` client-mod payload in bytes.
pub const MAX_SHELTER_MOD_BYTES: usize = 2 * 1024 * 1024;
/// Maximum nesting depth accepted in the `shelter` payload value.
pub const MAX_SHELTER_DEPTH: usize = 32;
/// Maximum JSON node count accepted in the `shelter` payload value.
pub const MAX_SHELTER_NODES: usize = 65_536;

/// Translated save-dialog title key (same catalog `localization.get_lang`
/// serves). The renderer supplies none of the dialog strings.
pub const DIALOG_TITLE_KEY: &str = "backup-dialogSave-title";
/// Fallback title, byte-identical to the embedded `en-US` catalog value.
pub const FALLBACK_DIALOG_TITLE: &str = "Save Legcord backup";
/// Save-dialog file-type display name, mirroring the Electron filter.
pub const DIALOG_FILTER_NAME: &str = "ZIP";
/// Save-dialog file-type spec, mirroring the Electron `zip` extension.
pub const DIALOG_FILTER_SPEC: &str = "*.zip";

/// The exact seven include flags Electron's backup UI sends
/// (`BackupIncludeOptions` in `src/common/backup.ts`). Unknown keys are
/// rejected: the renderer cannot invent a new archive section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupIncludes {
    #[serde(rename = "legcordConfig")]
    pub legcord_config: bool,
    #[serde(rename = "legcordThemesAndQuickCss")]
    pub legcord_themes_and_quick_css: bool,
    #[serde(rename = "legcordExtensionPlugins")]
    pub legcord_extension_plugins: bool,
    #[serde(rename = "vencordModData")]
    pub vencord_mod_data: bool,
    #[serde(rename = "equicordModData")]
    pub equicord_mod_data: bool,
    #[serde(rename = "shelterModData")]
    pub shelter_mod_data: bool,
    #[serde(rename = "modBundles")]
    pub mod_bundles: bool,
}

/// Renderer-built client-mod blobs. These never touch host files: they are
/// localStorage snapshots / Shelter plugin maps the renderer already holds,
/// carried into the manifest verbatim within the bounded envelope. No
/// additional field may be supplied.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupClientMods {
    #[serde(rename = "vencordLocalStorage", default)]
    pub vencord_local_storage: Option<String>,
    #[serde(rename = "equicordLocalStorage", default)]
    pub equicord_local_storage: Option<String>,
    #[serde(default)]
    pub shelter: Option<serde_json::Value>,
}

/// The exact renderer request shape: `{ includes, clientMods }` — no path, no
/// filename, no dialog configuration, no arbitrary options.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupSavePayload {
    pub includes: BackupIncludes,
    #[serde(rename = "clientMods")]
    pub client_mods: BackupClientMods,
}

/// A validated save request: the includes plus the manifest-ready (filtered)
/// `clientMods` JSON object. The manifest object carries ONLY flagged keys,
/// with present-but-missing values as `null` — Electron parity.
#[derive(Debug, Clone)]
pub struct ValidatedBackupRequest {
    pub includes: BackupIncludes,
    /// Filtered `clientMods` object for the manifest (bounded, see limits).
    pub client_mods_manifest: serde_json::Value,
}

/// Typed failure: a stable machine-readable `code` plus a human message that
/// never contains a filesystem path, filename, or OS error text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupFailure {
    pub code: &'static str,
    pub message: String,
}

fn failure(code: &'static str, message: &str) -> BackupFailure {
    BackupFailure {
        code,
        message: message.to_owned(),
    }
}

fn walk_shelter_value(
    value: &serde_json::Value,
    depth: usize,
    seen: &mut usize,
) -> Result<(), BackupFailure> {
    if depth > MAX_SHELTER_DEPTH {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup client data exceeds the size limit",
        ));
    }
    *seen = seen.saturating_add(1);
    if *seen > MAX_SHELTER_NODES {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup client data exceeds the size limit",
        ));
    }
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                walk_shelter_value(item, depth + 1, seen)?;
            }
        }
        serde_json::Value::Object(map) => {
            for (_, item) in map {
                walk_shelter_value(item, depth + 1, seen)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Validate the `clientMods` envelope and build the manifest-ready filtered
/// object: only flagged keys are carried (Electron parity — unflagged keys
/// stay absent, flagged-but-missing values become `null`).
pub fn validate_client_mods(
    includes: &BackupIncludes,
    mods: &BackupClientMods,
) -> Result<serde_json::Value, BackupFailure> {
    if let Some(text) = mods.vencord_local_storage.as_ref() {
        if text.len() > MAX_CLIENT_MOD_STRING_BYTES {
            return Err(failure(
                "BACKUP_TOO_LARGE",
                "backup client data exceeds the size limit",
            ));
        }
    }
    if let Some(text) = mods.equicord_local_storage.as_ref() {
        if text.len() > MAX_CLIENT_MOD_STRING_BYTES {
            return Err(failure(
                "BACKUP_TOO_LARGE",
                "backup client data exceeds the size limit",
            ));
        }
    }
    if let Some(shelter) = mods.shelter.as_ref() {
        let serialized = serde_json::to_string(shelter).map_err(|_| {
            failure(
                "BACKUP_TOO_LARGE",
                "backup client data exceeds the size limit",
            )
        })?;
        if serialized.len() > MAX_SHELTER_MOD_BYTES {
            return Err(failure(
                "BACKUP_TOO_LARGE",
                "backup client data exceeds the size limit",
            ));
        }
        walk_shelter_value(shelter, 0, &mut 0)?;
    }
    let mut map = serde_json::Map::new();
    if includes.vencord_mod_data {
        map.insert(
            "vencordLocalStorage".to_owned(),
            mods.vencord_local_storage
                .as_ref()
                .map(|text| serde_json::Value::String(text.clone()))
                .unwrap_or(serde_json::Value::Null),
        );
    }
    if includes.equicord_mod_data {
        map.insert(
            "equicordLocalStorage".to_owned(),
            mods.equicord_local_storage
                .as_ref()
                .map(|text| serde_json::Value::String(text.clone()))
                .unwrap_or(serde_json::Value::Null),
        );
    }
    if includes.shelter_mod_data {
        map.insert(
            "shelter".to_owned(),
            mods.shelter.clone().unwrap_or(serde_json::Value::Null),
        );
    }
    let envelope = serde_json::Value::Object(map);
    let serialized_len = serde_json::to_string(&envelope)
        .map_err(|_| failure("BACKUP_FAILED", "backup manifest cannot be built"))?
        .len();
    if serialized_len > MAX_CLIENT_MODS_BYTES {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup client data exceeds the size limit",
        ));
    }
    Ok(envelope)
}

/// Validate one raw renderer payload string into a manifest-ready request.
/// Rejects unknown top-level/include/clientMod keys, oversized envelopes, and
/// anything that is not exactly `{ includes, clientMods }`.
pub fn validate_backup_request(payload: &str) -> Result<ValidatedBackupRequest, BackupFailure> {
    if payload.len() > MAX_CLIENT_MODS_BYTES + 64 * 1024 {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup request exceeds the size limit",
        ));
    }
    let parsed: BackupSavePayload = serde_json::from_str(payload)
        .map_err(|_| failure("INVALID_BACKUP", "backup request is invalid"))?;
    let client_mods_manifest = validate_client_mods(&parsed.includes, &parsed.client_mods)?;
    Ok(ValidatedBackupRequest {
        includes: parsed.includes,
        client_mods_manifest,
    })
}

/// Convert Unix millis to UTC `YYYY-MM-DDTHH:MM:SS.sssZ` (Electron
/// `new Date().toISOString()` semantics). Pure and tested against known
/// instants; the civil-date conversion is Howard Hinnant's days algorithm.
pub fn unix_millis_to_iso8601(millis: u64) -> String {
    let secs = millis / 1000;
    let ms = millis % 1000;
    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    // days_to_civil: days since 1970-01-01 -> (year, month, day).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year,
        m,
        d,
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
        ms
    )
}

/// Current UTC time as Electron `new Date().toISOString()`.
pub fn created_at_now() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    unix_millis_to_iso8601(millis)
}

/// Current UTC calendar date as `YYYY-MM-DD` (Electron
/// `new Date().toISOString().slice(0, 10)` semantics).
pub fn utc_today() -> String {
    created_at_now()[..10].to_owned()
}

/// Default save filename for a UTC date: `legcord-backup-YYYY-MM-DD.zip`
/// (Electron `defaultPath` basename semantics; the host owns the directory).
pub fn default_backup_filename(utc_date: &str) -> String {
    format!("legcord-backup-{utc_date}.zip")
}

/// Append `.zip` when the selected filename lacks it (case-insensitive),
/// mirroring Electron's `if (!filePath.toLowerCase().endsWith(".zip"))`.
pub fn ensure_zip_extension(filename: &str) -> String {
    if filename.to_ascii_lowercase().ends_with(".zip") {
        filename.to_owned()
    } else {
        format!("{filename}.zip")
    }
}

/// Build the manifest JSON value: `{ version: 1, createdAt, appVersion,
/// includes, clientMods }` with the exact seven include fields and the
/// filtered clientMods envelope. No paths or runtime metadata are serialized.
pub fn build_backup_manifest(
    request: &ValidatedBackupRequest,
    app_version: &str,
    created_at: &str,
) -> serde_json::Value {
    serde_json::json!({
        "version": BACKUP_VERSION,
        "createdAt": created_at,
        "appVersion": app_version,
        "includes": {
            "legcordConfig": request.includes.legcord_config,
            "legcordThemesAndQuickCss": request.includes.legcord_themes_and_quick_css,
            "legcordExtensionPlugins": request.includes.legcord_extension_plugins,
            "vencordModData": request.includes.vencord_mod_data,
            "equicordModData": request.includes.equicord_mod_data,
            "shelterModData": request.includes.shelter_mod_data,
            "modBundles": request.includes.mod_bundles,
        },
        "clientMods": request.client_mods_manifest,
    })
}

/// One collected archive entry: the `/`-separated zip name plus exact bytes.
#[derive(Debug, Clone)]
pub struct BackupEntry {
    pub name: String,
    pub data: Vec<u8>,
}

/// Read one host file through the no-follow bounded policy shared by every
/// collection path: symlinks/junctions are refused by the metadata check
/// (never followed), oversized files fail the whole save closed as
/// `BACKUP_TOO_LARGE`, and a missing file is `Ok(None)` (Electron skips
/// missing files). Any other failure is `BACKUP_UNAVAILABLE` with no path.
/// `max_bytes` is the per-section ceiling (`MAX_BACKUP_FILE_BYTES` everywhere
/// except the bundle allowlist, which uses `MAX_BACKUP_BUNDLE_BYTES`).
fn read_bounded_file(path: &Path, max_bytes: usize) -> Result<Option<Vec<u8>>, BackupFailure> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(failure(
                "BACKUP_UNAVAILABLE",
                "backup source is unavailable",
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        // Planted link, junction, directory, or device at a file position:
        // skipped, never followed — the bundles inventory convention.
        return Ok(None);
    }
    if metadata.len() > max_bytes as u64 {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup source exceeds the size limit",
        ));
    }
    let file = std::fs::File::open(path)
        .map_err(|_| failure("BACKUP_UNAVAILABLE", "backup source is unavailable"))?;
    let mut buffer = Vec::new();
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut buffer)
        .map_err(|_| failure("BACKUP_UNAVAILABLE", "backup source is unavailable"))?;
    if buffer.len() > max_bytes {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup source exceeds the size limit",
        ));
    }
    Ok(Some(buffer))
}

/// Push one entry while enforcing the count and total-size budgets. Overflow
/// fails the whole save closed — never a truncated archive.
fn push_entry(
    entries: &mut Vec<BackupEntry>,
    total_bytes: &mut usize,
    name: String,
    data: Vec<u8>,
) -> Result<(), BackupFailure> {
    if entries.len() >= MAX_BACKUP_ENTRIES {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup source exceeds the size limit",
        ));
    }
    *total_bytes = total_bytes.saturating_add(data.len());
    if *total_bytes > MAX_BACKUP_TOTAL_BYTES {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup source exceeds the size limit",
        ));
    }
    entries.push(BackupEntry { name, data });
    Ok(())
}

/// Collect one recursive tree in deterministic sorted order: `disk_dir`
/// (host-built) becomes `zip_prefix/**`. A missing or unreadable root yields
/// zero entries (Electron parity); an unlistable-but-present root is
/// `BACKUP_UNAVAILABLE`. Links are never descended; only regular files are
/// collected; empty directories produce no entries.
fn collect_tree(
    disk_dir: &Path,
    zip_prefix: &str,
    entries: &mut Vec<BackupEntry>,
    total_bytes: &mut usize,
) -> Result<(), BackupFailure> {
    let root_meta = match std::fs::symlink_metadata(disk_dir) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            return Err(failure(
                "BACKUP_UNAVAILABLE",
                "backup source is unavailable",
            ));
        }
    };
    if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
        return Ok(());
    }
    // Stack of (disk path, zip prefix). Depth-first with sorted siblings for
    // deterministic archives (Electron uses filesystem order — the entry SET
    // matches; the ORDER is intentionally deterministic here).
    let mut stack = vec![(disk_dir.to_path_buf(), zip_prefix.to_owned())];
    while let Some((dir, prefix)) = stack.pop() {
        let mut names: Vec<String> = Vec::new();
        match std::fs::read_dir(&dir) {
            Ok(listing) => {
                for entry in listing.flatten() {
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                return Err(failure(
                    "BACKUP_UNAVAILABLE",
                    "backup source is unavailable",
                ));
            }
        }
        names.sort();
        // Push children in reverse so the sorted order pops first.
        for name in names.into_iter().rev() {
            if name.is_empty() {
                continue;
            }
            let disk_path = dir.join(&name);
            // `file_name` can never contain a separator; normalize `\` (a
            // legal filename char on non-Windows) so zip names always use `/`.
            let zip_name = format!("{prefix}/{}", name.replace('\\', "/"));
            let meta = match std::fs::symlink_metadata(&disk_path) {
                Ok(meta) => meta,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    return Err(failure(
                        "BACKUP_UNAVAILABLE",
                        "backup source is unavailable",
                    ));
                }
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push((disk_path, zip_name));
            } else if meta.is_file() {
                if let Some(data) = read_bounded_file(&disk_path, MAX_BACKUP_FILE_BYTES)? {
                    push_entry(entries, total_bytes, zip_name, data)?;
                }
            }
        }
    }
    Ok(())
}

/// Collect every filesystem entry for a validated request, in archive order:
/// `data/storage/settings.json`, `data/quickCss.css`, `data/themes/**`,
/// `data/plugins/**` (disk: `extensions/`), `data/runtime-plugins/**`
/// (disk: `plugins/`), `data/plugin-storage/**`, then `bundles/*` in the
/// fixed `BUNDLE_FILES` allowlist order. The manifest is NOT included here —
/// the zip builder writes it first. Nothing is created: missing files, roots,
/// and empty directories simply contribute no entries.
pub fn collect_backup_entries(
    profile_dir: &Path,
    includes: &BackupIncludes,
) -> Result<Vec<BackupEntry>, BackupFailure> {
    let mut entries = Vec::new();
    let mut total_bytes = 0usize;
    if includes.legcord_config {
        let path = profile_dir.join("storage").join("settings.json");
        if let Some(data) = read_bounded_file(&path, MAX_BACKUP_FILE_BYTES)? {
            push_entry(
                &mut entries,
                &mut total_bytes,
                "data/storage/settings.json".to_owned(),
                data,
            )?;
        }
    }
    if includes.legcord_themes_and_quick_css {
        let quick_css = profile_dir.join("quickCss.css");
        if let Some(data) = read_bounded_file(&quick_css, MAX_BACKUP_FILE_BYTES)? {
            push_entry(
                &mut entries,
                &mut total_bytes,
                "data/quickCss.css".to_owned(),
                data,
            )?;
        }
        collect_tree(
            &profile_dir.join("themes"),
            "data/themes",
            &mut entries,
            &mut total_bytes,
        )?;
    }
    if includes.legcord_extension_plugins {
        // ZIP names are NOT the literal disk directory names: Electron maps
        // disk `extensions/` -> `data/plugins` and disk `plugins/` ->
        // `data/runtime-plugins`. Preserved exactly.
        collect_tree(
            &profile_dir.join("extensions"),
            "data/plugins",
            &mut entries,
            &mut total_bytes,
        )?;
        collect_tree(
            &profile_dir.join("plugins"),
            "data/runtime-plugins",
            &mut entries,
            &mut total_bytes,
        )?;
        collect_tree(
            &profile_dir.join("plugin-storage"),
            "data/plugin-storage",
            &mut entries,
            &mut total_bytes,
        )?;
    }
    if includes.mod_bundles {
        // The exact seven-name allowlist, in order. Reuses the bundle
        // discovery owner — never duplicated here. Bundles carry their own
        // multi-megabyte ceiling (real build artifacts); every other section
        // keeps the 1 MiB file bound.
        for name in crate::bundles::BUNDLE_FILES {
            let path = profile_dir.join(name);
            if let Some(data) = read_bounded_file(&path, MAX_BACKUP_BUNDLE_BYTES)? {
                push_entry(
                    &mut entries,
                    &mut total_bytes,
                    format!("bundles/{name}"),
                    data,
                )?;
            }
        }
    }
    Ok(entries)
}

/// Build the final archive bytes: the manifest (2-space pretty JSON, Electron
/// `JSON.stringify(manifest, null, 2)` parity) FIRST, then the collected
/// entries in order. Deflate compression via the `zip` crate only — no shell,
/// no external utilities. No directory entries, no duplicates (collection
/// produces each name once), `/` separators throughout.
pub fn build_backup_zip(
    manifest: &serde_json::Value,
    entries: &[BackupEntry],
) -> Result<Vec<u8>, BackupFailure> {
    if entries.len() + 1 > MAX_BACKUP_ENTRIES {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup source exceeds the size limit",
        ));
    }
    let manifest_text = serde_json::to_string_pretty(manifest)
        .map_err(|_| failure("BACKUP_FAILED", "backup archive cannot be built"))?;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer
        .start_file(MANIFEST_ENTRY, options)
        .map_err(|_| failure("BACKUP_FAILED", "backup archive cannot be built"))?;
    writer
        .write_all(manifest_text.as_bytes())
        .map_err(|_| failure("BACKUP_FAILED", "backup archive cannot be built"))?;
    for entry in entries {
        writer
            .start_file(entry.name.clone(), options)
            .map_err(|_| failure("BACKUP_FAILED", "backup archive cannot be built"))?;
        writer
            .write_all(&entry.data)
            .map_err(|_| failure("BACKUP_FAILED", "backup archive cannot be built"))?;
    }
    let cursor = writer
        .finish()
        .map_err(|_| failure("BACKUP_FAILED", "backup archive cannot be built"))?;
    Ok(cursor.into_inner())
}

/// Bounded save summary: cancellation plus the final archive size. No ids, no
/// arrays, no paths — the smallest response that distinguishes cancel from
/// success for the existing renderer result shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupSaveSummary {
    /// True when the user dismissed the picker. `bytes` is then zero.
    pub canceled: bool,
    /// Final archive size in bytes (zero when canceled).
    pub bytes: u64,
}

/// Outcome of the host-owned save picker: cancelled/empty, one host-selected
/// target path, or a host failure that never leaks paths.
#[derive(Debug)]
pub enum SavePickerOutcome {
    /// The user dismissed the dialog. Normal control flow: resolves the
    /// canceled summary, never rejects.
    Canceled,
    /// Host-selected save target (extension already enforced). The path stays
    /// host-side; only the byte count crosses the bridge.
    Selected(PathBuf),
    /// The picker itself failed (COM/shell error, unsupported platform). No
    /// path and no OS error text cross this boundary.
    Failed(BackupFailure),
}

/// Resolve the host-owned save-dialog configuration: translated title plus the
/// UTC-date default filename. The renderer influences none of this.
pub fn save_dialog_config(profile_dir: &Path) -> (String, String) {
    let found = crate::localization::lookup(profile_dir, DIALOG_TITLE_KEY);
    let title = if found.value.is_empty() {
        FALLBACK_DIALOG_TITLE.to_owned()
    } else {
        found.value
    };
    (title, default_backup_filename(&utc_today()))
}

/// Show the host-owned backup save picker and return its outcome.
///
/// Windows Common Item Dialog (`IFileSaveDialog`) preserving the Electron
/// semantics: translated title, one `ZIP/*.zip` filter, UTC-date default
/// filename, `.zip` appended when missing. MUST run on an STA thread with COM
/// already initialized (the dedicated backup worker owns that); the dialog is
/// entirely host-side and modal to the worker, never to the WebView thread.
///
/// Deliberate Electron differences: no `defaultPath` (Electron defaults into
/// Documents; the host-owned dialog sets the filename only and leaves the
/// folder to the dialog default), and no renderer-supplied anything.
#[cfg(target_os = "windows")]
pub fn show_save_picker(profile_dir: &Path) -> SavePickerOutcome {
    use windows::Win32::{
        System::Com::{CoCreateInstance, CLSCTX_ALL},
        UI::Shell::{FileSaveDialog, IFileSaveDialog},
    };

    let failed =
        |code: &'static str, message: &str| SavePickerOutcome::Failed(failure(code, message));
    let (title, default_filename) = save_dialog_config(profile_dir);
    let dialog: IFileSaveDialog =
        match unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_ALL) } {
            Ok(dialog) => dialog,
            Err(_) => {
                return failed("BACKUP_FAILED", "backup save picker cannot be shown");
            }
        };
    // Every COM interface call below is unsafe by windows-crate contract;
    // the STA worker guarantees the threading model (STA + COM initialized
    // exactly once for its thread) that makes them sound to invoke.
    unsafe { show_save_picker_inner(&dialog, &title, &default_filename) }
}

/// Inner picker flow: every COM call. Split so the caller documents the
/// safety contract once instead of scattering `unsafe` per call.
#[cfg(target_os = "windows")]
unsafe fn show_save_picker_inner(
    dialog: &windows::Win32::UI::Shell::IFileSaveDialog,
    title: &str,
    default_filename: &str,
) -> SavePickerOutcome {
    use windows::{
        core::{HSTRING, PCWSTR},
        Win32::UI::Shell::{Common::COMDLG_FILTERSPEC, SIGDN_FILESYSPATH},
    };

    let failed =
        |code: &'static str, message: &str| SavePickerOutcome::Failed(failure(code, message));
    if dialog.SetTitle(&HSTRING::from(title)).is_err() {
        return failed("BACKUP_FAILED", "backup save picker cannot be shown");
    }
    // The single ZIP filter, mirroring Electron's `filters: [{ name: "ZIP",
    // extensions: ["zip"] }]`. HSTRING buffers outlive the borrowed PCWSTRs.
    let filter_name = HSTRING::from(DIALOG_FILTER_NAME);
    let filter_spec = HSTRING::from(DIALOG_FILTER_SPEC);
    let specs = [COMDLG_FILTERSPEC {
        pszName: PCWSTR(filter_name.as_ptr()),
        pszSpec: PCWSTR(filter_spec.as_ptr()),
    }];
    if dialog.SetFileTypes(&specs).is_err() {
        return failed("BACKUP_FAILED", "backup save picker cannot be shown");
    }
    if dialog.SetFileTypeIndex(1).is_err() {
        return failed("BACKUP_FAILED", "backup save picker cannot be shown");
    }
    // Default filename only (no default folder: the dialog default applies —
    // the renderer never supplies a `defaultPath`). No `SetOkButtonLabel`:
    // Electron sets no `buttonLabel` on the save dialog, so the stock Save
    // caption applies in both implementations.
    if dialog
        .SetFileName(&HSTRING::from(default_filename))
        .is_err()
    {
        return failed("BACKUP_FAILED", "backup save picker cannot be shown");
    }
    if dialog.SetDefaultExtension(&HSTRING::from("zip")).is_err() {
        return failed("BACKUP_FAILED", "backup save picker cannot be shown");
    }
    // No owner window: the picker is host-side and worker-modal, so the
    // WebView/UI thread stays responsive while it is open.
    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) => {
            // ERROR_CANCELLED (1223): the user dismissed the dialog. Normal
            // control flow, never an error.
            if error.code() == windows::core::HRESULT::from_win32(1223) {
                return SavePickerOutcome::Canceled;
            }
            return failed("BACKUP_FAILED", "backup save picker cannot be shown");
        }
    }
    let item = match dialog.GetResult() {
        Ok(item) => item,
        Err(_) => {
            return failed("BACKUP_FAILED", "backup save picker cannot be shown");
        }
    };
    let display = match item.GetDisplayName(SIGDN_FILESYSPATH) {
        Ok(display) => display,
        Err(_) => {
            return failed("BACKUP_FAILED", "backup save picker cannot be shown");
        }
    };
    let text = unsafe { display.to_string().unwrap_or_default() };
    unsafe {
        windows::Win32::System::Com::CoTaskMemFree(Some(display.0 as *const std::ffi::c_void));
    }
    if text.is_empty() {
        return failed("BACKUP_FAILED", "backup save picker cannot be shown");
    }
    // `.zip` appended when the selected filename lacks it (Electron parity).
    // The path stays host-side; only the byte count crosses the bridge.
    let enforced = ensure_zip_extension(&text);
    SavePickerOutcome::Selected(PathBuf::from(enforced))
}

/// Non-Windows builds fail closed: the picker is a Windows WebView2 surface
/// (project rule), never a guessed shell command.
#[cfg(not(target_os = "windows"))]
pub fn show_save_picker(_profile_dir: &Path) -> SavePickerOutcome {
    SavePickerOutcome::Failed(failure(
        "BACKUP_UNSUPPORTED_PLATFORM",
        "backup save is not supported on this platform",
    ))
}

/// Persist finished archive bytes at the host-selected target: unique sibling
/// temp file plus same-directory rename, with temp cleanup on either failure
/// and a final size check. The temp name is unique (`create_new` semantics)
/// so it cannot be pre-planted; the rename replaces any existing file, so the
/// final file is either the old bytes or the complete new ZIP — never a
/// partial write. Returns the persisted byte count. No shell, no paths in
/// errors.
pub fn write_backup_file(target: &Path, zip_bytes: &[u8]) -> Result<u64, BackupFailure> {
    let parent = target
        .parent()
        .ok_or_else(|| failure("BACKUP_FAILED", "backup file cannot be written"))?;
    if !parent.is_dir() {
        return Err(failure("BACKUP_FAILED", "backup file cannot be written"));
    }
    let temp_name = format!(
        ".legcord-backup-{}.tmp",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0),
    );
    let temp_path = parent.join(format!("{temp_name}-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
    {
        Ok(mut file) => {
            if file.write_all(zip_bytes).is_err() {
                let _ = std::fs::remove_file(&temp_path);
                return Err(failure("BACKUP_FAILED", "backup file cannot be written"));
            }
        }
        Err(_) => {
            return Err(failure("BACKUP_FAILED", "backup file cannot be written"));
        }
    }
    if std::fs::rename(&temp_path, target).is_err() {
        let _ = std::fs::remove_file(&temp_path);
        return Err(failure("BACKUP_FAILED", "backup file cannot be written"));
    }
    let persisted = std::fs::symlink_metadata(target)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if persisted != zip_bytes.len() as u64 {
        return Err(failure("BACKUP_FAILED", "backup file cannot be written"));
    }
    Ok(persisted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_profile(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must advance")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "legcord-native-backup-test-{}-{tag}-{nanos}",
            std::process::id(),
        ));
        std::fs::create_dir_all(&dir).expect("must create temp profile");
        dir
    }

    fn all_true() -> BackupIncludes {
        BackupIncludes {
            legcord_config: true,
            legcord_themes_and_quick_css: true,
            legcord_extension_plugins: true,
            vencord_mod_data: true,
            equicord_mod_data: true,
            shelter_mod_data: true,
            mod_bundles: true,
        }
    }

    fn all_false() -> BackupIncludes {
        BackupIncludes {
            legcord_config: false,
            legcord_themes_and_quick_css: false,
            legcord_extension_plugins: false,
            vencord_mod_data: false,
            equicord_mod_data: false,
            shelter_mod_data: false,
            mod_bundles: false,
        }
    }

    #[test]
    fn includes_rejects_unknown_keys() {
        let parsed: Result<BackupIncludes, _> = serde_json::from_str(
            r#"{"legcordConfig":true,"legcordThemesAndQuickCss":true,"legcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true,"evil":true}"#,
        );
        assert!(parsed.is_err());
        let parsed: Result<BackupSavePayload, _> = serde_json::from_str(
            r#"{"includes":{"legcordConfig":true,"legcordThemesAndQuickCss":true,"legcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true},"clientMods":{},"outputPath":"C:\\x.zip"}"#,
        );
        assert!(parsed.is_err());
        let parsed: Result<BackupSavePayload, _> = serde_json::from_str(
            r#"{"includes":{"legcordConfig":true,"legcordThemesAndQuickCss":true,"legcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true},"clientMods":{"vencordLocalStorage":"x","extra":{}}}"#,
        );
        assert!(parsed.is_err());
    }

    #[test]
    fn valid_request_filters_client_mods_by_includes() {
        let request = validate_backup_request(
            r#"{"includes":{"legcordConfig":true,"legcordThemesAndQuickCss":false,"legcordExtensionPlugins":false,"vencordModData":true,"equicordModData":false,"shelterModData":true,"modBundles":false},"clientMods":{"vencordLocalStorage":"{\"a\":1}","equicordLocalStorage":"dropped","shelter":{"plugins":{}}}}"#,
        )
        .expect("valid request must validate");
        assert!(request.includes.legcord_config);
        assert!(request.includes.vencord_mod_data);
        let map = request.client_mods_manifest.as_object().unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map["vencordLocalStorage"], "{\"a\":1}");
        assert_eq!(map["shelter"]["plugins"], serde_json::json!({}));
        assert!(map.get("equicordLocalStorage").is_none());
    }

    #[test]
    fn flagged_but_missing_values_become_null_like_electron() {
        let request = validate_backup_request(
            r#"{"includes":{"legcordConfig":false,"legcordThemesAndQuickCss":false,"legcordExtensionPlugins":false,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":false},"clientMods":{}}"#,
        )
        .expect("missing values must validate");
        let map = request.client_mods_manifest.as_object().unwrap();
        assert_eq!(map["vencordLocalStorage"], serde_json::Value::Null);
        assert_eq!(map["equicordLocalStorage"], serde_json::Value::Null);
        assert_eq!(map["shelter"], serde_json::Value::Null);
    }

    #[test]
    fn oversized_client_strings_are_rejected() {
        let big = "x".repeat(MAX_CLIENT_MOD_STRING_BYTES + 1);
        let payload = format!(
            "{{\"includes\":{{\"legcordConfig\":false,\"legcordThemesAndQuickCss\":false,\"legcordExtensionPlugins\":false,\"vencordModData\":true,\"equicordModData\":false,\"shelterModData\":false,\"modBundles\":false}},\"clientMods\":{{\"vencordLocalStorage\":\"{big}\"}}}}"
        );
        let failure = validate_backup_request(&payload).expect_err("oversized string must fail");
        assert_eq!(failure.code, "BACKUP_TOO_LARGE");
        assert!(!failure.message.contains(&big[..16]));
    }

    #[test]
    fn oversized_shelter_payload_is_rejected() {
        let big = "x".repeat(MAX_SHELTER_MOD_BYTES);
        let payload = format!(
            "{{\"includes\":{{\"legcordConfig\":false,\"legcordThemesAndQuickCss\":false,\"legcordExtensionPlugins\":false,\"vencordModData\":false,\"equicordModData\":false,\"shelterModData\":true,\"modBundles\":false}},\"clientMods\":{{\"shelter\":{{\"blob\":\"{big}\"}}}}}}"
        );
        let failure = validate_backup_request(&payload).expect_err("oversized shelter must fail");
        assert_eq!(failure.code, "BACKUP_TOO_LARGE");
    }

    #[test]
    fn deeply_nested_shelter_payload_is_rejected() {
        let mut nested = serde_json::json!({"leaf": true});
        for _ in 0..(MAX_SHELTER_DEPTH + 4) {
            nested = serde_json::json!({"next": nested});
        }
        let mods = BackupClientMods {
            vencord_local_storage: None,
            equicord_local_storage: None,
            shelter: Some(nested),
        };
        let failure = validate_client_mods(&all_true(), &mods).expect_err("deep nesting must fail");
        assert_eq!(failure.code, "BACKUP_TOO_LARGE");
    }

    #[test]
    fn iso8601_matches_electron_representation() {
        assert_eq!(unix_millis_to_iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(unix_millis_to_iso8601(1_000), "1970-01-01T00:00:01.000Z");
        assert_eq!(
            unix_millis_to_iso8601(1_770_163_200_123),
            "2026-02-04T00:00:00.123Z"
        );
        let now = created_at_now();
        assert_eq!(now.len(), 24);
        assert!(now.ends_with('Z'));
        assert_eq!(&now[10..11], "T");
        assert_eq!(&now[19..20], ".");
        assert_eq!(utc_today().len(), 10);
        assert_eq!(&now[..10], utc_today());
    }

    #[test]
    fn default_filename_and_extension_match_electron() {
        assert_eq!(
            default_backup_filename("2026-10-07"),
            "legcord-backup-2026-10-07.zip"
        );
        assert_eq!(ensure_zip_extension("backup"), "backup.zip");
        assert_eq!(ensure_zip_extension("backup.zip"), "backup.zip");
        assert_eq!(ensure_zip_extension("BACKUP.ZIP"), "BACKUP.ZIP");
        assert_eq!(ensure_zip_extension("backup.Zip"), "backup.Zip");
    }

    #[test]
    fn manifest_has_version_one_and_exact_shapes() {
        let request = validate_backup_request(
            r#"{"includes":{"legcordConfig":true,"legcordThemesAndQuickCss":true,"legcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true},"clientMods":{"vencordLocalStorage":"v"}}"#,
        )
        .unwrap();
        let manifest = build_backup_manifest(&request, "1.3.0", "2026-10-07T00:00:00.000Z");
        assert_eq!(manifest["version"], 1);
        assert_eq!(manifest["createdAt"], "2026-10-07T00:00:00.000Z");
        assert_eq!(manifest["appVersion"], "1.3.0");
        let includes = manifest["includes"].as_object().unwrap();
        assert_eq!(includes.len(), 7);
        for key in [
            "legcordConfig",
            "legcordThemesAndQuickCss",
            "legcordExtensionPlugins",
            "vencordModData",
            "equicordModData",
            "shelterModData",
            "modBundles",
        ] {
            assert_eq!(includes[key], true);
        }
        assert_eq!(manifest["clientMods"]["vencordLocalStorage"], "v");
        assert_eq!(
            manifest["clientMods"]["equicordLocalStorage"],
            serde_json::Value::Null
        );
        let text = serde_json::to_string(&manifest).unwrap();
        assert!(!text.contains("storage"));
    }

    #[test]
    fn empty_profile_collects_nothing() {
        let profile = temp_profile("empty");
        let entries = collect_backup_entries(&profile, &all_true()).unwrap();
        assert!(entries.is_empty());
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn disabled_includes_collect_nothing() {
        let profile = temp_profile("disabled");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(profile.join("storage").join("settings.json"), "{}").unwrap();
        let entries = collect_backup_entries(&profile, &all_false()).unwrap();
        assert!(entries.is_empty());
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn collection_uses_exact_zip_names_in_order() {
        let profile = temp_profile("names");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(profile.join("storage").join("settings.json"), "{\"a\":1}").unwrap();
        std::fs::write(profile.join("quickCss.css"), ":root{}").unwrap();
        std::fs::create_dir_all(profile.join("themes").join("b-theme")).unwrap();
        std::fs::write(profile.join("themes").join("b-theme").join("src.css"), "b").unwrap();
        std::fs::create_dir_all(profile.join("themes").join("a-theme")).unwrap();
        std::fs::write(profile.join("themes").join("a-theme").join("src.css"), "a").unwrap();
        std::fs::create_dir_all(profile.join("extensions").join("ext")).unwrap();
        std::fs::write(profile.join("extensions").join("ext").join("m.json"), "{}").unwrap();
        std::fs::create_dir_all(profile.join("plugins").join("rt")).unwrap();
        std::fs::write(profile.join("plugins").join("rt").join("index.js"), "x").unwrap();
        std::fs::create_dir_all(profile.join("plugin-storage").join("p")).unwrap();
        std::fs::write(profile.join("plugin-storage").join("p").join("n.txt"), "y").unwrap();
        std::fs::write(profile.join("shelter.js"), "shelter").unwrap();
        std::fs::write(profile.join("custom.css"), "custom").unwrap();
        // A directory at a file position contributes nothing.
        std::fs::create_dir_all(profile.join("vencord.js")).unwrap();
        let entries = collect_backup_entries(&profile, &all_true()).unwrap();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "data/storage/settings.json",
                "data/quickCss.css",
                "data/themes/a-theme/src.css",
                "data/themes/b-theme/src.css",
                "data/plugins/ext/m.json",
                "data/runtime-plugins/rt/index.js",
                "data/plugin-storage/p/n.txt",
                "bundles/shelter.js",
                "bundles/custom.css",
            ]
        );
        // Exact bytes are preserved.
        let settings = entries
            .iter()
            .find(|entry| entry.name == "data/storage/settings.json")
            .unwrap();
        assert_eq!(settings.data, b"{\"a\":1}");
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn missing_quickcss_is_not_created_by_collection() {
        let profile = temp_profile("no-create");
        let entries = collect_backup_entries(&profile, &all_true()).unwrap();
        assert!(entries.is_empty());
        assert!(!profile.join("quickCss.css").exists());
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn bundle_order_follows_the_fixed_allowlist() {
        let profile = temp_profile("bundles");
        for name in crate::bundles::BUNDLE_FILES {
            std::fs::write(profile.join(name), format!("content-{name}")).unwrap();
        }
        let entries = collect_backup_entries(&profile, &all_true()).unwrap();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "bundles/vencord.js",
                "bundles/vencord.css",
                "bundles/equicord.js",
                "bundles/equicord.css",
                "bundles/shelter.js",
                "bundles/custom.js",
                "bundles/custom.css",
            ]
        );
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn oversized_files_fail_the_save_closed() {
        let profile = temp_profile("big");
        let big = vec![b'x'; MAX_BACKUP_FILE_BYTES + 1];
        std::fs::write(profile.join("quickCss.css"), &big).unwrap();
        let failure = collect_backup_entries(&profile, &all_true()).expect_err("must fail");
        assert_eq!(failure.code, "BACKUP_TOO_LARGE");
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn bundles_carry_the_multi_megabyte_ceiling() {
        // Real build artifacts exceed the 1 MiB section bound (`equicord.js`
        // is ~2.5 MiB in the DEV profile): the allowlist ceiling accepts
        // them while the total cap still applies.
        let profile = temp_profile("bigbundle");
        let big = vec![b'x'; MAX_BACKUP_FILE_BYTES + 16];
        std::fs::write(profile.join("shelter.js"), &big).unwrap();
        let entries = collect_backup_entries(&profile, &all_true()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "bundles/shelter.js");
        assert_eq!(entries[0].data.len(), big.len());
        // Past the bundle ceiling, the save still fails closed.
        let huge = vec![b'x'; MAX_BACKUP_BUNDLE_BYTES + 1];
        std::fs::write(profile.join("shelter.js"), &huge).unwrap();
        let failure = collect_backup_entries(&profile, &all_true()).expect_err("must fail");
        assert_eq!(failure.code, "BACKUP_TOO_LARGE");
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn planted_links_are_skipped_never_followed() {
        let profile = temp_profile("links");
        let outside = temp_profile("links-outside");
        std::fs::write(outside.join("secret.txt"), "outside-bytes").unwrap();
        std::fs::create_dir_all(profile.join("themes")).unwrap();
        let link = profile.join("themes").join("evil.css");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(outside.join("secret.txt"), &link).is_ok();
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(outside.join("secret.txt"), &link).is_ok();
        if !made {
            eprintln!("SKIP: link creation needs privileges in this environment");
            std::fs::remove_dir_all(&profile).ok();
            std::fs::remove_dir_all(&outside).ok();
            return;
        }
        let entries = collect_backup_entries(&profile, &all_true()).unwrap();
        assert!(entries.is_empty());
        std::fs::remove_dir_all(&profile).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn zip_writes_manifest_first_with_slash_separators() {
        let request = validate_backup_request(
            r#"{"includes":{"legcordConfig":true,"legcordThemesAndQuickCss":true,"legcordExtensionPlugins":true,"vencordModData":false,"equicordModData":false,"shelterModData":false,"modBundles":true},"clientMods":{}}"#,
        )
        .unwrap();
        let manifest = build_backup_manifest(&request, "1.3.0", "2026-10-07T00:00:00.000Z");
        let entries = vec![BackupEntry {
            name: "data/themes/a/src.css".to_owned(),
            data: b":root{}".to_vec(),
        }];
        let bytes = build_backup_zip(&manifest, &entries).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        assert_eq!(archive.len(), 2);
        assert_eq!(archive.name_for_index(0).unwrap(), "manifest.json");
        assert_eq!(archive.name_for_index(1).unwrap(), "data/themes/a/src.css");
        for index in 0..archive.len() {
            let name = archive.name_for_index(index).unwrap().to_owned();
            assert!(!name.contains('\\'));
            assert!(!name.starts_with('/'));
            assert!(!name.contains(".."));
        }
        let mut stored = String::new();
        archive
            .by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut stored)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["includes"]["modBundles"], true);
    }

    #[test]
    fn write_creates_complete_files_and_overwrites_atomically() {
        let dir = temp_profile("write");
        let target = dir.join("out.zip");
        let bytes = build_backup_zip(&serde_json::json!({"version": 1}), &[]).unwrap();
        let persisted = write_backup_file(&target, &bytes).unwrap();
        assert_eq!(persisted, bytes.len() as u64);
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        // Overwrite replaces the whole file; no temp residue remains.
        let bytes2 = build_backup_zip(&serde_json::json!({"version": 1, "x": 2}), &[]).unwrap();
        assert_ne!(bytes, bytes2);
        write_backup_file(&target, &bytes2).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), bytes2);
        let temps: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
            .filter(|name| name.to_string_lossy().contains(".tmp"))
            .collect();
        assert!(temps.is_empty());
        // A missing parent directory fails without creating anything.
        let missing = dir.join("no-such-dir").join("out.zip");
        assert!(write_backup_file(&missing, &bytes).is_err());
        assert!(!missing.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn failures_carry_codes_only_never_paths() {
        let failure = failure("BACKUP_FAILED", "backup file cannot be written");
        assert!(!failure.message.contains('\\'));
        assert!(!failure.message.contains('/'));
        let profile = temp_profile("codes");
        let oversized = "x".repeat(MAX_CLIENT_MOD_STRING_BYTES + 1);
        let payload = format!(
            "{{\"includes\":{{\"legcordConfig\":false,\"legcordThemesAndQuickCss\":false,\"legcordExtensionPlugins\":false,\"vencordModData\":true,\"equicordModData\":false,\"shelterModData\":false,\"modBundles\":false}},\"clientMods\":{{\"vencordLocalStorage\":\"{oversized}\"}}}}"
        );
        let failure = validate_backup_request(&payload).unwrap_err();
        assert_eq!(failure.code, "BACKUP_TOO_LARGE");
        assert!(!failure.message.contains("vencord"));
        std::fs::remove_dir_all(&profile).ok();
    }

    /// Serializes the gated live-dialog tests: the picker carries the
    /// translated save title, so two concurrent dialogs would be
    /// indistinguishable to the window lookup.
    static UI_DIALOG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Live Windows dialog validation (gated: opens a real modal saver).
    ///
    /// Run explicitly with `LEGCORD_NATIVE_BACKUP_SAVE_UI_TEST=1`; ordinary
    /// `cargo test` skips it so no dialog ever appears in unattended runs.
    /// The picker runs on its own STA thread exactly like the production
    /// worker (COM initialized once for that thread); this thread polls for
    /// the dialog window by its translated title, asserts the filename field
    /// carries the UTC-date default (`legcord-backup-YYYY-MM-DD.zip` —
    /// proving `SetFileName` applied), then dismisses with `WM_CLOSE` and
    /// requires a silent `Canceled` outcome. Dialog appearance itself proves
    /// `SetTitle`/`SetFileTypes`/`SetFileTypeIndex`/`SetDefaultExtension`
    /// all succeeded (any failure returns before `Show`).
    #[test]
    fn live_dialog_shows_translated_title_default_filename_and_cancels_silently() {
        if std::env::var("LEGCORD_NATIVE_BACKUP_SAVE_UI_TEST").as_deref() != Ok("1") {
            eprintln!(
                "SKIP: live dialog test needs LEGCORD_NATIVE_BACKUP_SAVE_UI_TEST=1 (opens a real modal saver)"
            );
            return;
        }
        #[cfg(target_os = "windows")]
        {
            use super::{save_dialog_config, show_save_picker, SavePickerOutcome};
            use std::sync::mpsc;
            use windows::{
                core::{HSTRING, PCWSTR},
                Win32::{
                    Foundation::{HWND, LPARAM, WPARAM},
                    System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
                    UI::WindowsAndMessaging::{
                        FindWindowW, PostMessageW, SendMessageW, WM_CLOSE, WM_GETTEXT,
                        WM_GETTEXTLENGTH,
                    },
                },
            };

            let _dialog_guard = UI_DIALOG_LOCK.lock().expect("ui dialog lock must hold");
            let profile = temp_profile("ui");
            let (title, default_filename) = save_dialog_config(&profile);
            assert!(default_filename.starts_with("legcord-backup-"));
            assert!(default_filename.ends_with(".zip"));
            let (done_tx, done_rx) = mpsc::channel();
            std::thread::Builder::new()
                .name("legcord-backup-save-ui-test-sta".to_owned())
                .spawn(move || {
                    let com_init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
                    let com_owned = com_init == windows::core::HRESULT(0);
                    let outcome = show_save_picker(&profile);
                    let _ = done_tx.send(outcome);
                    std::fs::remove_dir_all(&profile).ok();
                    if com_owned {
                        unsafe {
                            CoUninitialize();
                        }
                    }
                })
                .expect("saver test thread must start");

            // The dialog must appear with the translated title. Appearance
            // proves every configuration call before Show succeeded.
            let title_h = HSTRING::from(title.as_str());
            let mut dialog: Option<HWND> = None;
            for _ in 0..100 {
                if let Ok(found) = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title_h.as_ptr())) }
                {
                    dialog = Some(found);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let dialog = dialog.expect("the backup save dialog must appear");
            // The filename field must carry the UTC-date default filename.
            // Child creation can lag the top-level window, so descendants
            // are enumerated with retries: the first Edit control under the
            // dialog is the filename field.
            fn find_filename_edit(dialog: HWND) -> Option<HWND> {
                use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;
                struct Search {
                    found: Option<HWND>,
                }
                unsafe extern "system" fn visit(
                    child: HWND,
                    context: LPARAM,
                ) -> windows::core::BOOL {
                    use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
                    let search = unsafe { &mut *(context.0 as *mut Search) };
                    let mut name = [0u16; 64];
                    let read = unsafe { GetClassNameW(child, &mut name) };
                    let class = String::from_utf16_lossy(&name[..read as usize]);
                    if class == "Edit" {
                        search.found = Some(child);
                        return windows::core::BOOL(0);
                    }
                    windows::core::BOOL(1)
                }
                let mut search = Search { found: None };
                let _ = unsafe {
                    EnumChildWindows(
                        Some(dialog),
                        Some(visit),
                        LPARAM(&mut search as *mut Search as isize),
                    )
                };
                search.found
            }
            let mut filename_edit: Option<HWND> = None;
            for _ in 0..50 {
                filename_edit = find_filename_edit(dialog);
                if filename_edit.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let filename_edit = filename_edit.expect("the dialog filename field must exist");
            let field_len =
                unsafe { SendMessageW(filename_edit, WM_GETTEXTLENGTH, Some(WPARAM(0)), None) };
            let mut field_text = vec![0u16; field_len.0 as usize + 1];
            let read = unsafe {
                SendMessageW(
                    filename_edit,
                    WM_GETTEXT,
                    Some(WPARAM(field_text.len())),
                    Some(LPARAM(field_text.as_mut_ptr() as isize)),
                )
            };
            let field_value =
                String::from_utf16_lossy(&field_text[..read.0.max(0) as usize]).replace('\0', "");
            assert_eq!(
                field_value, default_filename,
                "the filename field must carry the UTC-date default"
            );
            // Dismiss: cancellation is silent control flow, never an error.
            unsafe {
                PostMessageW(Some(dialog), WM_CLOSE, WPARAM(0), LPARAM(0))
                    .expect("the dialog must accept WM_CLOSE");
            }
            match done_rx.recv_timeout(std::time::Duration::from_secs(30)) {
                Ok(SavePickerOutcome::Canceled) => {}
                other => panic!("dismissed saver must cancel silently, got {other:?}"),
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            eprintln!("SKIP: live dialog test is Windows-only");
        }
    }
}

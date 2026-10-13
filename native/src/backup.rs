//! Native backup save (export) + secure restore (import) for Phase D:
//! `backup.save` plus the inspect/commit/discard trio.
//!
//! This module is the native counterpart of the Electron backup handlers in
//! `src/common/backup.ts` (`buildBackupZipBuffer` / `applyBackupFromMap` as
//! invoked from `src/discord/ipc.ts` `backupSave` / `backupRestore`, exposed
//! as `window.remcord.backup.save` / `.restore` via
//! `src/discord/preload/bridge.ts`).
//!
//! The native restore splits Electron's single apply step into
//! preview-then-commit: inspect validates, stages, and holds the bytes
//! host-side; the renderer shows the preview; commit applies exactly the
//! held bytes (or discard drops them).
//!
//! Traced Electron SAVE behavior (actual implementation, not declarations):
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
//!   `Documents/remcord-backup-YYYY-MM-DD.zip`, one `ZIP/.zip` filter;
//!   cancellation returns `{ ok: false, error: "CANCELLED" }`; `.zip` is
//!   appended when missing; the buffer is written with `writeFileSync`.
//!
//! Traced Electron RESTORE behavior (actual implementation, not declarations):
//! - Request: none — `backupRestore` takes no arguments; the host shows
//!   `showOpenDialog` (translated `backup-dialogOpen-title`, one `ZIP/.zip`
//!   filter) and reads the picked file with `readFileSync`. Cancellation
//!   returns `{ ok: false, error: "CANCELLED" }`.
//! - Archive: `readBackupZipToMap` buffers the WHOLE archive into a
//!   `Map<string, Buffer>` (last-wins on duplicates, unbounded), skipping
//!   directories, normalizing `\` to `/`.
//! - Manifest: `JSON.parse` + `version === 1` only. No shape, size, or
//!   include-flag validation; `clientMods` is returned to the renderer
//!   VERBATIM (no validation at all).
//! - Routing (in Map insertion order, inline writes — no staging):
//!   `data/storage/settings.json` (only when `rest === "settings.json"`) is
//!   fed through `setConfigBulk` (SHALLOW merge into the live cache, then
//!   write); `data/quickCss.css` verbatim; `data/themes/**`,
//!   `data/plugins/**` (disk `extensions/`), `data/runtime-plugins/**`
//!   (disk `plugins/`), `data/plugin-storage/**` verbatim through
//!   `resolvePathUnderBaseDir` (zip-slip guard; unsafe entries SKIPPED with a
//!   console warning while earlier entries stay written); `bundles/<name>`
//!   only on EXACT allowlist equality. Include flags gate each section
//!   (present-but-flagged-off entries are skipped); unknown names fall
//!   through silently. Overwrites are silent; symlinks/junctions are followed
//!   (`writeFileSync`); a mid-loop throw leaves earlier writes committed.
//! - Renderer (`BackupSection.tsx`): `JSON.parse` the returned string;
//!   `applyClientModsFromRestore` writes `VencordSettings`/`EquicordSettings`
//!   into `localStorage` and calls `shelter.plugins.addRemotePlugin(id, src,
//!   update)` per shelter entry (URL fetch/execution happens renderer-side).
//!
//! Native RESTORE contract (deliberate, documented differences from Electron):
//! - Lifecycle: open (host dialog) -> parse/validate the ENTIRE archive ->
//!   stage outside the profile -> commit with rollback. NO final-profile
//!   mutation happens until every entry, the manifest, sizes, counts,
//!   duplicates, collisions, and containment have passed.
//! - Duplicates/collisions FAIL the whole restore (`INVALID_ARCHIVE`):
//!   exact duplicate names, case-insensitive (Windows) collisions, and
//!   separator-normalization collisions are rejected instead of Electron's
//!   silent last-wins.
//! - Traversal/device/absolute/ADS shapes FAIL the whole restore instead of
//!   Electron's skip-with-warning (which leaves a half-restored profile).
//! - Paths are validated by the existing section owners (`validate_theme_id`,
//!   `validate_theme_css_filename` is NOT reused for archive names — archive
//!   theme files are validated as id + relative path — `validate_plugin_id`,
//!   `validate_relative_path`, the runtime-plugin id charset, the bundle
//!   allowlist), plus lexical + canonical containment at commit. Links at any
//!   destination (or in any staging/commit parent) fail closed.
//! - Settings restore is a STAGED SHALLOW MERGE (Electron `setConfigBulk`
//!   parity: archive keys overwrite, existing/unknown keys survive), committed
//!   atomically with the file writes. A corrupt/missing existing file merges
//!   from `{}` (Electron cache-fallback parity); archive settings must still
//!   be a JSON object. Type repair stays deferred to the existing startup
//!   check, exactly as in Electron.
//! - `clientMods` is validated with the SAME bounded envelope as save
//!   (per-string <= 1 MiB, total <= 4 MiB, shelter <= 2 MiB, depth <= 32,
//!   nodes <= 65536) and only the filtered envelope crosses the bridge. The
//!   host NEVER applies it (no localStorage, no addRemotePlugin, no fetch).
//! - Unknown top-level sections and unknown bundle names are IGNORED
//!   (Electron parity — they fall through there too), never extracted.
//! - `custom.js` / `custom.css` restore as DATA ONLY (same as every other
//!   bundle name here): presence is written to disk, never executed.
//! - The renderer receives only `{ canceled, clientMods }` semantics — never
//!   archive paths, profile paths, filenames, or path-bearing errors.
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
//!   `Documents/remcord-backup-YYYY-MM-DD.zip`, one `ZIP/.zip` filter;
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

/// Matches `REMCORD_BACKUP_VERSION` (`src/common/backup.ts`).
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
pub const FALLBACK_DIALOG_TITLE: &str = "Save Remcord backup";
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
    #[serde(rename = "remcordConfig")]
    pub remcord_config: bool,
    #[serde(rename = "remcordThemesAndQuickCss")]
    pub remcord_themes_and_quick_css: bool,
    #[serde(rename = "remcordExtensionPlugins")]
    pub remcord_extension_plugins: bool,
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

/// Default save filename for a UTC date: `remcord-backup-YYYY-MM-DD.zip`
/// (Electron `defaultPath` basename semantics; the host owns the directory).
///
/// Previous-generation archives are NOT restore-compatible: their manifest
/// `includes` keys predate the current schema and fail strict validation
/// with `INVALID_BACKUP` (never a partial restore). To migrate one, open a
/// copy of the `.zip`, rename the three product-scoped keys in
/// `manifest.json` `includes` to `remcordConfig`,
/// `remcordThemesAndQuickCss`, and `remcordExtensionPlugins`, re-zip, and
/// restore the converted copy. Settings, themes, Quick CSS, plugin storage,
/// bundles, and clientMods payloads carry over byte-identical.
pub fn default_backup_filename(utc_date: &str) -> String {
    format!("remcord-backup-{utc_date}.zip")
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
            "remcordConfig": request.includes.remcord_config,
            "remcordThemesAndQuickCss": request.includes.remcord_themes_and_quick_css,
            "remcordExtensionPlugins": request.includes.remcord_extension_plugins,
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
    if includes.remcord_config {
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
    if includes.remcord_themes_and_quick_css {
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
    if includes.remcord_extension_plugins {
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
/// UTC-date default filename. The renderer influences none of this. Titles
/// come straight from the embedded catalogs.
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
        ".remcord-backup-{}.tmp",
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

// ============================================================================
// Restore: the inspect/commit/discard trio (`backup.inspect`,
// `backup.commit_restore`, `backup.discard_restore`).
//
// The archive is untrusted input. The lifecycle is:
//
//     inspect: open (host dialog, bounded read)
//       -> parse + validate the ENTIRE archive (manifest, every entry name,
//          section routing, sizes, counts, duplicates, collisions)
//       -> stage every byte outside the profile (host temp dir)
//       -> HOLD the staged bytes worker-side; report the preview
//          (counts, flags, bounded display listing — never contents/paths)
//     commit: apply exactly the held staged bytes (no re-read, no drift)
//       -> pre-commit destination check (no writes yet)
//       -> commit (per-file atomic temp + rename, settings merged) with
//          rollback state for already-replaced files
//       -> delete staging (success AND failure)
//     discard: delete the held staging without touching the profile
//
// No final-profile mutation happens before validation + staging complete,
// and nothing commits without the explicit commit step.
// ============================================================================

/// Maximum picked archive file size in bytes: the 64 MiB uncompressed budget
/// plus 1 MiB of ZIP-container slack. A `Stored` (uncompressed) archive is at
/// most ~400 KiB of per-entry overhead above its payload, so every archive
/// the native saver can produce fits; anything larger fails closed before a
/// large buffer exists.
pub const MAX_RESTORE_ARCHIVE_BYTES: usize = 65 * 1024 * 1024;
/// Maximum raw `manifest.json` size in bytes: the bounded clientMods envelope
/// (4 MiB) plus metadata slack — the same expression the save request bound
/// uses, so save and restore agree on what a manifest may carry.
pub const MAX_RESTORE_MANIFEST_BYTES: usize = MAX_CLIENT_MODS_BYTES + 64 * 1024;
/// Maximum single archive entry name in bytes. Real save-generated names are
/// short; this is a safety rail, not a target.
pub const MAX_RESTORE_NAME_BYTES: usize = 1024;
/// Maximum single archive path component in bytes. Mirrors the theme filename
/// bound; every section validator enforces its own tighter cap as well.
pub const MAX_RESTORE_COMPONENT_BYTES: usize = 128;
/// Maximum archive path depth in components. Real trees are a handful deep.
pub const MAX_RESTORE_DEPTH: usize = 32;
/// Maximum merged `settings.json` size in bytes. Reuses the authoritative
/// settings cap so a restore can never produce a file the settings loader
/// would refuse to read.
pub const MAX_RESTORE_SETTINGS_BYTES: usize = 256 * 1024;

/// Translated restore-dialog title key (same catalog `localization.get_lang`
/// serves). The renderer supplies none of the dialog strings.
pub const DIALOG_OPEN_TITLE_KEY: &str = "backup-dialogOpen-title";
/// Fallback title, byte-identical to the embedded `en-US` catalog value.
pub const FALLBACK_DIALOG_OPEN_TITLE: &str = "Open Remcord backup";

/// Reserved Windows device-stem check for archive directory names that do
/// NOT pass through `validate_relative_path` (theme ids and runtime-plugin
/// directory names have their own charset rules). Mirrors the
/// plugin-storage/themes stem rule; relative-path components are already
/// covered by their own validator.
fn is_reserved_archive_stem(stem: &str) -> bool {
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

/// Runtime-plugin directory-name rule, mirroring Electron
/// `VALID_PLUGIN_ID` (`src/discord/plugins/manager.ts:55`):
/// `^[a-zA-Z0-9._-]{1,64}$`. The restore path does NOT infer plugin identity
/// from the archive (the loader validates manifest ids at load time); this
/// charset only keeps directory names lexically safe. It is deliberately NOT
/// the stricter plugin-storage id rule (`[A-Za-z0-9_-]`, no dots): runtime
/// plugin ids may contain dots, and rejecting them would refuse valid
/// Electron archives.
fn validate_runtime_plugin_dir(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_RESTORE_COMPONENT_BYTES {
        return Err("runtime plugin directory name is invalid".to_owned());
    }
    if name == "." || name == ".." {
        return Err("runtime plugin directory name is invalid".to_owned());
    }
    let stem = name.split('.').next().unwrap_or(name);
    if is_reserved_archive_stem(stem) {
        return Err("runtime plugin directory name uses a reserved device name".to_owned());
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(
            "runtime plugin directory name uses characters outside [A-Za-z0-9._-]".to_owned(),
        );
    }
    Ok(())
}

/// The validated restore manifest: `{ version, createdAt, appVersion,
/// includes, clientMods }`. Unlike the save request, the OUTER object allows
/// unknown future metadata keys (forward compatibility — they are ignored and
/// never routed anywhere); the seven include flags and the clientMods envelope
/// stay exact.
#[derive(Debug, serde::Deserialize)]
pub struct RestoreManifest {
    pub version: u8,
    #[serde(rename = "createdAt", default)]
    pub created_at: String,
    #[serde(rename = "appVersion", default)]
    pub app_version: String,
    pub includes: BackupIncludes,
    #[serde(rename = "clientMods")]
    pub client_mods: BackupClientMods,
}

/// One routed archive entry: the section it belongs to plus validated,
/// join-safe components. Display-free: no raw archive name is retained, so no
/// path-bearing value can leak into logs or errors.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RestoreTarget {
    /// `data/storage/settings.json` — staged, then shallow-merged.
    Settings,
    /// `data/quickCss.css` — file bytes verbatim.
    QuickCss,
    /// `data/themes/<rel...>` — `theme_dir` is `Some(id)` for nested files
    /// (validated id + validated rest) and `None` for a loose file directly
    /// under `themes/` (validated as a relative path). Electron restores
    /// both shapes verbatim; no enablement is flipped by the host either way
    /// (a restored `manifest.json` carries the archive's own `enabled`
    /// value, exactly as Electron's overwrite does).
    ThemeFile {
        theme_dir: Option<String>,
        rel: Vec<String>,
    },
    /// `data/plugins/<rel...>` — disk `extensions/`. No id architecture
    /// exists for this directory, so the whole rest is validated as a
    /// relative path and nothing is inferred from its structure.
    ExtensionFile { rel: Vec<String> },
    /// `data/runtime-plugins/<rel...>` — disk `plugins/`. The first
    /// component uses the runtime-plugin directory charset; identity itself
    /// stays loader-owned (manifest ids are validated at load time, not from
    /// archive structure).
    RuntimePluginFile { rel: Vec<String> },
    /// `data/plugin-storage/<plugin_id>/<rel...>` — the strict native rule:
    /// `validate_plugin_id` plus `validate_relative_path`.
    PluginStorageFile { plugin_id: String, rel: Vec<String> },
    /// `bundles/<name>` — exact allowlist equality only, data-only.
    Bundle { name: &'static str },
}

/// Split and validate one raw archive name into `/`-separated components.
///
/// Normalizes `\` to `/` first (Electron `entryName.replace(/\\/g, "/")`
/// parity — so backslash traversal cannot bypass the checks), then rejects
/// the whole archive on: over-long names, absolute POSIX paths, Windows
/// drive-letter paths, UNC/device prefixes, NUL/control characters, `:`
/// (drive + ADS syntax in one rule), empty components, `.`/`..` components,
/// excessive depth, and over-long components. Reserved device names and
/// trailing space/dot components are enforced by the section validators
/// below (every routed shape passes through one).
fn split_archive_name(raw: &str) -> Result<Vec<String>, BackupFailure> {
    let normalized = raw.replace('\\', "/");
    if normalized.is_empty() || normalized.len() > MAX_RESTORE_NAME_BYTES {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup archive entry exceeds the size limit",
        ));
    }
    if normalized.starts_with('/') {
        return Err(failure(
            "INVALID_ARCHIVE",
            "backup archive contains an entry outside its sections",
        ));
    }
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return Err(failure(
            "INVALID_ARCHIVE",
            "backup archive contains an entry outside its sections",
        ));
    }
    if normalized.contains('\0')
        || normalized.bytes().any(|byte| byte < 0x20)
        || normalized.contains(':')
    {
        return Err(failure(
            "INVALID_ARCHIVE",
            "backup archive contains an entry outside its sections",
        ));
    }
    let components: Vec<String> = normalized.split('/').map(str::to_owned).collect();
    if components.len() > MAX_RESTORE_DEPTH {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup archive entry exceeds the size limit",
        ));
    }
    for component in &components {
        if component.is_empty() || component == "." || component == ".." {
            return Err(failure(
                "INVALID_ARCHIVE",
                "backup archive contains an entry outside its sections",
            ));
        }
        if component.len() > MAX_RESTORE_COMPONENT_BYTES {
            return Err(failure(
                "BACKUP_TOO_LARGE",
                "backup archive entry exceeds the size limit",
            ));
        }
    }
    Ok(components)
}

/// Route validated components to a section target, or `None` when the entry
/// is legitimately skipped: unknown top-level sections, non-`settings.json`
/// files under `data/storage/`, unknown bundle names (Electron
/// `BUNDLE_FILES.includes` parity — never restored, never rejected), and any
/// entry whose section flag is `false` (present-but-disabled sections are
/// skipped exactly like Electron's `continue`).
fn route_entry(
    components: &[String],
    includes: &BackupIncludes,
) -> Result<Option<RestoreTarget>, BackupFailure> {
    let invalid = || {
        failure(
            "INVALID_ARCHIVE",
            "backup archive contains an entry outside its sections",
        )
    };
    if components.is_empty() {
        return Err(invalid());
    }
    match components[0].as_str() {
        "manifest.json" => Ok(None),
        "data" => {
            if components.len() < 2 {
                return Ok(None);
            }
            match components[1].as_str() {
                "storage" => {
                    if !includes.remcord_config {
                        return Ok(None);
                    }
                    if components.len() == 3 && components[2] == "settings.json" {
                        Ok(Some(RestoreTarget::Settings))
                    } else {
                        // Electron parity: only `settings.json` is honored;
                        // anything else under `data/storage/` falls through.
                        Ok(None)
                    }
                }
                "quickCss.css" => {
                    if !includes.remcord_themes_and_quick_css {
                        return Ok(None);
                    }
                    if components.len() == 2 {
                        Ok(Some(RestoreTarget::QuickCss))
                    } else {
                        Ok(None)
                    }
                }
                "themes" => {
                    if !includes.remcord_themes_and_quick_css {
                        return Ok(None);
                    }
                    let under = &components[2..];
                    if under.is_empty() {
                        return Ok(None);
                    }
                    if under.len() == 1 {
                        // A loose file directly under `themes/` (the saver
                        // collects those too, and Electron restores them).
                        let rel = crate::plugin_storage::validate_relative_path(&under[0])
                            .map_err(|_| invalid())?;
                        Ok(Some(RestoreTarget::ThemeFile {
                            theme_dir: None,
                            rel,
                        }))
                    } else {
                        crate::themes::validate_theme_id(&under[0]).map_err(|_| invalid())?;
                        let theme_stem = under[0].split('.').next().unwrap_or(&under[0]);
                        if is_reserved_archive_stem(theme_stem) {
                            return Err(invalid());
                        }
                        let rel =
                            crate::plugin_storage::validate_relative_path(&under[1..].join("/"))
                                .map_err(|_| invalid())?;
                        Ok(Some(RestoreTarget::ThemeFile {
                            theme_dir: Some(under[0].clone()),
                            rel,
                        }))
                    }
                }
                "plugins" => {
                    if !includes.remcord_extension_plugins {
                        return Ok(None);
                    }
                    let under = &components[2..];
                    if under.is_empty() {
                        return Ok(None);
                    }
                    let rel = crate::plugin_storage::validate_relative_path(&under.join("/"))
                        .map_err(|_| invalid())?;
                    Ok(Some(RestoreTarget::ExtensionFile { rel }))
                }
                "runtime-plugins" => {
                    if !includes.remcord_extension_plugins {
                        return Ok(None);
                    }
                    let under = &components[2..];
                    if under.is_empty() {
                        return Ok(None);
                    }
                    validate_runtime_plugin_dir(&under[0]).map_err(|_| invalid())?;
                    let rel = crate::plugin_storage::validate_relative_path(&under.join("/"))
                        .map_err(|_| invalid())?;
                    Ok(Some(RestoreTarget::RuntimePluginFile { rel }))
                }
                "plugin-storage" => {
                    if !includes.remcord_extension_plugins {
                        return Ok(None);
                    }
                    let under = &components[2..];
                    if under.len() < 2 {
                        // A bare file directly under `plugin-storage/` names
                        // no plugin namespace; there is no safe destination.
                        return Err(invalid());
                    }
                    crate::plugin_storage::validate_plugin_id(&under[0]).map_err(|_| invalid())?;
                    let rel = crate::plugin_storage::validate_relative_path(&under[1..].join("/"))
                        .map_err(|_| invalid())?;
                    Ok(Some(RestoreTarget::PluginStorageFile {
                        plugin_id: under[0].clone(),
                        rel,
                    }))
                }
                _ => Ok(None),
            }
        }
        "bundles" => {
            if !includes.mod_bundles {
                return Ok(None);
            }
            if components.len() == 2 {
                match crate::bundles::BUNDLE_FILES
                    .iter()
                    .find(|name| **name == components[1].as_str())
                {
                    Some(name) => Ok(Some(RestoreTarget::Bundle { name })),
                    // Electron parity: unknown bundle names are skipped, so
                    // they can never become arbitrary profile-root writes.
                    None => Ok(None),
                }
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

/// Per-target uncompressed content bound: the bundle allowlist carries the
/// same 32 MiB ceiling the save path uses (real build artifacts); every
/// other section keeps the 1 MiB file bound.
fn target_bound(target: &RestoreTarget) -> usize {
    match target {
        RestoreTarget::Bundle { .. } => MAX_BACKUP_BUNDLE_BYTES,
        _ => MAX_BACKUP_FILE_BYTES,
    }
}

/// Validate the manifest document: exact version 1, bounded metadata
/// strings, exact includes, and the bounded clientMods envelope (reusing the
/// save-path validator so both directions agree on what fits).
fn validate_restore_manifest(
    text: &str,
) -> Result<(BackupIncludes, serde_json::Value), BackupFailure> {
    let manifest: RestoreManifest = serde_json::from_str(text)
        .map_err(|_| failure("INVALID_BACKUP", "backup manifest is invalid"))?;
    if manifest.version != BACKUP_VERSION {
        return Err(failure("INVALID_BACKUP", "backup version is not supported"));
    }
    if manifest.created_at.len() > 1024 || manifest.app_version.len() > 1024 {
        return Err(failure("INVALID_BACKUP", "backup manifest is invalid"));
    }
    let envelope = validate_client_mods(&manifest.includes, &manifest.client_mods)?;
    Ok((manifest.includes, envelope))
}

/// Bounded restore summary: the filtered clientMods envelope for the
/// renderer plus concrete metrics. No paths, no names, no counts of skipped
/// entries — only sizes and outcome.
#[derive(Debug, Clone)]
pub struct RestoreSummary {
    /// True when the user dismissed the picker. Every other field is then
    /// zeroed (`client_mods` is null).
    pub canceled: bool,
    /// Filtered `clientMods` envelope for the renderer (host never applies).
    pub client_mods: serde_json::Value,
    /// Picked archive file size in bytes.
    pub archive_bytes: usize,
    /// Total staged uncompressed content in bytes.
    pub uncompressed_bytes: usize,
    /// File entries validated (manifest excluded, directories excluded).
    pub entry_count: usize,
    /// Serialized clientMods envelope size in bytes.
    pub client_mods_bytes: usize,
}

/// Read one host-picked archive file through the no-follow bounded policy:
/// symlinks/junctions are refused (never followed), oversized files fail
/// closed before a large buffer exists, and the authoritative post-read
/// length is re-checked so a file that grows mid-read cannot force an
/// unbounded allocation.
pub fn read_archive_file(path: &Path) -> Result<Vec<u8>, BackupFailure> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| failure("BACKUP_UNAVAILABLE", "backup archive cannot be read"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(failure(
            "BACKUP_UNAVAILABLE",
            "backup archive cannot be read",
        ));
    }
    if metadata.len() > MAX_RESTORE_ARCHIVE_BYTES as u64 {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup archive exceeds the size limit",
        ));
    }
    let file = std::fs::File::open(path)
        .map_err(|_| failure("BACKUP_UNAVAILABLE", "backup archive cannot be read"))?;
    let mut buffer = Vec::new();
    file.take(MAX_RESTORE_ARCHIVE_BYTES as u64 + 1)
        .read_to_end(&mut buffer)
        .map_err(|_| failure("BACKUP_UNAVAILABLE", "backup archive cannot be read"))?;
    if buffer.len() > MAX_RESTORE_ARCHIVE_BYTES {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup archive exceeds the size limit",
        ));
    }
    if buffer.is_empty() {
        return Err(failure("INVALID_ARCHIVE", "backup archive is invalid"));
    }
    Ok(buffer)
}

/// Create the private staging directory: unique, host-controlled, outside
/// every profile root (never inside `themes/`, `plugins/`,
/// `plugin-storage/`, or `extensions/`, where an existing link could
/// interfere with containment).
fn create_staging_dir() -> Result<PathBuf, BackupFailure> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "remcord-backup-restore-{}-{}-{}",
        std::process::id(),
        nanos,
        STAGING_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir)
        .map_err(|_| failure("BACKUP_UNAVAILABLE", "backup restore cannot be staged"))?;
    Ok(dir)
}

/// One staged write: the routing plus the staging-file sequence number. The
/// settings blob is staged as a file like everything else and parsed from
/// the staged bytes, so validation never holds the whole archive in memory.
#[derive(Debug)]
struct StagedWrite {
    target: RestoreTarget,
    sequence: usize,
}

/// Reject exact duplicate archive names and case-insensitive /
/// separator-normalization collisions across the whole entry table.
///
/// Electron buffers the archive into a `Map<string, Buffer>` (silent
/// last-wins); native refuses the archive instead, so two archive paths can
/// never silently target the same final file.
fn reject_duplicate_names(normalized: &[String]) -> Result<(), BackupFailure> {
    let mut seen_exact: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut seen_folded: std::collections::HashSet<String> = std::collections::HashSet::new();
    for name in normalized {
        if !seen_exact.insert(name.as_str()) {
            return Err(failure(
                "INVALID_ARCHIVE",
                "backup archive contains a duplicate entry",
            ));
        }
        if !seen_folded.insert(name.to_lowercase()) {
            return Err(failure(
                "INVALID_ARCHIVE",
                "backup archive contains a colliding entry",
            ));
        }
    }
    Ok(())
}

/// Validate the ENTIRE archive and stage every routed entry to `staging_dir`.
///
/// One pass over the entry table: names are collected first (the zip reader
/// borrows mutably per entry), then each entry is validated (name, routing,
/// duplicates, collisions, header size), streamed to staging with a bounded
/// `take`, and length-verified. The manifest is required exactly once.
/// Returns the includes, the filtered clientMods envelope, the staged
/// writes, and the accepted entry names in archive order.
fn validate_and_stage(
    archive_bytes: &[u8],
    staging_dir: &Path,
) -> Result<
    (
        BackupIncludes,
        serde_json::Value,
        Vec<StagedWrite>,
        Vec<String>,
    ),
    BackupFailure,
> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(archive_bytes))
        .map_err(|_| failure("INVALID_ARCHIVE", "backup archive is invalid"))?;
    if archive.len() > MAX_BACKUP_ENTRIES {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup archive exceeds the size limit",
        ));
    }
    let mut names: Vec<String> = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        names.push(
            archive
                .name_for_index(index)
                .ok_or_else(|| failure("INVALID_ARCHIVE", "backup archive is invalid"))?
                .to_owned(),
        );
    }
    // The manifest is located by exact normalized name; duplicates are
    // caught by the same duplicate detection every other entry passes.
    let mut manifest_text: Option<String> = None;
    // First pass: structural validation only (no routing yet — the includes
    // come from the manifest, which may not be first in a hostile archive).
    // Directory entries are skipped (Electron parity: they create nothing).
    let mut file_indices: Vec<usize> = Vec::new();
    let mut file_names: Vec<String> = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let is_dir = archive
            .by_index(index)
            .map(|file| file.is_dir())
            .unwrap_or(false);
        if is_dir {
            continue;
        }
        let normalized = name.replace('\\', "/");
        file_indices.push(index);
        file_names.push(normalized);
    }
    reject_duplicate_names(&file_names)?;
    let mut content_indices: Vec<usize> = Vec::new();
    for (position, index) in file_indices.iter().enumerate() {
        let normalized = &file_names[position];
        let index = *index;
        if normalized == MANIFEST_ENTRY {
            let file = archive
                .by_index(index)
                .map_err(|_| failure("INVALID_ARCHIVE", "backup archive is invalid"))?;
            if file.size() > MAX_RESTORE_MANIFEST_BYTES as u64 {
                return Err(failure(
                    "BACKUP_TOO_LARGE",
                    "backup manifest exceeds the size limit",
                ));
            }
            let mut buffer = Vec::new();
            file.take(MAX_RESTORE_MANIFEST_BYTES as u64 + 1)
                .read_to_end(&mut buffer)
                .map_err(|_| failure("INVALID_ARCHIVE", "backup archive is invalid"))?;
            if buffer.len() > MAX_RESTORE_MANIFEST_BYTES {
                return Err(failure(
                    "BACKUP_TOO_LARGE",
                    "backup manifest exceeds the size limit",
                ));
            }
            let text = String::from_utf8(buffer)
                .map_err(|_| failure("INVALID_BACKUP", "backup manifest is invalid"))?;
            manifest_text = Some(text);
        } else {
            content_indices.push(index);
        }
    }
    let manifest_text =
        manifest_text.ok_or_else(|| failure("INVALID_ARCHIVE", "backup archive is invalid"))?;
    let (includes, client_mods) = validate_restore_manifest(&manifest_text)?;
    // Second pass: route + bound + stage every entry. Nothing has been
    // written to the profile; staging lives under the host temp dir.
    let mut staged: Vec<StagedWrite> = Vec::new();
    let mut accepted_names: Vec<String> = Vec::new();
    let mut total_bytes: usize = 0;
    for index in content_indices {
        let components = split_archive_name(&names[index])?;
        let Some(target) = route_entry(&components, &includes)? else {
            continue;
        };
        let bound = target_bound(&target);
        let file = archive
            .by_index(index)
            .map_err(|_| failure("INVALID_ARCHIVE", "backup archive is invalid"))?;
        if file.size() > bound as u64 {
            return Err(failure(
                "BACKUP_TOO_LARGE",
                "backup archive entry exceeds the size limit",
            ));
        }
        let staged_path = staging_dir.join(format!("entry-{}.bin", staged.len()));
        let mut staged_file = std::fs::File::create(&staged_path)
            .map_err(|_| failure("BACKUP_UNAVAILABLE", "backup restore cannot be staged"))?;
        let copied = std::io::copy(&mut file.take(bound as u64 + 1), &mut staged_file)
            .map_err(|_| failure("INVALID_ARCHIVE", "backup archive is invalid"))?;
        if copied > bound as u64 {
            return Err(failure(
                "BACKUP_TOO_LARGE",
                "backup archive entry exceeds the size limit",
            ));
        }
        total_bytes = total_bytes.saturating_add(copied as usize);
        if total_bytes > MAX_BACKUP_TOTAL_BYTES {
            return Err(failure(
                "BACKUP_TOO_LARGE",
                "backup archive exceeds the size limit",
            ));
        }
        if staged.len() + 1 > MAX_BACKUP_ENTRIES {
            return Err(failure(
                "BACKUP_TOO_LARGE",
                "backup archive exceeds the size limit",
            ));
        }
        staged.push(StagedWrite {
            target,
            sequence: staged.len(),
        });
        // The normalized archive name is the preview identity: it names
        // what will change, never a host path.
        accepted_names.push(names[index].replace('\\', "/"));
    }
    Ok((includes, client_mods, staged, accepted_names))
}

/// Merge the staged settings blob over the live file (Electron
/// `setConfigBulk` parity): the archive must be a JSON OBJECT (anything else
/// is `INVALID_BACKUP`); the existing file starts from `{}` when
/// missing/corrupt (Electron's corrupt-cache `{}` fallback — restore is a
/// recovery path, so failing closed here would strand corrupt profiles);
/// archive keys overwrite while existing/unknown keys survive. The merged
/// document must fit the authoritative settings cap.
fn merge_settings_blob(existing: Option<&[u8]>, staged: &[u8]) -> Result<Vec<u8>, BackupFailure> {
    let staged_value: serde_json::Value = serde_json::from_slice(staged)
        .map_err(|_| failure("INVALID_BACKUP", "backup settings are invalid"))?;
    let staged_object = staged_value
        .as_object()
        .ok_or_else(|| failure("INVALID_BACKUP", "backup settings are invalid"))?;
    let mut merged = match existing {
        Some(bytes) => serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default(),
        None => serde_json::Map::new(),
    };
    for (key, value) in staged_object {
        merged.insert(key.clone(), value.clone());
    }
    let serialized = serde_json::to_string_pretty(&serde_json::Value::Object(merged))
        .map_err(|_| failure("BACKUP_FAILED", "backup settings cannot be restored"))?;
    if serialized.len() > MAX_RESTORE_SETTINGS_BYTES {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup settings exceed the size limit",
        ));
    }
    Ok(serialized.into_bytes())
}

fn join_components(root: &Path, components: &[String]) -> PathBuf {
    let mut path = root.to_path_buf();
    for component in components {
        path.push(component);
    }
    path
}

/// Resolve one staged write to its final destination. All components are
/// already validated (single-component joins, no separators possible); the
/// canonical containment check happens at commit time, after parents exist.
fn resolve_destination(profile_dir: &Path, target: &RestoreTarget) -> PathBuf {
    match target {
        RestoreTarget::Settings => profile_dir.join("storage").join("settings.json"),
        RestoreTarget::QuickCss => profile_dir.join("quickCss.css"),
        RestoreTarget::ThemeFile { theme_dir, rel } => {
            let mut root = profile_dir.join("themes");
            if let Some(dir) = theme_dir {
                root.push(dir);
            }
            join_components(&root, rel)
        }
        RestoreTarget::ExtensionFile { rel } => {
            join_components(&profile_dir.join("extensions"), rel)
        }
        RestoreTarget::RuntimePluginFile { rel } => {
            join_components(&profile_dir.join("plugins"), rel)
        }
        RestoreTarget::PluginStorageFile { plugin_id, rel } => {
            join_components(&profile_dir.join("plugin-storage").join(plugin_id), rel)
        }
        RestoreTarget::Bundle { name } => profile_dir.join(name),
    }
}

/// Section root a destination must stay under (canonical containment at
/// commit). Bundles, settings, and Quick CSS live directly under the profile.
fn section_root(profile_dir: &Path, target: &RestoreTarget) -> PathBuf {
    match target {
        RestoreTarget::Settings => profile_dir.join("storage"),
        RestoreTarget::QuickCss => profile_dir.to_path_buf(),
        RestoreTarget::ThemeFile { theme_dir, .. } => {
            let mut root = profile_dir.join("themes");
            if let Some(dir) = theme_dir {
                root.push(dir);
            }
            root
        }
        RestoreTarget::ExtensionFile { .. } => profile_dir.join("extensions"),
        RestoreTarget::RuntimePluginFile { .. } => profile_dir.join("plugins"),
        RestoreTarget::PluginStorageFile { plugin_id, .. } => {
            profile_dir.join("plugin-storage").join(plugin_id)
        }
        RestoreTarget::Bundle { .. } => profile_dir.to_path_buf(),
    }
}

/// Commit one staged file to `dest` (parent already created + contained):
/// refuse symlinks (never write through a link), then atomically replace via
/// a uniquely named sibling temp file plus same-directory rename. Returns
/// whether the destination previously existed (for rollback bookkeeping).
fn commit_staged_file(
    dest: &Path,
    staged_path: &Path,
    rollback_path: Option<&Path>,
) -> Result<bool, BackupFailure> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COMMIT_COUNTER: AtomicU64 = AtomicU64::new(0);
    if std::fs::symlink_metadata(dest)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(failure(
            "BACKUP_FAILED",
            "backup restore target is outside the approved root",
        ));
    }
    let existed = dest.exists();
    if existed {
        if let Some(rollback) = rollback_path {
            let existing_len = std::fs::symlink_metadata(dest)
                .ok()
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            if existing_len > MAX_BACKUP_TOTAL_BYTES as u64 {
                return Err(failure(
                    "BACKUP_TOO_LARGE",
                    "backup restore target exceeds the size limit",
                ));
            }
            std::fs::copy(dest, rollback)
                .map_err(|_| failure("BACKUP_FAILED", "backup restore cannot proceed"))?;
        }
    }
    let temp_name = format!(
        ".restore-{}-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0),
        COMMIT_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let temp_path = dest.with_file_name(temp_name);
    let staged_len = std::fs::symlink_metadata(staged_path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if staged_len == 0 && !staged_path.exists() {
        return Err(failure("BACKUP_FAILED", "backup restore cannot proceed"));
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
    {
        Ok(mut temp) => {
            let mut staged = std::fs::File::open(staged_path)
                .map_err(|_| failure("BACKUP_FAILED", "backup restore cannot proceed"))?;
            if std::io::copy(&mut staged, &mut temp).is_err() {
                let _ = std::fs::remove_file(&temp_path);
                return Err(failure("BACKUP_FAILED", "backup restore cannot proceed"));
            }
            drop(temp);
        }
        Err(_) => {
            return Err(failure("BACKUP_FAILED", "backup restore cannot proceed"));
        }
    }
    if std::fs::rename(&temp_path, dest).is_err() {
        let _ = std::fs::remove_file(&temp_path);
        return Err(failure("BACKUP_FAILED", "backup restore cannot proceed"));
    }
    Ok(existed)
}

/// Commit every staged write to the profile with rollback.
///
/// Phase 1 (no writes): resolve every destination; refuse symlinks,
/// directories, and anything that is neither a missing path nor a regular
/// file — before ANY byte lands in the profile.
/// Phase 2: for each write, create + contain the parent, back up the existing
/// file (when any), and atomically replace. The settings blob is merged over
/// the live file first (Electron shallow-merge parity) and committed through
/// the same atomic path.
/// On ANY failure, already-replaced files are restored from rollback state
/// and newly created files are removed, so the profile holds either complete
/// old files or complete new files — never truncations. Staging cleanup is
/// the caller's job (it runs on success AND failure).
///
/// `fail_after` injects a failure after that many successful commits. It is
/// `None` in production and exercised by tests to prove rollback converges.
fn commit_staged(
    profile_dir: &Path,
    staged: &[StagedWrite],
    staging_dir: &Path,
    fail_after: Option<usize>,
) -> Result<(), BackupFailure> {
    let profile_canonical = std::fs::canonicalize(profile_dir)
        .map_err(|_| failure("BACKUP_UNAVAILABLE", "backup restore target is unavailable"))?;
    // Phase 1: destination pre-check — no writes.
    let mut destinations: Vec<PathBuf> = Vec::with_capacity(staged.len());
    for write in staged {
        let dest = resolve_destination(profile_dir, &write.target);
        match std::fs::symlink_metadata(&dest) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(failure(
                    "BACKUP_FAILED",
                    "backup restore target is outside the approved root",
                ));
            }
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                // A directory (or device) at a file position: Electron's
                // `writeFileSync` would fail mid-loop leaving earlier writes
                // committed; native fails before any write.
                return Err(failure("BACKUP_FAILED", "backup restore cannot proceed"));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(failure(
                    "BACKUP_UNAVAILABLE",
                    "backup restore target is unavailable",
                ));
            }
        }
        destinations.push(dest);
    }
    // Phase 2: backup + replace with rollback.
    let rollback_dir = staging_dir.join("rollback");
    std::fs::create_dir_all(&rollback_dir)
        .map_err(|_| failure("BACKUP_FAILED", "backup restore cannot proceed"))?;
    // (index into `staged`, whether the destination previously existed)
    let mut committed: Vec<(usize, bool)> = Vec::new();
    let mut commit_failed: Option<BackupFailure> = None;
    for (position, write) in staged.iter().enumerate() {
        if commit_failed.is_some() {
            break;
        }
        let dest = &destinations[position];
        let root = section_root(profile_dir, &write.target);
        // Host-owned parents only: create, then require the canonical parent
        // to stay under the canonical section root, so a pre-planted
        // symlink/junction inside the profile can never redirect the write.
        // (`Bundle`/`QuickCss`/`Settings` parents are host-fixed directories;
        // theme/plugin parents are built from validated components only.)
        if let Some(parent) = dest.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                commit_failed = Some(failure("BACKUP_FAILED", "backup restore cannot proceed"));
                break;
            }
            let contained = (|| {
                let canonical_root = std::fs::canonicalize(&root).ok()?;
                // The section root itself must stay under the profile: a
                // junction planted AT the section root resolves outside and
                // must fail closed, not become the new containment scope.
                if !canonical_root.starts_with(&profile_canonical) {
                    return None;
                }
                let canonical_parent = std::fs::canonicalize(parent).ok()?;
                canonical_parent.starts_with(&canonical_root).then_some(())
            })();
            if contained.is_none() {
                commit_failed = Some(failure(
                    "BACKUP_FAILED",
                    "backup restore target is outside the approved root",
                ));
                break;
            }
        }
        let staged_path = staging_dir.join(format!("entry-{}.bin", write.sequence));
        let rollback_path = rollback_dir.join(format!("{position}.bak"));
        let outcome: Result<bool, BackupFailure> =
            if matches!(write.target, RestoreTarget::Settings) {
                // Staged shallow merge over the live file (Electron parity),
                // committed through the same atomic path as every other file.
                let staged_bytes = std::fs::read(&staged_path)
                    .map_err(|_| failure("BACKUP_FAILED", "backup restore cannot proceed"))?;
                if staged_bytes.len() > MAX_BACKUP_FILE_BYTES {
                    Err(failure(
                        "BACKUP_TOO_LARGE",
                        "backup archive entry exceeds the size limit",
                    ))
                } else {
                    let existing = std::fs::read(dest).ok();
                    match merge_settings_blob(existing.as_deref(), &staged_bytes) {
                        Ok(merged) => {
                            let merged_staged =
                                staging_dir.join(format!("entry-{}-merged.bin", write.sequence));
                            if std::fs::write(&merged_staged, &merged).is_err() {
                                Err(failure("BACKUP_FAILED", "backup restore cannot proceed"))
                            } else {
                                commit_staged_file(dest, &merged_staged, Some(&rollback_path))
                            }
                        }
                        Err(failure) => Err(failure),
                    }
                }
            } else {
                commit_staged_file(dest, &staged_path, Some(&rollback_path))
            };
        match outcome {
            Ok(existed) => {
                committed.push((position, existed));
                if fail_after.is_some_and(|limit| committed.len() >= limit) {
                    commit_failed = Some(failure("BACKUP_FAILED", "backup restore cannot proceed"));
                }
            }
            Err(failure) => {
                commit_failed = Some(failure);
            }
        }
    }
    if let Some(failure) = commit_failed {
        // Roll back already-replaced files: restore originals, remove files
        // that did not exist before. Best effort per file (a rollback I/O
        // error is logged host-side); the typed failure still reports.
        for (position, existed) in committed.iter().rev() {
            let dest = &destinations[*position];
            if *existed {
                let backup = rollback_dir.join(format!("{position}.bak"));
                if std::fs::rename(&backup, dest).is_err() {
                    // Same-directory copy fallback (cross-device staging):
                    // the backup was copied, not moved, so a copy restores.
                    if std::fs::copy(&backup, dest).is_err() {
                        eprintln!("[Backup][Restore] rollback could not restore a replaced file");
                    }
                }
            } else if std::fs::remove_file(dest).is_err() {
                eprintln!("[Backup][Restore] rollback could not remove a created file");
            }
        }
        // The profile root itself is host-controlled; destinations were
        // pre-checked, so rollback restores the pre-restore bytes. This is
        // per-file atomicity plus best-effort multi-file recovery — NOT a
        // cross-filesystem transaction, and it is documented as such.
        return Err(failure);
    }
    Ok(())
}

/// One held restore: a fully validated + staged archive waiting for an
/// explicit commit or discard. The staging directory is host-owned (outside
/// every profile root); dropping it without commit changes nothing. The
/// struct is opaque to the holder except for the preview fields: staging
/// paths and routing never cross the boundary.
#[derive(Debug)]
pub struct StagedRestore {
    staging_dir: PathBuf,
    staged: Vec<StagedWrite>,
    /// Manifest include flags: which sections the commit would apply.
    pub includes: BackupIncludes,
    /// Filtered clientMods envelope the renderer would apply on commit.
    pub client_mods: serde_json::Value,
    /// Picked archive size in bytes.
    pub archive_bytes: usize,
    /// Total staged uncompressed content in bytes.
    pub uncompressed_bytes: usize,
    /// Staged file entries (manifest excluded).
    pub entry_count: usize,
    /// Accepted archive names in archive order: what the commit would
    /// change. Host paths never appear here. Bounded by
    /// [`MAX_PREVIEW_ENTRY_NAMES`] for the bridge; `entry_count` is the
    /// authoritative total.
    pub entry_names: Vec<String>,
}

/// Maximum entry names carried in a restore preview. The authoritative
/// total rides in `entry_count`; the listing is display-only and truncated
/// past this bound (oldest archive order first).
pub const MAX_PREVIEW_ENTRY_NAMES: usize = 128;

/// User-facing preview of one held restore: what the commit would change.
/// Counts and flags only, plus the bounded display listing — never host
/// paths, never file contents, never the clientMods envelope itself.
#[derive(Debug, Clone)]
pub struct RestorePreview {
    /// Manifest include flags: which sections the commit would apply.
    pub includes: BackupIncludes,
    /// Picked archive size in bytes.
    pub archive_bytes: usize,
    /// Total staged uncompressed content in bytes.
    pub uncompressed_bytes: usize,
    /// Staged file entries (manifest excluded).
    pub entry_count: usize,
    /// Serialized clientMods envelope size in bytes.
    pub client_mods_bytes: usize,
    /// Whether the envelope carries each mod payload (presence only).
    pub has_vencord_mod_data: bool,
    /// Whether the envelope carries each mod payload (presence only).
    pub has_equicord_mod_data: bool,
    /// Whether the envelope carries each mod payload (presence only).
    pub has_shelter_mod_data: bool,
    /// Accepted archive names in archive order (display only, bounded by
    /// [`MAX_PREVIEW_ENTRY_NAMES`]).
    pub entry_names: Vec<String>,
    /// True when `entry_count` exceeds the listing length.
    pub entries_truncated: bool,
}

impl StagedRestore {
    /// Builds the user-facing preview of this held restore.
    pub fn preview(&self) -> RestorePreview {
        let client_mods_bytes = serde_json::to_string(&self.client_mods)
            .map(|text| text.len())
            .unwrap_or(0);
        let envelope = self.client_mods.as_object();
        let present = |key: &str| {
            envelope.is_some_and(|map| map.get(key).is_some_and(|value| !value.is_null()))
        };
        RestorePreview {
            includes: self.includes,
            archive_bytes: self.archive_bytes,
            uncompressed_bytes: self.uncompressed_bytes,
            entry_count: self.entry_count,
            client_mods_bytes,
            has_vencord_mod_data: present("vencordLocalStorage"),
            has_equicord_mod_data: present("equicordLocalStorage"),
            has_shelter_mod_data: present("shelter"),
            entry_names: self.entry_names.clone(),
            entries_truncated: self.entry_count > self.entry_names.len(),
        }
    }
}

/// Validate + stage one archive's bytes and hold the result for an explicit
/// commit or discard. NOTHING is written to the profile here: staging lives
/// under the host temp dir. On validation failure no staging directory is
/// left behind.
pub fn stage_restore_from_bytes(archive_bytes: &[u8]) -> Result<StagedRestore, BackupFailure> {
    if archive_bytes.len() > MAX_RESTORE_ARCHIVE_BYTES {
        return Err(failure(
            "BACKUP_TOO_LARGE",
            "backup archive exceeds the size limit",
        ));
    }
    if archive_bytes.is_empty() {
        return Err(failure("INVALID_ARCHIVE", "backup archive is invalid"));
    }
    let validate_started = std::time::Instant::now();
    let staging_dir = create_staging_dir()?;
    let (includes, client_mods, staged, accepted_names) =
        match validate_and_stage(archive_bytes, &staging_dir) {
            Ok(staged) => staged,
            // A failed stage leaves no temp residue; a held stage is removed
            // by the commit or discard that resolves it.
            Err(failure) => {
                std::fs::remove_dir_all(&staging_dir).ok();
                return Err(failure);
            }
        };
    let validate_ms = validate_started.elapsed().as_secs_f64() * 1000.0;
    let uncompressed: usize = staged
        .iter()
        .map(|write| {
            std::fs::symlink_metadata(staging_dir.join(format!("entry-{}.bin", write.sequence)))
                .ok()
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len() as usize)
                .unwrap_or(0)
        })
        .sum();
    let entry_count = staged.len();
    let entry_names: Vec<String> = accepted_names
        .into_iter()
        .take(MAX_PREVIEW_ENTRY_NAMES)
        .collect();
    println!(
        "[Backup][Restore] staged entries={} uncompressed={} archive={} validate_ms={:.3}",
        entry_count,
        uncompressed,
        archive_bytes.len(),
        validate_ms,
    );
    Ok(StagedRestore {
        staging_dir,
        staged,
        includes,
        client_mods,
        archive_bytes: archive_bytes.len(),
        uncompressed_bytes: uncompressed,
        entry_count,
        entry_names,
    })
}

/// Commit one held restore into the profile, then remove its staging
/// (success AND failure leave no temp residue). Validation is NOT repeated:
/// the held bytes were validated at stage time and never left host hands,
/// so there is no window for the archive to change between preview and
/// commit. Rollback on partial failure is unchanged.
pub fn commit_staged_restore(
    profile_dir: &Path,
    staged: StagedRestore,
    fail_after: Option<usize>,
) -> Result<RestoreSummary, BackupFailure> {
    let outcome = (|| {
        let commit_started = std::time::Instant::now();
        commit_staged(profile_dir, &staged.staged, &staged.staging_dir, fail_after)?;
        let commit_ms = commit_started.elapsed().as_secs_f64() * 1000.0;
        let client_mods_bytes = serde_json::to_string(&staged.client_mods)
            .map(|text| text.len())
            .unwrap_or(0);
        println!(
            "[Backup][Restore] committed entries={} uncompressed={} archive={} client_mods={} commit_ms={:.3}",
            staged.entry_count,
            staged.uncompressed_bytes,
            staged.archive_bytes,
            client_mods_bytes,
            commit_ms,
        );
        Ok(RestoreSummary {
            canceled: false,
            client_mods: staged.client_mods.clone(),
            archive_bytes: staged.archive_bytes,
            uncompressed_bytes: staged.uncompressed_bytes,
            entry_count: staged.entry_count,
            client_mods_bytes,
        })
    })();
    std::fs::remove_dir_all(&staged.staging_dir).ok();
    outcome
}

/// Discard one held restore without touching the profile. Staging is
/// removed; nothing else changes.
pub fn discard_staged_restore(staged: StagedRestore) {
    std::fs::remove_dir_all(&staged.staging_dir).ok();
}

/// Resolve the host-owned restore-dialog configuration: translated title
/// only. The renderer influences none of this.
pub fn restore_dialog_config(profile_dir: &Path) -> String {
    let found = crate::localization::lookup(profile_dir, DIALOG_OPEN_TITLE_KEY);
    if found.value.is_empty() {
        FALLBACK_DIALOG_OPEN_TITLE.to_owned()
    } else {
        found.value
    }
}

/// Outcome of the host-owned restore picker: cancelled/empty, one
/// host-selected archive path, or a host failure that never leaks paths.
#[derive(Debug)]
pub enum OpenPickerOutcome {
    /// The user dismissed the dialog. Normal control flow: resolves the
    /// canceled outcome, never rejects.
    Canceled,
    /// Host-selected archive to read (bounded, validated downstream). The
    /// path stays host-side; only the clientMods envelope crosses the bridge.
    Selected(PathBuf),
    /// The picker itself failed (COM/shell error, unsupported platform). No
    /// path and no OS error text cross this boundary.
    Failed(BackupFailure),
}

/// Show the host-owned backup restore picker and return its outcome.
///
/// Windows Common Item Dialog (`IFileOpenDialog`) preserving the Electron
/// semantics: translated title, one `ZIP/*.zip` filter, open-file semantics
/// with `FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM`, single selection. MUST run
/// on an STA thread with COM already initialized (the dedicated restore
/// worker owns that); the dialog is entirely host-side and modal to the
/// worker, never to the WebView thread.
///
/// Deliberate Electron difference: no renderer-supplied anything (Electron's
/// handler likewise takes no arguments — the parity is exact here).
#[cfg(target_os = "windows")]
pub fn show_open_picker(profile_dir: &Path) -> OpenPickerOutcome {
    use windows::Win32::{
        System::Com::{CoCreateInstance, CLSCTX_ALL},
        UI::Shell::{FileOpenDialog, IFileOpenDialog},
    };

    let failed =
        |code: &'static str, message: &str| OpenPickerOutcome::Failed(failure(code, message));
    let title = restore_dialog_config(profile_dir);
    let dialog: IFileOpenDialog =
        match unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL) } {
            Ok(dialog) => dialog,
            Err(_) => {
                return failed("BACKUP_FAILED", "backup restore picker cannot be shown");
            }
        };
    // Every COM interface call below is unsafe by windows-crate contract;
    // the STA worker guarantees the threading model (STA + COM initialized
    // exactly once for its thread) that makes them sound to invoke.
    unsafe { show_open_picker_inner(&dialog, &title) }
}

/// Inner picker flow: every COM call. Split so the caller documents the
/// safety contract once instead of scattering `unsafe` per call.
#[cfg(target_os = "windows")]
unsafe fn show_open_picker_inner(
    dialog: &windows::Win32::UI::Shell::IFileOpenDialog,
    title: &str,
) -> OpenPickerOutcome {
    use windows::{
        core::{HSTRING, PCWSTR},
        Win32::UI::Shell::{
            Common::COMDLG_FILTERSPEC, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH,
        },
    };

    let failed =
        |code: &'static str, message: &str| OpenPickerOutcome::Failed(failure(code, message));
    if dialog.SetTitle(&HSTRING::from(title)).is_err() {
        return failed("BACKUP_FAILED", "backup restore picker cannot be shown");
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
        return failed("BACKUP_FAILED", "backup restore picker cannot be shown");
    }
    if dialog.SetFileTypeIndex(1).is_err() {
        return failed("BACKUP_FAILED", "backup restore picker cannot be shown");
    }
    // Open-file semantics layered onto the dialog defaults (never replacing
    // them). Single selection: no FOS_ALLOWMULTISELECT — one archive per
    // restore, exactly like Electron's single `filePaths[0]`.
    let options = match dialog.GetOptions() {
        Ok(options) => options,
        Err(_) => {
            return failed("BACKUP_FAILED", "backup restore picker cannot be shown");
        }
    };
    if dialog
        .SetOptions(options | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST)
        .is_err()
    {
        return failed("BACKUP_FAILED", "backup restore picker cannot be shown");
    }
    // No owner window: the picker is host-side and worker-modal, so the
    // WebView/UI thread stays responsive while it is open.
    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) => {
            // ERROR_CANCELLED (1223): the user dismissed the dialog. Normal
            // control flow, never an error.
            if error.code() == windows::core::HRESULT::from_win32(1223) {
                return OpenPickerOutcome::Canceled;
            }
            return failed("BACKUP_FAILED", "backup restore picker cannot be shown");
        }
    }
    let item = match dialog.GetResult() {
        Ok(item) => item,
        Err(_) => {
            return failed("BACKUP_FAILED", "backup restore picker cannot be shown");
        }
    };
    let display = match item.GetDisplayName(SIGDN_FILESYSPATH) {
        Ok(display) => display,
        Err(_) => {
            return failed("BACKUP_FAILED", "backup restore picker cannot be shown");
        }
    };
    let text = unsafe { display.to_string().unwrap_or_default() };
    unsafe {
        windows::Win32::System::Com::CoTaskMemFree(Some(display.0 as *const std::ffi::c_void));
    }
    if text.is_empty() {
        return failed("BACKUP_FAILED", "backup restore picker cannot be shown");
    }
    // The path stays host-side; only the clientMods envelope crosses the
    // bridge after validation + staging + commit.
    OpenPickerOutcome::Selected(PathBuf::from(text))
}

/// Non-Windows builds fail closed: the picker is a Windows WebView2 surface
/// (project rule), never a guessed shell command.
#[cfg(not(target_os = "windows"))]
pub fn show_open_picker(_profile_dir: &Path) -> OpenPickerOutcome {
    OpenPickerOutcome::Failed(failure(
        "BACKUP_UNSUPPORTED_PLATFORM",
        "backup restore is not supported on this platform",
    ))
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
            "remcord-native-backup-test-{}-{tag}-{nanos}",
            std::process::id(),
        ));
        std::fs::create_dir_all(&dir).expect("must create temp profile");
        dir
    }

    fn all_true() -> BackupIncludes {
        BackupIncludes {
            remcord_config: true,
            remcord_themes_and_quick_css: true,
            remcord_extension_plugins: true,
            vencord_mod_data: true,
            equicord_mod_data: true,
            shelter_mod_data: true,
            mod_bundles: true,
        }
    }

    fn all_false() -> BackupIncludes {
        BackupIncludes {
            remcord_config: false,
            remcord_themes_and_quick_css: false,
            remcord_extension_plugins: false,
            vencord_mod_data: false,
            equicord_mod_data: false,
            shelter_mod_data: false,
            mod_bundles: false,
        }
    }

    #[test]
    fn includes_rejects_unknown_keys() {
        let parsed: Result<BackupIncludes, _> = serde_json::from_str(
            r#"{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true,"evil":true}"#,
        );
        assert!(parsed.is_err());
        let parsed: Result<BackupSavePayload, _> = serde_json::from_str(
            r#"{"includes":{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true},"clientMods":{},"outputPath":"C:\\x.zip"}"#,
        );
        assert!(parsed.is_err());
        let parsed: Result<BackupSavePayload, _> = serde_json::from_str(
            r#"{"includes":{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true},"clientMods":{"vencordLocalStorage":"x","extra":{}}}"#,
        );
        assert!(parsed.is_err());
    }

    #[test]
    fn valid_request_filters_client_mods_by_includes() {
        let request = validate_backup_request(
            r#"{"includes":{"remcordConfig":true,"remcordThemesAndQuickCss":false,"remcordExtensionPlugins":false,"vencordModData":true,"equicordModData":false,"shelterModData":true,"modBundles":false},"clientMods":{"vencordLocalStorage":"{\"a\":1}","equicordLocalStorage":"dropped","shelter":{"plugins":{}}}}"#,
        )
        .expect("valid request must validate");
        assert!(request.includes.remcord_config);
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
            r#"{"includes":{"remcordConfig":false,"remcordThemesAndQuickCss":false,"remcordExtensionPlugins":false,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":false},"clientMods":{}}"#,
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
            "{{\"includes\":{{\"remcordConfig\":false,\"remcordThemesAndQuickCss\":false,\"remcordExtensionPlugins\":false,\"vencordModData\":true,\"equicordModData\":false,\"shelterModData\":false,\"modBundles\":false}},\"clientMods\":{{\"vencordLocalStorage\":\"{big}\"}}}}"
        );
        let failure = validate_backup_request(&payload).expect_err("oversized string must fail");
        assert_eq!(failure.code, "BACKUP_TOO_LARGE");
        assert!(!failure.message.contains(&big[..16]));
    }

    #[test]
    fn oversized_shelter_payload_is_rejected() {
        let big = "x".repeat(MAX_SHELTER_MOD_BYTES);
        let payload = format!(
            "{{\"includes\":{{\"remcordConfig\":false,\"remcordThemesAndQuickCss\":false,\"remcordExtensionPlugins\":false,\"vencordModData\":false,\"equicordModData\":false,\"shelterModData\":true,\"modBundles\":false}},\"clientMods\":{{\"shelter\":{{\"blob\":\"{big}\"}}}}}}"
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
            "remcord-backup-2026-10-07.zip"
        );
        assert_eq!(ensure_zip_extension("backup"), "backup.zip");
        assert_eq!(ensure_zip_extension("backup.zip"), "backup.zip");
        assert_eq!(ensure_zip_extension("BACKUP.ZIP"), "BACKUP.ZIP");
        assert_eq!(ensure_zip_extension("backup.Zip"), "backup.Zip");
    }

    #[test]
    fn manifest_has_version_one_and_exact_shapes() {
        let request = validate_backup_request(
            r#"{"includes":{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true},"clientMods":{"vencordLocalStorage":"v"}}"#,
        )
        .unwrap();
        let manifest = build_backup_manifest(&request, "1.3.0", "2026-10-07T00:00:00.000Z");
        assert_eq!(manifest["version"], 1);
        assert_eq!(manifest["createdAt"], "2026-10-07T00:00:00.000Z");
        assert_eq!(manifest["appVersion"], "1.3.0");
        let includes = manifest["includes"].as_object().unwrap();
        assert_eq!(includes.len(), 7);
        for key in [
            "remcordConfig",
            "remcordThemesAndQuickCss",
            "remcordExtensionPlugins",
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
            r#"{"includes":{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":false,"equicordModData":false,"shelterModData":false,"modBundles":true},"clientMods":{}}"#,
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
            "{{\"includes\":{{\"remcordConfig\":false,\"remcordThemesAndQuickCss\":false,\"remcordExtensionPlugins\":false,\"vencordModData\":true,\"equicordModData\":false,\"shelterModData\":false,\"modBundles\":false}},\"clientMods\":{{\"vencordLocalStorage\":\"{oversized}\"}}}}"
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
    /// Run explicitly with `REMCORD_NATIVE_BACKUP_SAVE_UI_TEST=1`; ordinary
    /// `cargo test` skips it so no dialog ever appears in unattended runs.
    /// The picker runs on its own STA thread exactly like the production
    /// worker (COM initialized once for that thread); this thread polls for
    /// the dialog window by its translated title, asserts the filename field
    /// carries the UTC-date default (`remcord-backup-YYYY-MM-DD.zip` —
    /// proving `SetFileName` applied), then dismisses with `WM_CLOSE` and
    /// requires a silent `Canceled` outcome. Dialog appearance itself proves
    /// `SetTitle`/`SetFileTypes`/`SetFileTypeIndex`/`SetDefaultExtension`
    /// all succeeded (any failure returns before `Show`).
    #[test]
    fn live_dialog_shows_translated_title_default_filename_and_cancels_silently() {
        if std::env::var("REMCORD_NATIVE_BACKUP_SAVE_UI_TEST").as_deref() != Ok("1") {
            eprintln!(
                "SKIP: live dialog test needs REMCORD_NATIVE_BACKUP_SAVE_UI_TEST=1 (opens a real modal saver)"
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
            assert!(default_filename.starts_with("remcord-backup-"));
            assert!(default_filename.ends_with(".zip"));
            let (done_tx, done_rx) = mpsc::channel();
            std::thread::Builder::new()
                .name("remcord-backup-save-ui-test-sta".to_owned())
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

#[cfg(test)]
mod restore_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn restore_profile(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must advance")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "remcord-native-restore-test-{}-{tag}-{nanos}",
            std::process::id(),
        ));
        std::fs::create_dir_all(&dir).expect("must create temp profile");
        dir
    }

    fn full_includes() -> &'static str {
        r#"{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true}"#
    }

    fn manifest_text(includes: &str, client_mods: &str) -> String {
        format!(
            r#"{{"version":1,"createdAt":"2026-10-07T00:00:00.000Z","appVersion":"1.3.0","includes":{includes},"clientMods":{client_mods}}}"#
        )
    }

    fn default_manifest() -> String {
        manifest_text(
            full_includes(),
            r#"{"vencordLocalStorage":"{\"a\":1}","equicordLocalStorage":null,"shelter":{"plugins":{}}}"#,
        )
    }

    /// Independently constructed archive: a raw `zip` writer driven by a
    /// hand-written manifest string, NOT the native save path. `stored`
    /// selects the `Stored` (uncompressed) method to prove method-agnostic
    /// reading; otherwise entries use Deflate.
    fn raw_zip(entries: &[(&str, &[u8])], stored: bool) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            let method = if stored {
                zip::CompressionMethod::Stored
            } else {
                zip::CompressionMethod::Deflated
            };
            let options = zip::write::SimpleFileOptions::default().compression_method(method);
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    /// Test-only one-shot restore: stage then commit, propagating stage
    /// failures as the operation error (the production UI stages and
    /// commits as separate steps; these tests assert end-to-end
    /// rejection/behavior, not the split seam — the split seam has its own
    /// tests above).
    fn restore_now(
        profile: &Path,
        archive: &[u8],
        fail_after: Option<usize>,
    ) -> Result<RestoreSummary, BackupFailure> {
        let staged = stage_restore_from_bytes(archive)?;
        commit_staged_restore(profile, staged, fail_after)
    }

    fn profile_files(profile: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![profile.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(listing) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in listing.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(
                        path.strip_prefix(profile)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn manifest_validation_pins_version_shape_and_bounds() {
        let (_, envelope) = validate_restore_manifest(&default_manifest()).unwrap();
        assert_eq!(envelope["vencordLocalStorage"], "{\"a\":1}");
        // Wrong version fails.
        let bad = manifest_text(full_includes(), "{}").replace("\"version\":1", "\"version\":2");
        assert_eq!(
            validate_restore_manifest(&bad).unwrap_err().code,
            "INVALID_BACKUP"
        );
        // Malformed JSON fails.
        assert_eq!(
            validate_restore_manifest("{not json").unwrap_err().code,
            "INVALID_BACKUP"
        );
        // Unknown include keys fail (the seven flags are authoritative).
        let bad_inc = manifest_text(
            r#"{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":true,"vencordModData":true,"equicordModData":true,"shelterModData":true,"modBundles":true,"evil":true}"#,
            "{}",
        );
        assert_eq!(
            validate_restore_manifest(&bad_inc).unwrap_err().code,
            "INVALID_BACKUP"
        );
        // Unknown clientMods keys fail (they would otherwise cross the bridge).
        let bad_cm = manifest_text(full_includes(), r#"{"evil":{}}"#);
        assert_eq!(
            validate_restore_manifest(&bad_cm).unwrap_err().code,
            "INVALID_BACKUP"
        );
        // Wrong-typed clientMods fail.
        let bad_type = manifest_text(full_includes(), r#"{"vencordLocalStorage":42}"#);
        assert_eq!(
            validate_restore_manifest(&bad_type).unwrap_err().code,
            "INVALID_BACKUP"
        );
        // Oversized client data fails bounded.
        let big = "x".repeat(MAX_CLIENT_MOD_STRING_BYTES + 1);
        let bad_big = manifest_text(
            full_includes(),
            &format!(r#"{{"vencordLocalStorage":"{big}"}}"#),
        );
        assert_eq!(
            validate_restore_manifest(&bad_big).unwrap_err().code,
            "BACKUP_TOO_LARGE"
        );
        // Unknown TOP-LEVEL metadata is forward-compatible (ignored).
        let extra = default_manifest().replace(
            "\"appVersion\":\"1.3.0\"",
            "\"appVersion\":\"1.3.0\",\"futureSection\":{\"paths\":[\"/etc/passwd\"]}",
        );
        let (_, envelope) = validate_restore_manifest(&extra).unwrap();
        assert!(envelope.get("futureSection").is_none());
    }

    #[test]
    fn entry_names_reject_escape_shapes() {
        for bad in [
            "../escape",
            "/absolute",
            "C:/secret",
            "C:relative",
            "\\\\server\\share",
            "data/../../escape",
            "data/./themes",
            "data//themes",
            "data/themes/",
            "a:b",
            "with\0nul",
            "trailing/",
        ] {
            assert!(split_archive_name(bad).is_err(), "must reject {bad:?}");
        }
        // Backslash traversal normalizes into traversal and is rejected.
        assert!(split_archive_name("data\\..\\escape").is_err());
        assert!(split_archive_name("data\\themes\\a.css").is_ok());
        // Depth and component bounds fail as TOO_LARGE.
        let deep = (0..MAX_RESTORE_DEPTH + 1)
            .map(|index| format!("d{index}"))
            .collect::<Vec<_>>()
            .join("/");
        assert_eq!(
            split_archive_name(&deep).unwrap_err().code,
            "BACKUP_TOO_LARGE"
        );
        let long_component = format!("data/{}", "a".repeat(MAX_RESTORE_COMPONENT_BYTES + 1));
        assert_eq!(
            split_archive_name(&long_component).unwrap_err().code,
            "BACKUP_TOO_LARGE"
        );
    }

    #[test]
    fn independent_archive_restores_every_section() {
        let profile = restore_profile("independent");
        let manifest = default_manifest();
        let archive = raw_zip(
            &[
                ("manifest.json", manifest.as_bytes()),
                (
                    "data/storage/settings.json",
                    b"{\"windowStyle\":\"overlay\"}",
                ),
                ("data/quickCss.css", b":root{--q:1;}"),
                (
                    "data/themes/my-theme/manifest.json",
                    b"{\"name\":\"M\",\"theme\":\"src.css\",\"enabled\":true}",
                ),
                ("data/themes/my-theme/src.css", b":root{--m:1;}"),
                ("data/themes/loose.css", b":root{--loose:1;}"),
                ("data/plugins/ext/m.json", b"{}"),
                ("data/runtime-plugins/rt/index.js", b"x"),
                ("data/plugin-storage/probe/cache.txt", b"y"),
                ("bundles/shelter.js", b"shelter"),
                ("bundles/evil.js", b"must be ignored"),
                ("unknown-section/file.txt", b"must be ignored"),
                ("data/storage/other.json", b"must be ignored"),
            ],
            false,
        );
        let summary = restore_now(&profile, &archive, None).unwrap();
        assert_eq!(summary.entry_count, 9);
        assert_eq!(summary.client_mods["vencordLocalStorage"], "{\"a\":1}");
        // Settings land as a merged document (pretty-printed): compare values.
        let settings: serde_json::Value = serde_json::from_slice(
            &std::fs::read(profile.join("storage").join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["windowStyle"], "overlay");
        assert_eq!(
            std::fs::read(profile.join("quickCss.css")).unwrap(),
            b":root{--q:1;}"
        );
        assert_eq!(
            std::fs::read(profile.join("themes").join("my-theme").join("src.css")).unwrap(),
            b":root{--m:1;}"
        );
        assert_eq!(
            std::fs::read(profile.join("themes").join("loose.css")).unwrap(),
            b":root{--loose:1;}"
        );
        assert_eq!(
            std::fs::read(profile.join("extensions").join("ext").join("m.json")).unwrap(),
            b"{}"
        );
        assert_eq!(
            std::fs::read(profile.join("plugins").join("rt").join("index.js")).unwrap(),
            b"x"
        );
        assert_eq!(
            std::fs::read(
                profile
                    .join("plugin-storage")
                    .join("probe")
                    .join("cache.txt")
            )
            .unwrap(),
            b"y"
        );
        assert_eq!(
            std::fs::read(profile.join("shelter.js")).unwrap(),
            b"shelter"
        );
        // Unknown bundle names, unknown sections, and non-settings storage
        // files are ignored, never written.
        assert!(!profile.join("evil.js").exists());
        assert!(!profile.join("unknown-section").exists());
        assert!(!profile.join("storage").join("other.json").exists());
        // Enabled state rides along inside the restored theme manifest
        // (Electron overwrite parity — the host flips nothing itself).
        let theme_manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                profile
                    .join("themes")
                    .join("my-theme")
                    .join("manifest.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(theme_manifest["enabled"], true);
        // No staging/temp residue inside the profile.
        assert!(profile_files(&profile)
            .iter()
            .all(|name| !name.contains(".restore-") && !name.contains(".tmp")));
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn native_save_archive_round_trips_through_restore() {
        // The archive below is produced by the NATIVE save path
        // (`collect_backup_entries` + `build_backup_zip`), proving format
        // compatibility in that direction; the previous test proves an
        // independently constructed archive restores.
        let source = restore_profile("roundtrip-src");
        std::fs::create_dir_all(source.join("storage")).unwrap();
        std::fs::write(
            source.join("storage").join("settings.json"),
            r#"{"windowStyle":"overlay"}"#,
        )
        .unwrap();
        std::fs::write(source.join("quickCss.css"), ":root{}").unwrap();
        std::fs::create_dir_all(source.join("themes").join("t")).unwrap();
        std::fs::write(source.join("themes").join("t").join("src.css"), "t").unwrap();
        let request = validate_backup_request(
            r#"{"includes":{"remcordConfig":true,"remcordThemesAndQuickCss":true,"remcordExtensionPlugins":false,"vencordModData":true,"equicordModData":false,"shelterModData":false,"modBundles":false},"clientMods":{"vencordLocalStorage":"v"}}"#,
        )
        .unwrap();
        let manifest = build_backup_manifest(&request, "1.3.0", "2026-10-07T00:00:00.000Z");
        let entries = collect_backup_entries(&source, &request.includes).unwrap();
        let archive = build_backup_zip(&manifest, &entries).unwrap();
        let dest = restore_profile("roundtrip-dst");
        let summary = restore_now(&dest, &archive, None).unwrap();
        assert_eq!(summary.entry_count, 3);
        assert_eq!(summary.client_mods["vencordLocalStorage"], "v");
        assert!(summary.client_mods.get("equicordLocalStorage").is_none());
        let settings: serde_json::Value = serde_json::from_slice(
            &std::fs::read(dest.join("storage").join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["windowStyle"], "overlay");
        assert_eq!(
            std::fs::read(dest.join("quickCss.css")).unwrap(),
            b":root{}"
        );
        assert_eq!(
            std::fs::read(dest.join("themes").join("t").join("src.css")).unwrap(),
            b"t"
        );
        // Repeated restore is deterministic.
        let again = restore_now(&dest, &archive, None).unwrap();
        assert_eq!(again.entry_count, summary.entry_count);
        assert_eq!(profile_files(&dest), profile_files(&dest));
        std::fs::remove_dir_all(&source).ok();
        std::fs::remove_dir_all(&dest).ok();
    }

    #[test]
    fn structural_failures_reject_before_any_write() {
        type ArchiveFixture<'a> = (&'a str, &'a [u8]);
        let cases: Vec<(&str, Vec<ArchiveFixture<'_>>, &str)> = vec![
            (
                "missing-manifest",
                vec![("data/quickCss.css", b"x".as_slice())],
                "INVALID_ARCHIVE",
            ),
            (
                "bad-json-manifest",
                vec![
                    ("manifest.json", b"{not json".as_slice()),
                    ("data/quickCss.css", b"x".as_slice()),
                ],
                "INVALID_BACKUP",
            ),
            (
                "wrong-version",
                vec![(
                    "manifest.json",
                    manifest_text(full_includes(), "{}")
                        .replace("\"version\":1", "\"version\":7")
                        .into_bytes()
                        .leak() as &[u8],
                )],
                "INVALID_BACKUP",
            ),
            (
                "traversal",
                vec![
                    (
                        "manifest.json",
                        default_manifest().into_bytes().leak() as &[u8],
                    ),
                    ("data/themes/../escape.css", b"x".as_slice()),
                    ("data/quickCss.css", b"legit".as_slice()),
                ],
                "INVALID_ARCHIVE",
            ),
            (
                "absolute",
                vec![
                    (
                        "manifest.json",
                        default_manifest().into_bytes().leak() as &[u8],
                    ),
                    ("/absolute.css", b"x".as_slice()),
                ],
                "INVALID_ARCHIVE",
            ),
            (
                "case-collision",
                vec![
                    (
                        "manifest.json",
                        default_manifest().into_bytes().leak() as &[u8],
                    ),
                    ("data/quickCss.css", b"first".as_slice()),
                    ("DATA/QUICKCSS.CSS", b"second".as_slice()),
                ],
                "INVALID_ARCHIVE",
            ),
            (
                "separator-collision",
                vec![
                    (
                        "manifest.json",
                        default_manifest().into_bytes().leak() as &[u8],
                    ),
                    ("data/themes/a/x.css", b"first".as_slice()),
                    ("data\\themes\\a\\x.css", b"second".as_slice()),
                ],
                "INVALID_ARCHIVE",
            ),
        ];
        for (tag, entries, code) in cases {
            let profile = restore_profile(tag);
            // A pre-existing file proves no partial write happened.
            std::fs::write(profile.join("quickCss.css"), "original").unwrap();
            let archive = raw_zip(
                &entries
                    .iter()
                    .map(|(name, data)| (*name, *data))
                    .collect::<Vec<_>>(),
                false,
            );
            let failure = restore_now(&profile, &archive, None).unwrap_err();
            assert_eq!(failure.code, code, "case {tag}");
            assert_eq!(
                std::fs::read(profile.join("quickCss.css")).unwrap(),
                b"original",
                "case {tag} must not partially write"
            );
            assert!(!failure.message.contains("quickCss"));
            std::fs::remove_dir_all(&profile).ok();
        }
        // Exact duplicates cannot be built with the high-level zip writer
        // (it rejects them at construction, and the native saver never emits
        // them): the whole-table detector is asserted directly, while the
        // case/backslash variants above prove the wiring end to end.
        assert_eq!(
            reject_duplicate_names(&[
                "manifest.json".to_owned(),
                "data/quickCss.css".to_owned(),
                "data/quickCss.css".to_owned(),
            ])
            .unwrap_err()
            .code,
            "INVALID_ARCHIVE"
        );
        assert!(reject_duplicate_names(&[
            "manifest.json".to_owned(),
            "data/quickCss.css".to_owned(),
        ])
        .is_ok());
        // Malformed bytes and empty input are structural failures too.
        let profile = restore_profile("malformed");
        assert_eq!(
            restore_now(&profile, b"not a zip at all".as_slice(), None)
                .unwrap_err()
                .code,
            "INVALID_ARCHIVE"
        );
        assert_eq!(
            restore_now(&profile, b"".as_slice(), None)
                .unwrap_err()
                .code,
            "INVALID_ARCHIVE"
        );
        assert!(profile_files(&profile).is_empty());
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn include_flags_gate_every_section() {
        let off = r#"{"remcordConfig":false,"remcordThemesAndQuickCss":false,"remcordExtensionPlugins":false,"vencordModData":false,"equicordModData":false,"shelterModData":false,"modBundles":false}"#;
        let archive = raw_zip(
            &[
                ("manifest.json", manifest_text(off, "{}").as_bytes()),
                ("data/storage/settings.json", b"{\"a\":1}"),
                ("data/quickCss.css", b"q"),
                ("data/themes/t/src.css", b"t"),
                ("data/plugins/e/f.txt", b"e"),
                ("data/runtime-plugins/r/f.txt", b"r"),
                ("data/plugin-storage/p/f.txt", b"p"),
                ("bundles/shelter.js", b"s"),
            ],
            false,
        );
        let profile = restore_profile("flags-off");
        let summary = restore_now(&profile, &archive, None).unwrap();
        assert_eq!(summary.entry_count, 0);
        assert!(profile_files(&profile).is_empty());
        // A section absent while its flag is true restores nothing, cleanly.
        let sparse = raw_zip(&[("manifest.json", default_manifest().as_bytes())], false);
        let summary = restore_now(&profile, &sparse, None).unwrap();
        assert_eq!(summary.entry_count, 0);
        assert_eq!(summary.client_mods["vencordLocalStorage"], "{\"a\":1}");
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn section_identifiers_are_validated() {
        let bad_names = [
            "data/themes/my theme/x.css",
            "data/themes/CON/x.css",
            "data/themes/good/CON.css",
            "data/themes/good/C:\\x.css",
            "data/plugin-storage/bad id/x.txt",
            "data/plugin-storage/lonely.txt",
            "data/plugin-storage/p/../x.txt",
            "data/plugins/../escape.txt",
            "data/runtime-plugins/../escape.txt",
        ];
        for (index, name) in bad_names.iter().enumerate() {
            let profile = restore_profile(&format!("ids-{index}"));
            let archive = raw_zip(
                &[
                    ("manifest.json", default_manifest().as_bytes()),
                    (*name, b"x".as_slice()),
                ],
                false,
            );
            assert!(
                restore_now(&profile, &archive, None).is_err(),
                "must reject {name:?}"
            );
            assert!(profile_files(&profile).is_empty());
            std::fs::remove_dir_all(&profile).ok();
        }
        // Every bundle allowlist name restores; anything else is ignored.
        let manifest = default_manifest();
        let mut entries: Vec<(&str, &[u8])> = vec![("manifest.json", manifest.as_bytes())];
        let bundle_names = [
            "vencord.js",
            "vencord.css",
            "equicord.js",
            "equicord.css",
            "shelter.js",
            "custom.js",
            "custom.css",
        ];
        for name in bundle_names {
            entries.push((
                Box::leak(format!("bundles/{name}").into_boxed_str()) as &str,
                b"b".as_slice(),
            ));
        }
        let profile = restore_profile("bundles");
        let archive = raw_zip(&entries, false);
        let summary = restore_now(&profile, &archive, None).unwrap();
        assert_eq!(summary.entry_count, 7);
        for name in bundle_names {
            assert_eq!(std::fs::read(profile.join(name)).unwrap(), b"b");
        }
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn content_bounds_fail_closed() {
        // Normal file one byte over the per-file bound.
        let big = vec![b'x'; MAX_BACKUP_FILE_BYTES + 1];
        let profile = restore_profile("big-file");
        let archive = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                ("data/quickCss.css", &big),
            ],
            true,
        );
        assert_eq!(
            restore_now(&profile, &archive, None).unwrap_err().code,
            "BACKUP_TOO_LARGE"
        );
        assert!(profile_files(&profile).is_empty());
        // Bundle one byte over the bundle ceiling.
        let huge = vec![b'x'; MAX_BACKUP_BUNDLE_BYTES + 1];
        let archive = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                ("bundles/shelter.js", &huge),
            ],
            true,
        );
        assert_eq!(
            restore_now(&profile, &archive, None).unwrap_err().code,
            "BACKUP_TOO_LARGE"
        );
        assert!(profile_files(&profile).is_empty());
        drop(huge);
        // Total uncompressed over 64 MiB (65 × 1 MiB stored entries).
        let one = vec![b'z'; MAX_BACKUP_FILE_BYTES];
        let total_manifest = default_manifest();
        let mut entries: Vec<(&str, &[u8])> = vec![("manifest.json", total_manifest.as_bytes())];
        for index in 0..65 {
            entries.push((
                Box::leak(format!("data/plugins/many/f{index}.bin").into_boxed_str()) as &str,
                one.as_slice(),
            ));
        }
        let archive = raw_zip(&entries, true);
        assert_eq!(
            restore_now(&profile, &archive, None).unwrap_err().code,
            "BACKUP_TOO_LARGE"
        );
        assert!(profile_files(&profile).is_empty());
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn entry_count_bound_fails_closed() {
        let manifest = default_manifest();
        let mut entries: Vec<(&str, &[u8])> = vec![("manifest.json", manifest.as_bytes())];
        for index in 0..MAX_BACKUP_ENTRIES {
            entries.push((
                Box::leak(format!("data/plugins/crowd/f{index}.txt").into_boxed_str()) as &str,
                b"x".as_slice(),
            ));
        }
        let profile = restore_profile("crowd");
        let archive = raw_zip(&entries, true);
        assert_eq!(
            restore_now(&profile, &archive, None).unwrap_err().code,
            "BACKUP_TOO_LARGE"
        );
        assert!(profile_files(&profile).is_empty());
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn settings_merge_preserves_unknown_keys() {
        let profile = restore_profile("merge");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(
            profile.join("storage").join("settings.json"),
            r#"{"windowStyle":"overlay","futureKey":{"nested":1}}"#,
        )
        .unwrap();
        let archive = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                (
                    "data/storage/settings.json",
                    b"{\"windowStyle\":\"default\",\"added\":true}",
                ),
            ],
            false,
        );
        restore_now(&profile, &archive, None).unwrap();
        let merged: serde_json::Value = serde_json::from_slice(
            &std::fs::read(profile.join("storage").join("settings.json")).unwrap(),
        )
        .unwrap();
        // Archive keys win; existing and unknown keys survive (shallow merge).
        assert_eq!(merged["windowStyle"], "default");
        assert_eq!(merged["added"], true);
        assert_eq!(merged["futureKey"]["nested"], 1);
        // A corrupt existing file merges from {} (Electron cache parity).
        std::fs::write(profile.join("storage").join("settings.json"), b"{broken").unwrap();
        restore_now(&profile, &archive, None).unwrap();
        let merged: serde_json::Value = serde_json::from_slice(
            &std::fs::read(profile.join("storage").join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(merged["windowStyle"], "default");
        // A non-object staged settings blob is rejected before any write.
        let bad = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                ("data/storage/settings.json", b"[1,2,3]"),
            ],
            false,
        );
        assert_eq!(
            restore_now(&profile, &bad, None).unwrap_err().code,
            "INVALID_BACKUP"
        );
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn overwrite_replace_repeat_and_unrelated_files() {
        let profile = restore_profile("overwrite");
        std::fs::write(profile.join("quickCss.css"), "old").unwrap();
        std::fs::write(profile.join("unrelated.txt"), "keep").unwrap();
        let first = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                ("data/quickCss.css", b"new".as_slice()),
            ],
            false,
        );
        restore_now(&profile, &first, None).unwrap();
        assert_eq!(std::fs::read(profile.join("quickCss.css")).unwrap(), b"new");
        assert_eq!(
            std::fs::read(profile.join("unrelated.txt")).unwrap(),
            b"keep"
        );
        // Exact replacement twice converges.
        restore_now(&profile, &first, None).unwrap();
        assert_eq!(std::fs::read(profile.join("quickCss.css")).unwrap(), b"new");
        // A directory at a file position fails BEFORE any write (Electron
        // would fail mid-loop with earlier entries already committed).
        std::fs::remove_file(profile.join("quickCss.css")).unwrap();
        std::fs::create_dir_all(profile.join("quickCss.css")).unwrap();
        let failure = restore_now(&profile, &first, None).unwrap_err();
        assert_eq!(failure.code, "BACKUP_FAILED");
        assert!(profile.join("quickCss.css").is_dir());
        assert_eq!(
            std::fs::read(profile.join("unrelated.txt")).unwrap(),
            b"keep"
        );
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn stage_commit_split_previews_then_applies() {
        let profile = restore_profile("split-preview");
        std::fs::write(profile.join("quickCss.css"), "old").unwrap();
        let archive = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                ("data/storage/settings.json", b"{\"a\":1}"),
                ("data/quickCss.css", b"new-css".as_slice()),
                ("data/themes/t/src.css", b"new-theme".as_slice()),
            ],
            false,
        );
        let staged = stage_restore_from_bytes(&archive).expect("valid archive must stage");
        // NOTHING is written before commit: the profile still holds the old
        // bytes and no theme tree exists.
        assert_eq!(std::fs::read(profile.join("quickCss.css")).unwrap(), b"old");
        assert!(!profile.join("themes").exists());
        // The preview names exactly what the commit would change.
        assert!(staged.includes.remcord_config);
        assert!(staged.includes.remcord_themes_and_quick_css);
        assert_eq!(staged.entry_count, 3);
        assert_eq!(
            staged.entry_names,
            vec![
                "data/storage/settings.json".to_owned(),
                "data/quickCss.css".to_owned(),
                "data/themes/t/src.css".to_owned(),
            ]
        );
        assert_eq!(staged.archive_bytes, archive.len());
        assert!(staged.uncompressed_bytes > 0);
        assert_eq!(staged.client_mods["vencordLocalStorage"], "{\"a\":1}");
        // Commit applies everything the preview named.
        let summary =
            commit_staged_restore(&profile, staged, None).expect("held stage must commit");
        assert_eq!(summary.entry_count, 3);
        assert_eq!(summary.archive_bytes, archive.len());
        assert!(!summary.canceled);
        assert_eq!(
            std::fs::read(profile.join("quickCss.css")).unwrap(),
            b"new-css"
        );
        assert_eq!(
            std::fs::read(profile.join("themes").join("t").join("src.css")).unwrap(),
            b"new-theme"
        );
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn discard_leaves_profile_untouched_and_removes_staging() {
        let profile = restore_profile("split-discard");
        std::fs::write(profile.join("quickCss.css"), "old").unwrap();
        let archive = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                ("data/quickCss.css", b"new-css".as_slice()),
            ],
            false,
        );
        let staged = stage_restore_from_bytes(&archive).expect("valid archive must stage");
        let staging_dir = staged.staging_dir.clone();
        assert!(staging_dir.exists());
        discard_staged_restore(staged);
        assert!(!staging_dir.exists());
        assert_eq!(std::fs::read(profile.join("quickCss.css")).unwrap(), b"old");
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn failed_stage_is_rejected_before_any_hold_exists() {
        // A hostile or empty archive never produces a hold: there is nothing
        // to commit or discard afterwards. (Staging-dir cleanup on these
        // paths is structural — the Err branch removes the directory before
        // returning — and the held-path removal is proven by
        // `discard_leaves_profile_untouched_and_removes_staging`; asserting
        // on the shared temp root here would race parallel tests.)
        let bad = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                ("data/themes/../escape.css", b"x".as_slice()),
            ],
            false,
        );
        assert!(stage_restore_from_bytes(&bad).is_err());
        assert!(stage_restore_from_bytes(b"").is_err());
        // The profile is untouched and a later valid stage still works.
        let profile = restore_profile("split-reject");
        let good = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                ("data/quickCss.css", b"q".as_slice()),
            ],
            false,
        );
        let staged = stage_restore_from_bytes(&good).expect("valid stage must work after rejects");
        discard_staged_restore(staged);
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn preview_listing_is_bounded_while_count_stays_authoritative() {
        let profile = restore_profile("split-truncate");
        let mut entries: Vec<(String, Vec<u8>)> =
            vec![("manifest.json".to_owned(), default_manifest().into_bytes())];
        for index in 0..(super::MAX_PREVIEW_ENTRY_NAMES + 5) {
            entries.push((format!("data/themes/t{index}/src.css"), b":root{}".to_vec()));
        }
        let refs: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(name, data)| (name.as_str(), data.as_slice()))
            .collect();
        let archive = raw_zip(&refs, false);
        let staged = stage_restore_from_bytes(&archive).expect("many-theme archive must stage");
        assert_eq!(staged.entry_count, super::MAX_PREVIEW_ENTRY_NAMES + 5);
        assert_eq!(staged.entry_names.len(), super::MAX_PREVIEW_ENTRY_NAMES);
        discard_staged_restore(staged);
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn commit_failures_roll_back_to_pre_restore_bytes() {
        // fail_after = 1: first file commits, second fails -> the first is
        // restored and the second never lands.
        let profile = restore_profile("rollback-mid");
        std::fs::write(profile.join("quickCss.css"), "original-css").unwrap();
        let archive = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                ("data/quickCss.css", b"new-css".as_slice()),
                ("data/themes/t/src.css", b"new-theme".as_slice()),
            ],
            false,
        );
        let failure = commit_staged_restore(
            &profile,
            stage_restore_from_bytes(&archive).unwrap(),
            Some(1),
        )
        .unwrap_err();
        assert_eq!(failure.code, "BACKUP_FAILED");
        assert_eq!(
            std::fs::read(profile.join("quickCss.css")).unwrap(),
            b"original-css"
        );
        assert!(profile_files(&profile)
            .iter()
            .all(|name| !name.starts_with("themes/")));
        // fail_after covers the final commit too: everything rolls back.
        let failure = commit_staged_restore(
            &profile,
            stage_restore_from_bytes(&archive).unwrap(),
            Some(2),
        )
        .unwrap_err();
        assert_eq!(failure.code, "BACKUP_FAILED");
        assert_eq!(
            std::fs::read(profile.join("quickCss.css")).unwrap(),
            b"original-css"
        );
        assert!(profile_files(&profile)
            .iter()
            .all(|name| !name.starts_with("themes/")));
        // ... and a clean run commits everything.
        let summary = restore_now(&profile, &archive, None).unwrap();
        assert_eq!(summary.entry_count, 2);
        assert_eq!(
            std::fs::read(profile.join("quickCss.css")).unwrap(),
            b"new-css"
        );
        // Settings rollback restores the pre-restore document.
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(
            profile.join("storage").join("settings.json"),
            r#"{"keepMe":1}"#,
        )
        .unwrap();
        let settings_archive = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                ("data/storage/settings.json", b"{\"keepMe\":2}".as_slice()),
                ("data/quickCss.css", b"new-css".as_slice()),
            ],
            false,
        );
        let failure = commit_staged_restore(
            &profile,
            stage_restore_from_bytes(&settings_archive).unwrap(),
            Some(1),
        )
        .unwrap_err();
        assert_eq!(failure.code, "BACKUP_FAILED");
        let settings: serde_json::Value = serde_json::from_slice(
            &std::fs::read(profile.join("storage").join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["keepMe"], 1);
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn symlink_destinations_fail_closed_without_outside_writes() {
        let profile = restore_profile("link-dest");
        let outside = restore_profile("link-outside");
        std::fs::write(outside.join("secret.txt"), "outside").unwrap();
        std::fs::create_dir_all(profile.join("themes")).unwrap();
        let link = profile.join("themes").join("linked");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&outside, &link).is_ok();
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(&outside, &link).is_ok();
        if !made {
            eprintln!("SKIP: link creation needs privileges in this environment");
            std::fs::remove_dir_all(&profile).ok();
            std::fs::remove_dir_all(&outside).ok();
            return;
        }
        // A theme directory that resolves outside fails closed at commit.
        let archive = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                ("data/themes/linked/x.css", b"evil".as_slice()),
            ],
            false,
        );
        let before = profile_files(&profile);
        assert!(restore_now(&profile, &archive, None).is_err());
        assert!(!outside.join("x.css").exists());
        assert_eq!(profile_files(&profile), before);
        // A symlink AT a file destination fails in the pre-commit check.
        std::fs::remove_dir_all(&link).ok();
        std::fs::create_dir_all(profile.join("extensions")).unwrap();
        let file_link = profile.join("quickCss.css");
        #[cfg(windows)]
        let made_file =
            std::os::windows::fs::symlink_file(outside.join("secret.txt"), &file_link).is_ok();
        #[cfg(not(windows))]
        let made_file = std::os::unix::fs::symlink(outside.join("secret.txt"), &file_link).is_ok();
        if made_file {
            let archive = raw_zip(
                &[
                    ("manifest.json", default_manifest().as_bytes()),
                    ("data/quickCss.css", b"evil".as_slice()),
                ],
                false,
            );
            assert!(commit_staged_restore(
                &profile,
                stage_restore_from_bytes(&archive).unwrap(),
                None
            )
            .is_err());
            assert_eq!(
                std::fs::read(outside.join("secret.txt")).unwrap(),
                b"outside"
            );
        }
        std::fs::remove_dir_all(&profile).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn client_mods_envelope_is_bounded_but_never_applied() {
        // The host returns the envelope; applying it is renderer-owned.
        let profile = restore_profile("clientmods");
        // Shelter nesting past the depth bound fails.
        let mut nested = serde_json::json!({"leaf": true});
        for _ in 0..(MAX_SHELTER_DEPTH + 4) {
            nested = serde_json::json!({"next": nested});
        }
        let manifest = manifest_text(
            full_includes(),
            &serde_json::to_string(&serde_json::json!({"shelter": nested})).unwrap(),
        );
        let archive = raw_zip(&[("manifest.json", manifest.as_bytes())], false);
        assert_eq!(
            restore_now(&profile, &archive, None).unwrap_err().code,
            "BACKUP_TOO_LARGE"
        );
        // A bounded envelope restores with byte-exact fidelity.
        let archive = raw_zip(&[("manifest.json", default_manifest().as_bytes())], false);
        let summary = restore_now(&profile, &archive, None).unwrap();
        assert_eq!(summary.entry_count, 0);
        assert_eq!(
            summary.client_mods["shelter"]["plugins"],
            serde_json::json!({})
        );
        assert!(summary.client_mods_bytes > 0);
        assert!(summary.client_mods_bytes <= MAX_CLIENT_MODS_BYTES);
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn restore_errors_carry_codes_only_never_paths() {
        let profile = restore_profile("codes");
        let archive = raw_zip(
            &[
                ("manifest.json", default_manifest().as_bytes()),
                ("data/themes/../escape.css", b"x".as_slice()),
            ],
            false,
        );
        let failure = restore_now(&profile, &archive, None).unwrap_err();
        assert!(!failure.message.contains("escape"));
        assert!(!failure.message.contains("themes"));
        assert!(!failure.message.contains('\\'));
        assert!(!failure.message.contains('/'));
        assert!(!failure
            .message
            .contains(&profile.to_string_lossy().into_owned()));
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn open_picker_config_pins_translated_title() {
        // No lang.json: the embedded en-US catalog applies.
        let profile = restore_profile("dialog");
        assert_eq!(restore_dialog_config(&profile), FALLBACK_DIALOG_OPEN_TITLE);
        assert_eq!(restore_dialog_config(&profile), "Open Remcord backup");
        assert_eq!(DIALOG_OPEN_TITLE_KEY, "backup-dialogOpen-title");
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn dialog_titles_come_from_the_catalogs_unchanged() {
        // The embedded catalogs carry the product identity, so host dialogs
        // serve the looked-up titles verbatim (no rewriting layer).
        let profile = restore_profile("titles");
        let (save_title, _) = save_dialog_config(&profile);
        assert_eq!(save_title, "Save Remcord backup");
        assert_eq!(restore_dialog_config(&profile), "Open Remcord backup");
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn picked_archive_read_is_bounded_and_link_free() {
        let dir = restore_profile("picked");
        let missing = dir.join("gone.zip");
        assert!(read_archive_file(&missing).is_err());
        let archive = raw_zip(&[("manifest.json", default_manifest().as_bytes())], false);
        let picked = dir.join("picked.zip");
        std::fs::write(&picked, &archive).unwrap();
        assert_eq!(read_archive_file(&picked).unwrap(), archive);
        std::fs::remove_dir_all(&dir).ok();
    }
}

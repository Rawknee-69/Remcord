//! Native arRPC rich-presence game state (F11 core).
//!
//! This module is the native counterpart of Electron's arRPC worker slice
//! (`src/rpc.ts`, `src/discord/rpcProcess.ts`) plus the two filesystem
//! owners behind the registered-games UI (`src/common/blacklistGame.ts`,
//! `src/common/detectables.ts`), exposed through the `window.remcord.rpc`
//! bridge (`src/discord/preload/bridge.ts`, `src/discord/ipc.ts`).
//!
//! Traced Electron behavior (actual implementation, not declarations):
//! - `rpcProcess.ts::getRpcServerSettings` resolves the worker settings from
//!   the config: `processScanning` / `windowsLegacyScanning` booleans plus
//!   `scanInterval` floored so a bad/zero value cannot busy-loop the scanner
//!   (`>= 1000` kept, otherwise the 5000 ms default).
//! - `startRPC` spawns the arRPC worker with the custom detectables and the
//!   resolved settings; the worker pushes `activity` / `invite` messages and
//!   answers `refreshProcessList` with its cached process list.
//! - `bridge.ts::rpc` exposes eight members: `refreshProcessList` (fire-and-
//!   forget), `getProcessList` / `getDetectables` (sync reads), `addDetectable`
//!   / `removeDetectable` (fire-and-forget file writes),
//!   `getBlacklist` (sync read served from a preload cache seeded at startup
//!   and converged by the `rpcBlacklistUpdated` broadcast), and
//!   `blacklistGame` / `unblacklistGame` (sync mutations that return the new
//!   list and broadcast it).
//! - `blacklistGame.ts` keeps `<userData>/storage/blacklist.json` with a
//!   500 ms read cache: corrupt/missing files read as `[]`, entries normalize
//!   (`Number(id)` non-NaN, non-empty `name`), adds dedupe by id.
//! - `detectables.ts` keeps `<userData>/storage/detectables.json` (created as
//!   `[]` when missing; a corrupt file throws); adds push blindly, removals
//!   filter by string id.
//!
//! Native contract (deliberate, documented differences from Electron):
//! - There is NO Node worker and NO Discord-protocol WebSocket listener in
//!   this slice: serving the 6463 handshake plus the detectable-matching scan
//!   loop is deferred to F11b (it needs a socket server with no equivalent in
//!   the current dependency set). What ships here is every host-ownable piece
//!   around it: scan-settings resolution, the blacklist store, the
//!   detectables store, live Windows process enumeration, and the typed
//!   activity/invite/blacklist event delivery the presence layer consumes.
//! - Process enumeration is LIVE per call (ToolHelp32 snapshot, sorted by pid):
//!   there is no stale worker-side cache, so `refresh_process_list` and
//!   `get_process_list` always agree. `refresh` resolves `{refreshed, count}`
//!   instead of `void` so callers can confirm the scan ran.
//! - `getBlacklist` stays SYNCHRONOUS (a document-start-seeded local cache,
//!   exactly like Electron's preload cache) so Shelter's hot
//!   `rpc.getBlacklist()` call in `listen()` keeps working without an async
//!   rewrite. Every other member is async with typed outcomes.
//! - Bounds everywhere Electron is unbounded: 512 blacklist entries, 256
//!   custom detectables, 2048 enumerated processes, 32 KiB activity payloads.
//! - Renderer input is data only (names, ids, game objects, JSON text) —
//!   never code, paths, URLs, or selectors. Both store files live under the
//!   host-built `<profile>/storage/` directory; persistence is atomic
//!   (sibling temp file + rename).
//!
//! Logical roots owned by this module: `<profile>/storage/blacklist.json`,
//! `<profile>/storage/detectables.json`, and the read-only OS process
//! snapshot. Nothing else on the filesystem is touched.

use std::path::{Path, PathBuf};

/// Floor so a bad/zero config value cannot busy-loop a scanner
/// (`rpcProcess.ts::MIN_SCAN_INTERVAL_MS` parity). Values below the floor
/// resolve to the default, exactly like Electron (no clamping).
pub const DEFAULT_SCAN_INTERVAL_MS: u64 = 5_000;

/// Maximum blacklist entries (Electron: unbounded array).
pub const MAX_BLACKLIST_ENTRIES: usize = 512;
/// Maximum blacklist game name in bytes (Electron: unbounded string).
pub const MAX_BLACKLIST_NAME_BYTES: usize = 256;
/// Maximum custom detectables (Electron: unbounded array).
pub const MAX_DETECTABLES: usize = 256;
/// Maximum stored detectables file in bytes (Electron: unbounded file).
pub const MAX_DETECTABLES_FILE_BYTES: u64 = 1024 * 1024;
/// Maximum enumerated processes per snapshot (Electron: unbounded array).
pub const MAX_PROCESSES: usize = 2048;
/// Maximum single executable path in bytes (Electron: unbounded string).
pub const MAX_EXE_PATH_BYTES: usize = 1024;
/// Maximum activity payload in bytes (Discord activity JSON is small).
pub const MAX_ACTIVITY_BYTES: usize = 32 * 1024;
/// Maximum invite code in bytes (Discord codes are short ASCII tokens).
pub const MAX_INVITE_CODE_BYTES: usize = 64;
/// Maximum detectable/game id in bytes (mirrors the keybind id bound).
pub const MAX_DETECTABLE_ID_BYTES: usize = 64;
/// Maximum detectable display string in bytes.
pub const MAX_DETECTABLE_TEXT_BYTES: usize = 256;
/// Maximum executables/aliases/themes entries per detectable.
pub const MAX_DETECTABLE_LIST_ENTRIES: usize = 64;

/// Typed failure: a stable machine-readable `code` plus a human message
/// that never contains a path, URL, or file content.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcFailure {
    pub code: &'static str,
    pub message: String,
}

fn failure(code: &'static str, message: impl Into<String>) -> RpcFailure {
    RpcFailure {
        code,
        message: message.into(),
    }
}

/// Resolved arRPC scanner settings, mirroring `rpcProcess.ts::ServerSettings`
/// wire names (`processScanning`, `windowsLegacyScanning`, `scanInterval`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcServerSettings {
    pub process_scanning: bool,
    pub windows_legacy_scanning: bool,
    pub scan_interval_ms: u64,
}

/// Resolve the authoritative snapshot to worker settings, mirroring
/// `getRpcServerSettings` exactly: the interval is kept when `>= 1000` and
/// falls back to 5000 otherwise (a corrupt/hand-edited zero can never
/// install a busy loop). There is deliberately no ceiling: Electron applies
/// any finite number, and a large interval only scans rarely.
pub fn resolve_rpc_settings(snapshot: &crate::settings::SettingsSnapshot) -> RpcServerSettings {
    let scan_interval_ms = if snapshot.scan_interval >= crate::settings_import::MIN_SCAN_INTERVAL_MS
    {
        snapshot.scan_interval
    } else {
        DEFAULT_SCAN_INTERVAL_MS
    };
    RpcServerSettings {
        process_scanning: snapshot.process_scanning,
        windows_legacy_scanning: snapshot.windows_legacy_scanning,
        scan_interval_ms,
    }
}

/// One blacklist entry. Field names use Electron's exact keys (`name`, `id`)
/// so bridge JSON stays shape-compatible with the registered-games UI.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RpcBlacklistEntry {
    pub name: String,
    pub id: i64,
}

/// Host-constructed blacklist path. The renderer cannot influence this: the
/// profile directory is fixed and the remaining segments are constants.
pub fn blacklist_file_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join("storage").join("blacklist.json")
}

/// Host-constructed detectables path. Same fixed-root contract as above.
pub fn detectables_file_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join("storage").join("detectables.json")
}

/// Normalize one raw blacklist entry, mirroring `blacklistGame.ts`
/// (`Number(id)` non-NaN, non-empty `name`): anything else is skipped, never
/// stored. Names are sliced to the native bound (Electron slices similar
/// display strings at its own save paths).
pub fn normalize_blacklist_entry(value: &serde_json::Value) -> Option<RpcBlacklistEntry> {
    let object = value.as_object()?;
    let id = object.get("id")?.as_i64()?;
    let name = object.get("name")?.as_str()?;
    if name.is_empty() {
        return None;
    }
    Some(RpcBlacklistEntry {
        name: slice_to_bytes(name, MAX_BLACKLIST_NAME_BYTES),
        id,
    })
}

/// Slice a string to at most `max_bytes` UTF-8 bytes on a char boundary.
fn slice_to_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Load the blacklist. Mirrors Electron's tolerant read: a missing file, a
/// corrupt file, or a non-array document reads as `[]` (never an error), so
/// a damaged store degrades to "nothing blacklisted" instead of breaking
/// the hot presence path. Malformed entries are skipped.
pub fn load_blacklist(profile_dir: &Path) -> Vec<RpcBlacklistEntry> {
    let raw = match std::fs::read(blacklist_file_path(profile_dir)) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let array = match parsed.as_array() {
        Some(array) => array,
        None => return Vec::new(),
    };
    array
        .iter()
        .filter_map(normalize_blacklist_entry)
        .take(MAX_BLACKLIST_ENTRIES)
        .collect()
}

/// Atomically persist one JSON document (sibling temp file + rename, so
/// readers never observe a truncated store). A temp file is never left
/// behind on failure.
fn persist_json_document(path: &Path, document: &serde_json::Value) -> Result<usize, RpcFailure> {
    let serialized = serde_json::to_string_pretty(document).map_err(|error| {
        failure(
            "RPC_UNAVAILABLE",
            format!("rpc store cannot be serialized: {error}"),
        )
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            failure(
                "RPC_UNAVAILABLE",
                format!("rpc store cannot be written: {error}"),
            )
        })?;
    }
    let temp_path = path.with_extension("json.tmp");
    if let Err(error) = std::fs::write(&temp_path, serialized.as_bytes()) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(failure(
            "RPC_UNAVAILABLE",
            format!("rpc store cannot be written: {error}"),
        ));
    }
    if let Err(error) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(failure(
            "RPC_UNAVAILABLE",
            format!("rpc store cannot be written: {error}"),
        ));
    }
    Ok(serialized.len())
}

/// Add one game to the blacklist, mirroring `blacklistGame` (dedupe by id —
/// a second add of the same id is a no-op that still returns the list).
/// Returns the new full list (Electron's `blacklistGame` handler returns it
/// for the cache update + broadcast).
pub fn add_blacklist_game(
    profile_dir: &Path,
    name: &str,
    id: i64,
) -> Result<Vec<RpcBlacklistEntry>, RpcFailure> {
    if name.is_empty() || name.len() > MAX_BLACKLIST_NAME_BYTES {
        return Err(failure(
            "INVALID_BLACKLIST_ENTRY",
            "blacklist name is invalid",
        ));
    }
    let mut entries = load_blacklist(profile_dir);
    if !entries.iter().any(|entry| entry.id == id) {
        if entries.len() >= MAX_BLACKLIST_ENTRIES {
            return Err(failure("BLACKLIST_FULL", "too many games are blacklisted"));
        }
        entries.push(RpcBlacklistEntry {
            name: name.to_owned(),
            id,
        });
    }
    let document = serde_json::to_value(&entries).map_err(|error| {
        failure(
            "RPC_UNAVAILABLE",
            format!("rpc store cannot be serialized: {error}"),
        )
    })?;
    persist_json_document(&blacklist_file_path(profile_dir), &document)?;
    Ok(entries)
}

/// Remove one game from the blacklist by id, mirroring `unblacklistGame`.
/// Returns the new full list. Unknown ids still persist (normalizing a
/// hand-edited file) and return the unchanged list with `removed: false`
/// reported by the entry count comparison at the call site.
pub fn remove_blacklist_game(
    profile_dir: &Path,
    id: i64,
) -> Result<Vec<RpcBlacklistEntry>, RpcFailure> {
    let entries = load_blacklist(profile_dir);
    let kept: Vec<RpcBlacklistEntry> = entries.into_iter().filter(|entry| entry.id != id).collect();
    let document = serde_json::to_value(&kept).map_err(|error| {
        failure(
            "RPC_UNAVAILABLE",
            format!("rpc store cannot be serialized: {error}"),
        )
    })?;
    persist_json_document(&blacklist_file_path(profile_dir), &document)?;
    Ok(kept)
}

/// One enumerated OS process: pid plus executable path. `args` (the third
/// `ProcessInfo` tuple member) is unavailable from a ToolHelp snapshot and
/// is always served as `[]` — documented, never invented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub exe_path: String,
}

/// Enumerate running processes via a ToolHelp32 snapshot, sorted by pid for
/// a deterministic response. Entries with an empty path are skipped; the
/// inventory is capped (Electron: unbounded). Only the pid and the exe path
/// cross — no command lines, no window titles, no usernames.
#[cfg(target_os = "windows")]
pub fn enumerate_processes() -> Result<Vec<ProcessEntry>, RpcFailure> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPPROCESS,
    };

    // SAFETY: snapshot creation takes no pointers and only reads the OS
    // process table; the handle is closed below on every path.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|_| failure("RPC_UNAVAILABLE", "process snapshot failed"))?;
    let result = enumerate_from_snapshot(snapshot);
    let _ = unsafe { CloseHandle(snapshot) };
    result
}

/// Snapshot walk split out so the handle lifetime is explicit: the snapshot
/// is always closed by the caller, even when the walk fails.
#[cfg(target_os = "windows")]
fn enumerate_from_snapshot(
    snapshot: windows::Win32::Foundation::HANDLE,
) -> Result<Vec<ProcessEntry>, RpcFailure> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        Process32FirstW, Process32NextW, PROCESSENTRY32W,
    };

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut out = Vec::new();
    // SAFETY: `entry` is a caller-owned struct with `dwSize` initialized per
    // the ToolHelp contract; the API writes only within it.
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_err() {
        return Err(failure("RPC_UNAVAILABLE", "process snapshot failed"));
    }
    loop {
        let end = entry
            .szExeFile
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(entry.szExeFile.len());
        let path = String::from_utf16_lossy(&entry.szExeFile[..end]);
        if !path.is_empty() && entry.th32ProcessID != 0 {
            out.push(ProcessEntry {
                pid: entry.th32ProcessID,
                exe_path: slice_to_bytes(&path, MAX_EXE_PATH_BYTES),
            });
            if out.len() >= MAX_PROCESSES {
                break;
            }
        }
        // SAFETY: same contract as above; iteration ends on API error.
        if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
            break;
        }
    }
    out.sort_by_key(|entry| entry.pid);
    Ok(out)
}

/// Non-Windows fallback: native is Windows-only, so there is nothing to
/// enumerate. Kept total so shared code compiles on every target.
#[cfg(not(target_os = "windows"))]
pub fn enumerate_processes() -> Result<Vec<ProcessEntry>, RpcFailure> {
    Ok(Vec::new())
}

/// Map enumerated processes to arRPC `ProcessInfo` tuples
/// (`[pid, exe_path, args]`): args are always `[]` (see [`ProcessEntry`]).
pub fn process_tuples(entries: &[ProcessEntry]) -> Vec<serde_json::Value> {
    entries
        .iter()
        .map(|entry| serde_json::json!([entry.pid, entry.exe_path, []]))
        .collect()
}

/// Validate one custom detectable game object, mirroring the exact shape
/// `AddDetectableModal` builds (`id`, `name`, `executables[]`, `aliases[]`,
/// overlay flags, `themes[]`). Unknown members are refused (closed contract:
/// the renderer cannot smuggle a future arRPC field past validation), every
/// string is sliced to its bound, and the canonical rebuilt value is
/// returned so stored documents never carry renderer-exact bytes.
pub fn validate_detectable(value: &serde_json::Value) -> Result<serde_json::Value, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "detectable must be an object".to_owned())?;
    for key in object.keys() {
        match key.as_str() {
            "id"
            | "name"
            | "executables"
            | "aliases"
            | "hook"
            | "overlay"
            | "overlay_compatibility_hook"
            | "overlay_methods"
            | "overlay_warn"
            | "themes" => {}
            _ => return Err("detectable carries an unknown member".to_owned()),
        }
    }
    let id = object
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "detectable id must be a string".to_owned())?;
    if id.is_empty() || id.len() > MAX_DETECTABLE_ID_BYTES {
        return Err("detectable id is invalid".to_owned());
    }
    let name = object
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "detectable name must be a string".to_owned())?;
    if name.is_empty() || name.len() > MAX_DETECTABLE_TEXT_BYTES {
        return Err("detectable name is invalid".to_owned());
    }
    let executables = object
        .get("executables")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "detectable executables must be an array".to_owned())?;
    if executables.is_empty() || executables.len() > MAX_DETECTABLE_LIST_ENTRIES {
        return Err("detectable executables are invalid".to_owned());
    }
    let mut canonical_executables = Vec::with_capacity(executables.len());
    for executable in executables {
        let exe = executable
            .as_object()
            .ok_or_else(|| "detectable executable must be an object".to_owned())?;
        for key in exe.keys() {
            match key.as_str() {
                "name" | "is_launcher" | "os" => {}
                _ => return Err("detectable executable carries an unknown member".to_owned()),
            }
        }
        let exe_name = exe
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "detectable executable name must be a string".to_owned())?;
        if exe_name.is_empty() || exe_name.len() > MAX_DETECTABLE_TEXT_BYTES {
            return Err("detectable executable name is invalid".to_owned());
        }
        let is_launcher = exe
            .get("is_launcher")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| "detectable is_launcher must be a boolean".to_owned())?;
        let os = exe
            .get("os")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "detectable os must be a string".to_owned())?;
        if !matches!(os, "win32" | "linux" | "darwin") {
            return Err("detectable os is not a supported platform".to_owned());
        }
        canonical_executables.push(serde_json::json!({
            "name": slice_to_bytes(exe_name, MAX_DETECTABLE_TEXT_BYTES),
            "is_launcher": is_launcher,
            "os": os,
        }));
    }
    let aliases = validated_string_list(object.get("aliases"), "aliases")?;
    let themes = validated_string_list(object.get("themes"), "themes")?;
    let flag = |key: &str| {
        object
            .get(key)
            .and_then(|v| v.as_bool())
            .ok_or_else(|| format!("detectable {key} must be a boolean"))
    };
    let hook = flag("hook")?;
    let overlay = flag("overlay")?;
    let overlay_compatibility_hook = flag("overlay_compatibility_hook")?;
    let overlay_warn = flag("overlay_warn")?;
    let overlay_methods = match object.get("overlay_methods") {
        None | Some(serde_json::Value::Null) => serde_json::Value::Null,
        Some(serde_json::Value::Number(number)) => serde_json::Value::Number(number.clone()),
        Some(_) => return Err("detectable overlay_methods must be a number or null".to_owned()),
    };
    Ok(serde_json::json!({
        "id": id,
        "name": slice_to_bytes(name, MAX_DETECTABLE_TEXT_BYTES),
        "executables": canonical_executables,
        "aliases": aliases,
        "hook": hook,
        "overlay": overlay,
        "overlay_compatibility_hook": overlay_compatibility_hook,
        "overlay_methods": overlay_methods,
        "overlay_warn": overlay_warn,
        "themes": themes,
    }))
}

/// Validate one optional string-list member (`aliases`, `themes`): missing
/// means `[]` (the modal always sends both, but the file may predate them);
/// present must be an array of bounded non-empty strings.
fn validated_string_list(
    value: Option<&serde_json::Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("detectable {key} must be an array"))?;
    if array.len() > MAX_DETECTABLE_LIST_ENTRIES {
        return Err(format!("detectable {key} carries too many entries"));
    }
    let mut out = Vec::with_capacity(array.len());
    for entry in array {
        let text = entry
            .as_str()
            .ok_or_else(|| format!("detectable {key} entries must be strings"))?;
        if text.is_empty() || text.len() > MAX_DETECTABLE_TEXT_BYTES {
            return Err(format!("detectable {key} entry is invalid"));
        }
        out.push(slice_to_bytes(text, MAX_DETECTABLE_TEXT_BYTES));
    }
    Ok(out)
}

/// Load the custom detectables. A missing file reads as `[]` (Electron
/// creates it); a corrupt/oversized file or a non-array document is a typed
/// failure (Electron would throw — surfaced here as `RPC_UNAVAILABLE`
/// instead of an uncaught exception). Malformed entries are skipped.
pub fn load_detectables(profile_dir: &Path) -> Result<Vec<serde_json::Value>, RpcFailure> {
    let path = detectables_file_path(profile_dir);
    let raw = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(failure(
                "RPC_UNAVAILABLE",
                format!("detectables store is unreadable: {error}"),
            ));
        }
    };
    if raw.len() as u64 > MAX_DETECTABLES_FILE_BYTES {
        return Err(failure(
            "RPC_UNAVAILABLE",
            "detectables store exceeds the size limit",
        ));
    }
    let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|error| {
        failure(
            "RPC_UNAVAILABLE",
            format!("detectables store is invalid: {error}"),
        )
    })?;
    let array = parsed.as_array().ok_or_else(|| {
        failure(
            "RPC_UNAVAILABLE",
            "detectables store is invalid: top-level JSON must be an array",
        )
    })?;
    Ok(array
        .iter()
        .filter_map(|entry| validate_detectable(entry).ok())
        .take(MAX_DETECTABLES)
        .collect())
}

/// Add one custom detectable, mirroring `addDetectable` (Electron pushes
/// blindly — no dedupe). The entry is validated first; the canonical
/// rebuilt value is what persists. Returns the new entry count.
pub fn add_detectable(
    profile_dir: &Path,
    detectable: &serde_json::Value,
) -> Result<usize, RpcFailure> {
    let canonical = validate_detectable(detectable)
        .map_err(|message| failure("INVALID_DETECTABLE", message))?;
    let mut entries = load_detectables(profile_dir).unwrap_or_default();
    if entries.len() >= MAX_DETECTABLES {
        return Err(failure(
            "DETECTABLES_FULL",
            "too many custom detectables are stored",
        ));
    }
    entries.push(canonical);
    persist_json_document(
        &detectables_file_path(profile_dir),
        &serde_json::Value::Array(entries),
    )?;
    Ok(load_detectables(profile_dir).unwrap_or_default().len())
}

/// Remove one custom detectable by string id, mirroring `removeDetectable`.
/// Returns whether an entry was removed (Electron is `void`; the outcome
/// lets callers distinguish "removed" from "unknown id" without a second
/// read).
pub fn remove_detectable(profile_dir: &Path, id: &str) -> Result<bool, RpcFailure> {
    validate_detectable_id(id).map_err(|message| failure("INVALID_DETECTABLE", message))?;
    let entries = load_detectables(profile_dir).unwrap_or_default();
    let before = entries.len();
    let kept: Vec<serde_json::Value> = entries
        .into_iter()
        .filter(|entry| entry.get("id").and_then(|v| v.as_str()) != Some(id))
        .collect();
    let removed = kept.len() != before;
    persist_json_document(
        &detectables_file_path(profile_dir),
        &serde_json::Value::Array(kept),
    )?;
    Ok(removed)
}

/// Validate one detectable id for the remove path (presence + byte cap;
/// the file comparison is an exact string match).
pub fn validate_detectable_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > MAX_DETECTABLE_ID_BYTES {
        return Err("detectable id is invalid".to_owned());
    }
    Ok(())
}

/// Build the host→renderer activity event detail: the activity JSON text
/// must parse and fit the bound (it travels verbatim — the host never
/// interprets Discord's activity shape, so a future field cannot break
/// validation). Returns the serialized `remcord-native:rpc-activity` detail.
///
/// The F11b socket source is the shipped producer; unit tests pin the
/// contract until it lands.
#[allow(dead_code)]
pub fn rpc_activity_event_json(data: &str) -> Result<String, String> {
    if data.is_empty() || data.len() > MAX_ACTIVITY_BYTES {
        return Err("rpc activity exceeds the size limit".to_owned());
    }
    let activity: serde_json::Value =
        serde_json::from_str(data).map_err(|_| "rpc activity is not valid JSON".to_owned())?;
    Ok(serde_json::json!({
        "version": crate::protocol::PROTOCOL_VERSION,
        "activity": activity,
    })
    .to_string())
}

/// Build the host→renderer invite event detail: invite codes are short
/// ASCII tokens (`[A-Za-z0-9_-]`), so anything else is refused before
/// delivery. Returns the serialized `remcord-native:rpc-invite` detail.
///
/// The F11b socket source is the shipped producer; unit tests pin the
/// contract until it lands.
#[allow(dead_code)]
pub fn rpc_invite_event_json(code: &str) -> Result<String, String> {
    if code.is_empty()
        || code.len() > MAX_INVITE_CODE_BYTES
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("rpc invite code is invalid".to_owned());
    }
    Ok(serde_json::json!({
        "version": crate::protocol::PROTOCOL_VERSION,
        "code": code,
    })
    .to_string())
}

/// Build the host→renderer blacklist broadcast detail (Electron's
/// `rpcBlacklistUpdated` parity): the full validated list, bounded by
/// construction. Returns the serialized
/// `remcord-native:rpc-blacklist-updated` detail.
pub fn rpc_blacklist_event_json(entries: &[RpcBlacklistEntry]) -> Result<String, String> {
    Ok(serde_json::json!({
        "version": crate::protocol::PROTOCOL_VERSION,
        "entries": entries,
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_profile_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be available")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "remcord-arrpc-{tag}-{nanos}-{}",
            std::process::id()
        ))
    }

    fn snapshot_with(
        scan_interval: u64,
        process_scanning: bool,
    ) -> crate::settings::SettingsSnapshot {
        crate::settings::SettingsSnapshot {
            scan_interval,
            process_scanning,
            ..Default::default()
        }
    }

    #[test]
    fn scan_settings_mirror_the_worker_resolution() {
        // Defaults: scanning on, legacy off, 5000 ms.
        let resolved = resolve_rpc_settings(&crate::settings::SettingsSnapshot::default());
        assert!(resolved.process_scanning);
        assert!(!resolved.windows_legacy_scanning);
        assert_eq!(resolved.scan_interval_ms, 5_000);
        // A healthy custom interval is kept verbatim.
        let resolved = resolve_rpc_settings(&snapshot_with(10_000, true));
        assert_eq!(resolved.scan_interval_ms, 10_000);
        // A bad/zero interval falls back to the default (never a busy loop).
        for bad in [0, 1, 999] {
            let resolved = resolve_rpc_settings(&snapshot_with(bad, true));
            assert_eq!(resolved.scan_interval_ms, DEFAULT_SCAN_INTERVAL_MS);
        }
        // The floor itself is kept.
        let resolved = resolve_rpc_settings(&snapshot_with(1_000, false));
        assert_eq!(resolved.scan_interval_ms, 1_000);
        assert!(!resolved.process_scanning);
    }

    #[test]
    fn blacklist_reads_empty_when_missing_or_corrupt() {
        let profile = unique_profile_dir("blacklist-missing");
        assert!(load_blacklist(&profile).is_empty());
        let corrupt = unique_profile_dir("blacklist-corrupt");
        std::fs::create_dir_all(corrupt.join("storage")).unwrap();
        std::fs::write(corrupt.join("storage").join("blacklist.json"), b"{not json").unwrap();
        assert!(load_blacklist(&corrupt).is_empty());
        let scalar = unique_profile_dir("blacklist-scalar");
        std::fs::create_dir_all(scalar.join("storage")).unwrap();
        std::fs::write(
            scalar.join("storage").join("blacklist.json"),
            b"{\"name\":\"x\"}",
        )
        .unwrap();
        assert!(load_blacklist(&scalar).is_empty());
    }

    #[test]
    fn blacklist_add_dedupes_and_round_trips() {
        let profile = unique_profile_dir("blacklist-add");
        let entries = add_blacklist_game(&profile, "Some Game", 123).expect("add must persist");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Some Game");
        assert_eq!(entries[0].id, 123);
        // A second add of the same id is a no-op returning the same list.
        let again = add_blacklist_game(&profile, "Renamed", 123).expect("dedupe must persist");
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].name, "Some Game");
        // No temp file is left behind and the file parses.
        assert!(!profile.join("storage").join("blacklist.json.tmp").exists());
        let reread = load_blacklist(&profile);
        assert_eq!(reread, again);
        // Malformed stored entries are skipped on read.
        std::fs::write(
            profile.join("storage").join("blacklist.json"),
            r#"[{"name":"ok","id":1},{"name":"","id":2},{"id":3},{"name":"nan","id":"x"}]"#,
        )
        .unwrap();
        let reread = load_blacklist(&profile);
        assert_eq!(reread.len(), 1);
        assert_eq!(reread[0].id, 1);
    }

    #[test]
    fn blacklist_add_rejects_bad_names() {
        let profile = unique_profile_dir("blacklist-badname");
        assert!(add_blacklist_game(&profile, "", 1).is_err());
        assert!(add_blacklist_game(&profile, &"x".repeat(257), 1).is_err());
    }

    #[test]
    fn blacklist_remove_filters_by_id() {
        let profile = unique_profile_dir("blacklist-remove");
        add_blacklist_game(&profile, "A", 1).unwrap();
        add_blacklist_game(&profile, "B", 2).unwrap();
        let kept = remove_blacklist_game(&profile, 1).expect("remove must persist");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, 2);
        // Unknown ids persist the unchanged list (normalizing the file).
        let kept = remove_blacklist_game(&profile, 999).expect("unknown remove must persist");
        assert_eq!(kept.len(), 1);
    }

    fn modal_detectable() -> serde_json::Value {
        serde_json::json!({
            "name": "My Game",
            "executables": [{"name": "game.exe", "is_launcher": false, "os": "win32"}],
            "id": "123456",
            "aliases": ["mg"],
            "hook": false,
            "overlay": true,
            "overlay_compatibility_hook": false,
            "overlay_methods": null,
            "overlay_warn": false,
            "themes": ["dark"]
        })
    }

    #[test]
    fn detectable_validation_accepts_the_modal_shape() {
        let canonical =
            validate_detectable(&modal_detectable()).expect("modal shape must validate");
        assert_eq!(canonical["id"], "123456");
        assert_eq!(canonical["executables"][0]["os"], "win32");
        // Missing optional lists default to [].
        let mut minimal = modal_detectable();
        minimal.as_object_mut().unwrap().remove("aliases");
        minimal.as_object_mut().unwrap().remove("themes");
        let canonical = validate_detectable(&minimal).expect("missing lists must default");
        assert_eq!(canonical["aliases"], serde_json::json!([]));
    }

    #[test]
    fn detectable_validation_rejects_hostile_shapes() {
        let mut bad = modal_detectable();
        bad.as_object_mut()
            .unwrap()
            .insert("path".to_owned(), serde_json::json!("C:\\evil"));
        assert!(validate_detectable(&bad).is_err());
        for patch in [
            serde_json::json!({"id": ""}),
            serde_json::json!({"id": 1}),
            serde_json::json!({"name": ""}),
            serde_json::json!({"executables": []}),
            serde_json::json!({"executables": [{"name": "x", "is_launcher": false, "os": "plan9"}]}),
            serde_json::json!({"hook": "yes"}),
            serde_json::json!({"overlay_methods": "all"}),
        ] {
            let mut candidate = modal_detectable();
            for (key, value) in patch.as_object().unwrap() {
                candidate
                    .as_object_mut()
                    .unwrap()
                    .insert(key.clone(), value.clone());
            }
            assert!(
                validate_detectable(&candidate).is_err(),
                "must refuse {patch}"
            );
        }
        assert!(validate_detectable_id("").is_err());
        assert!(validate_detectable_id(&"x".repeat(65)).is_err());
        assert!(validate_detectable_id("abc-1").is_ok());
    }

    #[test]
    fn detectables_add_remove_round_trip() {
        let profile = unique_profile_dir("detectables");
        assert!(load_detectables(&profile)
            .expect("missing must read []")
            .is_empty());
        let count = add_detectable(&profile, &modal_detectable()).expect("add must persist");
        assert_eq!(count, 1);
        let entries = load_detectables(&profile).expect("stored must load");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["name"], "My Game");
        assert!(!profile
            .join("storage")
            .join("detectables.json.tmp")
            .exists());
        // Electron pushes blindly: a second identical add stores a duplicate.
        let count = add_detectable(&profile, &modal_detectable()).expect("dupe must persist");
        assert_eq!(count, 2);
        assert!(remove_detectable(&profile, "123456").expect("remove must persist"));
        assert!(load_detectables(&profile).expect("reload").is_empty());
        assert!(!remove_detectable(&profile, "nope").expect("unknown remove must persist"));
    }

    #[test]
    fn detectables_corrupt_file_is_a_typed_failure() {
        let profile = unique_profile_dir("detectables-corrupt");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(
            profile.join("storage").join("detectables.json"),
            b"{not json",
        )
        .unwrap();
        let failure = load_detectables(&profile).expect_err("corrupt must fail");
        assert_eq!(failure.code, "RPC_UNAVAILABLE");
        assert!(!failure.message.contains("storage"));
    }

    #[test]
    fn process_enumeration_returns_a_bounded_sorted_inventory() {
        let entries = enumerate_processes().expect("enumeration must succeed");
        assert!(entries.len() <= MAX_PROCESSES);
        let mut last = 0u32;
        for entry in &entries {
            assert!(entry.pid != 0);
            assert!(!entry.exe_path.is_empty());
            assert!(entry.exe_path.len() <= MAX_EXE_PATH_BYTES);
            assert!(entry.pid >= last, "entries must be sorted by pid");
            last = entry.pid;
        }
        // The current process must observe itself.
        assert!(entries.iter().any(|entry| entry.pid == std::process::id()));
        // Tuples carry the arRPC [pid, path, args] shape with empty args.
        let tuples = process_tuples(&entries[..entries.len().min(3)]);
        assert_eq!(tuples.len(), entries.len().min(3));
        for tuple in &tuples {
            let array = tuple.as_array().expect("tuple must be an array");
            assert_eq!(array.len(), 3);
            assert!(array[0].is_number());
            assert!(array[1].is_string());
            assert_eq!(array[2], serde_json::json!([]));
        }
    }

    #[test]
    fn event_builders_bound_and_shape_their_payloads() {
        let activity =
            rpc_activity_event_json(r#"{"activity":null}"#).expect("valid JSON must build");
        let parsed: serde_json::Value = serde_json::from_str(&activity).unwrap();
        assert_eq!(parsed["version"], crate::protocol::PROTOCOL_VERSION);
        assert!(rpc_activity_event_json("not json").is_err());
        assert!(rpc_activity_event_json(&"x".repeat(MAX_ACTIVITY_BYTES + 1)).is_err());
        let invite = rpc_invite_event_json("abc-DEF_123").expect("token must build");
        let parsed: serde_json::Value = serde_json::from_str(&invite).unwrap();
        assert_eq!(parsed["code"], "abc-DEF_123");
        for bad in ["", "has space", "semi;colon", "C:\\x", &"x".repeat(65)] {
            assert!(rpc_invite_event_json(bad).is_err(), "{bad} must be refused");
        }
        let entries = vec![RpcBlacklistEntry {
            name: "G".to_owned(),
            id: 7,
        }];
        let broadcast = rpc_blacklist_event_json(&entries).expect("broadcast must build");
        let parsed: serde_json::Value = serde_json::from_str(&broadcast).unwrap();
        assert_eq!(parsed["entries"][0]["id"], 7);
    }
}

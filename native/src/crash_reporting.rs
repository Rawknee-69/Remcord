//! Local-only crash reporting (F26).
//!
//! Native counterpart of Electron's crash-directory handling:
//! - `crashReporter.start({ uploadToServer: false })` (`src/main.ts`):
//!   local collection only, never sent anywhere. Natively the host ensures
//!   `<profile>/crashes/` at startup and persists one bounded JSON line per
//!   observed WebView2 process failure. There is no uploader, no network
//!   traffic, and no crashpad: failure records come from the existing Phase
//!   1 `process_failure` observer, and recovery policy stays in `recovery`,
//!   `child_recovery`, and `browser_recovery` (Electron's
//!   `render-process-gone -> app.relaunch()` maps to the bounded host-side
//!   reload there, never to a whole-app restart here).
//! - `openCrashesFolder` (`src/discord/ipc.ts`): reveal semantics through
//!   the new `crash.open_folder` capability, reusing
//!   `folders::reveal_folder` on a host-built root.
//! - `crash` (`process.crash()`, no renderer caller): deliberately NOT
//!   ported. A renderer-reachable self-destruct primitive has no place in
//!   the native bridge.
//!
//! Deliberate difference: Electron collects into the OS temp directory
//! (`<temp>/<name> Crashes`, wiped on reboot); native collects under the
//! profile so crash history survives restarts like every other native root.
//!
//! Privacy: records carry only fixed vocabulary (role, kind, reason) plus
//! bounded numerics and the recovery policy's own fixed-vocabulary outcome
//! line (already printed to stdout today). No URIs, page source, cookies,
//! tokens, message bodies, process descriptions, or frame data can appear
//! here by construction. Error messages carry only fixed labels, never host
//! paths.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::process_failure::{ProcessFailedEvent, ProcessFailedKind, ProcessFailedReason};

/// Fixed directory name under the profile root. The renderer never supplies
/// this string; both crash capabilities resolve it host-side.
pub const CRASH_DIR_NAME: &str = "crashes";
/// Fixed JSON-lines file inside the crash directory. One object per line.
pub const CRASH_LOG_FILE_NAME: &str = "process-failures.jsonl";
/// Schema version stamped on every record.
pub const CRASH_LOG_SCHEMA: u8 = 1;
/// Maximum records retained in the log file and returned by the inventory.
/// Oldest entries are dropped first; the newest record always lands.
pub const MAX_CRASH_RECORDS: usize = 500;
/// Maximum log file size in bytes before compaction keeps the newest half
/// of [`MAX_CRASH_RECORDS`]. Every read path compacts first, so no read is
/// ever unbounded.
pub const MAX_LOG_BYTES: u64 = 256 * 1024;
/// Maximum dump files reported by the inventory. Listing stops here; the
/// files themselves are never opened or read.
pub const MAX_DUMP_FILES: usize = 256;
/// Maximum Unicode scalar values kept from a recovery outcome label.
/// Longer labels are truncated; the record is never rejected for length.
pub const MAX_RECOVERY_CHARS: usize = 512;
/// Maximum single log line length honored when reading. Longer lines are
/// skipped, never allocated into records.
pub const MAX_LOG_LINE_CHARS: usize = 8 * 1024;
/// Maximum dump file name length in bytes. Longer names are skipped.
pub const MAX_DUMP_NAME_BYTES: usize = 128;
/// Renderer-facing location label echoed by `crash.open_folder`.
pub const CRASH_LOCATION_LABEL: &str = "crashes";

/// Typed failure for the capability: a stable machine-readable `code` plus a
/// human message that never contains a filesystem path.
#[derive(Debug)]
pub struct CrashFailure {
    pub code: &'static str,
    pub message: String,
}

fn failure(code: &'static str, message: &str) -> CrashFailure {
    CrashFailure {
        code,
        message: message.to_owned(),
    }
}

/// Host-built crash root. Single-component join onto the profile directory;
/// cannot escape lexically.
pub fn crash_dir(profile_dir: &Path) -> PathBuf {
    profile_dir.join(CRASH_DIR_NAME)
}

/// Ensure the fixed crash root exists and resolve it to its canonical path.
/// Creates the directory when missing (host-owned well-known root only),
/// then requires canonical containment under the canonical profile root so
/// a pre-planted link at the root can never redirect collection or reveal.
pub fn ensure_crash_dir(profile_dir: &Path) -> Result<PathBuf, CrashFailure> {
    let root = crash_dir(profile_dir);
    std::fs::create_dir_all(&root)
        .map_err(|_| failure("CRASH_UNAVAILABLE", "crash directory is unavailable"))?;
    if !root.is_dir() {
        return Err(failure(
            "CRASH_UNAVAILABLE",
            "crash directory is unavailable",
        ));
    }
    let canonical_root = std::fs::canonicalize(profile_dir)
        .map_err(|_| failure("CRASH_UNAVAILABLE", "crash directory is unavailable"))?;
    let canonical = std::fs::canonicalize(&root)
        .map_err(|_| failure("CRASH_UNAVAILABLE", "crash directory is unavailable"))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(failure(
            "OUTSIDE_ROOT",
            "crash directory is outside the approved root",
        ));
    }
    Ok(canonical)
}

/// Milliseconds since the Unix epoch. Clock failures degrade to zero rather
/// than blocking collection.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn kind_label(kind: ProcessFailedKind) -> String {
    match kind {
        ProcessFailedKind::Unrecognized(value) => format!("Unrecognized({value})"),
        known => known.as_str().to_owned(),
    }
}

fn reason_label(reason: Option<ProcessFailedReason>) -> String {
    match reason {
        Some(ProcessFailedReason::Unrecognized(value)) => format!("Unrecognized({value})"),
        Some(known) => known.as_str().to_owned(),
        None => "unavailable".to_owned(),
    }
}

fn truncate_recovery(recovery: &str) -> String {
    if recovery.chars().count() > MAX_RECOVERY_CHARS {
        recovery.chars().take(MAX_RECOVERY_CHARS).collect()
    } else {
        recovery.to_owned()
    }
}

/// One persisted process-failure record. Fixed schema only: closed-vocabulary
/// strings, bounded numerics, and the recovery policy's outcome line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashRecord {
    pub schema: u8,
    pub timestamp_ms: u64,
    pub role: String,
    pub kind: String,
    pub reason: String,
    pub exit_code: Option<i32>,
    pub recovery: String,
}

/// Pure constructor over one sanitized observer event plus the recovery
/// policy's outcome for it. Timestamps are caller-supplied so tests stay
/// deterministic; [`record_process_failure`] stamps wall-clock time.
pub fn record_from_event(
    event: &ProcessFailedEvent,
    recovery: &str,
    timestamp_ms: u64,
) -> CrashRecord {
    CrashRecord {
        schema: CRASH_LOG_SCHEMA,
        timestamp_ms,
        role: event.role.as_str().to_owned(),
        kind: kind_label(event.kind),
        reason: reason_label(event.reason),
        exit_code: event.exit_code,
        recovery: truncate_recovery(recovery),
    }
}

fn log_path(dir: &Path) -> PathBuf {
    dir.join(CRASH_LOG_FILE_NAME)
}

/// Compact an oversized log by keeping the newest half of
/// [`MAX_CRASH_RECORDS`]. Runs before every append and every read, so the
/// file can never grow without bound no matter how often the renderer
/// crashes.
fn compact_log_if_oversized(log: &Path) -> Result<(), CrashFailure> {
    let len = std::fs::metadata(log).map(|meta| meta.len()).unwrap_or(0);
    if len <= MAX_LOG_BYTES {
        return Ok(());
    }
    let content = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let keep_from = lines.len().saturating_sub(MAX_CRASH_RECORDS / 2);
    let kept = lines[keep_from..].join("\n");
    let next = if kept.is_empty() {
        String::new()
    } else {
        kept + "\n"
    };
    std::fs::write(log, next)
        .map_err(|_| failure("CRASH_UNAVAILABLE", "crash directory is unavailable"))?;
    Ok(())
}

/// Persist one record for an observed process failure. Creates the crash
/// directory on demand and compacts an oversized log first, so the newest
/// record always lands. Best-effort by contract: callers log the failure
/// and continue; crash history must never affect recovery decisions.
pub fn record_process_failure(
    profile_dir: &Path,
    event: &ProcessFailedEvent,
    recovery: &str,
) -> Result<(), CrashFailure> {
    let dir = ensure_crash_dir(profile_dir)?;
    let log = log_path(&dir);
    compact_log_if_oversized(&log)?;
    let record = record_from_event(event, recovery, now_ms());
    let mut line = serde_json::to_string(&record)
        .map_err(|_| failure("CRASH_UNAVAILABLE", "crash directory is unavailable"))?;
    line.push('\n');
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(|_| failure("CRASH_UNAVAILABLE", "crash directory is unavailable"))?;
    Ok(())
}

/// One minidump file present in the crash directory. Metadata only: the
/// host never opens or reads dump contents. Native ships no dump writer
/// (no crashpad), so this inventory is normally empty; it exists so a
/// future writer or a user-copied dump is visible to diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrashDumpEntry {
    pub name: String,
    pub bytes: u64,
}

/// Bounded crash inventory served by the `crash.list` capability.
/// `records` are newest-first and capped at [`MAX_CRASH_RECORDS`];
/// `record_count` is the number of records returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrashInventory {
    pub available: bool,
    pub record_count: u64,
    pub records: Vec<CrashRecord>,
    pub dump_files: Vec<CrashDumpEntry>,
}

fn has_dump_extension(name: &str) -> bool {
    name.len() <= MAX_DUMP_NAME_BYTES && name.to_ascii_lowercase().ends_with(".dmp")
}

fn list_dump_files(dir: &Path) -> Vec<CrashDumpEntry> {
    let mut entries = Vec::new();
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(_) => return entries,
    };
    for entry in read.flatten() {
        if entries.len() >= MAX_DUMP_FILES {
            break;
        }
        let name = match entry.file_name().to_str() {
            Some(name) => name.to_owned(),
            None => continue,
        };
        if !has_dump_extension(&name) {
            continue;
        }
        // `symlink_metadata` never follows links: a planted symlink is a
        // symlink, not a file, and is skipped instead of resolved.
        let meta = match std::fs::symlink_metadata(entry.path()) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if !meta.file_type().is_file() {
            continue;
        }
        entries.push(CrashDumpEntry {
            name,
            bytes: meta.len(),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// Read the bounded crash inventory. Ensures the directory (created when
/// missing, mirroring the reveal roots) and compacts an oversized log
/// first. Malformed lines are skipped, never fatal: a hand-edited log
/// cannot break the capability.
pub fn list_crashes(profile_dir: &Path) -> Result<CrashInventory, CrashFailure> {
    let dir = ensure_crash_dir(profile_dir)?;
    let log = log_path(&dir);
    compact_log_if_oversized(&log)?;
    let mut records = Vec::new();
    if let Ok(content) = std::fs::read_to_string(&log) {
        for line in content.lines().rev() {
            if records.len() >= MAX_CRASH_RECORDS {
                break;
            }
            if line.chars().count() > MAX_LOG_LINE_CHARS {
                continue;
            }
            let record: CrashRecord = match serde_json::from_str(line) {
                Ok(record) => record,
                Err(_) => continue,
            };
            if record.schema != CRASH_LOG_SCHEMA {
                continue;
            }
            records.push(record);
        }
    }
    let record_count = records.len() as u64;
    Ok(CrashInventory {
        available: true,
        record_count,
        records,
        dump_files: list_dump_files(&dir),
    })
}

/// Reveal the fixed crash root in Explorer (the `openCrashesFolder`
/// equivalent). Reuses the shared reveal helper on the host-built,
/// containment-checked directory; codes pass through unchanged.
pub fn open_crash_folder(profile_dir: &Path) -> Result<(), CrashFailure> {
    let dir = ensure_crash_dir(profile_dir)?;
    crate::folders::reveal_folder(&dir).map_err(|failure| CrashFailure {
        code: failure.code,
        message: failure.message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_failure::{ProcessFailedKind, ProcessFailedReason, WebViewRole};

    fn unique_temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "remcord-crash-test-{}-{}-{tag}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&root).expect("test temp root must be creatable");
        root
    }

    fn remove_temp_root(root: &Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    fn test_event() -> ProcessFailedEvent {
        ProcessFailedEvent {
            role: WebViewRole::Main,
            instance_id: 0,
            kind: ProcessFailedKind::RenderProcessExited,
            reason: Some(ProcessFailedReason::Crashed),
            exit_code: Some(1),
        }
    }

    #[test]
    fn crash_root_is_a_fixed_single_component_name() {
        assert_eq!(CRASH_DIR_NAME, "crashes");
        assert_eq!(CRASH_LOCATION_LABEL, "crashes");
        assert_eq!(CRASH_LOG_FILE_NAME, "process-failures.jsonl");
        let profile = PathBuf::from("C:\\profile");
        assert_eq!(crash_dir(&profile), profile.join("crashes"));
    }

    #[test]
    fn ensure_creates_the_missing_root_under_the_profile() {
        let profile = unique_temp_root("ensure");
        let resolved = ensure_crash_dir(&profile).expect("crash root must resolve");
        assert!(resolved.is_dir(), "crash root must exist afterwards");
        assert!(
            resolved
                .starts_with(std::fs::canonicalize(&profile).expect("profile must canonicalize")),
            "crash root must stay under the profile"
        );
        // Idempotent: a second ensure resolves the same directory.
        let again = ensure_crash_dir(&profile).expect("re-ensure must resolve");
        assert_eq!(resolved, again);
        remove_temp_root(&profile);
    }

    #[test]
    fn ensure_fails_closed_when_a_file_blocks_the_root() {
        let profile = unique_temp_root("blocked");
        std::fs::write(profile.join(CRASH_DIR_NAME), b"planted")
            .expect("blocker fixture must be writable");
        let failure = ensure_crash_dir(&profile).expect_err("blocked root must fail");
        assert_eq!(failure.code, "CRASH_UNAVAILABLE");
        assert!(
            !failure.message.contains("planted"),
            "messages carry fixed labels, never paths"
        );
        remove_temp_root(&profile);
    }

    #[test]
    fn event_mapping_uses_fixed_vocabulary_only() {
        let record = record_from_event(&test_event(), "reload_requested", 1_700_000_000_000);
        assert_eq!(
            record,
            CrashRecord {
                schema: CRASH_LOG_SCHEMA,
                timestamp_ms: 1_700_000_000_000,
                role: "main".to_owned(),
                kind: "RenderProcessExited".to_owned(),
                reason: "Crashed".to_owned(),
                exit_code: Some(1),
                recovery: "reload_requested".to_owned(),
            }
        );
        // Missing extended args degrade to the fixed token, never a guess.
        let degraded = ProcessFailedEvent {
            role: WebViewRole::Child,
            instance_id: 3,
            kind: ProcessFailedKind::UtilityProcessExited,
            reason: None,
            exit_code: None,
        };
        let record = record_from_event(&degraded, "observed", 7);
        assert_eq!(record.role, "child");
        assert_eq!(record.kind, "UtilityProcessExited");
        assert_eq!(record.reason, "unavailable");
        assert_eq!(record.exit_code, None);
        // Unrecognized numerics stay numeric; nothing else varies.
        let odd = ProcessFailedEvent {
            role: WebViewRole::Main,
            instance_id: 0,
            kind: ProcessFailedKind::Unrecognized(12),
            reason: Some(ProcessFailedReason::Unrecognized(9)),
            exit_code: Some(-1),
        };
        let record = record_from_event(&odd, "observed", 7);
        assert_eq!(record.kind, "Unrecognized(12)");
        assert_eq!(record.reason, "Unrecognized(9)");
    }

    #[test]
    fn recovery_labels_truncate_instead_of_rejecting() {
        let long = "r".repeat(MAX_RECOVERY_CHARS + 64);
        let record = record_from_event(&test_event(), &long, 1);
        assert_eq!(record.recovery.chars().count(), MAX_RECOVERY_CHARS);
        let short = "observed";
        let record = record_from_event(&test_event(), short, 1);
        assert_eq!(record.recovery, "observed");
    }

    #[test]
    fn record_then_list_round_trips_newest_first() {
        let profile = unique_temp_root("roundtrip");
        record_process_failure(&profile, &test_event(), "first").expect("record must land");
        record_process_failure(&profile, &test_event(), "second").expect("record must land");
        let inventory = list_crashes(&profile).expect("inventory must read");
        assert!(inventory.available);
        assert_eq!(inventory.record_count, 2);
        assert_eq!(inventory.records.len(), 2);
        assert_eq!(inventory.records[0].recovery, "second");
        assert_eq!(inventory.records[1].recovery, "first");
        assert!(inventory.dump_files.is_empty());
        // The log file is the only writer output: one JSON object per line.
        let log = std::fs::read_to_string(log_path(&ensure_crash_dir(&profile).unwrap()))
            .expect("log must be readable");
        assert_eq!(log.lines().count(), 2);
        for line in log.lines() {
            let value: serde_json::Value =
                serde_json::from_str(line).expect("every line must be one JSON object");
            assert_eq!(value["schema"], CRASH_LOG_SCHEMA);
        }
        remove_temp_root(&profile);
    }

    #[test]
    fn malformed_and_foreign_schema_lines_are_skipped() {
        let profile = unique_temp_root("malformed");
        let dir = ensure_crash_dir(&profile).expect("crash root must resolve");
        std::fs::write(
            log_path(&dir),
            "{\"schema\":999,\"timestamp_ms\":1,\"role\":\"main\",\"kind\":\"X\",\"reason\":\"Y\",\"exit_code\":null,\"recovery\":\"z\"}\nnot json\n\n",
        )
        .expect("fixture log must be writable");
        record_process_failure(&profile, &test_event(), "good").expect("record must land");
        let inventory = list_crashes(&profile).expect("inventory must read");
        assert_eq!(inventory.record_count, 1);
        assert_eq!(inventory.records[0].recovery, "good");
        remove_temp_root(&profile);
    }

    #[test]
    fn oversized_logs_compact_keeping_the_newest_record() {
        let profile = unique_temp_root("compact");
        let dir = ensure_crash_dir(&profile).expect("crash root must resolve");
        // One line per record at ~160 bytes: 3000 lines overshoot the cap.
        let filler = "x".repeat(140);
        let mut bulk = String::new();
        for index in 0..3000 {
            bulk.push_str(&format!(
                "{{\"schema\":1,\"timestamp_ms\":{index},\"role\":\"main\",\"kind\":\"RenderProcessExited\",\"reason\":\"Crashed\",\"exit_code\":1,\"recovery\":\"{filler}\"}}\n"
            ));
        }
        std::fs::write(log_path(&dir), bulk).expect("bulk log must be writable");
        assert!(std::fs::metadata(log_path(&dir)).unwrap().len() > MAX_LOG_BYTES);
        record_process_failure(&profile, &test_event(), "newest").expect("record must land");
        let size = std::fs::metadata(log_path(&dir)).unwrap().len();
        assert!(
            size <= MAX_LOG_BYTES + 1024,
            "compacted log must fit the cap, got {size}"
        );
        let inventory = list_crashes(&profile).expect("inventory must read");
        assert_eq!(inventory.records[0].recovery, "newest");
        assert!(
            inventory.records.len() <= MAX_CRASH_RECORDS,
            "inventory stays bounded"
        );
        remove_temp_root(&profile);
    }

    #[test]
    fn dump_inventory_reports_metadata_only_and_never_follows() {
        let profile = unique_temp_root("dumps");
        let dir = ensure_crash_dir(&profile).expect("crash root must resolve");
        std::fs::write(dir.join("a1b2c3d4.dmp"), vec![0u8; 17]).expect("dump must be writable");
        std::fs::write(dir.join("notes.txt"), b"not a dump").expect("text must be writable");
        std::fs::create_dir_all(dir.join("nested.dmp")).expect("dir must be creatable");
        let inventory = list_crashes(&profile).expect("inventory must read");
        assert_eq!(
            inventory.dump_files,
            vec![CrashDumpEntry {
                name: "a1b2c3d4.dmp".to_owned(),
                bytes: 17,
            }]
        );
        remove_temp_root(&profile);
    }

    #[test]
    fn failures_never_carry_host_paths() {
        let profile = unique_temp_root("paths");
        // A blocked root is the path-carrying risk; assert the fixed label.
        std::fs::create_dir_all(&profile).expect("profile must be creatable");
        std::fs::write(profile.join(CRASH_DIR_NAME), b"x").expect("blocker must be writable");
        let failure = ensure_crash_dir(&profile).expect_err("blocked root must fail");
        let profile_text = profile.to_string_lossy();
        assert!(
            !failure.message.contains(profile_text.as_ref()),
            "failure messages must never contain host paths"
        );
        remove_temp_root(&profile);
    }

    #[test]
    fn shipped_module_has_no_uploader_network_or_recovery_surface() {
        let source = include_str!("crash_reporting.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // No transmission, no crashpad, no renderer/process control, no
        // navigation or script execution, no page-data reads.
        for forbidden in [
            "upload",
            "http",
            "ureq",
            "fetch(",
            "crashpad",
            "process.crash",
            "relaunch",
            "Reload(",
            "Navigate(",
            "ExecuteScript",
            "postMessage",
            "ProcessDescription",
            "FrameInfos",
            "Cookie",
            "remcord://",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
    }
}

//! Native theme file-import picker for Phase D (Step 3c).
//!
//! This module is the native counterpart of Electron's `openImportPicker`
//! (`src/discord/ipc.ts:162`): a host-owned file dialog whose selections feed
//! the SAME shared installer as [`crate::themes::install_parsed_theme`].
//! There is exactly ONE theme installation implementation underneath both
//! `themes.install_code` and `themes.import_files`; no parser, manifest,
//! validation, bound, or atomic-write logic is duplicated here.
//!
//! ```text
//! native file picker (IFileOpenDialog, host-owned)
//!   -> bounded local file read (1 MiB, UTF-8, race re-checked)
//!   -> existing native BD parser (parse_bd_manifest)
//!   -> existing shared theme installer (install_parsed_theme)
//!   -> existing native theme watcher (observes the written files)
//!   -> existing managed CSS lifecycle
//! ```
//!
//! ## Exact Electron picker semantics (traced, unchanged)
//!
//! `ipcMain.on("openImportPicker")` shows a dialog with NO `defaultPath`,
//! `title`/`buttonLabel`/`filters[].name` from `getLang`, `properties:
//! ["openFile", "multiSelections"]`, filters `scss`/`css` then `*`, and on
//! success runs `for (const file of result.filePaths) { installTheme(file); }`
//! — each file dispatched WITHOUT awaiting the previous one. Cancellation
//! (`result.canceled`) returns silently. Failures log to the console and the
//! loop continues. `installTheme` reads the file unbounded as UTF-8 and may
//! fetch arbitrary remote URLs; neither unbounded reads nor remote fetching
//! is reproduced here.
//!
//! ## Native implementation
//!
//! The dialog uses the Windows Common Item Dialog (`IFileOpenDialog`) — the
//! modern replacement for `GetOpenFileNameW` — configured to preserve the
//! Electron semantics: no default path, translated title, translated `Import`
//! action-button label (`SetOkButtonLabel`), open-file semantics,
//! multi-selection (`FOS_ALLOWMULTISELECT` + `GetResults`), the SCSS/CSS
//! filter first and the all-files filter second. The dialog runs entirely
//! host-side on a dedicated STA worker thread (COM initialized exactly once
//! for that thread; the WebView/UI thread never blocks).
//!
//! ## Local file ingestion policy
//!
//! For every selected file, in order:
//! 1. `symlink_metadata` must succeed and describe a non-link regular file.
//!    Symlinks, junctions, and any other reparse point are REFUSED outright
//!    (`LINK_REJECTED`) and never followed — stricter than Electron's
//!    `readFileSync`, which follows a planted link and installs outside bytes.
//!    Directories, devices, and missing files are `NOT_A_FILE`/`UNREADABLE`.
//! 2. The pre-read size must fit the 1 MiB bound before a large buffer exists.
//! 3. The read is length-capped (`take(MAX + 1)`) so a file that grows between
//!    the check and the read cannot force an unbounded allocation; the
//!    authoritative post-read length is re-checked and races fail closed as
//!    `TOO_LARGE`.
//! 4. Bytes must be valid UTF-8 (`INVALID_UTF8`), matching Electron's `utf8`
//!    read.
//! 5. The text feeds the shared installer, so the theme id still comes
//!    exclusively from BetterDiscord `@name` through `validate_theme_id`.
//!    The source filename NEVER becomes the theme id.
//!
//! Residual TOCTOU note: the link check and the open are two syscalls, so a
//! party able to rewrite the user's own picked files mid-import could in
//! principle swap in a link between them. That actor already controls the
//! picked bytes, so nothing is gained; links planted in the PROFILE (the
//! threat this migration hardens against) are refused at both layers, and the
//! install/write paths additionally enforce canonical containment.
//!
//! ## Multi-file and cancellation semantics
//!
//! Files are processed SEQUENTIALLY on the worker thread in dialog selection
//! order (deterministic, bounded memory) instead of Electron's un-awaited
//! dispatch. Continue-after-failure is preserved: one bad file never blocks
//! its siblings. Cancellation (user dismisses the dialog, or an empty result)
//! is normal control flow and resolves `{ canceled: true }` — never a bridge
//! rejection, never an error toast, never a scary console error.
//!
//! ## Capability contract
//!
//! The renderer sends exactly `{}`: no path, filename, URL, directory,
//! filters, or dialog options. The host defines the entire picker
//! configuration. The bounded summary response `{ canceled, imported,
//! failed }` carries counts only — no theme ids (a future list refresh owns
//! that), no paths, no raw OS error text.

use std::path::{Path, PathBuf};

use crate::themes::{install_parsed_theme, parse_bd_manifest, MAX_THEME_CSS_BYTES};

/// Translated dialog strings, resolved per request from the embedded locale
/// (the same catalogs `localization.get_lang` serves). The renderer supplies
/// none of this.
pub const DIALOG_TITLE_KEY: &str = "dialog-importTheme-title";
/// Locale key for the dialog action-button label (`SetOkButtonLabel`).
pub const DIALOG_BUTTON_KEY: &str = "dialog-importTheme-button";
/// Locale key for the SCSS/CSS filter display name.
pub const DIALOG_STYLES_FILTER_KEY: &str = "dialog-importTheme-discordStyles";
/// Locale key for the all-files filter display name.
pub const DIALOG_ALL_FILTER_KEY: &str = "dialog-importTheme-allFiles";

/// Fallbacks when a catalog lacks a picker key (the embedded `en-US` catalog
/// carries all four, so these only fire for a catalog that removed them).
/// Values are byte-identical to the `en-US` source strings.
pub const FALLBACK_DIALOG_TITLE: &str = "Select a theme you want to import";
/// Fallback action-button label (byte-identical to `en-US`).
pub const FALLBACK_DIALOG_BUTTON: &str = "Import";
/// Fallback SCSS/CSS filter name (byte-identical to `en-US`).
pub const FALLBACK_STYLES_FILTER: &str = "Discord styles";
/// Fallback all-files filter name (byte-identical to `en-US`).
pub const FALLBACK_ALL_FILTER: &str = "All Files";

/// File-type spec for the first (styles) filter: the Electron `scss`/`css`
/// extensions as one Common Item Dialog spec.
pub const STYLES_FILTER_SPEC: &str = "*.scss;*.css";
/// File-type spec for the second (all-files) filter: the Electron `*`
/// extension as one Common Item Dialog spec.
pub const ALL_FILTER_SPEC: &str = "*.*";

/// Host-owned picker configuration: translated strings only, no paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDialogConfig {
    /// Window title (`SetTitle`).
    pub title: String,
    /// Action-button label (`SetOkButtonLabel`).
    pub ok_label: String,
    /// Display name of the SCSS/CSS filter (first).
    pub styles_filter_name: String,
    /// Display name of the all-files filter (second).
    pub all_filter_name: String,
}

fn localized_or(profile_dir: &Path, key: &str, fallback: &str) -> String {
    let found = crate::localization::lookup(profile_dir, key);
    if found.value.is_empty() {
        fallback.to_owned()
    } else {
        found.value
    }
}

/// Resolve the host-owned picker configuration. The renderer cannot influence
/// any of these values; the selection file (`<profile>/storage/lang.json`)
/// is host state.
pub fn import_dialog_config(profile_dir: &Path) -> ImportDialogConfig {
    ImportDialogConfig {
        title: localized_or(profile_dir, DIALOG_TITLE_KEY, FALLBACK_DIALOG_TITLE),
        ok_label: localized_or(profile_dir, DIALOG_BUTTON_KEY, FALLBACK_DIALOG_BUTTON),
        styles_filter_name: localized_or(
            profile_dir,
            DIALOG_STYLES_FILTER_KEY,
            FALLBACK_STYLES_FILTER,
        ),
        all_filter_name: localized_or(profile_dir, DIALOG_ALL_FILTER_KEY, FALLBACK_ALL_FILTER),
    }
}

/// Typed failure for the import capability: a stable machine-readable `code`
/// plus a human message that never contains a filesystem path or OS error
/// text.
#[derive(Debug)]
pub struct ThemeImportFailure {
    /// Machine-readable code (`IMPORT_FAILED`, `IMPORT_UNAVAILABLE`, ...).
    pub code: &'static str,
    /// Human message with no paths and no OS error text.
    pub message: String,
}

/// Outcome of the host-owned file picker: cancelled/empty, a selection in
/// dialog order, or a host failure that never leaks paths.
#[derive(Debug)]
pub enum PickerOutcome {
    /// The user dismissed the dialog, or it returned no items. Normal
    /// control flow: resolves `{ canceled: true }`, never rejects.
    Canceled,
    /// Host-selected local files in dialog order. Never empty (an empty
    /// result maps to `Canceled`).
    Selected(Vec<PathBuf>),
    /// The picker itself failed (COM/shell error, unsupported platform).
    /// No path and no OS error text cross this boundary.
    Failed(ThemeImportFailure),
}

/// Per-file outcome of the sequential import, in selection order. Host
/// logging only: ids stay host-side, paths never leave the host.
#[derive(Debug, PartialEq, Eq)]
pub enum FileImportOutcome {
    /// The file installed; carries the host-derived theme id for logging.
    Imported(String),
    /// The file was skipped; carries the typed failure code (never a path).
    Failed(&'static str),
}

/// Bounded import summary: cancellation plus counts only. No arrays, no ids,
/// no paths — the smallest response that distinguishes cancel, success, and
/// failure for a future UI refresh (which re-lists through its own surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeImportSummary {
    /// True when the user dismissed the picker (or it returned nothing).
    /// `imported` and `failed` are then both zero.
    pub canceled: bool,
    /// Files that installed through the shared installer.
    pub imported: u32,
    /// Files skipped with a typed code (host log carries the codes).
    pub failed: u32,
}

/// Read one host-picked file as bounded UTF-8 theme source.
///
/// Enforces the ingestion policy from the module docs: link refusal before
/// open, pre-read bound, length-capped read with an authoritative post-read
/// re-check (filesystem races fail closed), UTF-8 validation. Every failure
/// is a typed code; no path and no OS error text is ever returned.
pub fn read_theme_source(path: &Path) -> Result<String, &'static str> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "NOT_A_FILE")?;
    if metadata.file_type().is_symlink() {
        return Err("LINK_REJECTED");
    }
    if !metadata.file_type().is_file() {
        return Err("NOT_A_FILE");
    }
    if metadata.len() > MAX_THEME_CSS_BYTES as u64 {
        return Err("TOO_LARGE");
    }
    let file = std::fs::File::open(path).map_err(|_| "UNREADABLE")?;
    let mut buffer = Vec::new();
    {
        use std::io::Read;
        file.take(MAX_THEME_CSS_BYTES as u64 + 1)
            .read_to_end(&mut buffer)
            .map_err(|_| "UNREADABLE")?;
    }
    if buffer.len() > MAX_THEME_CSS_BYTES {
        return Err("TOO_LARGE");
    }
    String::from_utf8(buffer).map_err(|_| "INVALID_UTF8")
}

/// Import host-picked files through the shared installer, sequentially in
/// selection order with continue-after-failure.
///
/// An empty selection is cancellation (`canceled: true`), matching a
/// dismissed dialog. Returns the bounded summary plus the ordered per-file
/// outcomes for host logging (codes only, never paths).
pub fn import_from_paths(
    profile_dir: &Path,
    paths: &[PathBuf],
) -> (ThemeImportSummary, Vec<FileImportOutcome>) {
    if paths.is_empty() {
        return (
            ThemeImportSummary {
                canceled: true,
                imported: 0,
                failed: 0,
            },
            Vec::new(),
        );
    }
    let mut outcomes = Vec::with_capacity(paths.len());
    let mut imported: u32 = 0;
    let mut failed: u32 = 0;
    for path in paths {
        let outcome = match read_theme_source(path) {
            Err(code) => FileImportOutcome::Failed(code),
            Ok(css) => match parse_bd_manifest(&css) {
                Err(failure) => FileImportOutcome::Failed(failure.code),
                Ok(parsed) => match install_parsed_theme(profile_dir, &css, parsed) {
                    Ok(installed) => FileImportOutcome::Imported(installed.id),
                    Err(failure) => FileImportOutcome::Failed(failure.code),
                },
            },
        };
        match &outcome {
            FileImportOutcome::Imported(_) => imported = imported.saturating_add(1),
            FileImportOutcome::Failed(_) => failed = failed.saturating_add(1),
        }
        outcomes.push(outcome);
    }
    (
        ThemeImportSummary {
            canceled: false,
            imported,
            failed,
        },
        outcomes,
    )
}

/// Show the host-owned theme import picker and return its outcome.
///
/// Windows Common Item Dialog (`IFileOpenDialog`) preserving the Electron
/// configuration: no default path, translated title, translated `Import`
/// action label, open-file semantics, multi-selection, the SCSS/CSS filter
/// first and the all-files filter second. MUST run on an STA thread with COM
/// already initialized (the dedicated import worker owns that); the dialog is
/// entirely host-side and modal to the worker, never to the WebView thread.
#[cfg(target_os = "windows")]
pub fn show_import_picker(profile_dir: &Path) -> PickerOutcome {
    use windows::Win32::{
        System::Com::{CoCreateInstance, CLSCTX_ALL},
        UI::Shell::{FileOpenDialog, IFileOpenDialog},
    };

    let failed = |code: &'static str, message: &str| {
        PickerOutcome::Failed(ThemeImportFailure {
            code,
            message: message.to_owned(),
        })
    };
    let config = import_dialog_config(profile_dir);
    let dialog: IFileOpenDialog =
        match unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL) } {
            Ok(dialog) => dialog,
            Err(_) => {
                return failed("IMPORT_FAILED", "theme import picker cannot be shown");
            }
        };
    // Every COM interface call below is unsafe by windows-crate contract;
    // the STA worker guarantees the threading model (STA + COM initialized
    // exactly once for its thread) that makes them sound to invoke.
    unsafe { show_import_picker_inner(&dialog, &config) }
}

/// Inner picker flow: every COM call. Split so the caller documents the
/// safety contract once instead of scattering `unsafe` per call.
#[cfg(target_os = "windows")]
unsafe fn show_import_picker_inner(
    dialog: &windows::Win32::UI::Shell::IFileOpenDialog,
    config: &ImportDialogConfig,
) -> PickerOutcome {
    use windows::{
        core::{HSTRING, PCWSTR},
        Win32::UI::Shell::{
            Common::COMDLG_FILTERSPEC, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST,
            FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH,
        },
    };

    let failed = |code: &'static str, message: &str| {
        PickerOutcome::Failed(ThemeImportFailure {
            code,
            message: message.to_owned(),
        })
    };
    if dialog
        .SetTitle(&HSTRING::from(config.title.as_str()))
        .is_err()
    {
        return failed("IMPORT_FAILED", "theme import picker cannot be shown");
    }
    if dialog
        .SetOkButtonLabel(&HSTRING::from(config.ok_label.as_str()))
        .is_err()
    {
        return failed("IMPORT_FAILED", "theme import picker cannot be shown");
    }
    // Filter names are translated; specs mirror the Electron extensions.
    // HSTRING buffers must outlive the borrowed PCWSTRs in the specs.
    let styles_name = HSTRING::from(config.styles_filter_name.as_str());
    let styles_spec = HSTRING::from(STYLES_FILTER_SPEC);
    let all_name = HSTRING::from(config.all_filter_name.as_str());
    let all_spec = HSTRING::from(ALL_FILTER_SPEC);
    let specs = [
        COMDLG_FILTERSPEC {
            pszName: PCWSTR(styles_name.as_ptr()),
            pszSpec: PCWSTR(styles_spec.as_ptr()),
        },
        COMDLG_FILTERSPEC {
            pszName: PCWSTR(all_name.as_ptr()),
            pszSpec: PCWSTR(all_spec.as_ptr()),
        },
    ];
    if dialog.SetFileTypes(&specs).is_err() {
        return failed("IMPORT_FAILED", "theme import picker cannot be shown");
    }
    // Open-file semantics plus multi-selection, layered onto the dialog
    // defaults (never replacing them): the Electron `openFile` +
    // `multiSelections` properties.
    let options = match dialog.GetOptions() {
        Ok(options) => options,
        Err(_) => {
            return failed("IMPORT_FAILED", "theme import picker cannot be shown");
        }
    };
    if dialog
        .SetOptions(options | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST)
        .is_err()
    {
        return failed("IMPORT_FAILED", "theme import picker cannot be shown");
    }
    // No owner window: the picker is host-side and worker-modal, so the
    // WebView/UI thread stays responsive while it is open.
    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) => {
            // ERROR_CANCELLED (1223): the user dismissed the dialog. Normal
            // control flow, never an error.
            if error.code() == windows::core::HRESULT::from_win32(1223) {
                return PickerOutcome::Canceled;
            }
            return failed("IMPORT_FAILED", "theme import picker cannot be shown");
        }
    }
    let results = match dialog.GetResults() {
        Ok(results) => results,
        Err(_) => {
            return failed("IMPORT_FAILED", "theme import picker cannot be shown");
        }
    };
    let count = match results.GetCount() {
        Ok(count) => count,
        Err(_) => {
            return failed("IMPORT_FAILED", "theme import picker cannot be shown");
        }
    };
    let mut selected = Vec::new();
    for index in 0..count {
        let item = match results.GetItemAt(index) {
            Ok(item) => item,
            Err(_) => {
                return failed("IMPORT_FAILED", "theme import picker cannot be shown");
            }
        };
        let display = match item.GetDisplayName(SIGDN_FILESYSPATH) {
            Ok(display) => display,
            Err(_) => {
                return failed("IMPORT_FAILED", "theme import picker cannot be shown");
            }
        };
        let text = unsafe { display.to_string().unwrap_or_default() };
        unsafe {
            windows::Win32::System::Com::CoTaskMemFree(Some(display.0 as *const std::ffi::c_void));
        }
        if text.is_empty() {
            return failed("IMPORT_FAILED", "theme import picker cannot be shown");
        }
        selected.push(PathBuf::from(text));
    }
    if selected.is_empty() {
        // Defensive: a file-must-exist multi-select dialog should never
        // confirm with zero items; treat it as cancellation, not success.
        PickerOutcome::Canceled
    } else {
        PickerOutcome::Selected(selected)
    }
}

/// Non-Windows builds fail closed: the picker is a Windows WebView2 surface
/// (project rule), never a guessed shell command.
#[cfg(not(target_os = "windows"))]
pub fn show_import_picker(_profile_dir: &Path) -> PickerOutcome {
    PickerOutcome::Failed(ThemeImportFailure {
        code: "IMPORT_UNSUPPORTED_PLATFORM",
        message: "theme import is not supported on this platform".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        import_dialog_config, import_from_paths, read_theme_source, FileImportOutcome,
        ALL_FILTER_SPEC, FALLBACK_ALL_FILTER, FALLBACK_DIALOG_BUTTON, FALLBACK_DIALOG_TITLE,
        FALLBACK_STYLES_FILTER, STYLES_FILTER_SPEC,
    };
    use crate::themes::{install_theme_from_code, INSTALLED_THEME_CSS_FILE, MAX_THEME_CSS_BYTES};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Serializes the gated live-dialog tests: every picker carries the same
    /// translated title, so two concurrent dialogs would be indistinguishable
    /// to the window lookup.
    static UI_DIALOG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn temp_profile() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must advance")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "remcord-native-theme-import-test-{}-{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(dir.join("themes")).expect("must create temp theme root");
        dir
    }

    fn write_source(dir: &std::path::Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, bytes).expect("must write source fixture");
        path
    }

    const VALID_A: &str = "/**\n * @name Import Alpha\n * @version 1\n */\n:root { --a: 1; }\n";
    const VALID_B: &str = "/**\n * @name Import Beta\n */\n:root { --b: 2; }\n";

    #[test]
    fn dialog_config_pins_translated_electron_strings_and_filter_order() {
        // No lang.json: the embedded en-US catalog applies (Electron parity
        // for the default selection).
        let profile = temp_profile();
        let config = import_dialog_config(&profile);
        assert_eq!(config.title, FALLBACK_DIALOG_TITLE);
        assert_eq!(config.title, "Select a theme you want to import");
        assert_eq!(config.ok_label, FALLBACK_DIALOG_BUTTON);
        assert_eq!(config.ok_label, "Import");
        assert_eq!(config.styles_filter_name, FALLBACK_STYLES_FILTER);
        assert_eq!(config.styles_filter_name, "Discord styles");
        assert_eq!(config.all_filter_name, FALLBACK_ALL_FILTER);
        assert_eq!(config.all_filter_name, "All Files");
        // Specs mirror the Electron extensions: scss/css first, * second.
        assert_eq!(STYLES_FILTER_SPEC, "*.scss;*.css");
        assert_eq!(ALL_FILTER_SPEC, "*.*");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn empty_selection_is_cancellation_not_success() {
        let profile = temp_profile();
        let (summary, outcomes) = import_from_paths(&profile, &[]);
        assert!(summary.canceled);
        assert_eq!(summary.imported, 0);
        assert_eq!(summary.failed, 0);
        assert!(outcomes.is_empty());
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn reads_valid_utf8_and_rejects_invalid_utf8() {
        let dir = temp_profile();
        let good = write_source(&dir, "good.css", VALID_A.as_bytes());
        assert_eq!(read_theme_source(&good).as_deref(), Ok(VALID_A));
        let bad = write_source(
            &dir,
            "bad.css",
            b"/**\n * @name X\n */\n:root { --x: \xff; }\n",
        );
        assert_eq!(read_theme_source(&bad).unwrap_err(), "INVALID_UTF8");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_directories_and_missing_files_without_paths() {
        let dir = temp_profile();
        let sub = dir.join("sub");
        fs::create_dir(&sub).expect("must create subdir");
        assert_eq!(read_theme_source(&sub).unwrap_err(), "NOT_A_FILE");
        let missing = dir.join("gone.css");
        assert_eq!(read_theme_source(&missing).unwrap_err(), "NOT_A_FILE");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_oversized_files_at_the_exact_boundary() {
        let dir = temp_profile();
        // Exactly at the bound: the size gate passes (the content then fails
        // as a manifest, proving the gate passed rather than the size).
        let mut at = vec![b'x'; MAX_THEME_CSS_BYTES];
        at[..4].copy_from_slice(b"/**\n");
        let at_path = write_source(&dir, "at.css", &at);
        assert!(read_theme_source(&at_path).is_ok());
        // One byte over: rejected before any large parse work.
        let over = vec![b'x'; MAX_THEME_CSS_BYTES + 1];
        let over_path = write_source(&dir, "over.css", &over);
        assert_eq!(read_theme_source(&over_path).unwrap_err(), "TOO_LARGE");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_unreadable_locked_files() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = temp_profile();
        let path = write_source(&dir, "locked.css", VALID_A.as_bytes());
        // Exclusive share mode: the import open fails deterministically.
        let _guard = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("must lock fixture");
        assert_eq!(read_theme_source(&path).unwrap_err(), "UNREADABLE");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refuses_symlinks_without_following_them() {
        let dir = temp_profile();
        let outside = temp_profile();
        let target = write_source(&outside, "secret.css", VALID_A.as_bytes());
        let link = dir.join("link.css");
        if std::os::windows::fs::symlink_file(&target, &link).is_err() {
            eprintln!("SKIP: link creation needs privileges in this environment; lexical safety still holds");
            fs::remove_dir_all(&dir).ok();
            fs::remove_dir_all(&outside).ok();
            return;
        }
        assert_eq!(read_theme_source(&link).unwrap_err(), "LINK_REJECTED");
        // Policy document: a refused link installs nothing, and the link
        // target is never read (its theme id stays absent).
        let profile = temp_profile();
        let (summary, outcomes) = import_from_paths(&profile, std::slice::from_ref(&link));
        assert!(!summary.canceled);
        assert_eq!((summary.imported, summary.failed), (0, 1));
        assert_eq!(outcomes, vec![FileImportOutcome::Failed("LINK_REJECTED")]);
        assert!(fs::read_dir(profile.join("themes"))
            .expect("root must list")
            .next()
            .is_none());
        fs::remove_dir_all(&dir).ok();
        fs::remove_dir_all(&outside).ok();
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn installs_valid_files_with_shared_installer_semantics() {
        let profile = temp_profile();
        let sources = temp_profile();
        let a = write_source(&sources, "whatever-name.css", VALID_A.as_bytes());
        let (summary, outcomes) = import_from_paths(&profile, std::slice::from_ref(&a));
        assert!(!summary.canceled);
        assert_eq!((summary.imported, summary.failed), (1, 0));
        // The source filename NEVER becomes the id: @name derives it.
        assert_eq!(
            outcomes,
            vec![FileImportOutcome::Imported("Import-Alpha-BD".to_owned())]
        );
        let dir = profile.join("themes").join("Import-Alpha-BD");
        assert_eq!(
            fs::read_to_string(dir.join(INSTALLED_THEME_CSS_FILE)).expect("css must read"),
            VALID_A
        );
        // Byte-identical to the install_code path for the same input.
        let via_code = temp_profile();
        install_theme_from_code(&via_code, VALID_A).expect("install_code must succeed");
        let manifest_import: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.join("manifest.json")).expect("manifest must read"),
        )
        .expect("manifest must be JSON");
        let manifest_code: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(
                via_code
                    .join("themes")
                    .join("Import-Alpha-BD")
                    .join("manifest.json"),
            )
            .expect("manifest must read"),
        )
        .expect("manifest must be JSON");
        assert_eq!(manifest_import, manifest_code);
        fs::remove_dir_all(&profile).ok();
        fs::remove_dir_all(&sources).ok();
        fs::remove_dir_all(&via_code).ok();
    }

    #[test]
    fn rejects_non_manifest_and_blank_names_with_typed_codes() {
        let profile = temp_profile();
        let sources = temp_profile();
        let cases: &[(&str, &str)] = &[
            ("plain.css", ":root { --x: 1; }\n"),
            ("noname.css", "/**\n * @author Jane\n */\n:root {}\n"),
            ("empty.css", "/**\n * @name \n */\n:root {}\n"),
            ("blank.css", "/**\n * @name    \n */\n:root {}\n"),
            ("punct.css", "/**\n * @name (!!!)\n */\n:root {}\n"),
        ];
        let expected = [
            "NOT_A_MANIFEST",
            "INVALID_NAME",
            "INVALID_NAME",
            "INVALID_NAME",
            "INVALID_NAME",
        ];
        let mut paths = Vec::new();
        for (name, content) in cases {
            paths.push(write_source(&sources, name, content.as_bytes()));
        }
        let (summary, outcomes) = import_from_paths(&profile, &paths);
        assert!(!summary.canceled);
        assert_eq!((summary.imported, summary.failed), (0, 5));
        for (outcome, code) in outcomes.iter().zip(expected.iter()) {
            assert_eq!(outcome, &FileImportOutcome::Failed(code));
        }
        // Nothing was created for any rejection.
        assert!(fs::read_dir(profile.join("themes"))
            .expect("root must list")
            .next()
            .is_none());
        fs::remove_dir_all(&profile).ok();
        fs::remove_dir_all(&sources).ok();
    }

    #[test]
    fn imports_multi_word_names_with_sanitized_ids() {
        let profile = temp_profile();
        let sources = temp_profile();
        // The common real-world case that previously failed every import
        // with INVALID_NAME: multi-word @name values now sanitize.
        let spaced = write_source(
            &sources,
            "spaces.css",
            "/**\n * @name My Big Theme\n * @author Jane\n */\n:root {}\n".as_bytes(),
        );
        let (summary, outcomes) = import_from_paths(&profile, std::slice::from_ref(&spaced));
        assert!(!summary.canceled);
        assert_eq!((summary.imported, summary.failed), (1, 0));
        assert_eq!(
            outcomes,
            vec![FileImportOutcome::Imported("My-Big-Theme-BD".to_owned())]
        );
        assert!(profile
            .join("themes")
            .join("My-Big-Theme-BD")
            .join("manifest.json")
            .is_file());
        fs::remove_dir_all(&profile).ok();
        fs::remove_dir_all(&sources).ok();
    }

    #[test]
    fn multi_file_mixes_continue_after_failure_in_order() {
        let profile = temp_profile();
        let sources = temp_profile();
        let valid_a = write_source(&sources, "a.css", VALID_A.as_bytes());
        let valid_b = write_source(&sources, "b.css", VALID_B.as_bytes());
        let invalid = write_source(&sources, "bad.css", b":root {}\n");
        let invalid2 = write_source(&sources, "bad2.css", b"junk\n");

        // valid + valid
        let (both, _) = import_from_paths(&profile, &[valid_a.clone(), valid_b.clone()]);
        assert_eq!((both.canceled, both.imported, both.failed), (false, 2, 0));

        // valid + invalid and invalid + valid: same counts, order preserved.
        let (mixed, mixed_outcomes) =
            import_from_paths(&profile, &[valid_a.clone(), invalid.clone()]);
        assert_eq!((mixed.imported, mixed.failed), (1, 1));
        assert!(matches!(mixed_outcomes[0], FileImportOutcome::Imported(_)));
        assert_eq!(
            mixed_outcomes[1],
            FileImportOutcome::Failed("NOT_A_MANIFEST")
        );
        let (mixed2, mixed2_outcomes) =
            import_from_paths(&profile, &[invalid.clone(), valid_b.clone()]);
        assert_eq!((mixed2.imported, mixed2.failed), (1, 1));
        assert_eq!(
            mixed2_outcomes[0],
            FileImportOutcome::Failed("NOT_A_MANIFEST")
        );
        assert!(matches!(mixed2_outcomes[1], FileImportOutcome::Imported(_)));

        // multiple invalid: every failure is reported, nothing installed new.
        let (multi_bad, _) = import_from_paths(&profile, &[invalid.clone(), invalid2.clone()]);
        assert_eq!((multi_bad.imported, multi_bad.failed), (0, 2));

        // Deterministic: the same input always yields the same summary.
        let again = import_from_paths(&profile, &[valid_a.clone(), invalid.clone()]).0;
        assert_eq!(mixed, again);
        fs::remove_dir_all(&profile).ok();
        fs::remove_dir_all(&sources).ok();
    }

    #[test]
    fn repeated_import_overwrites_deterministically_in_selection_order() {
        let profile = temp_profile();
        let sources = temp_profile();
        let first = "/**\n * @name Steady\n * @version 1\n */\n:root { --a: 1; }\n";
        let second = "/**\n * @name Steady\n * @version 2\n */\n:root { --a: 2; }\n";
        let a = write_source(&sources, "v1.css", first.as_bytes());
        let b = write_source(&sources, "v2.css", second.as_bytes());
        // Selection order decides the winner: last write wins, matching
        // Electron's sequential (if un-awaited) installer calls.
        let (summary, _) = import_from_paths(&profile, &[a, b]);
        assert_eq!((summary.imported, summary.failed), (2, 0));
        assert_eq!(
            fs::read_to_string(
                profile
                    .join("themes")
                    .join("Steady-BD")
                    .join(INSTALLED_THEME_CSS_FILE)
            )
            .expect("css must read"),
            second
        );
        fs::remove_dir_all(&profile).ok();
        fs::remove_dir_all(&sources).ok();
    }

    #[test]
    fn failures_carry_codes_only_never_paths() {
        let profile = temp_profile();
        let sources = temp_profile();
        let bad = write_source(&sources, "bad.css", b":root {}\n");
        let (_, outcomes) = import_from_paths(&profile, std::slice::from_ref(&bad));
        let FileImportOutcome::Failed(code) = &outcomes[0] else {
            panic!("bad file must fail");
        };
        assert!(!code.contains(&sources.to_string_lossy().into_owned()));
        assert!(!code.contains("bad.css"));
        assert!(!code.contains('\\'));
        assert!(!code.contains('/'));
        fs::remove_dir_all(&profile).ok();
        fs::remove_dir_all(&sources).ok();
    }

    /// Live Windows dialog validation (gated: opens a real modal picker).
    ///
    /// Run explicitly with `REMCORD_NATIVE_PICKER_UI_TEST=1`; ordinary
    /// `cargo test` skips it so no dialog ever appears in unattended runs.
    /// The picker runs on its own STA thread exactly like the production
    /// worker (COM initialized once for that thread); this thread polls for
    /// the dialog window by its translated title, asserts the translated
    /// `Import` action button exists (proving `SetOkButtonLabel` applied —
    /// the default would read `Open`), then dismisses with `WM_CLOSE` and
    /// requires a silent `Canceled` outcome. Dialog appearance itself proves
    /// `SetTitle`/`SetFileTypes`/`SetOptions` all succeeded (any failure
    /// returns before `Show`).
    #[test]
    fn live_dialog_shows_translated_chrome_and_cancels_silently() {
        if std::env::var("REMCORD_NATIVE_PICKER_UI_TEST").as_deref() != Ok("1") {
            eprintln!(
                "SKIP: live dialog test needs REMCORD_NATIVE_PICKER_UI_TEST=1 (opens a real modal picker)"
            );
            return;
        }
        #[cfg(target_os = "windows")]
        {
            use super::{import_dialog_config, show_import_picker, PickerOutcome};
            use std::sync::mpsc;
            use windows::{
                core::{HSTRING, PCWSTR},
                Win32::{
                    Foundation::{HWND, LPARAM, WPARAM},
                    System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
                    UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, PostMessageW, WM_CLOSE},
                },
            };

            let _dialog_guard = UI_DIALOG_LOCK.lock().expect("ui dialog lock must hold");
            let profile = temp_profile();
            let config = import_dialog_config(&profile);
            let (done_tx, done_rx) = mpsc::channel();
            std::thread::Builder::new()
                .name("remcord-picker-ui-test-sta".to_owned())
                .spawn(move || {
                    let com_init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
                    let com_owned = com_init == windows::core::HRESULT(0);
                    let outcome = show_import_picker(&profile);
                    let _ = done_tx.send(outcome);
                    std::fs::remove_dir_all(&profile).ok();
                    if com_owned {
                        unsafe {
                            CoUninitialize();
                        }
                    }
                })
                .expect("picker test thread must start");

            // The dialog must appear with the translated title. Appearance
            // proves every configuration call before Show succeeded.
            let title = HSTRING::from(config.title.as_str());
            let mut dialog: Option<HWND> = None;
            for _ in 0..100 {
                if let Ok(found) = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) } {
                    dialog = Some(found);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let dialog = dialog.expect("the import picker dialog must appear");
            // The action button must carry the translated Import label
            // rather than the stock Open label.
            let button_class = HSTRING::from("Button");
            let ok_label = HSTRING::from(config.ok_label.as_str());
            let button = unsafe {
                FindWindowExW(
                    Some(dialog),
                    None,
                    PCWSTR(button_class.as_ptr()),
                    PCWSTR(ok_label.as_ptr()),
                )
            }
            .expect("the Import action button must exist");
            assert!(!button.is_invalid());
            // Dismiss: cancellation is silent control flow, never an error.
            unsafe {
                PostMessageW(Some(dialog), WM_CLOSE, WPARAM(0), LPARAM(0))
                    .expect("the dialog must accept WM_CLOSE");
            }
            match done_rx.recv_timeout(std::time::Duration::from_secs(30)) {
                Ok(PickerOutcome::Canceled) => {}
                other => panic!("dismissed picker must cancel silently, got {other:?}"),
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            eprintln!("SKIP: live dialog test is Windows-only");
        }
    }

    /// Live end-to-end selection through the real dialog (gated like above).
    ///
    /// Types one valid and one invalid theme path into the dialog's filename
    /// field and activates the translated `Import` button programmatically
    /// (`BM_CLICK`) — the same `Show` → `GetResults` path a human click
    /// takes. Requires the selection in order, then installs through the
    /// shared installer and asserts exact on-disk semantics (id from `@name`,
    /// Electron-shaped manifest, deterministic overwrite). Single-select
    /// behavior is the same code path with one item.
    #[test]
    fn live_dialog_selects_files_and_installs_through_the_shared_installer() {
        if std::env::var("REMCORD_NATIVE_PICKER_UI_TEST").as_deref() != Ok("1") {
            eprintln!(
                "SKIP: live dialog test needs REMCORD_NATIVE_PICKER_UI_TEST=1 (opens a real modal picker)"
            );
            return;
        }
        #[cfg(target_os = "windows")]
        {
            use super::{
                import_dialog_config, import_from_paths, show_import_picker, PickerOutcome,
            };
            use std::sync::mpsc;
            use windows::{
                core::{HSTRING, PCWSTR},
                Win32::{
                    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
                    System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
                    UI::WindowsAndMessaging::{
                        FindWindowExW, FindWindowW, SendMessageW, BM_CLICK, WM_SETTEXT,
                    },
                },
            };

            const VALID: &str =
                "/**\n * @name Live Pick\n * @version 1\n */\n:root { --live: 1; }\n";
            let _dialog_guard = UI_DIALOG_LOCK.lock().expect("ui dialog lock must hold");
            let profile = temp_profile();
            let sources = temp_profile();
            let valid_path = sources.join("live-valid.css");
            let bad_path = sources.join("live-bad.css");
            std::fs::write(&valid_path, VALID).expect("valid fixture must write");
            std::fs::write(&bad_path, b":root { --no: marker; }\n")
                .expect("bad fixture must write");
            let config = import_dialog_config(&profile);
            let (done_tx, done_rx) = mpsc::channel();
            let worker_profile = profile.clone();
            std::thread::Builder::new()
                .name("remcord-picker-select-test-sta".to_owned())
                .spawn(move || {
                    let com_init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
                    let com_owned = com_init == windows::core::HRESULT(0);
                    let outcome = show_import_picker(&worker_profile);
                    let _ = done_tx.send(outcome);
                    if com_owned {
                        unsafe {
                            CoUninitialize();
                        }
                    }
                })
                .expect("picker test thread must start");

            let title = HSTRING::from(config.title.as_str());
            let mut dialog: Option<HWND> = None;
            for _ in 0..100 {
                if let Ok(found) = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) } {
                    dialog = Some(found);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let dialog = dialog.expect("the import picker dialog must appear");

            // The filename field is the ComboBox nested in the dialog's
            // ComboBoxEx32 (the bare ComboBox children are the file-type
            // filter). Text goes to the nested box; no focus games needed.
            let comboex_class = HSTRING::from("ComboBoxEx32");
            let combo_class = HSTRING::from("ComboBox");
            let filename_combo = (|| {
                let comboex = unsafe {
                    FindWindowExW(
                        Some(dialog),
                        None,
                        PCWSTR(comboex_class.as_ptr()),
                        PCWSTR::null(),
                    )
                }
                .ok()?;
                let nested = unsafe {
                    FindWindowExW(
                        Some(comboex),
                        None,
                        PCWSTR(combo_class.as_ptr()),
                        PCWSTR::null(),
                    )
                }
                .ok()?;
                // The text lives in the Edit grandchild: setting the nested
                // ComboBox itself neither sticks nor reads back.
                let edit_class = HSTRING::from("Edit");
                unsafe {
                    FindWindowExW(
                        Some(nested),
                        None,
                        PCWSTR(edit_class.as_ptr()),
                        PCWSTR::null(),
                    )
                }
                .ok()
            })()
            .expect("the dialog filename field must exist");
            // Quoted absolute paths: the same multi-select the dialog
            // returns for a human Ctrl+click.
            let typed = HSTRING::from(
                format!("\"{}\" \"{}\"", valid_path.display(), bad_path.display()).as_str(),
            );
            unsafe {
                SendMessageW(
                    filename_combo,
                    WM_SETTEXT,
                    Some(WPARAM(0)),
                    Some(LPARAM(typed.as_ptr() as isize)),
                );
            }
            let mut echo = [0u16; 1024];
            let echo_read = unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(filename_combo, &mut echo)
            };
            let echoed = String::from_utf16_lossy(&echo[..echo_read.max(0) as usize]);
            // The typed selection must land verbatim before Import is
            // activated; otherwise the click would confirm a stale field.
            assert!(
                echoed.contains("live-valid.css") && echoed.contains("live-bad.css"),
                "the filename field must echo the typed selection"
            );
            let button_class = HSTRING::from("Button");
            let ok_label = HSTRING::from(config.ok_label.as_str());
            let button = unsafe {
                FindWindowExW(
                    Some(dialog),
                    None,
                    PCWSTR(button_class.as_ptr()),
                    PCWSTR(ok_label.as_ptr()),
                )
            }
            .expect("the Import action button must exist");
            let _result: LRESULT =
                unsafe { SendMessageW(button, BM_CLICK, Some(WPARAM(0)), Some(LPARAM(0))) };

            let outcome = done_rx
                .recv_timeout(std::time::Duration::from_secs(30))
                .expect("the picker must complete after Import");
            let PickerOutcome::Selected(paths) = outcome else {
                panic!("expected a two-file selection, got {outcome:?}");
            };
            assert_eq!(paths.len(), 2);
            assert!(paths.iter().any(|path| path == &valid_path));
            assert!(paths.iter().any(|path| path == &bad_path));

            // The selection installs through the single shared installer:
            // one valid theme from @name, one typed skip, exact on-disk
            // Electron-shaped layout.
            let (summary, _) = import_from_paths(&profile, &paths);
            assert!(!summary.canceled);
            assert_eq!((summary.imported, summary.failed), (1, 1));
            let installed = profile.join("themes").join("Live-Pick-BD");
            assert_eq!(
                std::fs::read_to_string(installed.join("src.css")).expect("css must read"),
                VALID
            );
            let manifest: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(installed.join("manifest.json"))
                    .expect("manifest must read"),
            )
            .expect("manifest must be JSON");
            assert_eq!(manifest["name"], "Live Pick");
            assert_eq!(manifest["theme"], "src.css");
            assert_eq!(manifest["enabled"], false);
            std::fs::remove_dir_all(&profile).ok();
            std::fs::remove_dir_all(&sources).ok();
        }
        #[cfg(not(target_os = "windows"))]
        {
            eprintln!("SKIP: live dialog test is Windows-only");
        }
    }
}

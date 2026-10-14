//! System CSS editor handoff (`useSystemCssEditor` parity with Electron
//! `shell.openPath()`).
//!
//! When the setting is on, QuickCSS edit actions open `<profile>/quickCss.css`
//! in the OS-associated editor instead of the in-Discord modal. The file is
//! host-resolved (never renderer-supplied); the live CSS watchers pick up
//! external saves exactly like in-app saves. A missing file is created empty
//! first so the editor always has something to open.

/// Typed failure for the system-editor handoff. Fixed vocabulary only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssEditorFailure {
    /// The QuickCSS file could not be ensured on disk.
    EnsureFailed,
    /// The OS editor could not be launched.
    OpenFailed,
}

impl CssEditorFailure {
    pub fn code(self) -> &'static str {
        match self {
            CssEditorFailure::EnsureFailed => "CSS_EDITOR_ENSURE_FAILED",
            CssEditorFailure::OpenFailed => "CSS_EDITOR_OPEN_FAILED",
        }
    }
}

/// Open the host-owned QuickCSS file in the OS-associated editor
/// (Electron `shell.openPath()` parity). Ensures the file exists first;
/// returns the fixed-vocabulary failure otherwise.
pub fn open_system_editor(profile_dir: &std::path::Path) -> Result<(), CssEditorFailure> {
    let path = crate::quick_css::quick_css_path(profile_dir);
    if !path.exists() {
        std::fs::write(&path, "").map_err(|_| CssEditorFailure::EnsureFailed)?;
    }
    open_host_path(&path)
}

/// Open one host-resolved path with the OS-associated handler. The path is
/// constructed by the host (never renderer input); dispatch goes straight
/// to the shell with no command line.
#[cfg(target_os = "windows")]
fn open_host_path(path: &std::path::Path) -> Result<(), CssEditorFailure> {
    use windows::{
        core::{HSTRING, PCWSTR},
        Win32::{
            Foundation::HWND,
            UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        },
    };
    let Some(path_str) = path.to_str() else {
        return Err(CssEditorFailure::OpenFailed);
    };
    let operation = HSTRING::from("open");
    let file = HSTRING::from(path_str);
    let instance = unsafe {
        ShellExecuteW(
            Some(HWND::default()),
            &operation,
            &file,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (instance.0 as isize) > 32 {
        Ok(())
    } else {
        Err(CssEditorFailure::OpenFailed)
    }
}

#[cfg(not(target_os = "windows"))]
fn open_host_path(_path: &std::path::Path) -> Result<(), CssEditorFailure> {
    Err(CssEditorFailure::OpenFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_codes_are_fixed_vocabulary() {
        assert_eq!(CssEditorFailure::EnsureFailed.code(), "CSS_EDITOR_ENSURE_FAILED");
        assert_eq!(CssEditorFailure::OpenFailed.code(), "CSS_EDITOR_OPEN_FAILED");
    }

    #[test]
    fn missing_file_is_created_then_opened_or_fails_typed() {
        // Touches only a unique temp dir; the open itself may fail on
        // headless CI (no associated editor) but must fail typed, and the
        // file must exist afterwards either way.
        let dir = std::env::temp_dir().join(format!(
            "remcord-css-editor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time must be available")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir must create");
        let _ = open_system_editor(&dir);
        assert!(crate::quick_css::quick_css_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

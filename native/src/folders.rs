//! Typed folder-opening capability for Phase D (Step 2).
//!
//! This module is the native counterpart of the Electron folder-opening
//! handlers in `src/discord/ipc.ts`, exposed to the renderer through
//! `src/discord/preload/bridge.ts`. Only the four caller-backed behaviors
//! are migrated; everything else stays Electron-only:
//!
//! | Renderer caller | IPC channel | Electron host call | Native mapping |
//! | --- | --- | --- | --- |
//! | `settings.openStorageFolder` (`SettingsPage.tsx`) | `openStorageFolder` | `shell.showItemInFolder(<userData>/storage/)` | reveal `storage` root |
//! | `settings.openThemesFolder` (`ThemesPage.tsx`) | `openThemesFolder` (registered twice, both fire identically) | `shell.showItemInFolder(<userData>/themes/)` | reveal `themes` root |
//! | `plugins.openFolder` (`PluginsPage.tsx`) | `openRuntimePluginsFolder` | `shell.showItemInFolder(<userData>/plugins/)` | reveal `plugins` root |
//! | `themes.folder(id)` (`ThemesCard.tsx`) | `openThemeFolder` | `shell.openPath(path.join(themesPath, id))` | open `themes/<validated id>/` |
//!
//! `openPluginsFolder` (`<userData>/extensions/`) and `openCrashesFolder`
//! (temp crashes dir) have NO renderer caller and no bridge entry, so they
//! are deferred. File-in-editor paths (`openQuickCss`, `editTheme` via
//! `openCssEditor`), picker dialogs (`openImportPicker`,
//! `openCustomIconDialog`), and browser navigation (`openExternal`) are NOT
//! folder opening and stay deferred with their own milestones.
//!
//! Semantic distinction (preserved, not collapsed):
//! - The three fixed roots use **reveal** semantics, the equivalent of
//!   Electron `showItemInFolder()`: Explorer opens at the parent with the
//!   folder selected. Implemented with `SHOpenFolderAndSelectItems`, a direct
//!   shell API — never a constructed command line.
//! - A theme directory uses **open** semantics, the equivalent of Electron
//!   `openPath()` on that directory: Explorer opens inside the folder itself.
//!   Implemented with `ShellExecuteW("open", …)`, a direct shell API.
//!
//! Both may result in an Explorer window, but they are different operations
//! and are implemented (and logged) as such.
//!
//! Native hardening (deliberate, documented differences from Electron):
//! - The three roots are a closed enum, not paths: the renderer can only
//!   name `storage`, `themes`, or `plugins`. There is no generic open(path).
//! - The theme id is validated with the SAME rule themes already enforce
//!   (`themes::validate_theme_id`: `[A-Za-z0-9._-]{1,64}`, never `.`/`..`,
//!   separators impossible by charset). Electron joins the raw id with
//!   `path.join`, so `../` escapes and an absolute id wins there.
//! - The final path is always built and canonicalized by the host, and must
//!   stay under the canonical approved root. A planted symlink/junction that
//!   resolves outside fails closed (`OUTSIDE_ROOT`) instead of opening an
//!   outside location — Electron would follow it.
//! - Fixed roots are created when missing (mirroring Electron's "Created
//!   missing theme folder"); a missing theme directory is a typed
//!   `UNKNOWN_THEME`, never created, never opened.
//! - All handlers are fire-and-forget in Electron (`send`, return ignored,
//!   failures silent). Natively every outcome is a typed success/error so a
//!   silent no-op can never pass as an opened folder.
//! - Error messages carry only fixed labels, never host paths.
//!
//! Logical roots owned by this module: `<profile>/storage/`,
//! `<profile>/themes/`, `<profile>/plugins/`, and single validated theme
//! directories under `<profile>/themes/`. No other root, no file picking, no
//! dialogs, no command execution.

use std::path::{Path, PathBuf};

/// The three host-controlled roots a renderer may ask to reveal. A closed
/// enum: unknown locations are rejected during deserialization, before any
/// filesystem or shell work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderLocation {
    Storage,
    Themes,
    Plugins,
}

impl FolderLocation {
    /// Fixed directory name under the profile root. The renderer never
    /// supplies this string; it only selects the variant.
    pub fn dir_name(self) -> &'static str {
        match self {
            FolderLocation::Storage => "storage",
            FolderLocation::Themes => "themes",
            FolderLocation::Plugins => "plugins",
        }
    }
}

/// Maximum theme id length in UTF-8 bytes, mirroring `themes.rs`.
pub const MAX_FOLDER_THEME_ID_BYTES: usize = 64;

/// Typed failure for the capability: a stable machine-readable `code` plus a
/// human message that never contains a filesystem path.
#[derive(Debug)]
pub struct FolderFailure {
    pub code: &'static str,
    pub message: String,
}

fn failure(code: &'static str, message: &str) -> FolderFailure {
    FolderFailure {
        code,
        message: message.to_owned(),
    }
}

/// Host-built root for a fixed location. Single-component join onto the
/// profile directory; cannot escape lexically.
pub fn folder_root(profile_dir: &Path, location: FolderLocation) -> PathBuf {
    profile_dir.join(location.dir_name())
}

fn contains(canonical_root: &Path, canonical_candidate: &Path) -> bool {
    canonical_candidate.starts_with(canonical_root)
}

/// Ensure a fixed root exists and resolve it to its canonical path.
/// Creates the directory when missing (host-owned well-known roots only),
/// then requires canonical containment under the canonical profile root so
/// a pre-planted junction at the root can never redirect the reveal.
pub fn ensure_folder_root(
    profile_dir: &Path,
    location: FolderLocation,
) -> Result<PathBuf, FolderFailure> {
    let root = folder_root(profile_dir, location);
    std::fs::create_dir_all(&root)
        .map_err(|_| failure("FOLDERS_UNAVAILABLE", "folder root is unavailable"))?;
    let canonical_root = std::fs::canonicalize(profile_dir)
        .map_err(|_| failure("FOLDERS_UNAVAILABLE", "folder root is unavailable"))?;
    let canonical = std::fs::canonicalize(&root)
        .map_err(|_| failure("FOLDERS_UNAVAILABLE", "folder root is unavailable"))?;
    if !contains(&canonical_root, &canonical) {
        return Err(failure(
            "OUTSIDE_ROOT",
            "folder target is outside the approved root",
        ));
    }
    Ok(canonical)
}

/// Resolve a single validated theme directory to its canonical path.
/// Reuses the exact theme-id rule (`themes::validate_theme_id`); a missing
/// directory is `UNKNOWN_THEME` (never created); a link resolving outside
/// the canonical themes root is `OUTSIDE_ROOT` (never followed outward).
pub fn resolve_theme_folder(profile_dir: &Path, theme_id: &str) -> Result<PathBuf, FolderFailure> {
    if theme_id.is_empty() || theme_id.len() > MAX_FOLDER_THEME_ID_BYTES {
        return Err(failure("INVALID_THEME_ID", "theme id is invalid"));
    }
    if crate::themes::validate_theme_id(theme_id).is_err() {
        return Err(failure("INVALID_THEME_ID", "theme id is invalid"));
    }
    let themes_root = folder_root(profile_dir, FolderLocation::Themes);
    std::fs::create_dir_all(&themes_root)
        .map_err(|_| failure("FOLDERS_UNAVAILABLE", "theme folder is unavailable"))?;
    let canonical_root = std::fs::canonicalize(&themes_root)
        .map_err(|_| failure("FOLDERS_UNAVAILABLE", "theme folder is unavailable"))?;
    let candidate = themes_root.join(theme_id);
    let canonical = match std::fs::canonicalize(&candidate) {
        Ok(resolved) => resolved,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(failure("UNKNOWN_THEME", "theme folder is unknown"));
        }
        Err(_) => {
            return Err(failure(
                "FOLDERS_UNAVAILABLE",
                "theme folder is unavailable",
            ));
        }
    };
    if !contains(&canonical_root, &canonical) {
        return Err(failure(
            "OUTSIDE_ROOT",
            "folder target is outside the approved root",
        ));
    }
    Ok(canonical)
}

/// Display string for shell APIs. Containment is always verified on the
/// canonical path by the caller; the shell itself receives the plain
/// Win32 form because verb handlers such as `SHParseDisplayName` reject
/// the canonical `\\?\` prefix with `E_INVALIDARG`.
fn shell_display_string(path: &Path, failure_code: &'static str) -> Result<String, FolderFailure> {
    let raw = path
        .as_os_str()
        .to_str()
        .ok_or_else(|| match failure_code {
            "REVEAL_FAILED" => failure("REVEAL_FAILED", "folder reveal failed"),
            _ => failure("OPEN_FAILED", "folder open failed"),
        })?;
    let display = raw
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or_else(|| raw.strip_prefix(r"\\?\").unwrap_or(raw).to_owned());
    Ok(display)
}

/// Reveal semantics (Electron `showItemInFolder()` equivalent): open the
/// parent in Explorer with this folder selected. `path` must already be a
/// host-resolved canonical directory.
#[cfg(target_os = "windows")]
pub fn reveal_folder(path: &Path) -> Result<(), FolderFailure> {
    use windows::{
        core::HSTRING,
        Win32::{
            System::Com::{
                CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{Common::ITEMIDLIST, SHOpenFolderAndSelectItems, SHParseDisplayName},
        },
    };

    let path_str = shell_display_string(path, "REVEAL_FAILED")?;
    let parent_display = path
        .parent()
        .map(|parent| shell_display_string(parent, "REVEAL_FAILED"))
        .transpose()?
        .ok_or_else(|| failure("REVEAL_FAILED", "folder reveal failed"))?;
    let parent_str = parent_display.as_str();
    // The shell PIDL APIs require COM. The WebView2 host thread is normally
    // STA-initialized already (S_FALSE); only a conflicting mode fails here,
    // and a silent fallback would change reveal semantics, so fail closed.
    let com_init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if com_init.is_err() {
        return Err(failure("REVEAL_FAILED", "folder reveal failed"));
    }
    let com_owned = com_init == windows::core::HRESULT(0);
    // `HSTRING`s must outlive the raw pointers derived from them.
    let parent_name = HSTRING::from(parent_str);
    let child_name = HSTRING::from(path_str.as_str());
    let mut parent_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    let mut child_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    let result = (|| {
        let parent_hr =
            unsafe { SHParseDisplayName(&parent_name, None, &mut parent_pidl, 0, None) };
        #[cfg(debug_assertions)]
        if parent_hr.is_err() {
            eprintln!("[Folders][Debug] SHParseDisplayName(parent) hr={parent_hr:?}");
        }
        if parent_hr.is_err() || parent_pidl.is_null() {
            return Err(failure("REVEAL_FAILED", "folder reveal failed"));
        }
        let child_hr = unsafe { SHParseDisplayName(&child_name, None, &mut child_pidl, 0, None) };
        #[cfg(debug_assertions)]
        if child_hr.is_err() {
            eprintln!("[Folders][Debug] SHParseDisplayName(child) hr={child_hr:?}");
        }
        if child_hr.is_err() || child_pidl.is_null() {
            return Err(failure("REVEAL_FAILED", "folder reveal failed"));
        }
        // Reveal (not open): the parent opens with this exact folder
        // selected. `apidl` carries absolute PIDLs of the items to select.
        let selected = [child_pidl as *const ITEMIDLIST];
        let reveal_hr = unsafe {
            SHOpenFolderAndSelectItems(parent_pidl as *const ITEMIDLIST, Some(&selected), 0)
        };
        #[cfg(debug_assertions)]
        if reveal_hr.is_err() {
            eprintln!("[Folders][Debug] SHOpenFolderAndSelectItems hr={reveal_hr:?}");
        }
        if reveal_hr.is_err() {
            return Err(failure("REVEAL_FAILED", "folder reveal failed"));
        }
        Ok(())
    })();
    unsafe {
        if !parent_pidl.is_null() {
            CoTaskMemFree(Some(parent_pidl as *const std::ffi::c_void));
        }
        if !child_pidl.is_null() {
            CoTaskMemFree(Some(child_pidl as *const std::ffi::c_void));
        }
        if com_owned {
            CoUninitialize();
        }
    }
    result
}

/// Open semantics (Electron `openPath()` equivalent for a directory): open
/// Explorer inside this folder. `path` must already be host-resolved. Direct
/// shell API; no command line is ever constructed.
#[cfg(target_os = "windows")]
pub fn open_folder(path: &Path) -> Result<(), FolderFailure> {
    use windows::core::PCWSTR;
    use windows::{
        core::HSTRING,
        Win32::{
            Foundation::HWND, UI::Shell::ShellExecuteW, UI::WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };

    let path_str = shell_display_string(path, "OPEN_FAILED")?;
    let operation = HSTRING::from("open");
    let file = HSTRING::from(path_str.as_str());
    // `ShellExecuteW` dispatches the verb and returns immediately; it does
    // not block the calling thread on the opened application.
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
        Err(failure("OPEN_FAILED", "folder open failed"))
    }
}

/// Folder opening targets Windows WebView2 first (project rule); other
/// platforms fail closed with a typed error instead of guessing a shell
/// command. No `xdg-open`/`open` abstraction is introduced.
#[cfg(not(target_os = "windows"))]
pub fn reveal_folder(_path: &Path) -> Result<(), FolderFailure> {
    Err(failure(
        "FOLDERS_UNSUPPORTED_PLATFORM",
        "folder opening is not supported on this platform",
    ))
}

/// See [`reveal_folder`]: Windows-only shell surface, typed failure elsewhere.
#[cfg(not(target_os = "windows"))]
pub fn open_folder(_path: &Path) -> Result<(), FolderFailure> {
    Err(failure(
        "FOLDERS_UNSUPPORTED_PLATFORM",
        "folder opening is not supported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn unique_temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "remcord-folders-test-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&root).expect("test temp root must be creatable");
        root
    }

    fn remove_temp_root(root: &Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fixed_roots_use_exact_host_controlled_directory_names() {
        assert_eq!(FolderLocation::Storage.dir_name(), "storage");
        assert_eq!(FolderLocation::Themes.dir_name(), "themes");
        assert_eq!(FolderLocation::Plugins.dir_name(), "plugins");
    }

    #[test]
    fn ensure_creates_missing_roots_under_the_profile() {
        let profile = unique_temp_root("roots");
        for location in [
            FolderLocation::Storage,
            FolderLocation::Themes,
            FolderLocation::Plugins,
        ] {
            let resolved = ensure_folder_root(&profile, location).expect("fixed root must resolve");
            assert!(resolved.is_dir(), "{location:?} root must exist afterwards");
            assert!(
                resolved.starts_with(
                    std::fs::canonicalize(&profile).expect("profile must canonicalize")
                ),
                "{location:?} root must stay under the profile"
            );
        }
        remove_temp_root(&profile);
    }

    #[test]
    fn theme_ids_share_the_exact_themes_rule() {
        let profile = unique_temp_root("ids");
        std::fs::create_dir_all(profile.join("themes").join("my-theme_1.2"))
            .expect("theme fixture must be creatable");
        let resolved =
            resolve_theme_folder(&profile, "my-theme_1.2").expect("valid id must resolve");
        assert!(resolved.is_dir());
        for bad in [
            "",
            ".",
            "..",
            "../escape",
            "..\\escape",
            "a/b",
            "a\\b",
            "/absolute",
            "C:\\themes",
            "\\\\server\\share",
            "has space",
            "semi;colon",
            "C:",
        ] {
            assert!(
                resolve_theme_folder(&profile, bad).is_err(),
                "{bad:?} must never resolve to a path"
            );
        }
        let oversized = "t".repeat(MAX_FOLDER_THEME_ID_BYTES + 1);
        assert!(resolve_theme_folder(&profile, &oversized).is_err());
        remove_temp_root(&profile);
    }

    #[test]
    fn missing_theme_directories_are_unknown_never_created() {
        let profile = unique_temp_root("missing");
        let failure =
            resolve_theme_folder(&profile, "no-such-theme").expect_err("missing dir must fail");
        assert_eq!(failure.code, "UNKNOWN_THEME");
        assert!(
            !profile.join("themes").join("no-such-theme").exists(),
            "resolution must never create a theme directory"
        );
        remove_temp_root(&profile);
    }

    #[test]
    fn failures_never_leak_host_paths() {
        let profile = unique_temp_root("leak");
        let profile_text = profile.to_string_lossy().into_owned();
        let missing = resolve_theme_folder(&profile, "no-such-theme").expect_err("must fail");
        assert!(!missing.message.contains(&profile_text));
        let traversal = resolve_theme_folder(&profile, "../escape").expect_err("must fail");
        assert!(!traversal.message.contains(&profile_text));
        assert!(!traversal.message.contains("escape"));
        remove_temp_root(&profile);
    }

    #[test]
    fn planted_links_resolving_outside_fail_closed() {
        let profile = unique_temp_root("link");
        let outside = unique_temp_root("outside");
        std::fs::create_dir_all(profile.join("themes")).expect("themes root must be creatable");
        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_dir(
            outside.as_path(),
            profile.join("themes").join("linked"),
        );
        #[cfg(not(windows))]
        let link_result =
            std::os::unix::fs::symlink(outside.as_path(), profile.join("themes").join("linked"));
        if link_result.is_err() {
            eprintln!("SKIP: link creation needs privileges in this environment; lexical safety still holds");
            remove_temp_root(&profile);
            remove_temp_root(&outside);
            return;
        }
        let failure =
            resolve_theme_folder(&profile, "linked").expect_err("outside-resolving link must fail");
        assert_eq!(failure.code, "OUTSIDE_ROOT");
        remove_temp_root(&profile);
        remove_temp_root(&outside);
    }
}

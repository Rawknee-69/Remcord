//! Native window chrome selection (Electron window-style parity).
//!
//! `windowStyle` / `transparency` / `windowMaterial` / `customIcon` are
//! build-time choices: tao bakes decorations and transparency into the
//! window at creation, so the snapshot is resolved once at startup
//! (restart-required model, like `channel`). The DWM backdrop applies
//! after the window exists (it only needs the `HWND`).
//!
//! Mapping (Electron `window.ts` / `config.ts` parity, Windows slice):
//! - `windowStyle: "native"` → OS decorations; anything else keeps the
//!   frameless overlay shell (the custom titlebar plugin draws the
//!   caption). `"transparent"` additionally forces a transparent window.
//! - `transparency: "universal" | "modern"` → transparent window (the
//!   WebView layer must go transparent too — see the builder flag); only
//!   `"none"` is opaque.
//! - `windowMaterial: "mica" | "tabbed" | "acrylic"` → the matching
//!   `DWMWA_SYSTEMBACKDROP_TYPE` (best-effort: older Windows rejects the
//!   attribute and the window stays standard); `"none"` clears it.
//! - `customIcon`: a host-validated `.ico`/`.png` file decoded to RGBA for
//!   the window icon; empty/invalid falls back to the embedded rebranded
//!   mark so the window never ships iconless.

/// Resolved build-time chrome for one startup.
pub struct WindowChrome {
    /// OS decorations (`windowStyle: "native"` only).
    pub decorations: bool,
    /// Transparent window (`transparency != "none"` or style
    /// `"transparent"`).
    pub transparent: bool,
    /// Window icon pixels (RGBA, top-down) or `None` for the tao default.
    pub icon_rgba: Option<(Vec<u8>, u32, u32)>,
    /// DWM backdrop kind requested (`windowMaterial`, `"none"` clears).
    pub material: WindowMaterial,
}

/// DWM system-backdrop types (`DWMWA_SYSTEMBACKDROP_TYPE` values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMaterial {
    None,
    Mica,
    Tabbed,
    Acrylic,
}

impl WindowMaterial {
    /// Parse the `windowMaterial` setting; unknown values fail safe to the
    /// compile-time default (`"mica"` per `settings.rs`).
    pub fn parse(raw: &str) -> WindowMaterial {
        match raw {
            "none" => WindowMaterial::None,
            "mica" => WindowMaterial::Mica,
            "tabbed" => WindowMaterial::Tabbed,
            "acrylic" => WindowMaterial::Acrylic,
            _ => WindowMaterial::Mica,
        }
    }

    /// Raw `DWMSBT_*` value for `DwmSetWindowAttribute`. `None` maps to
    /// `DWMSBT_NONE` (1), which clears any backdrop.
    pub fn dwm_value(self) -> i32 {
        match self {
            WindowMaterial::None => 1,
            WindowMaterial::Mica => 2,
            WindowMaterial::Acrylic => 3,
            WindowMaterial::Tabbed => 4,
        }
    }
}

/// Resolve build-time chrome from the authoritative snapshot values. Pure:
/// no filesystem beyond the optional custom-icon read, no window work.
pub fn resolve_window_chrome(
    window_style: &str,
    transparency: &str,
    window_material: &str,
    custom_icon: &str,
) -> WindowChrome {
    let decorations = window_style == "native";
    let transparent =
        window_style == "transparent" || (transparency == "universal" || transparency == "modern");
    let icon_rgba = resolve_window_icon(custom_icon);
    WindowChrome {
        decorations,
        transparent,
        icon_rgba,
        material: WindowMaterial::parse(window_material),
    }
}

/// Resolve the window icon pixels: the validated custom file when set,
/// else the embedded rebranded mark. `None` only when both are undecodable
/// (the window then keeps the tao default — never a crash).
fn resolve_window_icon(custom_icon: &str) -> Option<(Vec<u8>, u32, u32)> {
    let trimmed = custom_icon.trim();
    if !trimmed.is_empty() {
        if let Some(rgba) = load_icon_file(trimmed) {
            return Some(rgba);
        }
        eprintln!("[WindowChrome] custom icon unreadable, falling back to the embedded mark");
    }
    decode_icon_rgba(crate::app_icon::embedded_png_bytes())
}

/// Load and validate a user-selected icon file (`.ico`/`.png` only,
/// bounded by the shared icon size rail). Paths come from the host file
/// dialog behind `window.set_custom_icon` or from the settings document;
/// arbitrary extensions, oversize files, and undecodable bytes are refused.
fn load_icon_file(path: &str) -> Option<(Vec<u8>, u32, u32)> {
    let lower = path.to_ascii_lowercase();
    if !(lower.ends_with(".ico") || lower.ends_with(".png")) {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() > crate::app_icon::MAX_ICON_FILE_BYTES {
        return None;
    }
    // ICO containers wrap PNG/BMP entries WIC cannot always open directly;
    // accept embedded PNG bytes straight through, else decode normally.
    decode_icon_rgba(&bytes)
}

/// Decode image bytes to top-down RGBA via the shared WIC path. ICO files
/// whose first image is PNG-compressed decode directly; BMP-encoded ICO
/// entries are refused (no silent wrong-icon).
fn decode_icon_rgba(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let (width, height, bgra) = crate::clipboard::decode_image_bgra(bytes).ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    let mut rgba = bgra;
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Some((rgba, width, height))
}

/// Build a tao window icon from a user-selected file, falling back to the
/// embedded mark. Shared by startup chrome selection and the live
/// `window.set_custom_icon` path.
pub fn window_icon_from_file(path: &str) -> Option<tao::window::Icon> {
    let (rgba, width, height) = load_icon_file(path)
        .or_else(|| decode_icon_rgba(crate::app_icon::embedded_png_bytes()))?;
    tao::window::Icon::from_rgba(rgba, width, height).ok()
}

/// Validate a picked icon path for persistence: `.ico`/`.png` extension,
/// bounded size, and decodable bytes. Returns the path unchanged on
/// success, so the caller persists exactly what the dialog returned.
pub fn validate_icon_path(path: &str) -> Option<String> {
    load_icon_file(path).map(|_| path.to_owned())
}

/// Host single-file picker for the custom window icon (`.ico`/`.png`
/// only). STA-worker only. Returns `Ok(None)` on user dismissal.
#[cfg(target_os = "windows")]
pub fn show_custom_icon_picker() -> Result<Option<String>, String> {
    use windows::{
        core::HSTRING,
        Win32::{
            System::Com::{CoCreateInstance, CLSCTX_ALL},
            UI::Shell::{
                Common::COMDLG_FILTERSPEC, FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST,
                FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH,
            },
        },
    };
    let dialog: IFileOpenDialog = unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL) }
        .map_err(|_| "icon picker cannot be shown".to_owned())?;
    unsafe {
        dialog
            .SetTitle(&HSTRING::from("Choose window icon"))
            .map_err(|_| "icon picker cannot be shown".to_owned())?;
        dialog
            .SetOkButtonLabel(&HSTRING::from("Use icon"))
            .map_err(|_| "icon picker cannot be shown".to_owned())?;
        let icons_name = HSTRING::from("Icons");
        let icons_spec = HSTRING::from("*.ico;*.png");
        let specs = [COMDLG_FILTERSPEC {
            pszName: windows::core::PCWSTR(icons_name.as_ptr()),
            pszSpec: windows::core::PCWSTR(icons_spec.as_ptr()),
        }];
        dialog
            .SetFileTypes(&specs)
            .map_err(|_| "icon picker cannot be shown".to_owned())?;
        let options = dialog
            .GetOptions()
            .map_err(|_| "icon picker cannot be shown".to_owned())?;
        dialog
            .SetOptions(options | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST)
            .map_err(|_| "icon picker cannot be shown".to_owned())?;
        match dialog.Show(None) {
            Ok(()) => {}
            Err(error) => {
                if error.code() == windows::core::HRESULT::from_win32(1223) {
                    return Ok(None);
                }
                return Err("icon picker cannot be shown".to_owned());
            }
        }
        let result = dialog
            .GetResult()
            .map_err(|_| "icon picker cannot be shown".to_owned())?;
        let display = result
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|_| "icon picker cannot be shown".to_owned())?;
        let text = display.to_string().unwrap_or_default();
        windows::Win32::System::Com::CoTaskMemFree(Some(
            display.0 as *const std::ffi::c_void,
        ));
        if text.is_empty() {
            return Err("icon picker cannot be shown".to_owned());
        }
        Ok(Some(text))
    }
}

#[cfg(not(target_os = "windows"))]
pub fn show_custom_icon_picker() -> Result<Option<String>, String> {
    Err("custom icon is not supported on this platform".to_owned())
}

/// Apply the DWM system backdrop to a live window (best-effort: older
/// Windows rejects the attribute and the window stays standard). Logs
/// once; never fails boot.
#[cfg(target_os = "windows")]
pub fn apply_backdrop_material(hwnd_value: isize, material: WindowMaterial) {
    use windows::Win32::{
        Foundation::HWND,
        Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_SYSTEMBACKDROP_TYPE},
    };
    let value = material.dwm_value();
    let result = unsafe {
        DwmSetWindowAttribute(
            HWND(hwnd_value as _),
            DWMWA_SYSTEMBACKDROP_TYPE,
            &value as *const i32 as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        )
    };
    match result {
        Ok(()) => println!("[WindowChrome] backdrop material applied material={material:?}"),
        Err(_) => eprintln!("[WindowChrome] backdrop material unsupported on this system"),
    }
}

#[cfg(not(target_os = "windows"))]
pub fn apply_backdrop_material(_hwnd_value: isize, _material: WindowMaterial) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_parses_the_closed_vocabulary() {
        assert_eq!(WindowMaterial::parse("mica"), WindowMaterial::Mica);
        assert_eq!(WindowMaterial::parse("tabbed"), WindowMaterial::Tabbed);
        assert_eq!(WindowMaterial::parse("acrylic"), WindowMaterial::Acrylic);
        assert_eq!(WindowMaterial::parse("none"), WindowMaterial::None);
        assert_eq!(WindowMaterial::parse(""), WindowMaterial::Mica);
        assert_eq!(WindowMaterial::parse("glass"), WindowMaterial::Mica);
    }

    #[test]
    fn material_dwm_values_match_dwmsbt() {
        assert_eq!(WindowMaterial::None.dwm_value(), 1);
        assert_eq!(WindowMaterial::Mica.dwm_value(), 2);
        assert_eq!(WindowMaterial::Acrylic.dwm_value(), 3);
        assert_eq!(WindowMaterial::Tabbed.dwm_value(), 4);
    }

    #[test]
    fn chrome_selection_follows_style_and_transparency() {
        let native = resolve_window_chrome("native", "none", "mica", "");
        assert!(native.decorations);
        assert!(!native.transparent);
        let overlay = resolve_window_chrome("overlay", "none", "mica", "");
        assert!(!overlay.decorations);
        assert!(!overlay.transparent);
        let modern = resolve_window_chrome("overlay", "modern", "mica", "");
        assert!(modern.transparent);
        let transparent_style = resolve_window_chrome("transparent", "none", "mica", "");
        assert!(transparent_style.transparent);
        assert!(!transparent_style.decorations);
    }

    #[test]
    fn embedded_mark_decodes_for_chrome_use() {
        let chrome = resolve_window_chrome("overlay", "none", "mica", "");
        let (rgba, width, height) = chrome.icon_rgba.expect("embedded mark must decode");
        assert!(width > 0 && height > 0);
        assert_eq!(rgba.len(), width as usize * height as usize * 4);
    }

    #[test]
    fn hostile_icon_paths_resolve_to_no_custom_icon() {
        assert!(load_icon_file("").is_none());
        assert!(load_icon_file("C:\\evil\\x.exe").is_none());
        assert!(load_icon_file("C:\\evil\\x.svg").is_none());
        assert!(load_icon_file("C:\\nonexistent-dir-xyz\\x.ico").is_none());
    }

    #[test]
    fn icon_path_validation_accepts_only_decodable_assets() {
        assert!(validate_icon_path("").is_none());
        assert!(validate_icon_path("C:\\evil\\x.exe").is_none());
        assert!(validate_icon_path("C:\\nonexistent\\x.png").is_none());
    }
}

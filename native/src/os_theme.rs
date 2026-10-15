//! OS light/dark appearance for the native shell.
//!
//! The WebView2 content (Discord plus the Gallery settings page) resolves
//! CSS `prefers-color-scheme` from the WebView's `PreferredColorScheme`,
//! which stays light unless the host sets it — so without this module a
//! dark OS still renders a light page, and the Gallery's OS-aware fallbacks
//! (`GalleryPage.module.css`) can never engage. This module is the host
//! half of that contract:
//!
//! - [`current_os_theme`] reads the OS appearance once at startup
//!   (Windows `AppsUseLightTheme`; unknown/unreadable fails safe to
//!   [`OsTheme::Light`] — dark is never the default).
//! - [`apply_window_theme`] paints the native frame (immersive dark mode)
//!   to match, best-effort.
//! - [`apply_webview_theme`] mirrors the theme into the WebView2 profile
//!   (`PreferredColorScheme`), best-effort.
//! - tao forwards live OS switches as `WindowEvent::ThemeChanged`; the
//!   event loop maps them with [`from_is_dark`] and re-applies both, so a
//!   running app follows the OS without a restart.
//! - [`resolve_effective_theme`] folds an explicit override (`"light"` /
//!   `"dark"`, [`theme_override_from_str`]) over the OS value. No persisted
//!   appearance setting exists yet (Discord owns the in-page theme), so
//!   every caller passes `None` today — the parameter pins the call shape
//!   a future settings key can use without touching this module.
//!
//! This module is pure policy plus best-effort host calls: fallible OS
//! reads log once and fail safe, never fail boot, and never carry paths,
//! URLs, or user content into logs.

/// The two appearances the shell can paint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsTheme {
    Light,
    Dark,
}

impl OsTheme {
    /// Fixed vocabulary for logs (`system=light|dark`, never a path).
    pub fn as_str(self) -> &'static str {
        match self {
            OsTheme::Light => "light",
            OsTheme::Dark => "dark",
        }
    }

    pub fn is_dark(self) -> bool {
        matches!(self, OsTheme::Dark)
    }
}

/// Maps a boolean darkness reading (registry value, tao theme event) to
/// an [`OsTheme`]. Shared by every reader so the polarity is defined once:
/// `true` (lights off) is dark.
pub fn from_is_dark(is_dark: bool) -> OsTheme {
    if is_dark {
        OsTheme::Dark
    } else {
        OsTheme::Light
    }
}

/// Parses an explicit appearance override (`"light"` / `"dark"`,
/// case-insensitive, surrounding whitespace ignored). Anything else —
/// including empty — is `None` (follow the OS). Pure, so a future
/// settings key and its tests share one definition. Unconsumed until that
/// key exists (all callers pass `None` today).
#[allow(dead_code)]
pub fn theme_override_from_str(raw: &str) -> Option<OsTheme> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "light" => Some(OsTheme::Light),
        "dark" => Some(OsTheme::Dark),
        _ => None,
    }
}

/// Effective theme: the explicit override wins, otherwise the OS value.
/// Every caller passes `None` until a persisted appearance setting exists.
pub fn resolve_effective_theme(os: OsTheme, override_theme: Option<OsTheme>) -> OsTheme {
    override_theme.unwrap_or(os)
}

/// Reads the current OS appearance. Windows consults the per-user
/// `AppsUseLightTheme` value (`0` = dark); a missing key, an unreadable
/// value, or any registry failure fails safe to light. Non-Windows builds
/// report light (no second detection mechanism).
pub fn current_os_theme() -> OsTheme {
    #[cfg(target_os = "windows")]
    {
        from_is_dark(!read_apps_use_light_theme())
    }
    #[cfg(not(target_os = "windows"))]
    {
        OsTheme::Light
    }
}

/// Splash background for an OS theme. The legacy constant was dark-only
/// (`#202225`), which is unreadable as a light-mode frame — light gets a
/// genuinely light backdrop.
pub fn splash_background_for(theme: OsTheme) -> &'static str {
    match theme {
        OsTheme::Light => SPLASH_BACKGROUND_LIGHT,
        OsTheme::Dark => SPLASH_BACKGROUND_DARK,
    }
}

/// Light-mode splash backdrop (genuinely light).
pub const SPLASH_BACKGROUND_LIGHT: &str = "#f2f3f5";
/// Dark-mode splash backdrop (the legacy `#202225`).
pub const SPLASH_BACKGROUND_DARK: &str = "#202225";

/// Reads `HKCU\...\Personalize\AppsUseLightTheme`: `true` when apps
/// should use light (value nonzero or key missing — the Windows default
/// is light), `false` only on an explicit `0`.
#[cfg(target_os = "windows")]
fn read_apps_use_light_theme() -> bool {
    use windows::{
        core::PCWSTR,
        Win32::System::Registry::{
            RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_READ, REG_VALUE_TYPE,
        },
    };

    fn wide_null(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe {
        let path = wide_null(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let mut key = Default::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            KEY_READ,
            &mut key,
        )
        .is_err()
        {
            return true;
        }
        let name = wide_null("AppsUseLightTheme");
        let mut data: u32 = 1;
        let mut data_len = std::mem::size_of::<u32>() as u32;
        let mut kind = REG_VALUE_TYPE::default();
        let queried = RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some((&mut data as *mut u32).cast()),
            Some(&mut data_len),
        );
        let _ = windows::Win32::System::Registry::RegCloseKey(key);
        if queried.is_err() {
            return true;
        }
        data != 0
    }
}

/// Paints the native window frame for a theme (immersive dark mode on /
/// off). Best-effort: older Windows rejects the attribute and the frame
/// stays standard; never fails boot.
#[cfg(target_os = "windows")]
pub fn apply_window_theme(hwnd_value: isize, theme: OsTheme) {
    use windows::Win32::{
        Foundation::HWND,
        Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE},
    };
    let dark: u32 = if theme.is_dark() { 1 } else { 0 };
    let result = unsafe {
        DwmSetWindowAttribute(
            HWND(hwnd_value as _),
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const u32 as *const std::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };
    match result {
        Ok(()) => println!("[OsTheme] frame applied system={}", theme.as_str()),
        Err(_) => eprintln!("[OsTheme] frame attribute unsupported on this system"),
    }
}

#[cfg(not(target_os = "windows"))]
pub fn apply_window_theme(_hwnd_value: isize, _theme: OsTheme) {}

/// Mirrors a theme into the WebView2 profile's `PreferredColorScheme`, so
/// page `prefers-color-scheme` (Discord, the Gallery fallbacks) tracks the
/// OS. Best-effort: a missing profile interface only loses theme sync and
/// never blocks boot or navigation.
#[cfg(target_os = "windows")]
pub fn apply_webview_theme(
    core: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
    theme: OsTheme,
) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_13, COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK,
        COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT,
    };
    use windows::core::Interface;
    let scheme = if theme.is_dark() {
        COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK
    } else {
        COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT
    };
    let result = (|| {
        let webview13: ICoreWebView2_13 = core.cast()?;
        let profile = unsafe { webview13.Profile() }?;
        unsafe { profile.SetPreferredColorScheme(scheme) }?;
        Ok::<(), windows::core::Error>(())
    })();
    match result {
        Ok(()) => println!("[OsTheme] webview scheme applied system={}", theme.as_str()),
        Err(_) => eprintln!("[OsTheme] webview scheme unavailable on this runtime"),
    }
}

#[cfg(not(target_os = "windows"))]
pub fn apply_webview_theme(_core: (), _theme: OsTheme) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_is_never_the_default() {
        // Unknown/unreadable OS state must fail safe to light, and the
        // override parser must reject everything but the closed vocabulary.
        assert_eq!(from_is_dark(false), OsTheme::Light);
        assert_eq!(theme_override_from_str(""), None);
        assert_eq!(theme_override_from_str("system"), None);
        assert_eq!(theme_override_from_str("midnight"), None);
        assert!(!current_os_theme().as_str().is_empty());
    }

    #[test]
    fn override_grammar_is_closed_and_trims() {
        assert_eq!(theme_override_from_str("light"), Some(OsTheme::Light));
        assert_eq!(theme_override_from_str("DARK"), Some(OsTheme::Dark));
        assert_eq!(theme_override_from_str("  dark  "), Some(OsTheme::Dark));
        assert_eq!(theme_override_from_str("auto"), None);
    }

    #[test]
    fn effective_theme_prefers_the_override() {
        assert_eq!(resolve_effective_theme(OsTheme::Dark, None), OsTheme::Dark);
        assert_eq!(
            resolve_effective_theme(OsTheme::Dark, Some(OsTheme::Light)),
            OsTheme::Light
        );
        assert_eq!(
            resolve_effective_theme(OsTheme::Light, Some(OsTheme::Dark)),
            OsTheme::Dark
        );
    }

    #[test]
    fn splash_backgrounds_are_genuinely_light_and_dark() {
        assert_eq!(splash_background_for(OsTheme::Light), "#f2f3f5");
        assert_eq!(splash_background_for(OsTheme::Dark), "#202225");
        assert_ne!(
            splash_background_for(OsTheme::Light),
            splash_background_for(OsTheme::Dark)
        );
    }

    #[test]
    fn theme_vocabulary_is_fixed() {
        assert_eq!(OsTheme::Light.as_str(), "light");
        assert_eq!(OsTheme::Dark.as_str(), "dark");
        assert!(!OsTheme::Light.is_dark());
        assert!(OsTheme::Dark.is_dark());
    }
}

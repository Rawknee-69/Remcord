//! Evergreen WebView2 Runtime prerequisite detection (release blocker 4).
//!
//! Microsoft's distribution guidance prefers the Evergreen Runtime with
//! automatic updates; this module detects it with the supported loader
//! mechanism — `GetAvailableCoreWebView2BrowserVersionString` with a null
//! browser-executable folder, which resolves the default per-machine and
//! per-user registrations (no administrator rights, no profile access).
//! Microsoft Edge being installed is never treated as proof: only the
//! Runtime registration counts.
//!
//! Outcome model, kept separate from crash recovery (this runs once at
//! boot, before any WebView exists; recovery budgets never see it):
//!
//! - `Present`: loader reported a non-empty version string.
//! - `Absent`: the loader call failed, or reported an empty string, or the
//!   version could not be read. Missing, empty, invalid, and inaccessible
//!   version data all land here — fail closed, never guessed.
//!
//! On `Absent` the caller stops normal initialization cleanly with an
//! actionable diagnostic (stderr plus a message box for shortcut launches
//! with no console). Nothing falls back anywhere: no other profile, no
//! browser executable, no development configuration.
//!
//! Privacy: the only data read is the installed Runtime version string.
//! No paths, page data, or user content are involved.

/// Installed-Runtime verdict. Computed once at boot; `clone`/`copy` freely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCheck {
    /// The Evergreen Runtime is registered; carries the version string the
    /// loader reported (opaque, logged verbatim, never parsed for policy).
    Present { version: String },
    /// No usable Runtime registration: missing, empty, invalid, or
    /// unreadable version data.
    Absent,
}

/// Pure classification of one loader read. `None` is any failed or
/// unreadable read; `Some` text that is empty after trimming is treated as
/// absent — an empty version can never identify a usable Runtime.
pub fn classify_version_output(version: Option<&str>) -> RuntimeCheck {
    match version {
        Some(text) if !text.trim().is_empty() => RuntimeCheck::Present {
            version: text.to_owned(),
        },
        _ => RuntimeCheck::Absent,
    }
}

/// Query the loader for the registered Evergreen Runtime version.
/// Returns `None` on any failure (or off Windows — this shell targets
/// WebView2 on Windows only). No admin rights, no profile, no window.
#[cfg(target_os = "windows")]
pub fn available_version() -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Com::CoTaskMemFree;

    let mut raw = windows::core::PWSTR::null();
    // Null folder = default lookup across per-machine and per-user
    // registrations (the supported presence check).
    let result = unsafe {
        webview2_com_sys::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString(
            PCWSTR::null(),
            &mut raw as *mut windows::core::PWSTR,
        )
    };
    if result.is_err() {
        return None;
    }
    // The loader allocates the string with CoTaskMemAlloc; always free it.
    let version = unsafe { raw.to_string().unwrap_or_default() };
    unsafe {
        CoTaskMemFree(Some(raw.as_ptr() as *const core::ffi::c_void));
    }
    if version.trim().is_empty() {
        return None;
    }
    Some(version)
}

/// Non-Windows fallback: no WebView2 Runtime exists here.
#[cfg(not(target_os = "windows"))]
pub fn available_version() -> Option<String> {
    None
}

/// Run one prerequisite check: query the loader, classify the read.
pub fn check() -> RuntimeCheck {
    classify_version_output(available_version().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_reported_version_is_present() {
        assert_eq!(
            classify_version_output(Some("154.0.4258.62")),
            RuntimeCheck::Present {
                version: "154.0.4258.62".to_owned(),
            }
        );
        // Opaque: never parsed, never normalized — logged verbatim.
        assert_eq!(
            classify_version_output(Some("  99.0.1  ")),
            RuntimeCheck::Present {
                version: "  99.0.1  ".to_owned(),
            }
        );
    }

    #[test]
    fn missing_empty_and_unreadable_reads_are_absent() {
        // Failed/inaccessible loader read.
        assert_eq!(classify_version_output(None), RuntimeCheck::Absent);
        // Empty or whitespace-only version strings can never identify a
        // usable Runtime.
        for empty in ["", "   ", "\t\n "] {
            assert_eq!(
                classify_version_output(Some(empty)),
                RuntimeCheck::Absent,
                "{empty:?} must be absent"
            );
        }
    }

    #[test]
    fn absent_is_not_a_recovery_or_fallback_signal() {
        // The verdict carries no budget, no retry, and no alternate path —
        // the caller stops startup. Pinned structurally: only two variants,
        // and `Absent` holds no data that could select a fallback.
        assert_eq!(classify_version_output(None), RuntimeCheck::Absent);
        let source = include_str!("webview_runtime.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "fallback",
            "Fallback",
            "retry",
            "Reload(",
            "Navigate(",
            "profile",
            "DEV_PROFILE",
            "LOCALAPPDATA",
            "browser_exe",
            "msedge",
            "Edge\\Application",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "detection must not select any fallback: {forbidden}"
            );
        }
    }

    #[test]
    fn live_loader_reports_this_machines_runtime() {
        // Read-only integration check against the real loader: this DEV
        // machine has the Evergreen Runtime (per-machine registration),
        // so detection must succeed with the registered version. Proves
        // the unsafe loader call, string conversion, and memory release
        // work end to end. Cannot simulate absence here (uninstalling the
        // Runtime to test would destabilize Windows); absence is covered
        // by `missing_empty_and_unreadable_reads_are_absent`.
        let found = available_version();
        assert!(
            found.as_deref().is_some_and(|v| !v.trim().is_empty()),
            "this machine must report a Runtime version"
        );
        assert_eq!(
            check(),
            RuntimeCheck::Present {
                version: found.expect("checked above"),
            }
        );
    }
}

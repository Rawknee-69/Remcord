//! Native single-instance policy (F27).
//!
//! Electron parity: `src/main.ts` (`app.requestSingleInstanceLock()` gated on
//! the `multiInstance` setting, default `false`) plus the `second-instance`
//! handler in `src/discord/window.ts` (focus the existing window, run CLI
//! commands, navigate a `discord://-` deep link - or spawn another window
//! when multi-instance is allowed).
//!
//! Decision (explicit, as the checklist requires): **single-instance lock**,
//! not allow-everything. A secondary launch with `multiInstance=false` exits
//! quietly before any window is shown; with `multiInstance=true` it runs
//! alongside the primary. The lock is kernel-backed (Windows named mutex),
//! so - unlike a PID file - it can never go stale after a crash: the OS
//! releases it with the dying process, exactly like Electron's lock.
//!
//! Scope: startup enforcement only. The "primary focuses its window on a
//! second launch" half is decided here as pure policy
//! ([`decide_second_launch`]), but the cross-process handoff transport
//! (named-pipe arg forwarding) does not exist yet, so that arm is wired for
//! the future, mirroring how `close_behavior` ships the unreachable `Hide`
//! arm until a real tray exists. No renderer surface: Electron exposes no
//! multi-instance API to renderers either, and `multi_instance` is already
//! readable via `settings.getConfig`.
//!
//! Privacy: the log line carries only fixed vocabulary plus two bounded
//! flags - no argv, URIs, paths, or user content.

use std::path::Path;

/// What a starting process does after the lock attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupAction {
    /// Run normally as (or alongside) the primary instance.
    ContinuePrimary,
    /// A primary is already running and policy forbids a second copy:
    /// exit quietly before any window is shown.
    ExitAsSecondary,
}

/// What the already-running primary does when a second launch is
/// signalled. Pure policy for the future named-pipe handoff; the startup
/// half above is what is enforced today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondLaunchAction {
    /// Bring the existing window forward (restore + show + focus).
    FocusExisting,
    /// Multi-instance is allowed: open another window.
    SpawnAdditional,
}

/// Outcome of one lock acquisition attempt.
pub enum LockOutcome {
    /// This process now holds the single-instance lock; the guard must be
    /// kept alive for the whole process lifetime (dropping it releases the
    /// lock and lets a third copy start).
    Acquired(SingleInstanceGuard),
    /// Another live process holds the lock.
    AlreadyRunning,
}

/// Process-lifetime guard for the single-instance lock.
pub struct SingleInstanceGuard {
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
    #[cfg(not(windows))]
    _file: std::fs::File,
}

// The OS handle/file is an owned resource: safe to hold across threads,
// never cloned, never shared.
#[cfg(windows)]
unsafe impl Send for SingleInstanceGuard {}

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

/// Pure startup decision. Mirrors Electron exactly:
/// `if (!requestSingleInstanceLock() && multiInstance === false) quit()`.
///
/// - lock lost + not allowed -> [`StartupAction::ExitAsSecondary`]
/// - everything else (lock won, or multi-instance allowed) -> Continue.
pub fn decide_startup(multi_instance: bool, lock_acquired: bool) -> StartupAction {
    if !lock_acquired && !multi_instance {
        StartupAction::ExitAsSecondary
    } else {
        StartupAction::ContinuePrimary
    }
}

/// Pure second-launch decision for the running primary. Mirrors the
/// `second-instance` handler in `src/discord/window.ts`: focus the existing
/// window unless multi-instance is allowed, in which case `init()` opens
/// another window.
pub fn decide_second_launch(multi_instance: bool) -> SecondLaunchAction {
    if multi_instance {
        SecondLaunchAction::SpawnAdditional
    } else {
        SecondLaunchAction::FocusExisting
    }
}

/// Prefix Electron strips from the deep-link argument
/// (`src/discord/window.ts`: `lastArg.replace("discord://-", "")`).
pub const DEEP_LINK_PREFIX: &str = "discord://-";

/// Extract the in-app target from a second-instance argv. Mirrors Electron:
/// only the LAST argument is inspected, and exactly one `discord://-`
/// prefix is stripped. Returns `None` when there is no link (or nothing
/// remains after the prefix - Electron would navigate to an empty path,
/// which the native gate refuses to do).
pub fn extract_deep_link(argv: &[&str]) -> Option<String> {
    let last = argv.last()?;
    let target = last.strip_prefix(DEEP_LINK_PREFIX)?;
    if target.is_empty() {
        None
    } else {
        Some(target.to_owned())
    }
}

/// Leaf directory name of the resolved profile, for lock scoping. Debug and
/// release use different profiles (`profile.rs`), so they must not share
/// one global lock - each profile gets its own.
pub fn profile_leaf(profile_dir: &Path) -> String {
    profile_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| profile::fallback_leaf().to_owned())
}

/// Kernel mutex name for one profile. `Local\` keeps it session-local (same
/// scope as Electron's per-user lock); the leaf is sanitized to mutex-safe
/// characters and bounded so a hostile `REMCORD_NATIVE_PROFILE_DIR` cannot
/// smuggle path syntax into the name.
pub fn mutex_name_for_leaf(leaf: &str) -> String {
    const PREFIX: &str = r"Local\RemcordSingleInstance-";
    let mut sanitized = String::new();
    for ch in leaf.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
        if sanitized.len() >= 48 {
            break;
        }
    }
    if sanitized.is_empty() {
        sanitized.push_str("default");
    }
    format!("{PREFIX}{sanitized}")
}

fn format_action(action: &StartupAction) -> &'static str {
    match action {
        StartupAction::ContinuePrimary => "continue",
        StartupAction::ExitAsSecondary => "exit",
    }
}

/// Fixed-vocabulary rendering of the intended primary behavior, for the
/// exiting secondary's note below.
pub fn format_second_launch(action: &SecondLaunchAction) -> &'static str {
    match action {
        SecondLaunchAction::FocusExisting => "focus_existing",
        SecondLaunchAction::SpawnAdditional => "spawn_additional",
    }
}

/// Fixed-vocabulary `[SingleInstance]` log line for one enforcement.
pub fn format_log(action: &StartupAction, multi_instance: bool, lock_acquired: bool) -> String {
    format!(
        "[SingleInstance] action={} multi_instance={multi_instance} lock_acquired={lock_acquired}",
        format_action(action),
    )
}

#[cfg(windows)]
fn try_acquire_windows(name: &str) -> Result<LockOutcome, String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    let wide = HSTRING::from(name);
    // HSTRING storage is NUL-terminated UTF-16, so its pointer converts
    // directly; this avoids relying on Param inference at the call site.
    let handle = unsafe { CreateMutexW(None, false, windows::core::PCWSTR(wide.as_ptr())) }
        .map_err(|error| format!("single-instance mutex creation failed: {error}"))?;
    // A valid handle plus ALREADY_EXISTS means another live process owns
    // the mutex; the duplicate handle is closed immediately.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            let _ = CloseHandle(handle);
        }
        Ok(LockOutcome::AlreadyRunning)
    } else {
        Ok(LockOutcome::Acquired(SingleInstanceGuard { handle }))
    }
}

#[cfg(not(windows))]
fn try_acquire_fallback(profile_dir: &Path) -> Result<LockOutcome, String> {
    use std::fs::OpenOptions;

    // Production target is Windows (WebView2 hard requirement); this keeps
    // other targets compiling with create_new atomicity. Stale files after
    // a kill must be removed by hand - documented, not hidden.
    let path = profile_dir.join("single-instance.lock");
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => Ok(LockOutcome::Acquired(SingleInstanceGuard { _file: file })),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Ok(LockOutcome::AlreadyRunning)
        }
        Err(error) => Err(format!("single-instance lock file failed: {error}")),
    }
}

/// Acquire the single-instance lock for this profile.
pub fn try_acquire(profile_dir: &Path) -> Result<LockOutcome, String> {
    #[cfg(windows)]
    {
        let _ = profile_dir;
        try_acquire_windows(&mutex_name_for_leaf(&profile_leaf(profile_dir)))
    }
    #[cfg(not(windows))]
    {
        try_acquire_fallback(profile_dir)
    }
}

/// Enforce the policy at startup: acquire the lock, then decide. The
/// returned guard (when `Some`) must be kept alive for the whole process;
/// on [`StartupAction::ExitAsSecondary`] there is nothing to hold and the
/// caller exits. OS failure fails startup loudly - never a silent run
/// without enforcement.
pub fn enforce_startup_policy(
    multi_instance: bool,
    profile_dir: &Path,
) -> Result<(StartupAction, Option<SingleInstanceGuard>), String> {
    match try_acquire(profile_dir)? {
        LockOutcome::Acquired(guard) => {
            let action = decide_startup(multi_instance, true);
            debug_assert_eq!(action, StartupAction::ContinuePrimary);
            Ok((action, Some(guard)))
        }
        LockOutcome::AlreadyRunning => Ok((decide_startup(multi_instance, false), None)),
    }
}

/// Fallback leaf when the profile path has no file name. Lives on
/// `profile` so the policy module owns no profile layout knowledge beyond
/// this constant.
mod profile {
    pub fn fallback_leaf() -> &'static str {
        "Remcord"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secondary_exits_only_when_single_instance() {
        // Electron truth table: quit iff the lock was lost AND multi-instance
        // is off. Every other combination runs.
        assert_eq!(decide_startup(false, false), StartupAction::ExitAsSecondary);
        assert_eq!(decide_startup(false, true), StartupAction::ContinuePrimary);
        assert_eq!(decide_startup(true, false), StartupAction::ContinuePrimary);
        assert_eq!(decide_startup(true, true), StartupAction::ContinuePrimary);
    }

    #[test]
    fn second_launch_focuses_unless_multi_allowed() {
        assert_eq!(
            decide_second_launch(false),
            SecondLaunchAction::FocusExisting
        );
        assert_eq!(
            decide_second_launch(true),
            SecondLaunchAction::SpawnAdditional
        );
    }

    #[test]
    fn deep_link_uses_last_arg_and_strips_prefix_once() {
        // window.ts pops the last commandLine entry and strips one prefix.
        assert_eq!(
            extract_deep_link(&["remcord.exe", "discord://-channels/123"]),
            Some("channels/123".to_owned())
        );
        // No link present.
        assert_eq!(extract_deep_link(&["remcord.exe"]), None);
        assert_eq!(extract_deep_link(&[]), None);
        // Bare prefix navigates nowhere; the native gate refuses empty.
        assert_eq!(extract_deep_link(&["discord://-"]), None);
        // Link-shaped args anywhere but last are ignored, like Electron.
        assert_eq!(
            extract_deep_link(&["discord://-channels/1", "--background"]),
            None
        );
        // Only one prefix layer is stripped.
        assert_eq!(
            extract_deep_link(&["discord://-discord://-x"]),
            Some("discord://-x".to_owned())
        );
    }

    #[test]
    fn mutex_name_is_scoped_sanitized_and_bounded() {
        let dev = mutex_name_for_leaf("Remcord.webview-dev-user-data");
        assert_eq!(
            dev,
            r"Local\RemcordSingleInstance-Remcord.webview-dev-user-data"
        );
        // Debug and release profiles must not share a lock.
        assert_ne!(mutex_name_for_leaf("Remcord"), dev);
        // No path syntax survives: separators become underscores.
        let hostile = mutex_name_for_leaf(r"..\\..\\Evil\..\x");
        // No path syntax survives past the fixed prefix: separators and
        // drive syntax become underscores.
        assert!(!hostile[r"Local\".len()..].contains(['\\', '/', ':']));
        // Long leaves are bounded.
        let long = mutex_name_for_leaf(&"a".repeat(200));
        assert!(long.len() <= r"Local\RemcordSingleInstance-".len() + 48);
        // Empty leaf still yields a usable name.
        assert_eq!(
            mutex_name_for_leaf(""),
            r"Local\RemcordSingleInstance-default"
        );
    }

    #[test]
    fn profile_leaf_falls_back_without_file_name() {
        assert_eq!(
            profile_leaf(Path::new(r"E:\Remcord.webview-dev-user-data")),
            "Remcord.webview-dev-user-data"
        );
        assert_eq!(
            profile_leaf(Path::new(r"C:\Users\x\AppData\Local\Remcord")),
            "Remcord"
        );
        assert!(!profile_leaf(Path::new("")).is_empty());
    }

    #[test]
    fn log_lines_are_fixed_vocabulary() {
        assert_eq!(
            format_second_launch(&SecondLaunchAction::FocusExisting),
            "focus_existing"
        );
        assert_eq!(
            format_second_launch(&SecondLaunchAction::SpawnAdditional),
            "spawn_additional"
        );
        assert_eq!(
            format_log(&StartupAction::ExitAsSecondary, false, false),
            "[SingleInstance] action=exit multi_instance=false lock_acquired=false"
        );
        assert_eq!(
            format_log(&StartupAction::ContinuePrimary, true, false),
            "[SingleInstance] action=continue multi_instance=true lock_acquired=false"
        );
    }

    #[cfg(windows)]
    #[test]
    fn mutex_second_acquire_reports_running_then_releases() {
        // Kernel truth: a second acquisition in this process sees the live
        // first guard as AlreadyRunning; dropping the first frees the name.
        let name = format!("f27-test-{}-{}", std::process::id(), "reacquire");
        let first = try_acquire_windows(&mutex_name_for_leaf(&name)).expect("first acquire");
        assert!(matches!(first, LockOutcome::Acquired(_)));
        let second = try_acquire_windows(&mutex_name_for_leaf(&name)).expect("second acquire");
        assert!(matches!(second, LockOutcome::AlreadyRunning));
        drop(first);
        let third = try_acquire_windows(&mutex_name_for_leaf(&name)).expect("third acquire");
        assert!(matches!(third, LockOutcome::Acquired(_)));
    }

    #[cfg(windows)]
    #[test]
    fn enforcement_holds_guard_on_free_lock() {
        let dir = std::env::temp_dir().join(format!("f27-enforce-free-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        // Free profile-scoped name: lock won, guard held, primary continues.
        let (action, guard) =
            enforce_startup_policy(false, &dir).expect("enforcement must not fail");
        assert_eq!(action, StartupAction::ContinuePrimary);
        assert!(guard.is_some());
        drop(guard);
    }

    #[cfg(windows)]
    #[test]
    fn enforcement_exits_secondary_when_lock_held() {
        let dir = std::env::temp_dir().join(format!("f27-enforce-held-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        // This process plays the primary: the name stays held below.
        let held = try_acquire(&dir).expect("primary acquire");
        assert!(matches!(held, LockOutcome::Acquired(_)));
        // Same profile, single-instance: the second copy exits, lockless.
        let (action, guard) =
            enforce_startup_policy(false, &dir).expect("enforcement must not fail");
        assert_eq!(action, StartupAction::ExitAsSecondary);
        assert!(guard.is_none());
        // Same situation with multi-instance allowed: runs alongside.
        let (action, guard) =
            enforce_startup_policy(true, &dir).expect("enforcement must not fail");
        assert_eq!(action, StartupAction::ContinuePrimary);
        assert!(guard.is_none());
        drop(held);
        drop(guard);
    }

    #[cfg(not(windows))]
    #[test]
    fn fallback_lock_file_is_exclusive() {
        let dir = tempfile::tempdir().expect("scratch dir");
        let first = try_acquire(dir.path()).expect("first acquire");
        assert!(matches!(first, LockOutcome::Acquired(_)));
        let second = try_acquire(dir.path()).expect("second acquire");
        assert!(matches!(second, LockOutcome::AlreadyRunning));
    }

    #[test]
    fn shipped_module_has_no_renderer_or_window_surface() {
        let source = include_str!("single_instance.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // Policy + OS lock only: no webview scripting, no window handles,
        // no navigation, no settings writes, no protocol registration.
        for forbidden in [
            "evaluate_script",
            "WebView",
            "tao::window",
            "set_visible",
            "Navigate(",
            "Reload(",
            "register_uri_scheme",
            "load_settings",
            "apply_settings",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
    }
}

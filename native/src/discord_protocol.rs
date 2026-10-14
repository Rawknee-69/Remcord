//! OS-level `discord://` protocol-link handling (F19).
//!
//! Electron parity (the exact sources):
//! - `src/main.ts:80` — `app.setAsDefaultProtocolClient("discord")`.
//! - `src/discord/window.ts:125-147` — `second-instance`: focus the window,
//!   run `handleCommands(commandLine)`, then navigate to the last argument
//!   when it starts with `discord://-`.
//! - `src/discord/window.ts:256-258` — `open-url` (macOS): same navigation.
//! - `src/common/dom.ts:43` — `navigateTo`: strip every character outside
//!   `[a-zA-Z0-9/_\-@.]`, then `history.pushState` + a `popstate` dispatch,
//!   then restore/show/focus the window.
//!
//! What this module owns (host-only, no bridge surface):
//! - [`extract_link_target`]: last-argv scan for the `discord://-` prefix.
//! - [`sanitize_deeplink_path`]: the exact Electron strip class.
//! - [`deeplink_script`]: the fixed `pushState` + `popstate` script with the
//!   path JSON-quoted (Electron's `JSON.stringify` parity).
//! - OS registration of the `discord` URL scheme for this executable
//!   (`setAsDefaultProtocolClient` parity: unconditional overwrite, never
//!   fatal).
//! - Second-instance handoff: a session-local named mutex detects a running
//!   instance; the newcomer writes the sanitized path to a bounded handoff
//!   file under the profile and signals a named event, then exits. The
//!   running instance's listener thread wakes, consumes the file, and the
//!   event loop navigates + focuses. Payloads are re-validated on receipt:
//!   any local process can signal the event, so the file content is
//!   untrusted input, never a command.
//!
//! Deliberate deltas from Electron (documented, never silent):
//! - An empty post-prefix suffix is ignored (Electron would push an empty
//!   state). Fail-closed, logged.
//! - `setSkipTaskbar(false)` has no native equivalent (no tray icon yet);
//!   the window is shown, restored, and focused only.
//! - In `multi_instance` mode Electron opens a fresh window and drops the
//!   link; native navigates the new window to the link instead (strictly
//!   more useful, same document the user asked for).
//! - The general second-launch policy (allow vs block a link-less second
//!   window) stays F27's decision: only launches carrying a `discord://-`
//!   argument ever forward-and-exit, and only when `multi_instance` is off.
//!
//! No renderer surface: nothing here crosses the bridge, and no new
//! capability exists (pinned by `native/tests/compatibility.test.mjs`).

/// Exact prefix Electron matches (`window.ts:140,257`): `startsWith`
/// semantics, case-sensitive. Real Discord links always carry the dash
/// (`discord://-/channels/…`); anything else is not a deeplink.
pub const DISCORD_LINK_PREFIX: &str = "discord://-";

/// Upper bound for a sanitized deeplink path in bytes. Real channel paths
/// are tens of bytes; the bound only stops degenerate argv/pipe input from
/// growing the script or the handoff file.
pub const MAX_DEEPLINK_PATH_BYTES: usize = 2048;

/// Upper bound for one handoff-file read. The file carries exactly the
/// sanitized path text; anything larger is refused before parsing.
pub const MAX_HANDOFF_FILE_BYTES: u64 = 4096;

/// Transient handoff filename inside the profile directory. Fixed name:
/// the renderer never supplies it and no directory traversal is possible.
pub const HANDOFF_FILENAME: &str = "deeplink-pending.json";

/// How long a forwarding newcomer waits for the running instance to
/// consume the handoff file (ack by deletion) before giving up and
/// booting its own window. Bounded so a dead listener never hangs launch.
#[cfg(target_os = "windows")]
pub const FORWARD_ACK_TIMEOUT_MS: u64 = 1000;

/// Returns the deeplink suffix of the LAST argv element when it starts
/// with [`DISCORD_LINK_PREFIX`], Electron `commandLine.pop()` parity.
/// `argv[0]` (the executable) never counts. An empty suffix, a missing
/// prefix, or a flag-only tail resolves `None`: link-less launches boot
/// normally and stay F27's business.
pub fn extract_link_target(argv: &[String]) -> Option<String> {
    let last = argv.iter().skip(1).last()?;
    let suffix = last.strip_prefix(DISCORD_LINK_PREFIX)?;
    if suffix.is_empty() {
        return None;
    }
    Some(suffix.to_owned())
}

/// Strips every character outside Electron's `navigateTo` class
/// (`[^a-zA-Z0-9/_\-@.]`, `src/common/dom.ts:45`). Pure projection:
/// quotes, backslashes, `?`, `#`, `:`, whitespace, and non-ASCII never
/// survive, so the result cannot break out of the quoted script literal
/// (which is additionally JSON-encoded at [`deeplink_script`]).
pub fn sanitize_deeplink_path(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '@' | '.'))
        .collect()
}

/// Builds the fixed navigation script for an already-sanitized path:
/// `history.pushState` + `popstate` dispatch (Electron parity), with the
/// path JSON-quoted exactly like Electron's `JSON.stringify`. Re-validates
/// the charset and bound so a direct caller can never smuggle quoting or
/// bulk through; failures are typed errors, never a partial script.
pub fn deeplink_script(sanitized: &str) -> Result<String, &'static str> {
    if sanitized.is_empty() {
        return Err("deeplink path is empty");
    }
    if sanitized.len() > MAX_DEEPLINK_PATH_BYTES {
        return Err("deeplink path exceeds the size limit");
    }
    if !sanitized
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '@' | '.'))
    {
        return Err("deeplink path uses characters outside the allowlist");
    }
    let quoted = serde_json::to_string(sanitized).map_err(|_| "deeplink path is unquotable")?;
    Ok(format!(
        "history.pushState({{ }},null,{quoted});window.dispatchEvent(new PopStateEvent(\"popstate\",{{ }}));"
    ))
}

/// Sanitizes one OS-object name component (user name, profile tag):
/// ASCII alphanumerics plus `-_.`, truncated to 64 bytes, empty falls
/// back to `"x"`. Keeps mutex/event names well-formed no matter what the
/// environment reports.
pub fn sanitize_name_component(raw: &str) -> String {
    let kept: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(64)
        .collect();
    if kept.is_empty() {
        "x".to_owned()
    } else {
        kept
    }
}

/// Current user for scoping OS objects to this logon session's user.
/// Falls back to `"user"` when the environment does not say.
pub fn current_user_component() -> String {
    sanitize_name_component(&std::env::var("USERNAME").unwrap_or_else(|_| "user".to_owned()))
}

/// Profile tag for scoping OS objects to one profile generation (dev vs
/// release profiles must never cross-deliver links). Derived from the
/// profile directory's leaf name only — never the full path.
pub fn profile_tag_component(profile_dir: &std::path::Path) -> String {
    let leaf = profile_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    sanitize_name_component(leaf)
}

/// Session-local mutex name proving a live instance for this user +
/// profile. `Local\` keeps it in the logon session (no admin, no
/// cross-session delivery).
pub fn mutex_name(user: &str, profile_tag: &str) -> String {
    format!("Local\\RemcordDeeplinkMutex-{user}-{profile_tag}")
}

/// Session-local event name the listener waits on. Same scoping as the
/// mutex; any local process can signal it, which is why the handoff file
/// content is re-validated on receipt.
pub fn event_name(user: &str, profile_tag: &str) -> String {
    format!("Local\\RemcordDeeplinkEvent-{user}-{profile_tag}")
}

/// Absolute path of the transient handoff file. Fixed filename under the
/// host-owned profile directory — no caller-supplied path involved.
pub fn handoff_path(profile_dir: &std::path::Path) -> std::path::PathBuf {
    profile_dir.join(HANDOFF_FILENAME)
}

/// Removes a stale handoff file left by a crashed newcomer. Best-effort:
/// absence is the common case and every error is ignored.
pub fn clear_stale_handoff(profile_dir: &std::path::Path) {
    let _ = std::fs::remove_file(handoff_path(profile_dir));
}

/// Reads and consumes one handoff file: bounded read, UTF-8 required,
/// sanitized on the way in (the writer is another local process, so the
/// bytes are untrusted). Returns the sanitized path, or `None` when the
/// file is absent, oversized, non-UTF-8, or sanitizes to nothing. The file
/// is deleted whenever it was present and readable, consumed or not.
pub fn consume_handoff_file(profile_dir: &std::path::Path) -> Option<String> {
    let path = handoff_path(profile_dir);
    let metadata = std::fs::metadata(&path).ok()?;
    if metadata.len() > MAX_HANDOFF_FILE_BYTES {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let text = String::from_utf8(bytes).ok()?;
    let sanitized = sanitize_deeplink_path(text.trim());
    if sanitized.is_empty() || sanitized.len() > MAX_DEEPLINK_PATH_BYTES {
        return None;
    }
    Some(sanitized)
}

/// Builds the OS open-command value for an executable path:
/// `"<exe>" "%1"`. The executable is always quoted (install paths contain
/// spaces); `%1` is Windows' URL placeholder, never a shell interpolation
/// — no `cmd.exe` is involved at activation. Pure and platform-independent,
/// so the registry writer and the tests share one definition.
pub fn protocol_command_value(exe_path: &str) -> String {
    format!("\"{exe_path}\" \"%1\"")
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0},
            System::{
                Registry::{
                    HKEY_CURRENT_USER, KEY_WRITE, REG_CREATED_NEW_KEY, REG_CREATE_KEY_DISPOSITION,
                    REG_OPTION_NON_VOLATILE, REG_SZ,
                },
                Threading::{
                    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject,
                    EVENT_MODIFY_STATE, INFINITE, SYNCHRONIZATION_SYNCHRONIZE,
                },
            },
        },
    };

    /// `HKCU\SOFTWARE\Classes\discord`: the per-user URL-scheme key
    /// `setAsDefaultProtocolClient("discord")` writes. HKCU needs no
    /// elevation, matching the per-user installer.
    const PROTOCOL_KEY_PATH: &str = r"SOFTWARE\Classes\discord";
    const OPEN_COMMAND_SUBKEY: &str = r"shell\open\command";
    /// Description shown by Windows for the scheme. Electron writes
    /// `"URL:discord"`; the fuller label names the product.
    const PROTOCOL_FRIENDLY_NAME: &str = "URL:Discord Protocol";

    fn wide_null(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn write_string_value(key_path: &str, value_name: Option<&str>, data: &str) -> bool {
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY,
        };
        unsafe {
            // Buffers stay alive for the whole call; every Win32 string is
            // a borrowed `PCWSTR` over them — no allocation crosses into
            // the registry, only copies out.
            let path = wide_null(key_path);
            let mut key = HKEY::default();
            let mut disposition = REG_CREATED_NEW_KEY;
            if RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut key,
                Some(&mut disposition as *mut REG_CREATE_KEY_DISPOSITION),
            )
            .is_err()
            {
                return false;
            }
            let name_holder;
            let name_pcw: PCWSTR = match value_name {
                Some(name) => {
                    name_holder = wide_null(name);
                    PCWSTR(name_holder.as_ptr())
                }
                None => PCWSTR::null(),
            };
            let wide: Vec<u16> = data.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes: &[u8] = std::slice::from_raw_parts(
                wide.as_ptr().cast(),
                wide.len() * std::mem::size_of::<u16>(),
            );
            let wrote = RegSetValueExW(key, name_pcw, None, REG_SZ, Some(bytes)).is_ok();
            let _ = RegCloseKey(key);
            wrote
        }
    }

    /// Outcome of the OS registration. Boot never fails on it: a missing
    /// registration only means `discord://` links open whatever (if
    /// anything) previously owned the scheme.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RegistrationOutcome {
        Registered,
        Failed,
    }

    /// Registers `HKCU\SOFTWARE\Classes\discord` for the running
    /// executable on every boot (`setAsDefaultProtocolClient("discord")`
    /// parity — Electron overwrites unconditionally, so last writer wins
    /// against the stock Discord client too). Every registry error resolves
    /// [`RegistrationOutcome::Failed`]; boot continues regardless.
    pub fn ensure_discord_protocol_registered() -> RegistrationOutcome {
        let exe = match std::env::current_exe() {
            Ok(exe) => exe.to_string_lossy().into_owned(),
            Err(_) => return RegistrationOutcome::Failed,
        };
        let friendly = write_string_value(PROTOCOL_KEY_PATH, None, PROTOCOL_FRIENDLY_NAME);
        let marker = write_string_value(PROTOCOL_KEY_PATH, Some("URL Protocol"), "");
        let command = write_string_value(
            &format!("{PROTOCOL_KEY_PATH}\\{OPEN_COMMAND_SUBKEY}"),
            None,
            &protocol_command_value(&exe),
        );
        if friendly && marker && command {
            RegistrationOutcome::Registered
        } else {
            RegistrationOutcome::Failed
        }
    }

    /// Process-lifetime guard holding the deeplink mutex. The first
    /// instance keeps one alive in `main()`; dropping closes the handle.
    /// Never cloned, never shared: ownership IS the "running" signal.
    pub struct InstanceGuard {
        handle: HANDLE,
    }

    impl Drop for InstanceGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }

    // `HANDLE` is a raw pointer wrapper: safe to hold across threads for
    // a purely-owned wait/close lifecycle, and the guard never shares it.
    unsafe impl Send for InstanceGuard {}

    /// Creates (or opens, detecting `ALREADY_EXISTS`) the session mutex.
    /// Returns the guard plus whether a previous holder existed. The guard
    /// must stay alive exactly in the instance that serves links.
    pub fn acquire_instance_mutex(mutex: &str) -> Option<(InstanceGuard, bool)> {
        let name = wide_null(mutex);
        unsafe {
            match CreateMutexW(None, false, PCWSTR(name.as_ptr())) {
                Ok(handle) => {
                    let existed = GetLastError() == ERROR_ALREADY_EXISTS;
                    Some((InstanceGuard { handle }, existed))
                }
                Err(_) => None,
            }
        }
    }

    /// Outcome of a newcomer's forward attempt.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ForwardOutcome {
        /// The running instance consumed the handoff; the newcomer exits.
        Forwarded,
        /// No live listener (mutex free, event unwritable, or ack timeout):
        /// the newcomer boots its own window. F27 owns that window's fate.
        NotForwarded,
    }

    /// Writes the handoff file atomically (temp + rename, both inside the
    /// profile dir) so the listener never reads a torn payload.
    fn write_handoff_file(profile_dir: &std::path::Path, sanitized: &str) -> bool {
        let target = handoff_path(profile_dir);
        let temp = target.with_extension(format!("tmp-{}", std::process::id()));
        if std::fs::write(&temp, sanitized).is_err() {
            return false;
        }
        if std::fs::rename(&temp, &target).is_err() {
            let _ = std::fs::remove_file(&temp);
            return false;
        }
        true
    }

    /// Newcomer side: hand `sanitized` to the running instance and report
    /// whether it was consumed (file deleted by the listener within the
    /// ack timeout). Pure file+event mechanics; the caller decides policy
    /// (Electron parity: only when `multi_instance` is off).
    pub fn forward_to_running_instance(
        profile_dir: &std::path::Path,
        event: &str,
        sanitized: &str,
    ) -> ForwardOutcome {
        if !write_handoff_file(profile_dir, sanitized) {
            return ForwardOutcome::NotForwarded;
        }
        let signaled = unsafe {
            let name = wide_null(event);
            match OpenEventW(
                EVENT_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
                false,
                PCWSTR(name.as_ptr()),
            ) {
                Ok(handle) => {
                    let ok = SetEvent(handle).is_ok();
                    let _ = CloseHandle(handle);
                    ok
                }
                Err(_) => false,
            }
        };
        if !signaled {
            let _ = std::fs::remove_file(handoff_path(profile_dir));
            return ForwardOutcome::NotForwarded;
        }
        // Ack by deletion: the listener consumes the file promptly. A
        // stale (crashed-listener) file times out and the newcomer boots
        // its own window; boot cleanup removes the residue next launch.
        let target = handoff_path(profile_dir);
        let waited = std::time::Duration::from_millis(FORWARD_ACK_TIMEOUT_MS);
        let step = std::time::Duration::from_millis(50);
        let start = std::time::Instant::now();
        while start.elapsed() < waited {
            if !target.exists() {
                return ForwardOutcome::Forwarded;
            }
            std::thread::sleep(step);
        }
        if !target.exists() {
            ForwardOutcome::Forwarded
        } else {
            ForwardOutcome::NotForwarded
        }
    }

    /// Blocks forever waiting for newcomer handoffs, invoking `emit` with
    /// each consumed path. Runs on its own thread; errors are logged and
    /// the wait resumes — the listener never dies on a malformed file.
    fn run_listener_loop(profile_dir: std::path::PathBuf, event: String, emit: impl Fn(String)) {
        let name = wide_null(&event);
        let handle = unsafe {
            match CreateEventW(None, false, false, PCWSTR(name.as_ptr())) {
                Ok(handle) => handle,
                Err(_) => {
                    eprintln!("[Deeplink] listener event unavailable; second-instance links will open a new window");
                    return;
                }
            }
        };
        loop {
            let waited = unsafe { WaitForSingleObject(handle, INFINITE) };
            if waited != WAIT_OBJECT_0 {
                continue;
            }
            match consume_handoff_file(&profile_dir) {
                Some(path) => {
                    println!("[Deeplink] received second-instance link, navigating");
                    emit(path);
                }
                None => {
                    println!("[Deeplink] signaled without a consumable handoff; ignoring");
                }
            }
        }
    }

    /// Spawns the listener thread. The returned handle is intentionally
    /// detached by the caller (the loop never returns); a creation failure
    /// is impossible via `std::thread` (it would abort), so no Result.
    pub fn spawn_listener_thread(
        profile_dir: std::path::PathBuf,
        event: String,
        emit: impl Fn(String) + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || run_listener_loop(profile_dir, event, emit))
    }
}

#[cfg(target_os = "windows")]
pub use windows_impl::{
    acquire_instance_mutex, ensure_discord_protocol_registered, forward_to_running_instance,
    spawn_listener_thread, ForwardOutcome, InstanceGuard, RegistrationOutcome,
};

/// Non-Windows stubs: native is Windows-only, so protocol registration
/// and second-instance handoff are inert elsewhere. The pure parsers
/// above still compile and test everywhere.
#[cfg(not(target_os = "windows"))]
mod stub_impl {
    /// Mirrors the Windows outcome without touching any OS surface.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RegistrationOutcome {
        Registered,
        Failed,
    }

    /// Mirrors the Windows outcome without touching any OS surface.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ForwardOutcome {
        Forwarded,
        NotForwarded,
    }

    /// No OS scheme surface off Windows: always inert, never fatal.
    pub fn ensure_discord_protocol_registered() -> RegistrationOutcome {
        RegistrationOutcome::Failed
    }

    /// No mutex surface off Windows: nobody is ever "already running".
    pub fn acquire_instance_mutex(_mutex: &str) -> Option<(InstanceGuard, bool)> {
        None
    }

    /// No listener off Windows: forwarding never happens.
    pub fn forward_to_running_instance(
        _profile_dir: &std::path::Path,
        _event: &str,
        _sanitized: &str,
    ) -> ForwardOutcome {
        ForwardOutcome::NotForwarded
    }

    /// Inert stand-in for the Windows guard.
    pub struct InstanceGuard;

    /// Spawns an immediately-exiting thread so call sites stay uniform.
    pub fn spawn_listener_thread(
        _profile_dir: std::path::PathBuf,
        _event: String,
        _emit: impl Fn(String) + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(|| {})
    }
}

#[cfg(not(target_os = "windows"))]
pub use stub_impl::{
    acquire_instance_mutex, ensure_discord_protocol_registered, forward_to_running_instance,
    spawn_listener_thread, ForwardOutcome, InstanceGuard, RegistrationOutcome,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn extracts_the_last_arg_only_with_the_exact_prefix() {
        // Electron `commandLine.pop()` parity: exactly the tail counts.
        assert_eq!(
            extract_link_target(&args(&["remcord.exe", "discord://-/channels/1/2"])),
            Some("/channels/1/2".to_owned())
        );
        // A link buried before trailing flags is NOT the tail: ignored.
        assert_eq!(
            extract_link_target(&args(&[
                "remcord.exe",
                "discord://-/channels/1/2",
                "--start-minimized"
            ])),
            None
        );
        // Bare executable and flag-only tails carry no link.
        assert_eq!(extract_link_target(&args(&["remcord.exe"])), None);
        assert_eq!(
            extract_link_target(&args(&["remcord.exe", "--start-minimized"])),
            None
        );
        assert_eq!(extract_link_target(&[]), None);
    }

    #[test]
    fn rejects_prefix_variants_and_empty_suffixes() {
        // Wrong scheme, missing dash, and case drift are not deeplinks.
        for tail in [
            "remcord://-/channels/1",
            "discord:/channels/1",
            "discord:///channels/1",
            "discord://channels/1",
            "DISCORD://-/channels/1",
            "https://discord.com/channels/1",
        ] {
            assert_eq!(
                extract_link_target(&args(&["remcord.exe", tail])),
                None,
                "{tail} must not extract"
            );
        }
        // `discord://-` alone has no path: ignored (Electron would push an
        // empty state; native fails closed instead).
        assert_eq!(
            extract_link_target(&args(&["remcord.exe", "discord://-"])),
            None
        );
    }

    #[test]
    fn sanitize_matches_electron_navigate_to_exactly() {
        // The Electron class is [a-zA-Z0-9/_\-@.]: everything else strips.
        assert_eq!(
            sanitize_deeplink_path("/channels/123/456"),
            "/channels/123/456"
        );
        assert_eq!(sanitize_deeplink_path("/channels/@me"), "/channels/@me");
        assert_eq!(
            sanitize_deeplink_path("/channels/1?token=abc#frag"),
            "/channels/1tokenabcfrag"
        );
        assert_eq!(sanitize_deeplink_path("/x\";alert(1)//"), "/xalert1//");
        assert_eq!(sanitize_deeplink_path("/chännels/ü"), "/chnnels/");
        assert_eq!(sanitize_deeplink_path(""), "");
    }

    #[test]
    fn script_quotes_the_path_and_rejects_bad_input() {
        let script = deeplink_script("/channels/123/456").expect("valid path builds");
        assert!(script.contains("history.pushState"));
        assert!(script.contains("PopStateEvent(\"popstate\""));
        assert!(script.contains("\"/channels/123/456\""));
        // No unquoted interpolation of the path anywhere.
        assert!(!script.contains("channels/123/456\";"));
        for bad in [
            "",
            "/x\"y",
            "/x\\y",
            "/x\ny",
            &"a".repeat(MAX_DEEPLINK_PATH_BYTES + 1),
        ] {
            assert!(deeplink_script(bad).is_err(), "{bad:?} must not build");
        }
    }

    #[test]
    fn protocol_command_quotes_the_exe_and_passes_the_url_through() {
        // Install paths contain spaces: the exe is always quoted, and `%1`
        // is Windows' own placeholder (no shell involved at activation).
        assert_eq!(
            protocol_command_value(r"C:\Users\me\AppData\Local\Programs\Remcord\remcord.exe"),
            r#""C:\Users\me\AppData\Local\Programs\Remcord\remcord.exe" "%1""#
        );
    }

    #[test]
    fn os_object_names_stay_well_formed() {
        assert_eq!(
            mutex_name("srijan", "Remcord"),
            "Local\\RemcordDeeplinkMutex-srijan-Remcord"
        );
        assert_eq!(
            event_name("srijan", "Remcord"),
            "Local\\RemcordDeeplinkEvent-srijan-Remcord"
        );
        // Hostile environment values collapse to the safe fallback/shape.
        assert_eq!(sanitize_name_component("..\\.."), "....");
        assert_eq!(sanitize_name_component(""), "x");
        assert_eq!(
            profile_tag_component(std::path::Path::new(r"C:\x\Remcord")),
            "Remcord"
        );
        assert_eq!(
            handoff_path(std::path::Path::new(r"C:\p"))
                .file_name()
                .unwrap(),
            "deeplink-pending.json"
        );
    }

    #[test]
    fn handoff_file_round_trips_and_rejects_junk() {
        let dir = std::env::temp_dir().join(format!("remcord-f19-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("fixture dir");
        // Absent file: no link.
        assert_eq!(consume_handoff_file(&dir), None);
        // Exact payload survives a write/read cycle.
        std::fs::write(handoff_path(&dir), "/channels/7/8").expect("write");
        assert_eq!(consume_handoff_file(&dir), Some("/channels/7/8".to_owned()));
        // Consumed: the file is gone.
        assert_eq!(consume_handoff_file(&dir), None);
        // Hostile bytes sanitize down (quotes/backslashes strip) or vanish.
        std::fs::write(handoff_path(&dir), "/x\";evil()").expect("write");
        assert_eq!(consume_handoff_file(&dir), Some("/xevil".to_owned()));
        std::fs::write(handoff_path(&dir), "???").expect("write");
        assert_eq!(consume_handoff_file(&dir), None);
        // Oversized payloads are refused and removed.
        std::fs::write(handoff_path(&dir), "a".repeat(5000)).expect("write");
        assert_eq!(consume_handoff_file(&dir), None);
        assert!(!handoff_path(&dir).exists());
        // Non-UTF-8 is refused and removed.
        std::fs::write(handoff_path(&dir), [0xff, 0xfe]).expect("write");
        assert_eq!(consume_handoff_file(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

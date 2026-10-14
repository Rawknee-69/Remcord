//! TEMPORARY user-agent override for UA-spoof testing (NOT a feature).
//!
//! Reads `REMCORD_NATIVE_USER_AGENT`. Unset or blank = current behavior
//! exactly (no override, no log beyond silence). Set = the value replaces
//! the WebView2 user agent (HTTP header *and* in-page
//! `navigator.userAgent`, via `ICoreWebView2Settings2::SetUserAgent`) after
//! validation; anything failing validation falls back to no override with
//! a stderr note — never a partial or injected string. Only printable
//! ASCII (0x20–0x7E, so no CR/LF header split) and at most
//! [`MAX_USER_AGENT_BYTES`] bytes are accepted.
//!
//! Purpose: test whether Discord serves different UI/experiments (e.g. the
//! Go Live quality picker) to the official desktop-client UA. To end the
//! experiment, unset the variable (no rebuild needed) or revert the commit
//! that added this module.

/// Environment variable carrying the override value.
pub const USER_AGENT_OVERRIDE_ENV: &str = "REMCORD_NATIVE_USER_AGENT";
/// Maximum accepted override length in bytes (real UAs are ~150 bytes).
pub const MAX_USER_AGENT_BYTES: usize = 512;

/// Validate one raw candidate: trimmed, non-empty, bounded, printable
/// ASCII only. Returns the effective value or `None` (invalid input fails
/// safe to no override).
pub fn validate_user_agent_override(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_USER_AGENT_BYTES {
        return None;
    }
    if !trimmed.bytes().all(|byte| (0x20..=0x7E).contains(&byte)) {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Mobile layout user agent: Electron parity (`src/discord/window.ts`
/// `mobileMode` branch). Exact Android Nexus string Electron serves so
/// Discord renders its hidden mobile layout.
pub const MOBILE_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 6.0; Nexus 5 Build/MRA58N) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/102.0.5005.149 Mobile Safari/537.36";

/// Resolve the settings-driven mobile UA. `None` means desktop behavior.
pub fn resolve_mobile_user_agent(mobile_mode: bool) -> Option<String> {
    mobile_mode.then(|| MOBILE_USER_AGENT.to_owned())
}

/// Resolve the effective override from the environment. `None` means
/// current behavior (unset, blank, or invalid — the last case logs once
/// to stderr so a typo is visible instead of silent).
pub fn resolve_user_agent_override() -> Option<String> {
    let raw = crate::dev_hooks::override_value(USER_AGENT_OVERRIDE_ENV)?;
    match validate_user_agent_override(&raw) {
        Some(value) => Some(value),
        None => {
            if !raw.trim().is_empty() {
                eprintln!(
                    "[UserAgent] ignoring invalid {USER_AGENT_OVERRIDE_ENV} (must be 1-{MAX_USER_AGENT_BYTES} printable ASCII chars, no CR/LF)"
                );
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_user_agent_override, MAX_USER_AGENT_BYTES};

    #[test]
    fn accepts_browser_and_desktop_client_shapes() {
        for candidate in [
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) discord/1.0.9187 Chrome/134.0.0.0 Electron/35.1.5 Safari/537.36",
            "Mozilla/5.0 (Windows NT 10.0; Trident/7.0; rv:11.0) like Gecko",
            "TestAgent/1.0",
        ] {
            let got = validate_user_agent_override(candidate);
            assert_eq!(got.as_deref(), Some(candidate), "{candidate:?} must validate");
        }
        // Surrounding whitespace trims to the effective value.
        assert_eq!(
            validate_user_agent_override("  TestAgent/1.0  ").as_deref(),
            Some("TestAgent/1.0")
        );
    }

    #[test]
    fn rejects_blank_oversized_and_non_printable_input() {
        assert_eq!(validate_user_agent_override(""), None);
        assert_eq!(validate_user_agent_override("   "), None);
        assert_eq!(
            validate_user_agent_override(&"x".repeat(MAX_USER_AGENT_BYTES + 1)),
            None
        );
        assert_eq!(validate_user_agent_override("A/B\r\nInjected: yes"), None);
        assert_eq!(validate_user_agent_override("A/B\nX"), None);
        assert_eq!(validate_user_agent_override("täst/1.0"), None);
        assert_eq!(validate_user_agent_override("tab\there"), None);
        // Exactly the bound is fine.
        assert_eq!(
            validate_user_agent_override(&"y".repeat(MAX_USER_AGENT_BYTES))
                .map(|value| value.len()),
            Some(MAX_USER_AGENT_BYTES)
        );
    }
}

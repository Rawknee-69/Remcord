//! Native Discord release-channel selection (`channel` setting).
//!
//! Traced Electron behavior (actual implementation, not declarations):
//! - `src/splash/redirect.html` maps the stored `channel` to its entry URL:
//!   `canary` → `https://canary.discord.com/app`,
//!   `ptb` → `https://ptb.discord.com/app`, anything else → stable
//!   (`https://discord.com/app`, the default from `src/common/config.ts`).
//! - The choice applies at startup navigation and takes effect on next
//!   launch (restart-required model): the renderer persists the value, the
//!   host reads the authoritative snapshot once per process.
//!
//! Native contract:
//! - Exactly three accepted values (`stable`, `canary`, `ptb`); anything
//!   else (including empty/unknown) fails safe to stable, mirroring
//!   Electron's anything-else fallback. The `settings.set` allowlist rejects
//!   unknown modes at parse time; this module's fallback only guards direct
//!   file edits and corrupt snapshots.
//! - Pure URL mapping: no renderer-supplied URL, host, or path can cross.
//!   The returned `&'static str` values are compile-time constants.

/// Exact channel ids Electron's redirect page switches on.
pub const VALID_CHANNELS: &[&str] = &["stable", "canary", "ptb"];

/// Exact entry hosts for the three release channels. This is a closed
/// first-party list, never a wildcard: `evil.discord.com`, deeper
/// subdomains, and lookalike hosts are NOT entry hosts. Every document and
/// permission gate that must work on any configured channel matches exactly
/// this list (mirroring the popup classifier, which already trusts the same
/// three hosts).
pub const ENTRY_HOSTS: &[&str] = &["discord.com", "canary.discord.com", "ptb.discord.com"];

/// True for exactly the three entry hosts (case-insensitive, hosts are
/// case-insensitive per RFC). Ports, userinfo, and paths never reach here:
/// callers compare the parsed host only.
pub fn is_entry_host(host: &str) -> bool {
    ENTRY_HOSTS
        .iter()
        .any(|entry| host.eq_ignore_ascii_case(entry))
}

/// Entry URL for one validated channel id.
pub fn discord_app_url(channel: &str) -> &'static str {
    match channel {
        "canary" => "https://canary.discord.com/app",
        "ptb" => "https://ptb.discord.com/app",
        _ => "https://discord.com/app",
    }
}

/// Resolve the authoritative snapshot value to its entry URL, failing safe
/// to stable for anything outside [`VALID_CHANNELS`] (hand-edited or
/// forward-compat values the allowlist never wrote).
pub fn resolve_channel_url(channel: &str) -> &'static str {
    if VALID_CHANNELS.contains(&channel) {
        discord_app_url(channel)
    } else {
        discord_app_url("stable")
    }
}

#[cfg(test)]
mod tests {
    use super::{discord_app_url, resolve_channel_url, VALID_CHANNELS};

    #[test]
    fn known_channels_map_to_their_entry_urls() {
        assert_eq!(VALID_CHANNELS, &["stable", "canary", "ptb"]);
        assert_eq!(discord_app_url("stable"), "https://discord.com/app");
        assert_eq!(discord_app_url("canary"), "https://canary.discord.com/app");
        assert_eq!(discord_app_url("ptb"), "https://ptb.discord.com/app");
    }

    #[test]
    fn unknown_and_empty_channels_fall_back_to_stable() {
        for hostile in [
            "",
            "STABLE",
            "Stable",
            " stable",
            "canary ",
            "beta",
            "https://canary.discord.com/app",
            "canary; rm -rf",
            "ptb\0",
        ] {
            assert!(
                !VALID_CHANNELS.contains(&hostile),
                "{hostile:?} must not validate"
            );
            assert_eq!(
                resolve_channel_url(hostile),
                "https://discord.com/app",
                "{hostile:?} must fall back to stable"
            );
        }
    }

    #[test]
    fn entry_hosts_are_exactly_the_three_channel_hosts() {
        use super::is_entry_host;
        assert_eq!(
            super::ENTRY_HOSTS,
            &["discord.com", "canary.discord.com", "ptb.discord.com"]
        );
        for host in [
            "discord.com",
            "DISCORD.COM",
            "canary.discord.com",
            "Canary.Discord.Com",
            "ptb.discord.com",
            "PTB.DISCORD.COM",
        ] {
            assert!(is_entry_host(host), "{host:?} must be an entry host");
        }
        for hostile in [
            "",
            "evil.discord.com",
            "deep.sub.discord.com",
            "discord.com.evil.com",
            "discord.com:443",
            "discord.com ",
            " discord.com",
            "example.com",
            "canary.discord.com.attacker.invalid",
        ] {
            assert!(!is_entry_host(hostile), "{hostile:?} must not validate");
        }
    }

    #[test]
    fn resolved_urls_stay_within_the_discord_entry_surface() {
        for channel in ["stable", "canary", "ptb", "anything-else"] {
            let url = resolve_channel_url(channel);
            assert!(
                url == "https://discord.com/app"
                    || url == "https://canary.discord.com/app"
                    || url == "https://ptb.discord.com/app",
                "{channel:?} resolved outside the entry surface: {url}"
            );
        }
    }
}

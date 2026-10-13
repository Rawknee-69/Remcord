//! Native diagnostics readout (`diagnostics.get_info` capability).
//!
//! Traced Electron behavior (actual implementation, not declarations):
//! - `window.remcord.settings.copyDebugInfo` copies a JSON blob of app,
//!   versions, GPU, and settings diagnostics to the clipboard
//!   (`src/discord/ipc.ts`, via Electron's clipboard + `app.getGPUInfo`).
//! - `window.remcord.settings.copyGPUInfo` copies the complete GPU info
//!   dump (`app.getGPUInfo("complete")`).
//!
//! Native contract (closest safe equivalent, documented differences):
//! - The host returns ONLY host-owned facts it can state truthfully: the
//!   compile-time app version, the installed WebView2 Runtime version read
//!   from the Evergreen loader, the profile-kind label (never an absolute
//!   path), the validated channel/csp modes, and the mod ids filtered to
//!   the exact `ValidMods` union. No GPU-device dump exists: WebView2
//!   exposes no renderer-safe GPU inventory through wry, so the GPU copy
//!   reports the graphics stack (WebView2 Runtime version) instead of
//!   inventing device data. The renderer performs the clipboard write with
//!   `navigator.clipboard` — no new host clipboard surface.
//! - Read-only, parameter-free (`deny_unknown_fields` rejects any member).
//!   No paths, tokens, cookies, settings secrets, or file contents cross.

use serde::Serialize;

/// Maximum mod ids echoed. The snapshot file itself is capped at 256 KiB,
/// but a hand-edited file could list many entries; the readout filters to
/// the exact `ValidMods` union, which bounds both count and content.
pub const MAX_DIAGNOSTIC_MODS: usize = 8;

/// Exact `ValidMods` union mirrored from `crate::settings::VALID_MODS`.
/// Kept local so this module has no reason to import the settings owner.
const VALID_MODS: &[&str] = &["vencord", "equicord", "custom", "shelter"];

/// Host-owned diagnostics facts. Every field is a fixed-vocabulary string
/// or a filtered id list — nothing renderer-supplied, nothing secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DebugInfo {
    pub app_version: String,
    pub webview2_runtime: String,
    pub profile_kind: &'static str,
    pub channel: String,
    pub csp: String,
    pub screenshare_quality: String,
    pub mods: Vec<String>,
}

/// Build the readout from host-known inputs. `webview2_runtime` is `None`
/// when the loader reported no usable Runtime (fail-closed to `"unknown"`,
/// never guessed). `channel`/`csp`/`screenshare_quality` fall back to their
/// compiled defaults when the snapshot carries an unknown mode; `mods` keeps
/// only exact `ValidMods` members in snapshot order, deduplicated.
pub fn collect_debug_info(
    app_version: &str,
    webview2_runtime: Option<&str>,
    profile_kind: &'static str,
    channel: &str,
    csp: &str,
    screenshare_quality: &str,
    mods: &[String],
) -> DebugInfo {
    let channel = if crate::channel::VALID_CHANNELS.contains(&channel) {
        channel.to_owned()
    } else {
        "stable".to_owned()
    };
    let csp = if crate::settings::VALID_CSP_MODES.contains(&csp) {
        csp.to_owned()
    } else {
        "none".to_owned()
    };
    let screenshare_quality = if crate::screenshare_quality::VALID_SCREENSHARE_QUALITY_MODES
        .contains(&screenshare_quality)
    {
        screenshare_quality.to_owned()
    } else {
        crate::screenshare_quality::DEFAULT_MODE.to_owned()
    };
    let mut seen = std::collections::HashSet::new();
    let mut filtered = Vec::new();
    for entry in mods {
        if filtered.len() >= MAX_DIAGNOSTIC_MODS {
            break;
        }
        if VALID_MODS.contains(&entry.as_str()) && seen.insert(entry.clone()) {
            filtered.push(entry.clone());
        }
    }
    DebugInfo {
        app_version: app_version.to_owned(),
        webview2_runtime: webview2_runtime
            .filter(|version| !version.trim().is_empty())
            .unwrap_or("unknown")
            .to_owned(),
        profile_kind,
        channel,
        csp,
        screenshare_quality,
        mods: filtered,
    }
}

#[cfg(test)]
mod tests {
    use super::{collect_debug_info, MAX_DIAGNOSTIC_MODS};

    fn info(mods: &[&str]) -> super::DebugInfo {
        collect_debug_info(
            "1.3.0",
            Some("154.0.4258.62"),
            "release",
            "canary",
            "strict",
            "balanced",
            &mods
                .iter()
                .map(|entry| entry.to_string())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn echoes_validated_host_facts() {
        let got = info(&["vencord", "shelter"]);
        assert_eq!(got.app_version, "1.3.0");
        assert_eq!(got.webview2_runtime, "154.0.4258.62");
        assert_eq!(got.profile_kind, "release");
        assert_eq!(got.channel, "canary");
        assert_eq!(got.csp, "strict");
        assert_eq!(got.screenshare_quality, "balanced");
        assert_eq!(got.mods, vec!["vencord".to_owned(), "shelter".to_owned()]);
    }

    #[test]
    fn filters_mods_to_exact_valid_ids_deduped_and_bounded() {
        let hostile = vec![
            "vencord".to_owned(),
            "../escape".to_owned(),
            "Vencord".to_owned(),
            "https://attacker.invalid".to_owned(),
            "vencord".to_owned(),
            "equicord".to_owned(),
            String::new(),
            "custom".to_owned(),
            "shelter".to_owned(),
            "extra0".to_owned(),
            "extra1".to_owned(),
        ];
        let got = collect_debug_info(
            "1.3.0",
            Some("1.0"),
            "dev",
            "stable",
            "none",
            "low",
            &hostile,
        );
        assert_eq!(
            got.mods,
            vec![
                "vencord".to_owned(),
                "equicord".to_owned(),
                "custom".to_owned(),
                "shelter".to_owned()
            ]
        );
        assert!(got.mods.len() <= MAX_DIAGNOSTIC_MODS);
    }

    #[test]
    fn unknown_modes_fall_back_and_absent_runtime_is_unknown() {
        let got = collect_debug_info(
            "1.3.0",
            None,
            "override",
            "beta",
            "none; script-src *",
            "ultra",
            &[],
        );
        assert_eq!(got.channel, "stable");
        assert_eq!(got.csp, "none");
        assert_eq!(got.screenshare_quality, "default");
        assert_eq!(got.webview2_runtime, "unknown");
        let blank = collect_debug_info(
            "1.3.0",
            Some("   "),
            "dev",
            "stable",
            "none",
            "default",
            &[],
        );
        assert_eq!(blank.webview2_runtime, "unknown");
    }

    #[test]
    fn serializes_without_paths_or_secrets() {
        let value = serde_json::to_value(info(&["equicord"])).expect("must serialize");
        let text = serde_json::to_string(&value).expect("must stringify");
        for forbidden in ["\\", "EBWebView", "token", "cookie", "Local\\", "AppData"] {
            assert!(
                !text.contains(forbidden),
                "readout must not carry {forbidden:?}: {text}"
            );
        }
        assert_eq!(value["mods"], serde_json::json!(["equicord"]));
    }
}

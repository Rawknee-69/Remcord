//! In-mod plugin settings pages for Vencord/Equicord/Shelter (F4).
//!
//! This module is the native counterpart of *nothing*: that is the entire
//! point. Each client mod renders its own settings UI inside Discord from
//! its own bundle once the shared substrate executes it (F1), and Remcord
//! ships no host settings page for any of them:
//! - Vencord and Equicord (a Vencord fork publishing the same `Vencord`
//!   page-world global) render their plugin/settings sections from the
//!   executed bundle and persist them in `localStorage` under
//!   `"VencordSettings"` / `"EquicordSettings"` (see
//!   `src/shelter/settings/components/BackupSection.tsx`, which reads and
//!   restores exactly those two keys, and
//!   `src/shelter/settings/quickCssEditorSource.ts`, whose editor modes are
//!   `"vencord" | "equicord" | "remcord"`).
//! - Shelter renders its own settings UI from the executed bundle and keeps
//!   its state in IndexedDB, never `localStorage` — which is why its
//!   descriptor declares `settings_key: None` and the substrate snapshot
//!   reports `not_applicable` instead of inventing a key.
//!
//! Native contract (deliberate: no new bridge surface for F4):
//! - No `mod_settings_*` / `plugin_settings_*` / `settings_page` capability
//!   or wire command exists. The boot snapshot already carries everything
//!   F4 relies on: the shared `mod.get_bundle` capability (bundle
//!   execution), the document-start storage preservation
//!   (`native/web/mod-substrate/storage-preservation.js`, keeping
//!   `localStorage` reachable when Discord re-initializes it), the
//!   substrate storage snapshot (`VencordSettings` / `EquicordSettings`
//!   reachability diagnostics), and the `quickcss.read` / `quickcss.write`
//!   pair behind the Remcord in-Discord editor.
//! - The one F4-adjacent host hook is the Vencord-family QuickCSS editor
//!   handoff: upstream `VencordNative.quickCss.openEditor` opens an
//!   `about:blank` popup the native managed-child policy denies, so the
//!   vencord/equicord adapters replace it post-bootstrap with a cancelable
//!   `remcord-native:open-quickcss-editor` DOM event carrying the owning
//!   mode. The Remcord settings plugin takes the handoff; shelter/custom
//!   adapters install no such override.
//! - The renderer supplies nothing here: storage keys, handoff event name,
//!   and modes are host constants asserted below.

/// `localStorage` key holding Vencord's persisted settings (page-side,
/// owned by the Vencord bundle — the host only probes its reachability).
pub const VENCORD_SETTINGS_KEY: &str = "VencordSettings";

/// `localStorage` key holding Equicord's persisted settings (page-side,
/// owned by the Equicord bundle — the host only probes its reachability).
pub const EQUICORD_SETTINGS_KEY: &str = "EquicordSettings";

/// Cancelable DOM event the patched Vencord-family `openEditor` dispatches
/// instead of opening the dead `about:blank` popup.
pub const QUICKCSS_HANDOFF_EVENT: &str = "remcord-native:open-quickcss-editor";

/// Owning modes the handoff may carry (`quickCssEditorSource.ts`
/// `QuickCssEditorMode`; the settings plugin ignores anything else).
pub const QUICKCSS_HANDOFF_MODES: &[&str] = &["vencord", "equicord", "remcord"];

/// Wire/capability name fragments that must never appear as a bridge
/// capability for F4: in-mod settings UI is page-side and host-uninvolved.
pub const FORBIDDEN_F4_CAPABILITY_FRAGMENTS: &[&str] = &[
    "mod_settings",
    "mod.settings",
    "plugin_settings",
    "plugin.settings",
    "settings_page",
    "settings-page",
    "open_settings",
];

/// Validates the F4 contract against the registered descriptors and the
/// boot snapshot capabilities. Fails fast on any drift: a descriptor
/// claiming a different settings key, a missing QuickCSS handoff shape, or
/// a newly added settings-page capability would each mean F4 is no longer
/// "no new code" and needs an explicit re-assessment.
pub fn validate_mod_settings_contract(
    vencord_settings_key: Option<&'static str>,
    equicord_settings_key: Option<&'static str>,
    shelter_settings_key: Option<&'static str>,
    custom_settings_key: Option<&'static str>,
    capabilities: &[&'static str],
) -> Result<(), String> {
    if vencord_settings_key != Some(VENCORD_SETTINGS_KEY) {
        return Err(format!(
            "[mod-settings] vencord settings key must be Some({VENCORD_SETTINGS_KEY:?})"
        ));
    }
    if equicord_settings_key != Some(EQUICORD_SETTINGS_KEY) {
        return Err(format!(
            "[mod-settings] equicord settings key must be Some({EQUICORD_SETTINGS_KEY:?})"
        ));
    }
    // Shelter persists in IndexedDB, never localStorage: inventing a key
    // would fabricate a storage contract the bundle does not have.
    if shelter_settings_key.is_some() {
        return Err("[mod-settings] shelter must declare no localStorage settings key".to_owned());
    }
    // Custom bundles are arbitrary user bytes with no settings schema.
    if custom_settings_key.is_some() {
        return Err("[mod-settings] custom must declare no localStorage settings key".to_owned());
    }
    if QUICKCSS_HANDOFF_EVENT.is_empty()
        || QUICKCSS_HANDOFF_MODES != ["vencord", "equicord", "remcord"]
    {
        return Err("[mod-settings] QuickCSS handoff shape drifted".to_owned());
    }
    for capability in capabilities {
        for fragment in FORBIDDEN_F4_CAPABILITY_FRAGMENTS {
            if capability.contains(fragment) {
                return Err(format!(
                    "[mod-settings] capability '{capability}' claims in-mod settings surface"
                ));
            }
        }
    }
    if !capabilities.contains(&"mod.get_bundle") {
        return Err(
            "[mod-settings] shared bundle capability is required for in-mod settings".to_owned(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities() -> Vec<&'static str> {
        vec![
            "bridge.ping",
            "mod.get_bundle",
            "quickcss.read",
            "quickcss.write",
        ]
    }

    #[test]
    fn f4_contract_accepts_the_registered_descriptor_shape() {
        validate_mod_settings_contract(
            Some("VencordSettings"),
            Some("EquicordSettings"),
            None,
            None,
            &capabilities(),
        )
        .expect("the F4 contract must hold for the registered descriptors");
    }

    #[test]
    fn f4_contract_rejects_settings_key_drift() {
        assert!(validate_mod_settings_contract(
            Some("Vencord"),
            Some("EquicordSettings"),
            None,
            None,
            &capabilities(),
        )
        .is_err());
        assert!(validate_mod_settings_contract(
            Some("VencordSettings"),
            Some("EquicordSettings"),
            Some("ShelterSettings"),
            None,
            &capabilities(),
        )
        .is_err());
    }

    #[test]
    fn f4_contract_rejects_a_new_settings_page_capability() {
        let mut with_new_surface = capabilities();
        with_new_surface.push("mod.settings_page");
        assert!(validate_mod_settings_contract(
            Some("VencordSettings"),
            Some("EquicordSettings"),
            None,
            None,
            &with_new_surface,
        )
        .is_err());
    }

    #[test]
    fn f4_contract_requires_the_shared_bundle_capability() {
        assert!(validate_mod_settings_contract(
            Some("VencordSettings"),
            Some("EquicordSettings"),
            None,
            None,
            &["bridge.ping"],
        )
        .is_err());
    }
}

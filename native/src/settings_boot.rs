//! Synchronous settings boot snapshot for the Remcord settings plugin.
//!
//! The real settings plugin (`src/shelter/settings/index.ts::onLoad`) reads
//! five synchronous members before any async work: `getConfig()` (whole
//! `Settings`), `getThemes()` (`ThemeManifest[]`), `translations` (whole
//! catalog), plus the fire-and-forget `setLang(locale)` call it makes on every
//! load. `restart()` is only referenced by later modal/button callbacks, but
//! its function existence is part of the boot surface. The native bridge is
//! Promise-only, so none of these can be served through the existing async
//! capabilities at plugin-init time.
//!
//! This module builds ONE host-created snapshot per launch from the actual
//! capability owners — never from duplicated filesystem logic and never from
//! invented values:
//!
//! - `settings` / `settingsSource` from [`crate::settings::load_settings`]
//!   (missing file → compiled-in defaults, exactly like the capability;
//!   corrupt/unreadable file → `"unavailable"` with a null config, never
//!   guessed values — a corrupt cache must not masquerade as defaults),
//! - `themes` from [`crate::themes::raw_theme_manifests`] (raw
//!   `{ ...manifest, id }` documents, Electron `getThemes()` shape),
//! - `lang` / `i18n` from [`crate::localization::load_raw_lang`] (merged
//!   catalog with the Electron fallback rules).
//!
//! `setLang` and `restart` are NOT synchronous host calls: the renderer
//! surface exposes them as narrow fire-and-forget senders over two dedicated
//! one-way events (`settings_boot_set_lang` with one bounded locale,
//! `settings_boot_restart` with an empty payload). There is deliberately no
//! generic synchronous invoke, no eval, and no renderer-to-host call beyond
//! these two closed shapes.
//!
//! Lifecycle: the JSON is built once at startup and spliced into the
//! document-start initialization script BEFORE the mod adapters, so it exists
//! before the settings plugin could execute. Every document executes the same
//! script text and receives its own frozen object identity (per-document
//! isolation); values reflect launch-time state. Mid-run host changes (a
//! `setLang` write, a settings mutation, a theme install) take effect for the
//! async capabilities immediately but are reflected in the boot snapshot on
//! the next launch — consistent with Electron's restart-required model for
//! most settings changes. This boundary is documented, not hidden.
//!
//! The snapshot carries no paths, URLs, filenames, or profile directories,
//! and the renderer supplies no values into it.

use std::path::Path;

/// Maximum serialized snapshot size in bytes. The real snapshot is tens of
/// KiB (settings ~2 KiB, one locale catalog ~30 KiB); this is a safety rail.
pub const MAX_BOOT_SNAPSHOT_BYTES: usize = 1024 * 1024;

/// Builds the settings boot snapshot JSON for one launch.
///
/// The returned text is spliced verbatim into the document-start script. The
/// schema is exactly:
/// `{ settingsSource: "file" | "defaults" | "unavailable", settings: object | null,
///    themes: Array<{ ...manifest, id }>, lang: string, i18n: Record<string, string> }`.
/// A corrupt settings file yields `settingsSource: "unavailable"` with
/// `settings: null` (full plugin execution stays blocked); it never yields
/// fabricated defaults.
pub fn build_settings_boot_json(profile_dir: &Path) -> String {
    let (settings_source, settings) = match crate::settings::load_settings(profile_dir) {
        Ok(loaded) => {
            let source = match loaded.source {
                crate::settings::SettingsSource::File => "file",
                crate::settings::SettingsSource::Defaults => "defaults",
            };
            let config = serde_json::to_value(&loaded.snapshot)
                .expect("the validated settings snapshot must serialize");
            (source, config)
        }
        Err(_) => ("unavailable", serde_json::Value::Null),
    };

    let mut themes = Vec::new();
    for raw in crate::themes::raw_theme_manifests(profile_dir) {
        let mut document = raw.manifest.as_object().cloned().unwrap_or_default();
        document.insert("id".to_owned(), serde_json::Value::String(raw.id.clone()));
        themes.push(serde_json::Value::Object(document));
    }

    let lang = crate::localization::load_raw_lang(profile_dir)
        .expect("the embedded translation catalogs must load");
    let i18n = serde_json::to_value(&lang.entries)
        .expect("the validated translation catalog must serialize");

    let snapshot = serde_json::json!({
        "settingsSource": settings_source,
        "settings": settings,
        "themes": themes,
        "lang": lang.lang,
        "i18n": i18n,
    });
    let serialized = serde_json::to_string(&snapshot).expect("the boot snapshot must serialize");
    assert!(
        serialized.len() <= MAX_BOOT_SNAPSHOT_BYTES,
        "settings boot snapshot exceeds the size limit"
    );
    serialized
}

/// Minimal schema-valid snapshot JSON for host-side tests that only need the
/// initialization-script plumbing (placeholder replacement, ordering) without
/// touching the filesystem.
#[cfg(test)]
pub fn test_fixture_json() -> &'static str {
    r#"{"settingsSource":"defaults","settings":{"mods":[]},"themes":[],"lang":"en-US","i18n":{"invite-goBackToApp":"Go back to the Discord app"}}"#
}

#[cfg(test)]
mod tests {
    use super::{build_settings_boot_json, MAX_BOOT_SNAPSHOT_BYTES};

    fn unique_profile_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time must be available")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "remcord-settings-boot-{tag}-{nanos}-{}",
            std::process::id()
        ))
    }

    fn seed_settings(profile: &std::path::Path, body: impl AsRef<[u8]>) {
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(profile.join("storage").join("settings.json"), body).unwrap();
    }

    fn seed_theme(profile: &std::path::Path, id: &str, manifest: &str) {
        let dir = profile.join("themes").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), manifest).unwrap();
    }

    fn parse_snapshot(profile: &std::path::Path) -> serde_json::Value {
        let json = build_settings_boot_json(profile);
        assert!(
            json.len() <= MAX_BOOT_SNAPSHOT_BYTES,
            "snapshot must respect the size bound"
        );
        serde_json::from_str(&json).expect("snapshot must be valid JSON")
    }

    #[test]
    fn missing_files_yield_defaults_empty_themes_and_en_us() {
        let snapshot = parse_snapshot(&unique_profile_dir("defaults"));
        assert_eq!(snapshot["settingsSource"], "defaults");
        assert!(snapshot["settings"].is_object());
        assert_eq!(snapshot["themes"], serde_json::json!([]));
        assert_eq!(snapshot["lang"], "en-US");
        assert_eq!(
            snapshot["i18n"]["invite-goBackToApp"],
            "Go back to the Discord app"
        );
    }

    #[test]
    fn corrupt_settings_yield_unavailable_not_fabricated_defaults() {
        let profile = unique_profile_dir("corrupt");
        seed_settings(&profile, b"{not json");
        let snapshot = parse_snapshot(&profile);
        assert_eq!(snapshot["settingsSource"], "unavailable");
        assert!(snapshot["settings"].is_null());
    }

    #[test]
    fn file_settings_round_trip_with_source_file() {
        let profile = unique_profile_dir("file");
        seed_settings(
            &profile,
            r#"{"customIcon":"","blockPowerSavingInVoiceChat":true,"mods":["shelter"]}"#,
        );
        let snapshot = parse_snapshot(&profile);
        assert_eq!(snapshot["settingsSource"], "file");
        assert_eq!(snapshot["settings"]["blockPowerSavingInVoiceChat"], true);
        assert_eq!(snapshot["settings"]["mods"], serde_json::json!(["shelter"]));
    }

    #[test]
    fn themes_pass_through_as_manifest_plus_id_sorted() {
        let profile = unique_profile_dir("themes");
        seed_theme(
            &profile,
            "z-theme",
            r#"{"name":"Zed","enabled":true,"theme":"src.css"}"#,
        );
        seed_theme(
            &profile,
            "a-theme",
            r#"{"name":"Aye","author":"me","enabled":false,"theme":"src.css"}"#,
        );
        seed_theme(&profile, "broken", "{not json");
        let snapshot = parse_snapshot(&profile);
        let themes = snapshot["themes"].as_array().unwrap();
        assert_eq!(themes.len(), 2, "the corrupt manifest must be skipped");
        assert_eq!(themes[0]["id"], "a-theme");
        assert_eq!(themes[0]["author"], "me");
        assert_eq!(themes[1]["id"], "z-theme");
        assert_eq!(themes[1]["theme"], "src.css");
    }

    #[test]
    fn snapshot_carries_no_host_paths() {
        let profile = unique_profile_dir("clean");
        seed_theme(
            &profile,
            "t",
            r#"{"name":"T","enabled":true,"theme":"src.css"}"#,
        );
        let snapshot = parse_snapshot(&profile);
        // Host filesystem layout must never cross the boundary. Translation
        // *content* may legitimately contain URLs (Electron-parity catalog
        // strings), so the path assertion is scoped to the host-read state.
        let state_text = serde_json::to_string(&serde_json::json!({
            "settings": snapshot["settings"],
            "themes": snapshot["themes"],
        }))
        .unwrap();
        for forbidden in [
            "ts-out",
            "legcord://",
            "E:\\",
            "C:\\",
            "userData",
            "AppData",
            "Temp",
        ] {
            assert!(
                !state_text.contains(forbidden),
                "snapshot state must not expose {forbidden:?}"
            );
        }
        assert!(
            !state_text.contains(&profile.to_string_lossy().replace('\\', "\\\\")),
            "snapshot state must not expose the profile directory"
        );
        // The catalog itself is a flat string map within the owner caps.
        let i18n = snapshot["i18n"].as_object().unwrap();
        assert!(!i18n.is_empty());
        assert!(i18n.len() <= crate::localization::MAX_BOOT_I18N_KEYS);
        for value in i18n.values() {
            assert!(value.is_string());
        }
    }

    #[test]
    fn fixture_is_schema_valid() {
        let parsed: serde_json::Value = serde_json::from_str(super::test_fixture_json()).unwrap();
        for key in ["settingsSource", "settings", "themes", "lang", "i18n"] {
            assert!(parsed.get(key).is_some(), "fixture needs `{key}`");
        }
    }
}

//! Native client-mod cache clearing (`mod.clear_cache` capability).
//!
//! Traced Electron behavior (actual implementation, not declarations):
//! - `window.remcord.settings.clearClientModCache` runs
//!   `setConfig("modCache", {})` (`SettingsPage.tsx`), resetting the
//!   opaque `modCache` map in `<userData>/storage/settings.json`
//!   (`src/common/config.ts`, `src/@types/settings.d.ts`).
//!
//! Native contract (superset with the same safety shape, documented):
//! - Resets the `modCache` document field to `{}` (the exact Electron
//!   outcome) AND deletes the host-downloaded mod bundle cache files, so
//!   the next launch re-validates and re-downloads from the fixed release
//!   endpoints. Deleted names are exactly the five host-managed artifacts
//!   (`vencord.js`, `vencord.css`, `equicord.js`, `equicord.css`,
//!   `shelter.js`) joined onto the host-built profile root.
//! - `custom.js` / `custom.css` are NEVER touched: they are user-owned
//!   bundles (executed from the same profile paths by the `custom` mod
//!   pipeline), not downloaded caches — deleting them would destroy user
//!   data.
//! - Links, junctions, directories, and non-regular files are skipped
//!   (reported absent, never followed). A non-NotFound IO error fails the
//!   whole operation closed before any settings write, naming only the
//!   fixed file name — never a path.
//! - The settings document is validated BEFORE any deletion: a corrupt or
//!   unreadable file is a typed failure with nothing touched (no
//!   destroy-evidence write, no partial cache wipe).
//! - Parameter-free (`deny_unknown_fields` rejects any member).

use std::path::Path;

/// Host-managed downloaded bundle caches. `custom.js` / `custom.css` are
/// deliberately absent (user-owned, never deleted here).
pub const MOD_CACHE_FILES: &[&str] = &[
    "vencord.js",
    "vencord.css",
    "equicord.js",
    "equicord.css",
    "shelter.js",
];

/// Typed failure: stable machine-readable `code` plus a human message that
/// names only a fixed allowlisted file, never a path.
#[derive(Debug)]
pub struct ModCacheFailure {
    pub code: &'static str,
    pub message: String,
}

fn unavailable(name: &str) -> ModCacheFailure {
    ModCacheFailure {
        code: "MOD_CACHE_UNAVAILABLE",
        message: format!("client-mod cache cannot be cleared for {name}"),
    }
}

/// Outcome: which cache files were deleted plus whether a stored `modCache`
/// map existed and was reset.
#[derive(Debug)]
pub struct ModCacheCleared {
    pub cleared: Vec<String>,
    pub mod_cache_cleared: bool,
}

/// Clear the downloaded bundle caches and reset the stored `modCache` map.
///
/// Ordering: the settings document is loaded and validated first (a corrupt
/// file fails with nothing touched); then each cache file that exists as a
/// regular file is removed; then the document's `modCache` field is reset
/// to `{}` when present and persisted atomically.
pub fn clear_mod_cache(profile_dir: &Path) -> Result<ModCacheCleared, ModCacheFailure> {
    // Validate the settings document BEFORE touching any cache file.
    let settings_path = crate::settings::settings_file_path(profile_dir);
    let document: Option<serde_json::Value> = match std::fs::read(&settings_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(unavailable("settings")),
        Ok(bytes) => {
            if bytes.len() as u64 > crate::settings::MAX_SETTINGS_FILE_BYTES {
                return Err(unavailable("settings"));
            }
            let document: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| unavailable("settings"))?;
            if !document.is_object() {
                return Err(unavailable("settings"));
            }
            // Full snapshot validation so a wrongly-typed file fails closed
            // instead of being rewritten into a near-empty document.
            let _: crate::settings::SettingsSnapshot =
                serde_json::from_value(document.clone()).map_err(|_| unavailable("settings"))?;
            Some(document)
        }
    };

    let mut cleared = Vec::new();
    for name in MOD_CACHE_FILES {
        let candidate = profile_dir.join(name);
        match std::fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(unavailable(name)),
            Ok(meta) => {
                if meta.is_file() {
                    std::fs::remove_file(&candidate).map_err(|_| unavailable(name))?;
                    cleared.push(name.to_string());
                }
                // Links, junctions, directories: skipped, never followed.
            }
        }
    }

    let mut mod_cache_cleared = false;
    if let Some(mut document) = document {
        let object = document
            .as_object_mut()
            .ok_or_else(|| unavailable("settings"))?;
        if object.contains_key("modCache") {
            object.insert(
                "modCache".to_owned(),
                serde_json::Value::Object(serde_json::Map::new()),
            );
            let serialized =
                serde_json::to_string_pretty(&document).map_err(|_| unavailable("settings"))?;
            let temp_path = settings_path.with_extension("json.tmp");
            if std::fs::write(&temp_path, serialized.as_bytes()).is_err() {
                let _ = std::fs::remove_file(&temp_path);
                return Err(unavailable("settings"));
            }
            if std::fs::rename(&temp_path, &settings_path).is_err() {
                let _ = std::fs::remove_file(&temp_path);
                return Err(unavailable("settings"));
            }
            mod_cache_cleared = true;
        }
    }
    Ok(ModCacheCleared {
        cleared,
        mod_cache_cleared,
    })
}

#[cfg(test)]
mod tests {
    use super::{clear_mod_cache, MOD_CACHE_FILES};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_profile_dir(tag: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be available")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "remcord-modcache-{tag}-{nanos}-{}",
            std::process::id()
        ))
    }

    #[test]
    fn deletes_only_host_managed_caches_and_resets_mod_cache_map() {
        assert_eq!(
            MOD_CACHE_FILES,
            &[
                "vencord.js",
                "vencord.css",
                "equicord.js",
                "equicord.css",
                "shelter.js"
            ]
        );
        let profile = unique_profile_dir("full");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        for name in MOD_CACHE_FILES {
            std::fs::write(profile.join(name), b"cached-bytes").unwrap();
        }
        // User-owned bundles must survive.
        std::fs::write(profile.join("custom.js"), b"user-bytes").unwrap();
        std::fs::write(profile.join("custom.css"), b"user-bytes").unwrap();
        std::fs::write(
            profile.join("storage").join("settings.json"),
            r#"{"mods":["vencord"],"modCache":{"vencord":"abc"}}"#,
        )
        .unwrap();
        let cleared = clear_mod_cache(&profile).expect("a valid profile must clear");
        assert_eq!(
            cleared.cleared,
            MOD_CACHE_FILES
                .iter()
                .map(|entry| entry.to_string())
                .collect::<Vec<_>>()
        );
        assert!(cleared.mod_cache_cleared);
        for name in MOD_CACHE_FILES {
            assert!(!profile.join(name).exists(), "{name} must be deleted");
        }
        assert_eq!(
            std::fs::read(profile.join("custom.js")).unwrap(),
            b"user-bytes"
        );
        assert_eq!(
            std::fs::read(profile.join("custom.css")).unwrap(),
            b"user-bytes"
        );
        let document: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(profile.join("storage").join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(document["modCache"], serde_json::json!({}));
        assert_eq!(document["mods"], serde_json::json!(["vencord"]));
    }

    #[test]
    fn empty_profile_reports_nothing_cleared() {
        let profile = unique_profile_dir("empty");
        let cleared = clear_mod_cache(&profile).expect("an empty profile must clear");
        assert!(cleared.cleared.is_empty());
        assert!(!cleared.mod_cache_cleared);
    }

    #[test]
    fn corrupt_settings_fail_with_nothing_touched() {
        let profile = unique_profile_dir("corrupt");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(profile.join("storage").join("settings.json"), b"{not json").unwrap();
        std::fs::write(profile.join("vencord.js"), b"cached-bytes").unwrap();
        let error = clear_mod_cache(&profile).expect_err("a corrupt file must fail");
        assert_eq!(error.code, "MOD_CACHE_UNAVAILABLE");
        assert_eq!(
            std::fs::read(profile.join("storage").join("settings.json")).unwrap(),
            b"{not json"
        );
        assert!(profile.join("vencord.js").exists());
    }

    #[test]
    fn planted_links_are_skipped_never_followed() {
        // A link where a cache file would live must not be followed or
        // deleted through: reported absent, target untouched.
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::fs::symlink_file;
            let profile = unique_profile_dir("link");
            let outside = unique_profile_dir("outside");
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(outside.join("real.txt"), b"do-not-touch").unwrap();
            if symlink_file(outside.join("real.txt"), profile.join("vencord.js")).is_err() {
                return;
            }
            let cleared = clear_mod_cache(&profile).expect("links must be skipped");
            assert!(cleared.cleared.is_empty());
            assert_eq!(
                std::fs::read(outside.join("real.txt")).unwrap(),
                b"do-not-touch"
            );
        }
    }
}

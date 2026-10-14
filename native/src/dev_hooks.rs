//! F28: central inventory + compile-time gate for the `REMCORD_NATIVE_*`
//! DEV hooks.
//!
//! The native host historically read ~30 `REMCORD_NATIVE_*` environment
//! variables at startup (mod/settings/theme/backup/loader/injector smoke
//! probes, diagnostics, and two path/UA overrides). Each read was guarded
//! only by "equals `1`" (or `Option` presence for overrides), so a release
//! binary still *contained* every probe path: any parent process that
//! planted the variable could activate bundle loads, OS shell reveals,
//! profile redirects, or UA spoofing in a shipped build.
//!
//! This module is the single source of truth for the hook family:
//!
//! - [`DEV_HOOKS`] inventories every known name with its kind, so audits
//!   (`grep REMCORD_NATIVE_`) converge here instead of scattering across
//!   `main.rs`.
//! - [`dev_hooks_compiled_in`] is the compile-time kill switch. Debug and
//!   `cargo test` builds keep the established DEV behavior; release builds
//!   strip every hook unless built with `--features dev-hooks`.
//! - [`hook_enabled`], [`override_value`], and [`smoke_reload_count`] are
//!   the only sanctioned readers. In a stripped release they return
//!   `false` / `None` / `0` without touching the environment, so the
//!   downstream probe branches are unreachable no matter what the parent
//!   process exports.
//!
//! Renderer contract: none. Hooks are host-process environment only; the
//! renderer never supplies names or values, and no capability, facade
//! method, or bridge event is added here.

/// How one hook activates and what it may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevHookKind {
    /// `=1` observability probe (loads bundles, logs, or performs one
    /// host-side action and reports). Never ships active: stripped release
    /// binaries ignore the variable.
    SmokeProbe,
    /// `=1` (or value-carrying) diagnostic that alters startup behavior
    /// (mod allowlist, bare page, interaction trace).
    Diagnostic,
    /// Value-carrying host override (profile directory, user agent).
    /// Honored only when hooks are compiled in.
    Override,
    /// `=1` gate for a live-dialog `#[cfg(test)]` that opens a real OS
    /// modal. Test code only — never linked into release binaries — but
    /// inventoried here so the family is complete.
    UiTest,
    /// Named in host docs but never read from the environment. Inert by
    /// construction; inventoried so audits do not chase a ghost.
    DocOnly,
}

/// One inventoried hook: its exact variable name, its kind, and what
/// enabling it does.
pub struct DevHook {
    pub var: &'static str,
    pub kind: DevHookKind,
    pub description: &'static str,
}

/// Compile-time kill switch. True for debug/`cargo test` builds (the
/// established DEV behavior) and for release builds explicitly built with
/// `--features dev-hooks`. Plain release builds report false, and every
/// reader below degrades to its inert value.
pub const fn dev_hooks_compiled_in() -> bool {
    cfg!(debug_assertions) || cfg!(feature = "dev-hooks")
}

// ---------------------------------------------------------------------------
// Hook names (moved here from `main.rs` so the inventory cannot drift).
// Each `pub const` below is referenced by [`DEV_HOOKS`] and by exactly one
// startup read site.
// ---------------------------------------------------------------------------

/// DEV diagnostic: force a mod's adapter OFF regardless of `settings.mods`, so
/// a "native without <mod>" control run needs no profile edit.
pub const MOD_ADAPTER_OVERRIDE_ENV: &str = "REMCORD_NATIVE_DIAGNOSTIC_MODS";
/// DEV-only: enable the post-load mod runtime observation.
pub const MOD_SMOKE_ENV: &str = "REMCORD_NATIVE_MOD_SMOKE";
pub const SETTINGS_SMOKE_ENV: &str = "REMCORD_NATIVE_SETTINGS_SMOKE";
pub const SETTINGS_MUTATION_SMOKE_ENV: &str = "REMCORD_NATIVE_SETTINGS_MUTATION_SMOKE";
pub const SETTINGS_SET_SMOKE_ENV: &str = "REMCORD_NATIVE_SETTINGS_SET_SMOKE";
pub const GETLANG_SMOKE_ENV: &str = "REMCORD_NATIVE_GETLANG_SMOKE";
pub const APP_VERSION_SMOKE_ENV: &str = "REMCORD_NATIVE_APP_VERSION_SMOKE";
pub const GRAPHICS_SMOKE_ENV: &str = "REMCORD_NATIVE_GRAPHICS_SMOKE";
pub const PLUGIN_STORAGE_SMOKE_ENV: &str = "REMCORD_NATIVE_PLUGIN_STORAGE_SMOKE";
pub const THEME_INSTALL_SMOKE_ENV: &str = "REMCORD_NATIVE_THEME_INSTALL_SMOKE";
pub const THEME_WRITE_SMOKE_ENV: &str = "REMCORD_NATIVE_THEME_WRITE_SMOKE";
pub const THEME_IMPORT_SMOKE_ENV: &str = "REMCORD_NATIVE_THEME_IMPORT_SMOKE";
pub const THEME_SET_ENABLED_SMOKE_ENV: &str = "REMCORD_NATIVE_THEME_SET_ENABLED_SMOKE";
pub const THEME_LIST_SMOKE_ENV: &str = "REMCORD_NATIVE_THEME_LIST_SMOKE";
pub const BACKUP_SAVE_SMOKE_ENV: &str = "REMCORD_NATIVE_BACKUP_SAVE_SMOKE";
pub const BACKUP_RESTORE_SMOKE_ENV: &str = "REMCORD_NATIVE_BACKUP_RESTORE_SMOKE";
pub const SHELTER_LOADER_PROBE_ENV: &str = "REMCORD_NATIVE_LOADER_PROBE";
/// DEV-only: enable the post-marker Shelter injector observation.
pub const INJECTOR_PROBE_ENV: &str = "REMCORD_NATIVE_INJECTOR_PROBE";
pub const SETTINGS_BOOT_SMOKE_ENV: &str = "REMCORD_NATIVE_SETTINGS_BOOT_SMOKE";
pub const SETTINGS_UI_SMOKE_ENV: &str = "REMCORD_NATIVE_SETTINGS_UI_SMOKE";
pub const FOLDERS_SMOKE_ENV: &str = "REMCORD_NATIVE_FOLDERS_SMOKE";
pub const REMCORD_PROTOCOL_SMOKE_ENV: &str = "REMCORD_NATIVE_PROTOCOL_SMOKE";
pub const PERMISSIONS_SMOKE_ENV: &str = "REMCORD_NATIVE_PERMISSIONS_SMOKE";
pub const THEMES_SMOKE_ENV: &str = "REMCORD_NATIVE_THEMES_SMOKE";
pub const QUICKCSS_SMOKE_ENV: &str = "REMCORD_NATIVE_QUICKCSS_SMOKE";
pub const BARE_PAGE_ENV: &str = "REMCORD_NATIVE_DIAGNOSTIC_BARE_PAGE";
pub const INTERACTION_DIAGNOSTIC_ENV: &str = "REMCORD_NATIVE_DIAGNOSTIC_INTERACTION";
/// DEV-only: repeat the mod DEV smoke reload this many times (parsed as
/// `u32`, anything else is 0).
pub const SMOKE_RELOAD_ENV: &str = "REMCORD_NATIVE_SMOKE_RELOAD";

/// Every known `REMCORD_NATIVE_*` hook. Names owned by other modules are
/// spelled literally here and pinned by tests against their owners, so this
/// table stays complete without creating module cycles.
pub const DEV_HOOKS: &[DevHook] = &[
    DevHook {
        var: MOD_ADAPTER_OVERRIDE_ENV,
        kind: DevHookKind::Diagnostic,
        description: "comma-separated mod allowlist forcing adapters off regardless of settings.mods",
    },
    DevHook {
        var: MOD_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "post-load mod runtime observation",
    },
    DevHook {
        var: SETTINGS_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "settings capability smoke probe",
    },
    DevHook {
        var: SETTINGS_MUTATION_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "settings mutation smoke probe",
    },
    DevHook {
        var: SETTINGS_SET_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "settings.set smoke probe",
    },
    DevHook {
        var: GETLANG_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "localization capability smoke probe",
    },
    DevHook {
        var: APP_VERSION_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "app-version capability smoke probe",
    },
    DevHook {
        var: GRAPHICS_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "graphics capability smoke probe",
    },
    DevHook {
        var: PLUGIN_STORAGE_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "plugin storage smoke probe",
    },
    DevHook {
        var: THEME_INSTALL_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "theme install smoke probe",
    },
    DevHook {
        var: THEME_WRITE_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "theme write smoke probe",
    },
    DevHook {
        var: THEME_IMPORT_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "theme import smoke probe",
    },
    DevHook {
        var: THEME_SET_ENABLED_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "theme enable smoke probe",
    },
    DevHook {
        var: THEME_LIST_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "theme list smoke probe",
    },
    DevHook {
        var: BACKUP_SAVE_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "backup save smoke probe",
    },
    DevHook {
        var: BACKUP_RESTORE_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "backup restore smoke probe",
    },
    DevHook {
        var: SHELTER_LOADER_PROBE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "Shelter plugin-loader probe",
    },
    DevHook {
        var: INJECTOR_PROBE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "post-marker Shelter injector observation",
    },
    DevHook {
        var: SETTINGS_BOOT_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "settings boot snapshot probe",
    },
    DevHook {
        var: SETTINGS_UI_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "settings UI workflow probe",
    },
    DevHook {
        var: FOLDERS_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "folder opening smoke probe (one real OS shell reveal + rejection paths)",
    },
    DevHook {
        var: REMCORD_PROTOCOL_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "remcord:// protocol smoke probe",
    },
    DevHook {
        var: PERMISSIONS_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "permissions smoke probe",
    },
    DevHook {
        var: THEMES_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "native theme injection smoke observation",
    },
    DevHook {
        var: QUICKCSS_SMOKE_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "native Quick CSS smoke observation",
    },
    DevHook {
        var: BARE_PAGE_ENV,
        kind: DevHookKind::Diagnostic,
        description: "boot a bare Discord page with bridge/mod scripts omitted",
    },
    DevHook {
        var: INTERACTION_DIAGNOSTIC_ENV,
        kind: DevHookKind::Diagnostic,
        description: "interaction diagnostic trace",
    },
    DevHook {
        var: SMOKE_RELOAD_ENV,
        kind: DevHookKind::SmokeProbe,
        description: "repeat the mod DEV smoke reload N times (u32, default 0)",
    },
    DevHook {
        var: "REMCORD_NATIVE_PROFILE_DIR",
        kind: DevHookKind::Override,
        description: "explicit absolute profile-directory override (owned by profile::PROFILE_OVERRIDE_ENV)",
    },
    DevHook {
        var: "REMCORD_NATIVE_USER_AGENT",
        kind: DevHookKind::Override,
        description: "temporary user-agent override for UA-spoof testing (owned by user_agent::USER_AGENT_OVERRIDE_ENV)",
    },
    DevHook {
        var: "REMCORD_NATIVE_BACKUP_SAVE_UI_TEST",
        kind: DevHookKind::UiTest,
        description: "live-dialog #[cfg(test)] opening a real modal saver; never linked into release binaries",
    },
    DevHook {
        var: "REMCORD_NATIVE_PICKER_UI_TEST",
        kind: DevHookKind::UiTest,
        description: "live-dialog #[cfg(test)] opening a real modal picker; never linked into release binaries",
    },
    DevHook {
        var: "REMCORD_NATIVE_WINDOW_SMOKE",
        kind: DevHookKind::DocOnly,
        description: "named in main.rs docs only; no environment read exists, so nothing can activate it",
    },
];

/// The inventoried kind of `var`, or `None` for unknown names. This is the
/// membership query: `is_some()` means inventoried.
pub fn hook_kind(var: &str) -> Option<DevHookKind> {
    DEV_HOOKS
        .iter()
        .find(|hook| hook.var == var)
        .map(|hook| hook.kind)
}

/// Sanctioned `=1` reader for smoke-probe and diagnostic hooks. Closed
/// world: unknown names — and names of any other kind — are always `false`.
/// In stripped release builds this is `false` without consulting the
/// environment.
pub fn hook_enabled(var: &str) -> bool {
    if !dev_hooks_compiled_in() {
        return false;
    }
    if !matches!(
        hook_kind(var),
        Some(DevHookKind::SmokeProbe) | Some(DevHookKind::Diagnostic)
    ) {
        return false;
    }
    std::env::var(var).is_ok_and(|value| value == "1")
}

/// Sanctioned value reader for override and diagnostic hooks. Closed world:
/// unknown names — and names of any other kind — always yield `None`. In
/// stripped release builds this is `None` without consulting the
/// environment.
pub fn override_value(var: &str) -> Option<String> {
    if !dev_hooks_compiled_in() {
        return None;
    }
    if !matches!(
        hook_kind(var),
        Some(DevHookKind::Override) | Some(DevHookKind::Diagnostic)
    ) {
        return None;
    }
    std::env::var(var).ok()
}

/// Sanctioned reader for [`SMOKE_RELOAD_ENV`]: the parsed `u32` reload
/// count, or 0 when unset/unparseable — and always 0 in stripped release
/// builds.
pub fn smoke_reload_count() -> u32 {
    if !dev_hooks_compiled_in() {
        return 0;
    }
    std::env::var(SMOKE_RELOAD_ENV)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0)
}

/// Hooks that are both compiled in and currently `=1` in this process.
/// Names only — override *values* may carry paths and are never logged.
pub fn active_hooks() -> Vec<&'static str> {
    if !dev_hooks_compiled_in() {
        return Vec::new();
    }
    DEV_HOOKS
        .iter()
        .filter(|hook| {
            matches!(hook.kind, DevHookKind::SmokeProbe | DevHookKind::Diagnostic)
                && std::env::var(hook.var).is_ok_and(|value| value == "1")
        })
        .map(|hook| hook.var)
        .collect()
}

/// One startup line stating the hook posture: stripped in plain release
/// builds, otherwise the active hook names (never their values).
pub fn log_startup_summary() {
    if !dev_hooks_compiled_in() {
        println!(
            "[DevHooks] dev hooks stripped in this build (release without the `dev-hooks` feature); REMCORD_NATIVE_* is ignored"
        );
        return;
    }
    let active = active_hooks();
    if active.is_empty() {
        println!("[DevHooks] dev hooks compiled in; none active");
    } else {
        // Names plus their inventoried purpose — never override *values*,
        // which may carry filesystem paths.
        let detail = active
            .iter()
            .map(|var| {
                let purpose = DEV_HOOKS
                    .iter()
                    .find(|hook| hook.var == *var)
                    .map(|hook| hook.description)
                    .unwrap_or("dev hook");
                format!("{var} ({purpose})")
            })
            .collect::<Vec<_>>()
            .join(", ");
        println!("[DevHooks] dev hooks active: {detail}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Environment mutation is process-global while tests run on threads;
    /// serialize the tests in this module that mutate it. (No other
    /// module's tests touch these exact variables: the `*_UI_TEST` gates
    /// only skip when unset, and main.rs tests assert const values.)
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn inventory_names_are_wellformed_unique_and_complete() {
        assert_eq!(
            DEV_HOOKS.len(),
            33,
            "inventory must stay complete; update F28 with new hooks"
        );
        let mut seen = std::collections::HashSet::new();
        for hook in DEV_HOOKS {
            assert!(
                hook.var.starts_with("REMCORD_NATIVE_"),
                "hook must carry the family prefix: {}",
                hook.var
            );
            let suffix = hook.var.strip_prefix("REMCORD_NATIVE_").expect("prefix");
            assert!(!suffix.is_empty(), "hook suffix must not be empty");
            assert!(
                suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_' || byte.is_ascii_digit()),
                "hook suffix must be UPPER_SNAKE: {}",
                hook.var
            );
            assert!(
                !hook.description.is_empty(),
                "hook needs a description: {}",
                hook.var
            );
            assert!(seen.insert(hook.var), "duplicate hook: {}", hook.var);
        }
    }

    #[test]
    fn inventory_covers_the_externally_owned_names() {
        // The two value-carrying overrides live with their owners; the
        // inventory spells them literally, so pin both sides together.
        assert_eq!(
            hook_kind(crate::profile::PROFILE_OVERRIDE_ENV),
            Some(DevHookKind::Override)
        );
        assert_eq!(
            hook_kind(crate::user_agent::USER_AGENT_OVERRIDE_ENV),
            Some(DevHookKind::Override)
        );
        // Every name the old main.rs const block owned is inventoried.
        for var in [
            MOD_ADAPTER_OVERRIDE_ENV,
            MOD_SMOKE_ENV,
            SETTINGS_SMOKE_ENV,
            SETTINGS_MUTATION_SMOKE_ENV,
            SETTINGS_SET_SMOKE_ENV,
            GETLANG_SMOKE_ENV,
            APP_VERSION_SMOKE_ENV,
            GRAPHICS_SMOKE_ENV,
            PLUGIN_STORAGE_SMOKE_ENV,
            THEME_INSTALL_SMOKE_ENV,
            THEME_WRITE_SMOKE_ENV,
            THEME_IMPORT_SMOKE_ENV,
            THEME_SET_ENABLED_SMOKE_ENV,
            THEME_LIST_SMOKE_ENV,
            BACKUP_SAVE_SMOKE_ENV,
            BACKUP_RESTORE_SMOKE_ENV,
            SHELTER_LOADER_PROBE_ENV,
            INJECTOR_PROBE_ENV,
            SETTINGS_BOOT_SMOKE_ENV,
            SETTINGS_UI_SMOKE_ENV,
            FOLDERS_SMOKE_ENV,
            REMCORD_PROTOCOL_SMOKE_ENV,
            PERMISSIONS_SMOKE_ENV,
            THEMES_SMOKE_ENV,
            QUICKCSS_SMOKE_ENV,
            BARE_PAGE_ENV,
            INTERACTION_DIAGNOSTIC_ENV,
            SMOKE_RELOAD_ENV,
        ] {
            assert!(hook_kind(var).is_some(), "{var} must be inventoried");
        }
    }

    #[test]
    fn unknown_names_are_rejected() {
        assert_eq!(hook_kind("REMCORD_NATIVE_NOPE"), None);
        assert_eq!(hook_kind("REMCORD_IMPORT_FROM"), None);
        assert_eq!(hook_kind(""), None);
        // Closed world: even a planted value cannot enable an unknown name.
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::set_var("REMCORD_NATIVE_NOPE", "1");
        assert!(!hook_enabled("REMCORD_NATIVE_NOPE"));
        assert_eq!(override_value("REMCORD_NATIVE_NOPE"), None);
        std::env::remove_var("REMCORD_NATIVE_NOPE");
    }

    #[test]
    fn readers_refuse_the_wrong_kind() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        // Overrides are value-carrying: `=1` must not count as enabled.
        std::env::set_var(crate::profile::PROFILE_OVERRIDE_ENV, "1");
        assert!(!hook_enabled(crate::profile::PROFILE_OVERRIDE_ENV));
        std::env::remove_var(crate::profile::PROFILE_OVERRIDE_ENV);
        // Probes are `=1` flags: they carry no override value.
        std::env::set_var(MOD_SMOKE_ENV, "shelter");
        assert_eq!(override_value(MOD_SMOKE_ENV), None);
        std::env::remove_var(MOD_SMOKE_ENV);
    }

    #[test]
    fn gate_is_open_in_test_builds_and_reads_exactly_one() {
        assert!(
            dev_hooks_compiled_in(),
            "tests build with dev hooks compiled in"
        );
        let _guard = ENV_LOCK.lock().expect("env lock");
        // Chosen probe: nothing else in the suite touches it.
        std::env::remove_var(MOD_SMOKE_ENV);
        assert!(!hook_enabled(MOD_SMOKE_ENV));
        std::env::set_var(MOD_SMOKE_ENV, "1");
        assert!(hook_enabled(MOD_SMOKE_ENV));
        std::env::set_var(MOD_SMOKE_ENV, "true");
        assert!(!hook_enabled(MOD_SMOKE_ENV), "only exactly `1` enables");
        std::env::set_var(MOD_SMOKE_ENV, "");
        assert!(!hook_enabled(MOD_SMOKE_ENV));
        std::env::remove_var(MOD_SMOKE_ENV);
        assert!(!hook_enabled(MOD_SMOKE_ENV));
    }

    #[test]
    fn override_reader_roundtrips_values_in_test_builds() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::remove_var(MOD_ADAPTER_OVERRIDE_ENV);
        assert_eq!(override_value(MOD_ADAPTER_OVERRIDE_ENV), None);
        std::env::set_var(MOD_ADAPTER_OVERRIDE_ENV, "shelter");
        assert_eq!(
            override_value(MOD_ADAPTER_OVERRIDE_ENV).as_deref(),
            Some("shelter")
        );
        std::env::remove_var(MOD_ADAPTER_OVERRIDE_ENV);
    }

    #[test]
    fn smoke_reload_count_parses_like_the_old_startup_read() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::remove_var(SMOKE_RELOAD_ENV);
        assert_eq!(smoke_reload_count(), 0);
        std::env::set_var(SMOKE_RELOAD_ENV, "3");
        assert_eq!(smoke_reload_count(), 3);
        std::env::set_var(SMOKE_RELOAD_ENV, "abc");
        assert_eq!(smoke_reload_count(), 0);
        std::env::remove_var(SMOKE_RELOAD_ENV);
    }

    #[test]
    fn active_hooks_lists_only_enabled_known_probes() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::remove_var(MOD_SMOKE_ENV);
        std::env::remove_var(SETTINGS_SMOKE_ENV);
        assert!(active_hooks().is_empty());
        std::env::set_var(MOD_SMOKE_ENV, "1");
        std::env::set_var(SETTINGS_SMOKE_ENV, "1");
        let active = active_hooks();
        assert!(active.contains(&MOD_SMOKE_ENV));
        assert!(active.contains(&SETTINGS_SMOKE_ENV));
        std::env::remove_var(MOD_SMOKE_ENV);
        std::env::remove_var(SETTINGS_SMOKE_ENV);
    }

    // The three tests below only compile into a *stripped* release
    // (`cargo test --release` without `--features dev-hooks`). They pin the
    // F28 guarantee: no environment value can activate a hook there.

    #[cfg(not(any(debug_assertions, feature = "dev-hooks")))]
    #[test]
    fn stripped_release_ignores_enabled_probes() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        assert!(!dev_hooks_compiled_in());
        std::env::set_var(MOD_SMOKE_ENV, "1");
        assert!(!hook_enabled(MOD_SMOKE_ENV));
        assert!(active_hooks().is_empty());
        std::env::remove_var(MOD_SMOKE_ENV);
    }

    #[cfg(not(any(debug_assertions, feature = "dev-hooks")))]
    #[test]
    fn stripped_release_ignores_overrides() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::set_var(MOD_ADAPTER_OVERRIDE_ENV, "shelter");
        assert_eq!(override_value(MOD_ADAPTER_OVERRIDE_ENV), None);
        std::env::remove_var(MOD_ADAPTER_OVERRIDE_ENV);
    }

    #[cfg(not(any(debug_assertions, feature = "dev-hooks")))]
    #[test]
    fn stripped_release_smoke_reload_is_zero() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::set_var(SMOKE_RELOAD_ENV, "3");
        assert_eq!(smoke_reload_count(), 0);
        std::env::remove_var(SMOKE_RELOAD_ENV);
    }
}

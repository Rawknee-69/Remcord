//! Splash / first-run setup / CSS editor windows (F24).
//!
//! Electron contracts (`src/splash/main.ts`, `src/setup/main.ts`,
//! `src/cssEditor/main.ts`, plus the launch decision in `src/main.ts:110-123`
//! and the first-run flag in `src/common/config.ts:185-189`):
//! - Splash: a frameless 300x350 window (`backgroundColor #202225`) shown while
//!   the main window boots, unless `skipSplash` is set or the launch is a
//!   background start. Its preload exposes `window.internal` (`restart`,
//!   `version`, `isDev`, `isMicrosoftStore`, `mods`, `getLang`, `splashEnd`).
//! - Setup: a fixed 840x720 non-resizable, non-maximizable window shown on
//!   first run (no config file yet, or `doneSetup` not set). Its preload
//!   exposes `window.setup` (`restart`, `os`, `saveSettings`, `getLang`,
//!   `getRawLang`). The setup page (`src/setup/setup.tsx`) saves exactly four
//!   keys over its lifetime: `windowStyle` (one of `overlay` / `native` /
//!   `default`), `mods` (`[]` for Shelter, otherwise one of `vencord` /
//!   `equicord`), `tray` (`dynamic` / `disabled`), and finally
//!   `doneSetup: true`.
//! - CSS editor: an 800x600 window opened by `openCssEditor(file)` for either
//!   the fixed Quick CSS file or one theme's manifest-declared CSS file
//!   (`src/discord/ipc.ts:148-154` `openQuickCss`, `:206-217` `editTheme`).
//!   Its preload exposes `window.cssEditor` (`get` / `set`).
//!
//! Native mapping (single-window shell — no second window is created; this
//! module owns the host-side policy so the deferred windows cannot drift):
//! - Startup flow: [`decide_startup_flow`] reproduces `init()`'s branch
//!   exactly (first run → setup, otherwise main with the splash overlay
//!   unless skipped or background). [`is_first_run`] maps the native
//!   settings load to Electron's `firstRun` flag.
//! - Splash facts: [`build_splash_info`] serves the host-owned `internal`
//!   surface (`version` from [`crate::app`], `isDev`/`isMicrosoftStore`
//!   constants, `mods` + `skipSplash` from the authoritative snapshot). The
//!   splash document itself (`remcord://html/splash.html`) is already served
//!   by the protocol handler as a resource; windowing it is deferred.
//! - Setup saves: [`validate_setup_save`] pins the exact four-key allowlist
//!   the setup page may write, with Electron's value grammars. Application
//!   reuses the existing `settings.set` path — this module validates, never
//!   persists.
//! - CSS editor targets: [`parse_css_editor_target`] restricts the editor to
//!   the same two targets Electron opens (fixed Quick CSS file, or one
//!   validated theme id resolved under the host themes root). Reading and
//!   writing reuse `quickcss.read` / `quickcss.write` / `themes.write_css` —
//!   the detached editor window is deferred, never a second file writer.
//!
//! Deliberate, documented differences from Electron:
//! - No `BrowserWindow` is created here (no second HWND, no window manager
//!   calls): the native shell is single-window and multi-window support is
//!   deferred by platform boundary (see `COMPATIBILITY.md`). The tests below
//!   forbid windowing tokens in shipped code.
//! - `setup-minimize`, `setup-quit` (delete settings + quit), and
//!   `setup-restart` relaunch semantics belong to a real setup window and
//!   are deferred with it; `settingsBoot.restart` already covers relaunch.
//! - `useSystemCssEditor` (open in the OS editor) has no native counterpart
//!   yet — the in-app editor targets below are the only ones.
//! - The darwin-only setup chrome (`titleBarStyle`, `vibrancy`) and the
//!   Microsoft Store flag have no Windows-native meaning; the store flag is
//!   pinned `false`.
//!
//! This module is pure policy plus tests: no window, no settings I/O, no
//! bridge, no filesystem, no fetch. The only authorities are the compiled-in
//! window specs, the caller-supplied settings snapshot, and the closed
//! setup-save / editor-target grammars.
//!
//! Privacy: log lines carry only fixed vocabulary plus booleans — no URIs,
//! page data, or user content.

//! Dead code by construction: the single-window shell creates no second
//! window and persists no setup state through this module, so no shipped
//! caller consumes this policy yet. Every item is pinned by the unit tests
//! below and consumed by the deferred multi-window implementer, which is why
//! this allow lives at module scope instead of on individual items.
#![allow(dead_code)]

use crate::settings::{SettingsSnapshot, SettingsSource};

/// Electron splash window width (`src/splash/main.ts`).
pub const SPLASH_WIDTH: u32 = 300;
/// Electron splash window height.
pub const SPLASH_HEIGHT: u32 = 350;
/// The splash window is frameless (`frame: false`).
pub const SPLASH_FRAMELESS: bool = true;
/// Electron splash background color (dark appearance).
///
/// Dark-only by history: light mode must use
/// [`splash_background_for_theme`] instead so the splash frame is
/// genuinely light when the OS is.
pub const SPLASH_BACKGROUND: &str = "#202225";
/// Localization key for the splash window title.
pub const SPLASH_TITLE_KEY: &str = "splash-title";

/// Splash background for an OS theme: dark keeps Electron's `#202225`,
/// light gets a genuinely light backdrop. Deferred-window policy (no HWND
/// here): the implementer paints this color on the splash surface for the
/// theme read at startup.
pub fn splash_background_for_theme(theme: crate::os_theme::OsTheme) -> &'static str {
    crate::os_theme::splash_background_for(theme)
}

/// Electron setup window width (`src/setup/main.ts`).
pub const SETUP_WIDTH: u32 = 840;
/// Electron setup window height.
pub const SETUP_HEIGHT: u32 = 720;
/// The setup window is not resizable (`resizable: false`).
pub const SETUP_RESIZABLE: bool = false;
/// The setup window is not maximizable (`maximizable: false`).
pub const SETUP_MAXIMIZABLE: bool = false;
/// Localization key for the setup window title.
pub const SETUP_TITLE_KEY: &str = "setup-windowTitle";

/// Electron CSS editor window width (`src/cssEditor/main.ts`).
pub const CSS_EDITOR_WIDTH: u32 = 800;
/// Electron CSS editor window height.
pub const CSS_EDITOR_HEIGHT: u32 = 600;

/// Window styles the setup page offers (`src/setup/setup.tsx` `WindowStyle`).
pub const SETUP_WINDOW_STYLES: &[&str] = &["overlay", "native", "default"];
/// Tray options the setup page offers (`TraySettings`).
pub const SETUP_TRAY_OPTIONS: &[&str] = &["dynamic", "disabled"];
/// Client mods the setup page offers (`ModSelector`); Shelter is represented
/// by an empty `mods` list, so it never appears as a stored entry.
pub const SETUP_MOD_OPTIONS: &[&str] = &["vencord", "equicord"];
/// Native is Windows-only, so the Microsoft Store build flag is always false.
pub const IS_MICROSOFT_STORE_BUILD: bool = false;

/// Which top-level flow the launch takes, mirroring `src/main.ts:110-123`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupFlow {
    /// First run: the setup window owns the session.
    Setup,
    /// Normal boot: the main window loads, with or without the splash phase.
    Main {
        /// Show the splash phase (`!skipSplash && !background_start`).
        show_splash: bool,
    },
}

impl StartupFlow {
    fn as_str(self) -> &'static str {
        match self {
            StartupFlow::Setup => "setup",
            StartupFlow::Main { .. } => "main",
        }
    }
}

/// Decide the startup flow from the three Electron inputs.
///
/// - `first_run`: `src/common/config.ts` `firstRun` (no config file yet, or
///   `doneSetup` unset). A `--bypass-setup` launch maps here as
///   `first_run = false` (it generates defaults and marks setup done).
/// - `skip_splash`: the `skipSplash` setting.
/// - `background_start`: Electron's `isBackgroundStart()` (tray/launcher).
///
/// First run always wins (Electron creates the setup window even for a
/// background start — the background check lives in the splash path only).
pub fn decide_startup_flow(
    first_run: bool,
    skip_splash: bool,
    background_start: bool,
) -> StartupFlow {
    if first_run {
        return StartupFlow::Setup;
    }
    StartupFlow::Main {
        show_splash: !skip_splash && !background_start,
    }
}

/// Map the native settings load to Electron's `firstRun` flag
/// (`src/common/config.ts:185-189`: set when no config file exists or when
/// the stored config never completed setup).
///
/// - `Ok(load)` with `source == Defaults` (no file) → first run.
/// - `Ok(load)` with `source == File` → first run unless `done_setup`.
/// - `Err` (corrupt/unreadable file) → first run: surfacing setup beats
///   booting into an unknowable state, matching Electron's broken-config
///   handling which never silently continues.
pub fn is_first_run(
    load: &Result<crate::settings::SettingsLoad, String>,
    done_setup: bool,
) -> bool {
    match load {
        Ok(loaded) => match loaded.source {
            SettingsSource::Defaults => true,
            SettingsSource::File => !done_setup,
        },
        Err(_) => true,
    }
}

/// Host-owned splash facts: the native counterpart of Electron's
/// `window.internal` splash surface (`src/splash/preload.mts`), minus the
/// window-bound `restart` / `splashEnd` senders (deferred with the window).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplashInfo {
    /// Application version from [`crate::app::current_app_version`].
    pub version: String,
    /// Always `false` natively (no dev bypass exists).
    pub is_dev: bool,
    /// Always `false`: native is Windows-only, never a Store build.
    pub is_microsoft_store: bool,
    /// Enabled client mods from the authoritative snapshot.
    pub mods: Vec<String>,
    /// Whether the splash phase is skipped.
    pub skip_splash: bool,
}

/// Build the splash facts for the running application: the version comes
/// from [`crate::app::current_app_version`], everything else from the
/// authoritative settings snapshot. No renderer input participates.
pub fn splash_info_for_snapshot(snapshot: &SettingsSnapshot) -> SplashInfo {
    build_splash_info(crate::app::current_app_version(), snapshot)
}

/// Build the splash facts from an explicit host version plus the
/// authoritative settings snapshot. No renderer input participates.
pub fn build_splash_info(version: &str, snapshot: &SettingsSnapshot) -> SplashInfo {
    SplashInfo {
        version: version.to_owned(),
        is_dev: false,
        is_microsoft_store: IS_MICROSOFT_STORE_BUILD,
        mods: snapshot.mods.clone(),
        skip_splash: snapshot.skip_splash,
    }
}

/// One setup-page save: exactly the keys `src/setup/setup.tsx` ever writes.
/// `None` means "the page did not touch this key on this step". There is
/// deliberately NO path, URL, locale, channel, proxy, keybind, audio, or
/// arbitrary-settings member: anything outside the setup page's vocabulary
/// cannot be expressed, let alone persisted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetupSave {
    pub window_style: Option<String>,
    pub mods: Option<Vec<String>>,
    pub tray: Option<String>,
    pub done_setup: Option<bool>,
}

/// The validated form of [`SetupSave`]: every present member matched
/// Electron's grammar. Application reuses the existing `settings.set` path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedSetupSave {
    pub window_style: Option<String>,
    pub mods: Option<Vec<String>>,
    pub tray: Option<String>,
    pub done_setup: Option<bool>,
}

/// Validate one setup-page save against Electron's exact grammars.
///
/// - `windowStyle` must be one of [`SETUP_WINDOW_STYLES`].
/// - `mods` must be empty (Shelter) or exactly one of [`SETUP_MOD_OPTIONS`]
///   — the page picks a single card, never a combination.
/// - `tray` must be one of [`SETUP_TRAY_OPTIONS`].
/// - `doneSetup`, when present, must be `true`: the page only ever completes
///   setup; un-completing it is rejected.
pub fn validate_setup_save(save: &SetupSave) -> Result<ValidatedSetupSave, String> {
    if let Some(style) = &save.window_style {
        if !SETUP_WINDOW_STYLES.contains(&style.as_str()) {
            return Err("setup windowStyle must be one of overlay, native, default".to_owned());
        }
    }
    if let Some(mods) = &save.mods {
        if mods.len() > 1
            || !mods
                .iter()
                .all(|entry| SETUP_MOD_OPTIONS.contains(&entry.as_str()))
        {
            return Err("setup mods must be empty or one of vencord, equicord".to_owned());
        }
    }
    if let Some(tray) = &save.tray {
        if !SETUP_TRAY_OPTIONS.contains(&tray.as_str()) {
            return Err("setup tray must be one of dynamic, disabled".to_owned());
        }
    }
    if let Some(done) = save.done_setup {
        if !done {
            return Err("setup doneSetup must be true when present".to_owned());
        }
    }
    Ok(ValidatedSetupSave {
        window_style: save.window_style.clone(),
        mods: save.mods.clone(),
        tray: save.tray.clone(),
        done_setup: save.done_setup,
    })
}

/// CSS editor target: exactly what Electron's `openCssEditor(file)` opens
/// (`src/discord/ipc.ts`): the fixed Quick CSS file, or one theme's CSS file
/// under the host themes root. There is deliberately NO path, filename, URL,
/// or file-handle parameter: the host resolves the target itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CssEditorTarget {
    /// The fixed `<profile>/quickCss.css` (served via `quickcss.read/write`).
    QuickCss,
    /// One theme's CSS (served via `themes.write_css`); the id is validated,
    /// never sanitized, and the manifest-declared file is resolved host-side.
    ThemeCss { id: String },
}

/// Parse a renderer-supplied editor target into the closed target enum.
///
/// `kind` is exactly `"quickcss"` or `"theme"`; `id` is required for theme
/// targets (validated by [`crate::themes::validate_theme_id`], which rejects
/// separators, drive/UNC syntax, and `.`/`..`) and must be absent for the
/// Quick CSS target. Anything else is refused before any filesystem work.
pub fn parse_css_editor_target(kind: &str, id: Option<&str>) -> Result<CssEditorTarget, String> {
    match kind {
        "quickcss" => {
            if id.is_some() {
                return Err("quickcss editor target takes no id".to_owned());
            }
            Ok(CssEditorTarget::QuickCss)
        }
        "theme" => {
            let theme_id = id.ok_or_else(|| "theme editor target requires an id".to_owned())?;
            crate::themes::validate_theme_id(theme_id)?;
            Ok(CssEditorTarget::ThemeCss {
                id: theme_id.to_owned(),
            })
        }
        _ => Err("CSS editor target must be quickcss or theme".to_owned()),
    }
}

/// Fixed-vocabulary `[AuxWindows][Startup]` log line for one launch decision.
/// Booleans only — no URIs, page data, or user content.
pub fn format_startup_log(flow: StartupFlow, background_start: bool) -> String {
    let splash = match flow {
        StartupFlow::Setup => false,
        StartupFlow::Main { show_splash } => show_splash,
    };
    format!(
        "[AuxWindows][Startup] flow={} show_splash={splash} background={background_start}",
        flow.as_str(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_with(mods: &[&str], skip_splash: bool, done_setup: bool) -> SettingsSnapshot {
        SettingsSnapshot {
            mods: mods.iter().map(|entry| entry.to_string()).collect(),
            skip_splash,
            done_setup,
            ..SettingsSnapshot::default()
        }
    }

    #[test]
    fn splash_spec_matches_electron_main() {
        // Traced from src/splash/main.ts, not inferred. Bound to locals:
        // clippy's assertions_on_constants rejects asserting on consts.
        let (width, height) = (SPLASH_WIDTH, SPLASH_HEIGHT);
        let (frameless, background, title) =
            (SPLASH_FRAMELESS, SPLASH_BACKGROUND, SPLASH_TITLE_KEY);
        assert_eq!((width, height), (300, 350));
        assert!(frameless);
        assert_eq!(background, "#202225");
        assert_eq!(title, "splash-title");
    }

    #[test]
    fn splash_background_follows_the_os_theme() {
        // The legacy constant stays dark-only; themed callers must use the
        // per-theme function so light mode is genuinely light.
        assert_eq!(
            splash_background_for_theme(crate::os_theme::OsTheme::Dark),
            "#202225"
        );
        assert_eq!(
            splash_background_for_theme(crate::os_theme::OsTheme::Light),
            "#f2f3f5"
        );
    }

    #[test]
    fn setup_spec_matches_electron_main() {
        // Traced from src/setup/main.ts, not inferred.
        let (width, height) = (SETUP_WIDTH, SETUP_HEIGHT);
        let (resizable, maximizable, title) = (SETUP_RESIZABLE, SETUP_MAXIMIZABLE, SETUP_TITLE_KEY);
        assert_eq!((width, height), (840, 720));
        assert!(!resizable);
        assert!(!maximizable);
        assert_eq!(title, "setup-windowTitle");
    }

    #[test]
    fn css_editor_spec_matches_electron_main() {
        // Traced from src/cssEditor/main.ts, not inferred.
        let (width, height) = (CSS_EDITOR_WIDTH, CSS_EDITOR_HEIGHT);
        assert_eq!((width, height), (800, 600));
    }

    #[test]
    fn startup_flow_reproduces_electron_init_branches() {
        // src/main.ts: first run owns the session; otherwise main boots,
        // with the splash phase unless skipped or background.
        assert_eq!(decide_startup_flow(true, false, false), StartupFlow::Setup);
        // First run wins even for background starts (the background check
        // lives in the splash path only).
        assert_eq!(decide_startup_flow(true, false, true), StartupFlow::Setup);
        assert_eq!(
            decide_startup_flow(false, false, false),
            StartupFlow::Main { show_splash: true }
        );
        assert_eq!(
            decide_startup_flow(false, true, false),
            StartupFlow::Main { show_splash: false }
        );
        assert_eq!(
            decide_startup_flow(false, false, true),
            StartupFlow::Main { show_splash: false }
        );
        assert_eq!(
            decide_startup_flow(false, true, true),
            StartupFlow::Main { show_splash: false }
        );
    }

    #[test]
    fn first_run_maps_missing_unfinished_and_broken_settings() {
        use crate::settings::{SettingsLoad, SettingsSource};
        // No file (compiled-in defaults) is always first run.
        let snapshot = snapshot_with(&[], false, false);
        let load: Result<SettingsLoad, String> = Ok(SettingsLoad {
            snapshot: snapshot.clone(),
            source: SettingsSource::Defaults,
            payload_bytes: 0,
        });
        assert!(is_first_run(&load, false));
        // A stored file that never completed setup is first run, even when
        // the caller passes a stale done_setup=false through.
        let load: Result<SettingsLoad, String> = Ok(SettingsLoad {
            snapshot,
            source: SettingsSource::File,
            payload_bytes: 0,
        });
        assert!(is_first_run(&load, false));
        // A completed stored file is not first run.
        let done = snapshot_with(&["vencord"], false, true);
        let load: Result<SettingsLoad, String> = Ok(SettingsLoad {
            snapshot: done,
            source: SettingsSource::File,
            payload_bytes: 0,
        });
        assert!(!is_first_run(&load, true));
        // Corrupt settings surface setup instead of booting blind.
        let load: Result<SettingsLoad, String> = Err("broken".to_owned());
        assert!(is_first_run(&load, false));
    }

    #[test]
    fn splash_info_serves_host_facts_with_fail_closed_flags() {
        let snapshot = snapshot_with(&["vencord"], false, false);
        let info = build_splash_info("1.2.3", &snapshot);
        assert_eq!(info.version, "1.2.3");
        assert!(!info.is_dev);
        assert!(!info.is_microsoft_store);
        assert_eq!(info.mods, vec!["vencord".to_owned()]);
        assert!(!info.skip_splash);
    }

    #[test]
    fn shipped_version_passes_the_version_gate() {
        let version = crate::app::current_app_version();
        assert!(crate::app::validate_version(version).is_ok());
        let snapshot = snapshot_with(&[], true, true);
        let info = splash_info_for_snapshot(&snapshot);
        assert_eq!(info.version, version);
        assert!(info.skip_splash);
        assert!(info.mods.is_empty());
    }

    #[test]
    fn setup_save_accepts_every_setup_page_step() {
        // onMount defaults (src/setup/setup.tsx:346).
        let save = SetupSave {
            window_style: Some("overlay".to_owned()),
            mods: Some(vec![]),
            ..SetupSave::default()
        };
        assert!(validate_setup_save(&save).is_ok());
        // Each step's real payloads.
        for style in SETUP_WINDOW_STYLES {
            let save = SetupSave {
                window_style: Some((*style).to_owned()),
                ..SetupSave::default()
            };
            assert!(validate_setup_save(&save).is_ok(), "style {style}");
        }
        for module in SETUP_MOD_OPTIONS {
            let save = SetupSave {
                mods: Some(vec![(*module).to_owned()]),
                ..SetupSave::default()
            };
            assert!(validate_setup_save(&save).is_ok(), "mod {module}");
        }
        for tray in SETUP_TRAY_OPTIONS {
            let save = SetupSave {
                tray: Some((*tray).to_owned()),
                ..SetupSave::default()
            };
            assert!(validate_setup_save(&save).is_ok(), "tray {tray}");
        }
        // Finish step.
        let save = SetupSave {
            done_setup: Some(true),
            ..SetupSave::default()
        };
        assert!(validate_setup_save(&save).is_ok());
        // Empty save (no step touched anything) is valid.
        assert!(validate_setup_save(&SetupSave::default()).is_ok());
    }

    #[test]
    fn setup_save_rejects_everything_outside_the_page_vocabulary() {
        // Unknown window style.
        let save = SetupSave {
            window_style: Some("transparent".to_owned()),
            ..SetupSave::default()
        };
        assert!(validate_setup_save(&save).is_err());
        // Stored-code mods, Shelter-as-entry, multi-select, and unknowns.
        for mods in [
            vec!["custom"],
            vec!["shelter"],
            vec!["vencord", "equicord"],
            vec!["../../../etc"],
            vec!["VENCORD"],
        ] {
            let save = SetupSave {
                mods: Some(mods.iter().map(|entry| entry.to_string()).collect()),
                ..SetupSave::default()
            };
            assert!(
                validate_setup_save(&save).is_err(),
                "mods {mods:?} must be refused"
            );
        }
        // Legacy boolean tray and unknown tray names.
        for tray in ["true", "false", "enabled", "static"] {
            let save = SetupSave {
                tray: Some(tray.to_owned()),
                ..SetupSave::default()
            };
            assert!(
                validate_setup_save(&save).is_err(),
                "tray {tray:?} must be refused"
            );
        }
        // Un-completing setup is rejected.
        let save = SetupSave {
            done_setup: Some(false),
            ..SetupSave::default()
        };
        assert!(validate_setup_save(&save).is_err());
    }

    #[test]
    fn css_editor_targets_match_the_ipc_open_sites() {
        // src/discord/ipc.ts openQuickCss opens the fixed Quick CSS file.
        assert_eq!(
            parse_css_editor_target("quickcss", None),
            Ok(CssEditorTarget::QuickCss)
        );
        // editTheme opens one validated theme id.
        assert_eq!(
            parse_css_editor_target("theme", Some("my-theme")),
            Ok(CssEditorTarget::ThemeCss {
                id: "my-theme".to_owned()
            })
        );
    }

    #[test]
    fn css_editor_target_rejects_paths_urls_and_mismatches() {
        // Unknown kinds (including file paths and URLs) never parse.
        for kind in [
            "",
            "file",
            "quickCss.css",
            "C:\\themes\\x",
            "file:///quickCss.css",
            "remcord://themes/x",
            "QUICKCSS",
        ] {
            assert!(
                parse_css_editor_target(kind, None).is_err(),
                "kind {kind:?} must be refused"
            );
        }
        // Traversal, separators, drive syntax, and dot-dots die in id
        // validation before any filesystem work.
        for id in [
            "../quickCss",
            "..",
            ".",
            "a/b",
            "a\\b",
            "C:\\x",
            "C:x",
            "/absolute",
            "",
        ] {
            assert!(
                parse_css_editor_target("theme", Some(id)).is_err(),
                "id {id:?} must be refused"
            );
        }
        // Missing id for theme, stray id for quickcss.
        assert!(parse_css_editor_target("theme", None).is_err());
        assert!(parse_css_editor_target("quickcss", Some("x")).is_err());
    }

    #[test]
    fn startup_log_lines_use_fixed_vocabulary() {
        assert_eq!(
            format_startup_log(StartupFlow::Setup, false),
            "[AuxWindows][Startup] flow=setup show_splash=false background=false"
        );
        assert_eq!(
            format_startup_log(StartupFlow::Main { show_splash: true }, false),
            "[AuxWindows][Startup] flow=main show_splash=true background=false"
        );
        assert_eq!(
            format_startup_log(StartupFlow::Main { show_splash: false }, true),
            "[AuxWindows][Startup] flow=main show_splash=false background=true"
        );
    }

    #[test]
    fn shipped_module_stays_policy_only() {
        let source = include_str!("aux_windows.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // No enforcement surface of its own: no window calls, no WebView2
        // state writes, no settings persistence, no bridge, no script
        // execution, no filesystem, no fetch, no editor machinery.
        // Enforcement lives in settings.rs (persistence), quick_css.rs and
        // themes.rs (editor reads/writes), and the single-window shell
        // (startup); this module only states policy.
        for forbidden in [
            "BrowserWindow",
            "WebView",
            "tao::",
            "wry::",
            "set_visible",
            "ControlFlow::Exit",
            "apply_settings_set",
            "evaluate_script",
            "executeJavaScript",
            "postMessage",
            "window.open",
            "openCssEditor",
            "createSetupWindow",
            "createSplashWindow",
            "std::fs",
            "std::process",
            "ureq",
            "webview2_com",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
        // Delegation is real, not documented-only.
        assert!(shipped.contains("crate::themes::validate_theme_id"));
        assert!(shipped.contains("crate::settings::"));
        assert!(shipped.contains("crate::app::"));
    }
}

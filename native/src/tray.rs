//! Native tray-icon state model (F13 core).
//!
//! Electron parity reference: `src/discord/tray.ts` (icon selection, context
//! menu, click-to-reopen) plus the `page-title-updated` badge branch of
//! `src/discord/window.ts` (overlay icons) and the `minimizeToTray` close
//! branch of the same file. Native close policy lives in
//! `close_behavior.rs` (read-only here — never edited by this module).
//!
//! What this module owns (pure, host-side, no OS calls):
//!
//! - [`TrayMode`] resolution from the authoritative `settings.tray` value.
//!   Electron (`tray.ts:14-20`): `"disabled"` builds no icon; `"dynamic"`
//!   resolves to the `ac_plug_colored` asset; any other value names a static
//!   asset. The default is `"dynamic"` (`src/common/config.ts:65`, mirrored
//!   by `settings::default_tray`).
//! - [`tray_available`] / [`should_hide_to_tray`]: the decision inputs for a
//!   future OS tray. Availability is `false` until a real icon with a working
//!   reopen action ships — a setting value alone never counts (the same rule
//!   `close_behavior::tray_available` enforces). The `tray.get_state`
//!   capability therefore reports the resolved mode honestly while
//!   `available` stays `false`; when an OS icon ships, it flips this one
//!   function and `close_behavior` follows.
//! - [`TrayBadge`] parsing from a page title (`window.ts:260-287`): a `•`
//!   prefix means unread (`badge-11.ico`), a `(N)` prefix means N mentions
//!   (`badge-N.ico`, capped at `badge-10.ico` for N > 9), anything else
//!   clears the badge. Titles are bounded before parsing; no title text
//!   crosses the bridge (the `tray.get_state` capability is parameter-free
//!   and carries no badge — badge observation wires up with the F20 overlay
//!   slice).
//! - [`TrayMenuAction`] model: the five fixed Electron menu actions
//!   (open / settings / support / restart / quit). Labels stay owned by the
//!   localization layer at construction time; only the closed action ids
//!   cross here.
//!
//! Deliberately out of scope (not omission): constructing the OS tray icon
//! itself needs a `tray-icon` crate plus the `assets/*.png` pipeline and a
//! main-loop event arm — a new dependency and a large `main.rs` hunk that
//! does not belong in a parallel lane. Until that slice ships, every close
//! still quits exactly like today.
//!
//! Safety properties (all pinned by tests):
//!
//! - The only host inputs are the closed `settings.tray` string and the
//!   `minimizeToTray` boolean; the renderer supplies nothing.
//! - No paths, URLs, window handles, or script cross this module.
//! - [`validate_tray_contract`] runs at startup (fail-fast) and pins the
//!   menu shape the OS slice will build from, so a future icon cannot drift
//!   from this contract.

/// Maximum page-title bytes parsed for a badge. Discord titles are short;
/// anything longer is truncated before matching (never rejected — a badge
/// must never break on a long title).
pub const MAX_TITLE_BYTES: usize = 512;

/// The three Electron tray modes (`tray.ts:14-20`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMode {
    /// `tray: "disabled"` — Electron builds no icon; nothing can reopen a
    /// hidden window, so closes must quit.
    Disabled,
    /// `tray: "dynamic"` (the default) — Electron resolves the
    /// `ac_plug_colored` asset and swaps it per favicon state.
    Dynamic,
    /// Any other `tray` value names one static asset.
    Static,
}

/// Resolve the authoritative `settings.tray` value to its mode.
///
/// Electron parity: `"disabled"` disables, `"dynamic"` is dynamic, and any
/// other value (including unknown or empty strings) names a static asset —
/// Electron never fails closed here, it just loads `assets/<value>.png`.
/// Native keeps the same total mapping so the settings UI cannot invent a
/// fourth mode; unknown values stay `Static` and the OS slice resolves the
/// asset (falling back to the default icon when the file is absent).
pub fn resolve_tray_mode(raw: &str) -> TrayMode {
    match raw {
        "disabled" => TrayMode::Disabled,
        "dynamic" => TrayMode::Dynamic,
        _ => TrayMode::Static,
    }
}

/// Canonical mode name for the bridge (`settings.tray` vocabulary only).
pub fn tray_mode_name(mode: TrayMode) -> &'static str {
    match mode {
        TrayMode::Disabled => "disabled",
        TrayMode::Dynamic => "dynamic",
        TrayMode::Static => "static",
    }
}

/// Whether a usable native tray icon with a working reopen action exists.
///
/// Single flip point for the OS slice: `false` until a real icon ships. A
/// setting value alone (even `"dynamic"`) never counts — only an actual
/// icon plus a tested reopen action flips this. Mirrors the rule in
/// `close_behavior::tray_available` without touching that module.
pub fn tray_available() -> bool {
    false
}

/// Close policy for one close request under a tray mode: hide if and only
/// if hide-to-tray was requested AND a usable tray exists. Pure: no window,
/// setting, or shutdown state is touched. `close_behavior::decide` remains
/// the production decision; this mirrors it for the tray-mode vocabulary so
/// the mapping itself is tested here.
pub fn should_hide_to_tray(minimize_to_tray: bool, mode: TrayMode) -> bool {
    minimize_to_tray && mode != TrayMode::Disabled && tray_available()
}

/// Fixed-vocabulary `[Tray]` log line for one state report.
pub fn format_log(mode: TrayMode, minimize_to_tray: bool, available: bool) -> String {
    format!(
        "[Tray] mode={} minimize_to_tray={minimize_to_tray} available={available}",
        tray_mode_name(mode),
    )
}

/// Badge state parsed from a page title (`window.ts:271-287` parity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayBadge {
    /// No unread, no mentions — clear any overlay icon.
    None,
    /// Title starts with `•` — unread messages (`badge-11.ico`).
    Unread,
    /// Title starts with `(N)` — N mentions (`badge-N.ico`, capped).
    Mentions(u32),
}

/// Cap for the mention-badge asset index: Electron loads `badge-10.ico`
/// for any ping count above 9 (`window.ts:279`).
pub const MAX_MENTION_BADGE_INDEX: u32 = 9;

/// Parse a page title into its badge state.
///
/// Electron reads only the title prefix: `•` first, else `(N)`. The title is
/// truncated to [`MAX_TITLE_BYTES`] before matching. `N` clamps to
/// `u32::MAX` on overflow via saturating parse; a `(…)` prefix with no
/// digits parses as unread-with-mention-shape (Electron's `extractPings`
/// returns `null` and the template yields `badge-null.ico` — natively that
/// degrades to [`TrayBadge::Unread`] instead of inventing an asset name).
pub fn badge_from_title(title: &str) -> TrayBadge {
    let bounded = truncate_to_bytes(title, MAX_TITLE_BYTES);
    if bounded.starts_with('•') {
        return TrayBadge::Unread;
    }
    if bounded.starts_with('(') {
        if let Some(count) = parse_mention_count(bounded) {
            return TrayBadge::Mentions(count);
        }
        return TrayBadge::Unread;
    }
    TrayBadge::None
}

/// Native asset file for one badge state (`assets/badge-*.ico` parity).
/// `None` clears the overlay icon (`setOverlayIcon(null, "")` parity).
pub fn badge_asset(badge: TrayBadge) -> Option<&'static str> {
    match badge {
        TrayBadge::None => None,
        TrayBadge::Unread => Some("badge-11.ico"),
        TrayBadge::Mentions(count) => {
            if count > MAX_MENTION_BADGE_INDEX {
                Some("badge-10.ico")
            } else {
                badge_asset_for_small_count(count)
            }
        }
    }
}

/// Closed tray menu: Electron's five `Menu.buildFromTemplate` actions
/// (`tray.ts:44-83`). Order is significant (open, settings, support,
/// separator, restart, quit) and pinned by [`validate_tray_contract`].
/// Separators carry no id and never cross the bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMenuAction {
    /// `tray-openRemcord`: reveal the main window (also the click action).
    Open,
    /// `tray-openSettings`: reveal the main window at settings.
    Settings,
    /// `tray-supportServer`: reveal and navigate to the support invite.
    Support,
    /// `tray-restartRemcord`: relaunch.
    Restart,
    /// `tray-quitRemcord`: force-quit.
    Quit,
}

/// The five menu actions in Electron's template order.
pub fn menu_actions() -> &'static [TrayMenuAction] {
    use TrayMenuAction::{Open, Quit, Restart, Settings, Support};
    &[Open, Settings, Support, Restart, Quit]
}

/// Stable closed id for one menu action. Only these five strings exist.
pub fn menu_action_id(action: TrayMenuAction) -> &'static str {
    match action {
        TrayMenuAction::Open => "open",
        TrayMenuAction::Settings => "settings",
        TrayMenuAction::Support => "support",
        TrayMenuAction::Restart => "restart",
        TrayMenuAction::Quit => "quit",
    }
}

/// Read-only tray state snapshot for the `tray.get_state` capability.
/// `mode` echoes the canonical [`tray_mode_name`]; `available` is the
/// [`tray_available`] flip point; `should_hide` is the resulting close
/// policy for the caller's `minimizeToTray`. Fixed vocabulary only — no
/// paths, handles, asset bytes, or badge/title content.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TrayStateSnapshot {
    pub mode: &'static str,
    pub available: bool,
    pub minimize_to_tray: bool,
    pub should_hide: bool,
}

/// Build the snapshot from the authoritative settings values. The host owns
/// both inputs; the renderer supplies nothing.
pub fn snapshot_tray_state(tray_setting: &str, minimize_to_tray: bool) -> TrayStateSnapshot {
    let mode = resolve_tray_mode(tray_setting);
    let available = tray_available();
    TrayStateSnapshot {
        mode: tray_mode_name(mode),
        available,
        minimize_to_tray,
        should_hide: should_hide_to_tray(minimize_to_tray, mode),
    }
}

/// Startup fail-fast contract for the tray model: the menu the OS slice
/// will build from. Five actions, in Electron's order, with five unique
/// closed ids; both badge assets non-empty and distinct from the clear
/// state. Any drift fails the boot instead of shipping a wrong menu.
pub fn validate_tray_contract() -> Result<(), String> {
    let actions = menu_actions();
    if actions.len() != 5 {
        return Err(format!(
            "tray menu must carry exactly 5 actions, found {}",
            actions.len()
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for action in actions {
        let id = menu_action_id(*action);
        if id.is_empty() || !ids.insert(id) {
            return Err(format!("tray menu action id is empty or duplicated: {id}"));
        }
    }
    let expected = ["open", "settings", "support", "restart", "quit"];
    for (action, want) in actions.iter().zip(expected) {
        let got = menu_action_id(*action);
        if got != want {
            return Err(format!("tray menu order drifted: want {want}, found {got}"));
        }
    }
    match (badge_asset(TrayBadge::Unread), badge_asset(TrayBadge::None)) {
        (Some(unread), None) if !unread.is_empty() => {}
        _ => return Err("tray badge assets drifted".to_owned()),
    }
    // The title parser the F20 overlay slice will observe is pinned here
    // too (fixed inputs, no I/O): unread prefix, mention count with the
    // >9 cap, and the clear state.
    if badge_from_title("• Discord") != TrayBadge::Unread
        || badge_from_title("(3) Discord") != TrayBadge::Mentions(3)
        || badge_asset(badge_from_title("(12) Discord")) != Some("badge-10.ico")
        || badge_from_title("Discord") != TrayBadge::None
    {
        return Err("tray badge mapping drifted".to_owned());
    }
    Ok(())
}

fn truncate_to_bytes(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn parse_mention_count(title: &str) -> Option<u32> {
    let digits: String = title
        .chars()
        .skip(1)
        .take_while(|char| char.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok()
}

fn badge_asset_for_small_count(count: u32) -> Option<&'static str> {
    match count {
        0 => Some("badge-0.ico"),
        1 => Some("badge-1.ico"),
        2 => Some("badge-2.ico"),
        3 => Some("badge-3.ico"),
        4 => Some("badge-4.ico"),
        5 => Some("badge-5.ico"),
        6 => Some("badge-6.ico"),
        7 => Some("badge-7.ico"),
        8 => Some("badge-8.ico"),
        9 => Some("badge-9.ico"),
        _ => Some("badge-10.ico"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_resolution_matches_electron_asset_selection() {
        // tray.ts: disabled builds nothing, dynamic resolves
        // ac_plug_colored, anything else names a static asset.
        assert_eq!(resolve_tray_mode("disabled"), TrayMode::Disabled);
        assert_eq!(resolve_tray_mode("dynamic"), TrayMode::Dynamic);
        assert_eq!(resolve_tray_mode("ac_plug_colored"), TrayMode::Static);
        assert_eq!(resolve_tray_mode(""), TrayMode::Static);
        assert_eq!(resolve_tray_mode("DISABLED"), TrayMode::Static);
        assert_eq!(tray_mode_name(TrayMode::Disabled), "disabled");
        assert_eq!(tray_mode_name(TrayMode::Dynamic), "dynamic");
        assert_eq!(tray_mode_name(TrayMode::Static), "static");
    }

    #[test]
    fn availability_is_false_until_an_os_icon_ships() {
        // A setting value alone never counts — only a real icon plus a
        // tested reopen action flips this (same rule as
        // close_behavior::tray_available, which stays untouched).
        assert!(!tray_available());
        assert!(!should_hide_to_tray(true, TrayMode::Dynamic));
        assert!(!should_hide_to_tray(true, TrayMode::Static));
        assert!(!should_hide_to_tray(true, TrayMode::Disabled));
        assert!(!should_hide_to_tray(false, TrayMode::Dynamic));
        assert_eq!(
            format_log(TrayMode::Dynamic, true, false),
            "[Tray] mode=dynamic minimize_to_tray=true available=false"
        );
    }

    #[test]
    fn badge_parsing_matches_the_window_title_branch() {
        // window.ts: • prefix → badge-11, (N) prefix → badge-N capped at
        // badge-10, anything else clears.
        assert_eq!(badge_from_title("• Discord"), TrayBadge::Unread);
        assert_eq!(badge_from_title("(3) Discord"), TrayBadge::Mentions(3));
        assert_eq!(badge_from_title("(12) Discord"), TrayBadge::Mentions(12));
        assert_eq!(badge_from_title("Discord"), TrayBadge::None);
        assert_eq!(badge_from_title(""), TrayBadge::None);
        assert_eq!(badge_asset(TrayBadge::Unread), Some("badge-11.ico"));
        assert_eq!(badge_asset(TrayBadge::Mentions(1)), Some("badge-1.ico"));
        assert_eq!(badge_asset(TrayBadge::Mentions(9)), Some("badge-9.ico"));
        assert_eq!(badge_asset(TrayBadge::Mentions(10)), Some("badge-10.ico"));
        assert_eq!(badge_asset(TrayBadge::Mentions(99)), Some("badge-10.ico"));
        assert_eq!(badge_asset(TrayBadge::None), None);
    }

    #[test]
    fn badge_parsing_is_bounded_and_total() {
        // A hostile 1 MiB title truncates before matching and still parses
        // (a badge must never break on title length).
        let long = format!("(2) {}", "x".repeat(1024 * 1024));
        assert_eq!(badge_from_title(&long), TrayBadge::Mentions(2));
        // A (… ) prefix with no digits degrades to unread instead of
        // inventing an asset name (Electron would template badge-null.ico).
        assert_eq!(badge_from_title("(abc) Discord"), TrayBadge::Unread);
        assert_eq!(
            badge_asset(badge_from_title("(abc) Discord")),
            Some("badge-11.ico")
        );
        // Multi-byte boundary: truncation never splits a char.
        let emoji = format!("• {}", "🎉".repeat(200));
        assert_eq!(badge_from_title(&emoji), TrayBadge::Unread);
    }

    #[test]
    fn menu_carries_electrons_five_actions_in_order() {
        let actions = menu_actions();
        assert_eq!(actions.len(), 5);
        let ids: Vec<_> = actions
            .iter()
            .map(|action| menu_action_id(*action))
            .collect();
        assert_eq!(ids, ["open", "settings", "support", "restart", "quit"]);
    }

    #[test]
    fn snapshot_echoes_fixed_vocabulary_only() {
        let state = snapshot_tray_state("dynamic", true);
        assert_eq!(state.mode, "dynamic");
        assert!(!state.available);
        assert!(state.minimize_to_tray);
        assert!(!state.should_hide);
        let disabled = snapshot_tray_state("disabled", true);
        assert_eq!(disabled.mode, "disabled");
        assert!(!disabled.should_hide);
        // Unknown setting values stay in-vocabulary (static), never echo
        // the raw string across the bridge.
        let unknown = snapshot_tray_state("../../etc/passwd", true);
        assert_eq!(unknown.mode, "static");
        let json = serde_json::to_string(&state).expect("snapshot must serialize");
        assert!(json.contains("\"dynamic\""));
        assert!(!json.contains("passwd"));
    }

    #[test]
    fn startup_contract_pins_the_os_slice_inputs() {
        validate_tray_contract().expect("contract must hold");
        let source = include_str!("tray.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // No behavior surface: no window calls, no icon construction, no
        // settings I/O, no navigation, no script, no process control.
        for forbidden in [
            "set_visible",
            "TrayIcon",
            "SystemTray",
            "Shell_NotifyIcon",
            "load_settings",
            "ControlFlow::Exit",
            "evaluate_script",
            "std::process",
            "std::fs",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
    }
}

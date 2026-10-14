//! Native menus (F15 core).
//!
//! Electron Remcord builds its desktop integration through two menu
//! surfaces (`src/discord/menu.ts`, `src/discord/window.ts`):
//!
//! - **App menu**: `setMenu()` installs five top-level menus — Remcord
//!   (about/services, developer tools, open settings, reload, restart,
//!   hide roles, quit), Edit (undo/redo/cut/copy/paste/select-all), View
//!   (fullscreen, zoom in/out/reset), Window (minimize/close), and
//!   Keybind (one item per enabled + non-global keybind, firing through
//!   menu `click` → `runAction` — focused-app-only by construction).
//! - **Page context menu**: fully suppressed. `window.ts` calls
//!   `preventDefault()` on every `context-menu` event because Discord
//!   renders its own in-app menus (where "Save to Gallery" now lives via
//!   the Gallery shelter plugin calling `gallery.addUrl` — F5) and a
//!   webview menu would only duplicate entries and expose devtools.
//!
//! Native constraints (platform boundary, not omission):
//!
//! - `tao` 0.37 exposes no menu-bar API and `wry` 0.57 exposes no
//!   WebView2 context-menu event, so no Win32 menu bar can be installed
//!   without a new dependency. The native window is frameless
//!   (`with_decorations(false)`), exactly like Electron's
//!   `frame: false` + `autoHideMenuBar` — Electron's own menu bar is
//!   hidden too, and the menu's live function there is the non-global
//!   keybind accelerators, which native already serves through the F8
//!   in-page matcher (menu-accelerator parity, focused-app-only).
//! - Native is Windows-only, so the template carries the Windows
//!   accelerators (`Ctrl+Shift+I` for developer tools). The `Cmd+,`
//!   / `CmdOrCtrl+*` Electron accelerator spellings are preserved
//!   verbatim in the template (the renderer resolves them); nothing is
//!   reinterpreted host-side.
//!
//! Host-ownable slice (this module, pure + capability-backed):
//!
//! - The Electron template mirrored as host-owned data: five fixed menus
//!   with stable item ids. Labels stay renderer-side — the template
//!   carries `labelId` values (`menu-remcord`, `keybind-mute`, …) that
//!   the renderer resolves through the existing `getLang` capability,
//!   exactly like Electron's `getLang("menu-remcord")` calls.
//! - The Keybind submenu built from the F8-validated inventory
//!   (enabled + non-global only, `runJavascript` structurally excluded —
//!   it has no `KeybindAction` variant and no label id here).
//! - The context-menu decision: always [`ContextMenuDecision::Suppress`]
//!   (Electron parity — Discord's in-app menus own every context), plus
//!   [`context_menu_owner`] naming the owning surface per context kind
//!   so a future WebView2 context-menu event has a documented mapping.
//! - One parameter-free read capability (`menus.get_template`) serving
//!   the template above; the renderer supplies nothing.
//!
//! Logical roots owned by this module: none beyond the keybind inventory
//! it reads through `keybinds` (settings file only). No window handle, no
//! WebView, no settings write, no navigation, no script evaluation.

use crate::keybinds::{KeybindAction, KeybindEntry};

/// Fixed context-menu state served in every template: the page context
/// menu is suppressed (Electron parity — see module docs).
pub const CONTEXT_MENU_STATE: &str = "suppressed";

/// Maximum keybind submenu entries. The inventory itself is bounded by
/// `keybinds::MAX_KEYBINDS`; the submenu never exceeds what the
/// inventory can hold.
pub const MAX_KEYBIND_MENU_ENTRIES: usize = 64;

/// What the host does with a page context-menu request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuDecision {
    /// Suppress the native menu (Electron parity: `preventDefault()`).
    Suppress,
}

/// Decide what to do with a page context-menu request for one context
/// kind (`"text"`, `"image"`, `"video"`, `"link"`, …). Always
/// [`ContextMenuDecision::Suppress`]: Discord renders its own in-app
/// menus and a webview menu would duplicate entries and expose
/// devtools. The kind is accepted so callers can classify first; it
/// never changes the outcome.
pub fn context_menu_decision(_kind: &str) -> ContextMenuDecision {
    ContextMenuDecision::Suppress
}

/// Name the surface that owns one context kind, for operators and for a
/// future WebView2 context-menu event. The host builds no menu for any
/// of these today (the decision above suppresses); this documents where
/// each entry already lives.
pub fn context_menu_owner(kind: &str) -> &'static str {
    match kind {
        // Plain text / editable fields: Discord's own in-app menu
        // (Copy/Paste live there; Electron never showed a native one).
        "text" | "editable" | "selection" => "discord-in-app",
        // Images: Discord's in-app menu plus the Gallery shelter plugin
        // saver (`gallery.addUrl` — F5). No native Save-image entry.
        "image" => "discord-in-app+gallery-plugin",
        // Links, video, audio: Discord's in-app menu owns them.
        "link" | "video" | "audio" => "discord-in-app",
        // Anything unrecognized stays suppressed under Discord's menu —
        // never falls through to a native menu.
        _ => "discord-in-app",
    }
}

/// Stable label id for one executable keybind action. Mirrors Electron's
/// `keybindActionLabels` table in `menu.ts` (`mute` → `keybind-mute`,
/// …). `runJavascript` has no entry here — it has no `KeybindAction`
/// variant, so no label id can ever be produced for it.
pub fn keybind_action_label_id(action: KeybindAction) -> &'static str {
    match action {
        KeybindAction::Mute => "keybind-mute",
        KeybindAction::Deafen => "keybind-deafen",
        KeybindAction::PushToTalk => "keybind-pushToTalk",
        KeybindAction::LeaveCall => "keybind-leaveCall",
        KeybindAction::NavigateBack => "keybind-navigateBack",
        KeybindAction::NavigateForward => "keybind-navigateForward",
        KeybindAction::OpenQuickCss => "keybind-openQuickCss",
        KeybindAction::OpenSettings => "keybind-openSettings",
    }
}

/// One keybind submenu entry as JSON: `{kind, id, labelId, accelerator}`.
/// The renderer resolves `labelId` through `getLang` and fires through
/// the existing keybind execution path (F8); the id is the only link.
pub fn keybind_menu_entry_json(entry: &KeybindEntry) -> serde_json::Value {
    serde_json::json!({
        "kind": "keybind",
        "id": entry.id,
        "labelId": keybind_action_label_id(entry.action),
        "accelerator": entry.accelerator,
    })
}

/// Build the Keybind submenu from a validated inventory. Keeps enabled +
/// non-global entries only (Electron's `setMenu` filter: global entries
/// are system hotkeys, disabled entries have no accelerator). Order is
/// inventory order; the result never exceeds
/// [`MAX_KEYBIND_MENU_ENTRIES`].
pub fn build_keybind_menu(entries: &[KeybindEntry]) -> Vec<serde_json::Value> {
    entries
        .iter()
        .filter(|entry| entry.enabled && !entry.global)
        .take(MAX_KEYBIND_MENU_ENTRIES)
        .map(keybind_menu_entry_json)
        .collect()
}

/// Fixed top-level menu ids in Electron's `setMenu` order.
pub const TOP_LEVEL_MENU_IDS: [&str; 5] = ["remcord", "edit", "view", "window", "keybind"];

/// Fixed top-level label ids in the same order (renderer resolves each
/// through `getLang`, exactly like Electron's `getLang("menu-remcord")`).
pub const TOP_LEVEL_MENU_LABEL_IDS: [&str; 5] = [
    "menu-remcord",
    "menu-edit",
    "menu-view",
    "menu-window",
    "menu-keybind",
];

fn role_item(role: &str, label_id: &str) -> serde_json::Value {
    serde_json::json!({"kind": "role", "role": role, "labelId": label_id})
}

fn action_item(id: &str, label_id: &str, accelerator: &str) -> serde_json::Value {
    serde_json::json!({"kind": "action", "id": id, "labelId": label_id, "accelerator": accelerator})
}

fn separator_item() -> serde_json::Value {
    serde_json::json!({"kind": "separator"})
}

/// The four static menus as JSON, mirroring `menu.ts::setMenu()`. The
/// Keybind menu is dynamic ([`build_keybind_menu`]) and not included
/// here. Accelerators are Electron's verbatim spellings; on native
/// (Windows-only) developer tools uses `Ctrl+Shift+I`.
pub fn static_menus_json() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "id": "remcord",
            "labelId": "menu-remcord",
            "items": [
                role_item("about", "menu-about"),
                separator_item(),
                role_item("services", "menu-services"),
                separator_item(),
                action_item("developerTools", "menu-developerTools", "Ctrl+Shift+I"),
                action_item("openSettings", "menu-openSettings", "Cmd+,"),
                action_item("reload", "menu-reload", "CmdOrCtrl+R"),
                action_item("restart", "menu-restart", "CmdOrCtrl+Shift+R"),
                separator_item(),
                role_item("hide", "menu-hide"),
                role_item("hideOthers", "menu-hideOthers"),
                role_item("unhide", "menu-unhide"),
                separator_item(),
                action_item("quit", "menu-quit", "CmdOrCtrl+Q"),
            ],
        }),
        serde_json::json!({
            "id": "edit",
            "labelId": "menu-edit",
            "items": [
                action_item("undo", "menu-undo", "CmdOrCtrl+Z"),
                action_item("redo", "menu-redo", "Shift+CmdOrCtrl+Z"),
                separator_item(),
                role_item("cut", "menu-cut"),
                role_item("copy", "menu-copy"),
                role_item("paste", "menu-paste"),
                role_item("selectAll", "menu-selectAll"),
            ],
        }),
        serde_json::json!({
            "id": "view",
            "labelId": "menu-view",
            "items": [
                role_item("togglefullscreen", "menu-toggleFullscreen"),
                role_item("zoomIn", "menu-zoomIn"),
                serde_json::json!({"kind": "role", "role": "zoomIn", "labelId": "menu-zoomIn", "accelerator": "CmdOrCtrl+=", "visible": false}),
                role_item("zoomOut", "menu-zoomOut"),
                separator_item(),
                role_item("resetZoom", "menu-resetZoom"),
            ],
        }),
        serde_json::json!({
            "id": "window",
            "labelId": "menu-window",
            "items": [
                role_item("minimize", "menu-minimize"),
                role_item("close", "menu-close"),
            ],
        }),
    ]
}

/// Template parts for the `menus.get_template` dispatch arm: the four
/// static menus plus the dynamic Keybind submenu.
pub fn template_parts(
    entries: &[KeybindEntry],
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    (static_menus_json(), build_keybind_menu(entries))
}

/// Startup fail-fast: the served template keeps its contract — five
/// top-level ids in Electron order, four static menus matching the
/// first four ids, renderer-resolvable label ids, an empty-inventory
/// template with the capability shape, and a context menu that stays
/// suppressed for every known kind. Mirrors the `mod_settings`
/// fail-fast precedent: a drifted template aborts startup rather than
/// serving a shape the facade validator would reject.
pub fn validate_menu_contract() -> Result<(), String> {
    if TOP_LEVEL_MENU_IDS.len() != 5 || TOP_LEVEL_MENU_LABEL_IDS.len() != 5 {
        return Err("menu template must carry five top-level menus".to_owned());
    }
    let statics = static_menus_json();
    if statics.len() != 4 {
        return Err("menu template must carry four static menus".to_owned());
    }
    for (index, menu) in statics.iter().enumerate() {
        let id = menu.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let label_id = menu.get("labelId").and_then(|v| v.as_str()).unwrap_or("");
        if id != TOP_LEVEL_MENU_IDS[index] {
            return Err(format!("static menu {index} drifted from its top-level id"));
        }
        if label_id != TOP_LEVEL_MENU_LABEL_IDS[index]
            || !menu.get("items").and_then(|v| v.as_array()).is_some()
        {
            return Err(format!("static menu {id} drifted from its contract shape"));
        }
    }
    if TOP_LEVEL_MENU_IDS[4] != "keybind" || TOP_LEVEL_MENU_LABEL_IDS[4] != "menu-keybind" {
        return Err("keybind submenu id drifted from its contract".to_owned());
    }
    // The context menu stays suppressed for every documented kind —
    // never falls through to a native menu.
    for kind in [
        "text",
        "editable",
        "selection",
        "image",
        "link",
        "video",
        "audio",
        "mystery",
    ] {
        if context_menu_decision(kind) != ContextMenuDecision::Suppress {
            return Err(format!("context menu must suppress {kind}"));
        }
    }
    if context_menu_owner("image") != "discord-in-app+gallery-plugin" {
        return Err("image context ownership drifted from the gallery plugin".to_owned());
    }
    // The empty-inventory template keeps the exact capability shape the
    // facade validator enforces.
    let template = menu_template_json(&[]);
    if template
        .get("menus")
        .and_then(|v| v.as_array())
        .map(|v| v.len())
        != Some(4)
        || template
            .get("keybindMenu")
            .and_then(|v| v.as_array())
            .map(|v| v.len())
            != Some(0)
        || template.get("contextMenu").and_then(|v| v.as_str()) != Some(CONTEXT_MENU_STATE)
    {
        return Err("menu template drifted from its capability shape".to_owned());
    }
    Ok(())
}

/// Full template as JSON: the four static menus, the dynamic Keybind
/// submenu (built from `entries`), and the fixed context-menu state.
/// `{menus, keybindMenu, contextMenu}` — the shape the
/// `menus.get_template` capability serves.
pub fn menu_template_json(entries: &[KeybindEntry]) -> serde_json::Value {
    let (menus, keybind_menu) = template_parts(entries);
    serde_json::json!({
        "menus": menus,
        "keybindMenu": keybind_menu,
        "contextMenu": CONTEXT_MENU_STATE,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_entry(id: &str, action: KeybindAction, global: bool, enabled: bool) -> KeybindEntry {
        KeybindEntry {
            id: id.to_owned(),
            accelerator: "Ctrl+M".to_owned(),
            action,
            global,
            enabled,
            parsed: crate::keybinds::parse_accelerator("Ctrl+M").expect("fixture parses"),
        }
    }

    #[test]
    fn top_level_menus_match_electron_order_and_labels() {
        assert_eq!(
            TOP_LEVEL_MENU_IDS,
            ["remcord", "edit", "view", "window", "keybind"]
        );
        assert_eq!(
            TOP_LEVEL_MENU_LABEL_IDS,
            [
                "menu-remcord",
                "menu-edit",
                "menu-view",
                "menu-window",
                "menu-keybind"
            ]
        );
        let statics = static_menus_json();
        assert_eq!(statics.len(), 4);
        for (menu, expected) in statics.iter().zip(
            TOP_LEVEL_MENU_IDS
                .iter()
                .zip(TOP_LEVEL_MENU_LABEL_IDS.iter()),
        ) {
            assert_eq!(
                menu["id"],
                serde_json::Value::String(expected.0.to_string())
            );
            assert_eq!(
                menu["labelId"],
                serde_json::Value::String(expected.1.to_string())
            );
        }
    }

    #[test]
    fn static_menus_carry_electron_accelerators_and_no_keybind_content() {
        let text = serde_json::to_string(&static_menus_json()).expect("must serialize");
        // Electron-verbatim accelerator spellings.
        for accelerator in [
            "Ctrl+Shift+I",
            "Cmd+,",
            "CmdOrCtrl+R",
            "CmdOrCtrl+Shift+R",
            "CmdOrCtrl+Q",
            "CmdOrCtrl+Z",
            "Shift+CmdOrCtrl+Z",
            "CmdOrCtrl+=",
        ] {
            assert!(text.contains(accelerator), "{accelerator} must be served");
        }
        // The static half never carries keybind entries or the keybind
        // submenu id.
        assert!(!text.contains("\"kind\":\"keybind\""));
        assert!(!text.contains("\"id\":\"keybind\""));
        // QWERTZ/numpad zoom duplicate stays hidden, like Electron.
        assert!(text.contains("\"visible\":false"));
    }

    #[test]
    fn keybind_submenu_keeps_enabled_non_global_entries_only() {
        let entries = vec![
            fixture_entry("local-on", KeybindAction::Mute, false, true),
            fixture_entry("global-on", KeybindAction::Deafen, true, true),
            fixture_entry("local-off", KeybindAction::LeaveCall, false, false),
            fixture_entry("global-off", KeybindAction::OpenSettings, true, false),
        ];
        let menu = build_keybind_menu(&entries);
        assert_eq!(menu.len(), 1);
        assert_eq!(
            menu[0]["id"],
            serde_json::Value::String("local-on".to_owned())
        );
        assert_eq!(
            menu[0]["labelId"],
            serde_json::Value::String("keybind-mute".to_owned())
        );
        assert_eq!(
            menu[0]["accelerator"],
            serde_json::Value::String("Ctrl+M".to_owned())
        );
    }

    #[test]
    fn run_javascript_can_never_appear_in_any_menu() {
        // No variant, no label id, no action string anywhere in the
        // served template.
        assert!(KeybindAction::parse("runJavascript").is_none());
        let entries = vec![fixture_entry("a-1", KeybindAction::Mute, false, true)];
        let text = serde_json::to_string(&menu_template_json(&entries)).expect("must serialize");
        assert!(!text.contains("runJavascript"));
        assert!(!text.contains("keybind-runJavascript"));
        let statics = serde_json::to_string(&static_menus_json()).expect("must serialize");
        assert!(!statics.contains("runJavascript"));
    }

    #[test]
    fn context_menu_is_always_suppressed_with_a_documented_owner() {
        // Electron parity: window.ts preventDefault()s every context-menu
        // event. The kind never changes the outcome.
        for kind in [
            "text",
            "editable",
            "selection",
            "image",
            "link",
            "video",
            "audio",
            "",
            "mystery",
        ] {
            assert_eq!(context_menu_decision(kind), ContextMenuDecision::Suppress);
        }
        assert_eq!(context_menu_owner("image"), "discord-in-app+gallery-plugin");
        assert_eq!(context_menu_owner("text"), "discord-in-app");
        assert_eq!(context_menu_owner("link"), "discord-in-app");
        assert_eq!(context_menu_owner("mystery-kind"), "discord-in-app");
        assert_eq!(CONTEXT_MENU_STATE, "suppressed");
    }

    #[test]
    fn template_json_has_the_capability_shape() {
        let entries = vec![fixture_entry(
            "a-1",
            KeybindAction::OpenSettings,
            false,
            true,
        )];
        let template = menu_template_json(&entries);
        assert!(template.get("menus").and_then(|v| v.as_array()).is_some());
        assert_eq!(template["menus"].as_array().map(|v| v.len()), Some(4));
        assert_eq!(template["keybindMenu"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(
            template["contextMenu"],
            serde_json::Value::String("suppressed".to_owned())
        );
        // Every label id resolves through getLang renderer-side; the host
        // never serves a localized string.
        let text = serde_json::to_string(&template).expect("must serialize");
        assert!(text.contains("\"labelId\":\"menu-remcord\""));
        assert!(text.contains("\"labelId\":\"keybind-openSettings\""));
    }

    #[test]
    fn startup_contract_holds_for_the_shipped_template() {
        // The same check main() runs at boot: five top-level ids, four
        // static menus, suppressed context, capability-shaped template.
        assert!(validate_menu_contract().is_ok());
    }

    #[test]
    fn shipped_module_has_no_forbidden_surfaces() {
        let source = include_str!("menus.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // No behavior surface: no window calls, no settings writes, no
        // navigation, no script evaluation, no network, no locale reads —
        // labels resolve renderer-side through getLang.
        for forbidden in [
            "set_visible",
            "evaluate_script",
            "ControlFlow::Exit",
            "TrayIcon",
            "SystemTray",
            "set_minimized",
            "set_maximized",
            "Navigate(",
            "Reload(",
            "std::process",
            "std::fs",
            "fetch(",
            "eval(",
            "require(",
            "http://",
            "https://",
            "load_settings",
            "persist",
            "MenuBar",
            "popup_menu",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
    }
}

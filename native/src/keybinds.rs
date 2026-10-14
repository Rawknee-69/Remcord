//! Native keybind execution (F8 core).
//!
//! This module is the native counterpart of `src/discord/globalKeybinds.ts`
//! (registration), `src/common/keybindActions.ts` (the nine actions), and
//! the `addKeybind` / `toggleKeybind` / `removeKeybind` handlers in
//! `src/discord/ipc.ts` (exposed as fire-and-forget `send`s through
//! `src/discord/preload/bridge.ts`).
//!
//! Traced Electron behavior (actual implementation, not declarations):
//! - A keybind is `{accelerator, action, global, enabled, id, js?}` where
//!   `accelerator` is an Electron Accelerator string, `action` is one of
//!   nine `KeybindActions`, and `js` rides along only for `runJavascript`.
//! - Management is three sync `send`s that mutate `settings.keybinds` and
//!   call `refreshGlobalKeybinds()` (`unregisterAll` + re-register every
//!   enabled + global entry via `globalShortcut.register`, silently
//!   skipping failures, then `setMenu()`).
//! - Only enabled + **global** entries become system-wide hotkeys.
//!   Enabled + **non-global** entries become app-menu accelerators
//!   instead (`menu.ts::setMenu`, firing through menu `click` → one
//!   `runAction` call each — focused-app-only by construction).
//! - Actions run through `runAction`: voice toggles dispatch fixed Shelter
//!   flux events page-side (`AUDIO_TOGGLE_SELF_MUTE`,
//!   `AUDIO_TOGGLE_SELF_DEAF`, `VOICE_CHANNEL_SELECT` with null channel,
//!   `PUSH_TO_TALK_STATE_CHANGE` active-only and latching); navigation runs
//!   history back/forward; `openQuickCss` opens the CSS file in the system
//!   editor; `openSettings` pushes `/settings/remcord-settings` history
//!   state; `runJavascript` executes the stored `js` string in the page.
//!   (`pushToTalk` is disabled in Electron's own maker UI — "requires
//!   additional logic" — but the action shape still exists.)
//!
//! Native contract (deliberate, documented differences from Electron):
//! - Management is three typed async capabilities (`keybinds.add` /
//!   `keybinds.toggle` / `keybinds.remove`) plus a read-only inventory
//!   (`keybinds.list`), all over the native `settings.keybinds` array
//!   through the same authoritative-load + atomic-persist path as every
//!   other settings mutation. Every management call re-registers the
//!   global set (Electron's refresh model).
//! - Global hotkeys use Win32 `RegisterHotKey` on a dedicated worker
//!   thread (its own message loop; firings cross to the event loop as
//!   `KeybindFired`). Registration conflicts fail per-keybind and loud,
//!   never aborting the set. No hooks, no `SendInput`, no synthetic key
//!   events anywhere.
//! - Non-global keybinds have no native menu to attach to (F15 missing),
//!   so a document-start matcher script compares `keydown` events against
//!   the enabled non-global inventory served by `keybinds.list` —
//!   focused-app-only, exactly like menu accelerators. Matches arrive as
//!   one-way `keybind_matched` events carrying only the id.
//! - Every firing — global or matched — is re-validated host-side
//!   (known id, enabled, correct scope) before execution. Actions run
//!   through fixed page scripts (flux dispatch, history calls, the F4
//!   QuickCSS handoff) or fixed host operations (WebView navigation for
//!   `openSettings`); the renderer supplies only the id.
//! - `runJavascript` is STRUCTURALLY EXCLUDED, permanently: it executes
//!   arbitrary stored JS, which no bridge may transport. The action has
//!   no wire shape (parse refuses it), a `js` member on any request is
//!   refused by `deny_unknown_fields`, and stored entries carrying either
//!   are never executed — the worker skips them loudly at registration.
//! - Voice actions (`mute`, `deafen`, `leaveCall`, `pushToTalk`) dispatch
//!   the exact Electron flux events page-side. They are harmless without
//!   an active call (Discord ignores them); full voice parity stays
//!   deferred with F10, and `pushToTalk` keeps Electron's press-only
//!   latching shape.
//!
//! Logical roots owned by this module: the native settings file (keybind
//! array only, via `settings.rs`) and the Win32 hotkey table (worker
//! thread). Renderer input is data only (ids, accelerators, actions,
//! flags) — never code, paths, or URLs.

use std::path::Path;

/// The eight executable actions. `runJavascript` is deliberately absent:
/// no variant, no string, no path to it (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeybindAction {
    Mute,
    Deafen,
    LeaveCall,
    PushToTalk,
    NavigateBack,
    NavigateForward,
    OpenQuickCss,
    OpenSettings,
}

impl KeybindAction {
    pub fn as_str(self) -> &'static str {
        match self {
            KeybindAction::Mute => "mute",
            KeybindAction::Deafen => "deafen",
            KeybindAction::LeaveCall => "leaveCall",
            KeybindAction::PushToTalk => "pushToTalk",
            KeybindAction::NavigateBack => "navigateBack",
            KeybindAction::NavigateForward => "navigateForward",
            KeybindAction::OpenQuickCss => "openQuickCss",
            KeybindAction::OpenSettings => "openSettings",
        }
    }

    pub fn parse(value: &str) -> Option<KeybindAction> {
        match value {
            "mute" => Some(KeybindAction::Mute),
            "deafen" => Some(KeybindAction::Deafen),
            "leaveCall" => Some(KeybindAction::LeaveCall),
            "pushToTalk" => Some(KeybindAction::PushToTalk),
            "navigateBack" => Some(KeybindAction::NavigateBack),
            "navigateForward" => Some(KeybindAction::NavigateForward),
            "openQuickCss" => Some(KeybindAction::OpenQuickCss),
            "openSettings" => Some(KeybindAction::OpenSettings),
            _ => None,
        }
    }
}

/// Maximum keybinds stored (Electron: unbounded array).
pub const MAX_KEYBINDS: usize = 64;
/// Maximum accelerator length in bytes (Electron: unbounded string).
pub const MAX_ACCELERATOR_BYTES: usize = 32;
/// Maximum keybind id length in bytes (mirrors the plugin-id bound).
pub const MAX_KEYBIND_ID_BYTES: usize = 64;

/// Win32 hotkey modifier flags (`WinUser.h`). `MOD_NOREPEAT` is
/// deliberately NOT used: Electron repeats on hold, so parity repeats.
pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;

/// One parsed accelerator: modifier flags plus one virtual-key code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedAccelerator {
    pub modifiers: u32,
    pub vk: u32,
}

/// Canonical non-modifier key tokens. The maker records raw `event.key`
/// values, so arrow keys arrive in `ArrowUp` form; both spellings parse
/// to one canonical token (matching compares parsed forms, never raw
/// strings, so maker release-order noise cannot break a match).
fn canonical_key_token(token: &str) -> Option<(&'static str, u32)> {
    let upper = token.to_ascii_uppercase();
    // Single printable characters (letters/digits/punctuation).
    if token.len() == 1 {
        let ch = upper.as_bytes()[0];
        let vk = match ch {
            b'A'..=b'Z' => ch as u32,
            b'0'..=b'9' => ch as u32,
            b' ' => 0x20,
            _ => fixed_punctuation_vk(ch)?,
        };
        return Some((key_token_name(ch), vk));
    }
    match upper.as_str() {
        "SPACE" => Some(("Space", 0x20)),
        "TAB" => Some(("Tab", 0x09)),
        "ENTER" | "RETURN" => Some(("Enter", 0x0D)),
        "ESC" | "ESCAPE" => Some(("Escape", 0x1B)),
        "BACKSPACE" => Some(("Backspace", 0x08)),
        "DELETE" | "DEL" => Some(("Delete", 0x2E)),
        "INSERT" | "INS" => Some(("Insert", 0x2D)),
        "HOME" => Some(("Home", 0x24)),
        "END" => Some(("End", 0x23)),
        "PAGEUP" | "PGUP" => Some(("PageUp", 0x21)),
        "PAGEDOWN" | "PGDN" => Some(("PageDown", 0x22)),
        "UP" | "ARROWUP" => Some(("Up", 0x26)),
        "DOWN" | "ARROWDOWN" => Some(("Down", 0x28)),
        "LEFT" | "ARROWLEFT" => Some(("Left", 0x25)),
        "RIGHT" | "ARROWRIGHT" => Some(("Right", 0x27)),
        _ => {
            // F1..F24.
            let digits = upper.strip_prefix('F')?;
            let n: u32 = digits.parse().ok()?;
            if (1..=24).contains(&n) {
                return Some(("F-key", 0x70 + n - 1));
            }
            None
        }
    }
}

/// Canonical display name for a single-character key (letters/digits keep
/// their uppercase form; space and punctuation keep readable names).
fn key_token_name(ch: u8) -> &'static str {
    match ch {
        b' ' => "Space",
        b';' => "Semicolon",
        b'=' => "Equal",
        b',' => "Comma",
        b'-' => "Minus",
        b'.' => "Period",
        b'/' => "Slash",
        b'`' => "Backquote",
        b'[' => "BracketLeft",
        b'\\' => "Backslash",
        b']' => "BracketRight",
        b'\'' => "Quote",
        _ => "Char",
    }
}

/// Virtual-key codes for US-layout punctuation (fixed table, documented;
/// non-US layouts may differ — the accelerator still registers, but the
/// physical key follows the layout, exactly like Electron).
fn fixed_punctuation_vk(ch: u8) -> Option<u32> {
    match ch {
        b';' => Some(0xBA),
        b'=' => Some(0xBB),
        b',' => Some(0xBC),
        b'-' => Some(0xBD),
        b'.' => Some(0xBE),
        b'/' => Some(0xBF),
        b'`' => Some(0xC0),
        b'[' => Some(0xDB),
        b'\\' => Some(0xDC),
        b']' => Some(0xDD),
        b'\'' => Some(0xDE),
        _ => None,
    }
}

/// Modifier token to its Win32 flag. `Command` maps to Control on
/// Windows (Electron's `CommandOrControl` behavior there); `AltGr` maps
/// to Alt. Anything else (including numpad tokens, which Electron's own
/// maker marks invalid) is refused.
fn modifier_flag(token: &str) -> Option<u32> {
    match token.to_ascii_uppercase().as_str() {
        "CONTROL" | "CTRL" | "COMMANDORCONTROL" | "CMDORCTRL" | "COMMAND" | "CMD" => {
            Some(MOD_CONTROL)
        }
        "ALT" | "OPTION" | "ALTGR" => Some(MOD_ALT),
        "SHIFT" => Some(MOD_SHIFT),
        "SUPER" | "META" | "WIN" | "WINDOWS" => Some(MOD_WIN),
        _ => None,
    }
}

/// Parses one Electron Accelerator string into modifier flags plus one
/// virtual-key code. Grammar: `Modifier+...+Key`, order-insensitive, no
/// duplicates, exactly one non-modifier key, at most one of each
/// modifier, at most 4 tokens total. `runJavascript` accelerators parse
/// the same way — exclusion happens on the action, not the keys.
pub fn parse_accelerator(accelerator: &str) -> Result<ParsedAccelerator, String> {
    if accelerator.is_empty() || accelerator.len() > MAX_ACCELERATOR_BYTES {
        return Err("accelerator must be 1..=32 bytes".to_owned());
    }
    let tokens: Vec<&str> = accelerator.split('+').collect();
    if tokens.len() > 4 || tokens.iter().any(|token| token.is_empty()) {
        return Err("accelerator has an unusable shape".to_owned());
    }
    let mut modifiers: u32 = 0;
    let mut seen_modifiers: u32 = 0;
    let mut key: Option<u32> = None;
    for token in &tokens {
        if let Some(flag) = modifier_flag(token) {
            if seen_modifiers & flag != 0 {
                return Err("accelerator repeats a modifier".to_owned());
            }
            seen_modifiers |= flag;
            modifiers |= flag;
        } else if canonical_key_token(token).is_some() {
            if key.is_some() {
                return Err("accelerator carries more than one key".to_owned());
            }
            key = canonical_key_token(token).map(|(_, vk)| vk);
        } else {
            return Err(format!("accelerator carries an unknown token: {token}"));
        }
    }
    match key {
        Some(vk) => Ok(ParsedAccelerator { modifiers, vk }),
        None => Err("accelerator needs exactly one non-modifier key".to_owned()),
    }
}

/// Keybind id rule: `[A-Za-z0-9_-]{1,64}`, never `.`/`..` (same shape as
/// plugin ids; the maker sends shelter `genId()` values through it).
pub fn validate_keybind_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > MAX_KEYBIND_ID_BYTES {
        return Err("keybind id must be 1..=64 bytes".to_owned());
    }
    if id == "." || id == ".." {
        return Err("keybind id must not be a relative path segment".to_owned());
    }
    if !id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("keybind id uses characters outside [A-Za-z0-9_-]".to_owned());
    }
    Ok(())
}

/// Typed failure: a stable machine-readable `code` plus a human message
/// that never contains a path, URL, or code text.
#[derive(Debug, Clone, PartialEq)]
pub struct KeybindFailure {
    pub code: &'static str,
    pub message: String,
}

fn failure(code: &'static str, message: impl Into<String>) -> KeybindFailure {
    KeybindFailure {
        code,
        message: message.into(),
    }
}

/// One validated keybind entry. Field names use Electron's exact keys so
/// bridge JSON stays shape-compatible with `KeybindCard`/`KeybindMaker`.
#[derive(Debug, Clone, PartialEq)]
pub struct KeybindEntry {
    pub id: String,
    pub accelerator: String,
    pub action: KeybindAction,
    pub global: bool,
    pub enabled: bool,
    pub parsed: ParsedAccelerator,
}

fn entry_from_json(value: &serde_json::Value) -> Option<KeybindEntry> {
    let id = value.get("id")?.as_str()?;
    if validate_keybind_id(id).is_err() {
        return None;
    }
    let accelerator = value.get("accelerator")?.as_str()?;
    let parsed = parse_accelerator(accelerator).ok()?;
    let action = KeybindAction::parse(value.get("action")?.as_str()?)?;
    Some(KeybindEntry {
        id: id.to_owned(),
        accelerator: accelerator.to_owned(),
        action,
        global: value.get("global")?.as_bool()?,
        enabled: value.get("enabled")?.as_bool()?,
        parsed,
    })
}

fn entry_to_json(entry: &KeybindEntry) -> serde_json::Value {
    serde_json::json!({
        "id": entry.id,
        "accelerator": entry.accelerator,
        "action": entry.action.as_str(),
        "global": entry.global,
        "enabled": entry.enabled,
    })
}

/// Reads the validated keybind inventory from the native settings.
/// Entries that fail validation (unknown action — including
/// `runJavascript` — unparsable accelerator, bad id) are skipped loudly
/// at registration, never executed. Malformed non-array `keybinds`
/// members are an error, never silently treated as empty.
pub fn list_keybinds(profile_dir: &Path) -> Result<Vec<KeybindEntry>, KeybindFailure> {
    let load = crate::settings::load_settings(profile_dir)
        .map_err(|message| failure("KEYBINDS_UNAVAILABLE", message))?;
    let raw = load.snapshot.keybinds;
    let mut out = Vec::new();
    for value in raw.iter().take(MAX_KEYBINDS + 1) {
        if let Some(entry) = entry_from_json(value) {
            out.push(entry);
        }
    }
    if raw.len() > MAX_KEYBINDS {
        return Err(failure(
            "KEYBINDS_UNAVAILABLE",
            "too many keybinds are stored",
        ));
    }
    Ok(out)
}

fn persist_keybinds(profile_dir: &Path, entries: &[KeybindEntry]) -> Result<usize, KeybindFailure> {
    let values: Vec<serde_json::Value> = entries.iter().map(entry_to_json).collect();
    crate::settings::write_keybinds(profile_dir, &values)
        .map_err(|message| failure("KEYBINDS_UNAVAILABLE", message))
}

/// Adds one keybind: validates id/accelerator/action/flags, refuses
/// duplicates, persists, and returns the stored entry. A `js` member can
/// never arrive here (protocol `deny_unknown_fields`); `runJavascript`
/// has no parse shape so it fails as an unknown action.
pub fn add_keybind(
    profile_dir: &Path,
    id: &str,
    accelerator: &str,
    action: &str,
    global: bool,
    enabled: bool,
) -> Result<KeybindEntry, KeybindFailure> {
    validate_keybind_id(id).map_err(|message| failure("INVALID_KEYBIND", message))?;
    let parsed =
        parse_accelerator(accelerator).map_err(|message| failure("INVALID_KEYBIND", message))?;
    let action = KeybindAction::parse(action)
        .ok_or_else(|| failure("INVALID_KEYBIND", "keybind action is not executable"))?;
    let mut entries = list_keybinds(profile_dir)?;
    if entries.iter().any(|entry| entry.id == id) {
        return Err(failure(
            "KEYBIND_EXISTS",
            "a keybind with this id already exists",
        ));
    }
    if entries.len() >= MAX_KEYBINDS {
        return Err(failure("KEYBINDS_FULL", "too many keybinds are stored"));
    }
    let entry = KeybindEntry {
        id: id.to_owned(),
        accelerator: accelerator.to_owned(),
        action,
        global,
        enabled,
        parsed,
    };
    entries.push(entry.clone());
    persist_keybinds(profile_dir, &entries)?;
    Ok(entry)
}

/// Flips one keybind's enablement and returns the new state. Unknown ids
/// fail typed without writing.
pub fn toggle_keybind(profile_dir: &Path, id: &str) -> Result<bool, KeybindFailure> {
    validate_keybind_id(id).map_err(|_| failure("NOT_FOUND", "keybind was not found"))?;
    let mut entries = list_keybinds(profile_dir)?;
    let entry = entries
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or_else(|| failure("NOT_FOUND", "keybind was not found"))?;
    entry.enabled = !entry.enabled;
    let enabled = entry.enabled;
    persist_keybinds(profile_dir, &entries)?;
    Ok(enabled)
}

/// Removes one keybind. Unknown ids fail typed without writing.
pub fn remove_keybind(profile_dir: &Path, id: &str) -> Result<(), KeybindFailure> {
    validate_keybind_id(id).map_err(|_| failure("NOT_FOUND", "keybind was not found"))?;
    let mut entries = list_keybinds(profile_dir)?;
    let before = entries.len();
    entries.retain(|entry| entry.id != id);
    if entries.len() == before {
        return Err(failure("NOT_FOUND", "keybind was not found"));
    }
    persist_keybinds(profile_dir, &entries)?;
    Ok(())
}

/// One hotkey-table sync for the worker thread: the full replacement
/// set of bindings. The worker unregisters its table and registers
/// exactly this set (Electron's `unregisterAll` + re-register model).
#[derive(Debug)]
pub struct HotkeySync(pub Vec<HotkeyBinding>);

/// One hotkey-table binding for the worker thread: the keybind id plus
/// its parsed Win32 modifier flags and virtual-key code. Ids never reach
/// Win32: the worker maps its own table index to the hotkey id and
/// reports back only the keybind id.
#[derive(Debug, Clone)]
pub struct HotkeyBinding {
    pub id: String,
    pub modifiers: u32,
    pub vk: u32,
}

/// Enabled + global entries for hotkey registration, in stored order.
/// Entries that cannot parse (including `runJavascript` survivors from
/// hand-edited files) are already excluded by [`list_keybinds`].
pub fn global_hotkey_set(entries: &[KeybindEntry]) -> Vec<&KeybindEntry> {
    entries
        .iter()
        .filter(|entry| entry.enabled && entry.global)
        .collect()
}

/// Enabled + global entries as worker bindings, in stored order.
pub fn global_hotkey_bindings(entries: &[KeybindEntry]) -> Vec<HotkeyBinding> {
    global_hotkey_set(entries)
        .into_iter()
        .map(|entry| HotkeyBinding {
            id: entry.id.clone(),
            modifiers: entry.parsed.modifiers,
            vk: entry.parsed.vk,
        })
        .collect()
}

/// Builds the fixed page script executing one action. Voice actions
/// dispatch Electron's exact flux event shapes; navigation runs history
/// calls; QuickCSS dispatches the F4 handoff (remcord mode); settings
/// navigation pushes [`KEYBIND_OPEN_SETTINGS_PATH`] exactly like
/// Electron's `navigateTo` (history pushState + popstate). No input is
/// interpolated: the action selects one constant template, and the one
/// constant carrying the path is built from the constant itself.
pub fn keybind_action_script(action: KeybindAction) -> String {
    match action {
        KeybindAction::Mute => r#"(()=>{try{window.shelter.flux.dispatcher.dispatch({type:"MEDIA_ENGINE_SET_AUDIO_ENABLED",enabled:true,unmute:true});window.shelter.flux.dispatcher.dispatch({type:"AUDIO_TOGGLE_SELF_MUTE",context:"default",syncRemote:true,playSoundEffect:true});}catch(e){console.error("[Remcord native][keybinds] mute failed",e);}})();"#.to_owned(),
        KeybindAction::Deafen => r#"(()=>{try{window.shelter.flux.dispatcher.dispatch({type:"MEDIA_ENGINE_SET_AUDIO_ENABLED",enabled:true,unmute:true});window.shelter.flux.dispatcher.dispatch({type:"AUDIO_TOGGLE_SELF_DEAF",context:"default",syncRemote:true});}catch(e){console.error("[Remcord native][keybinds] deafen failed",e);}})();"#.to_owned(),
        KeybindAction::LeaveCall => r#"(()=>{try{window.shelter.flux.dispatcher.dispatch({type:"VOICE_CHANNEL_SELECT",channelId:null,currentVoiceChannelId:"",video:false,stream:false});}catch(e){console.error("[Remcord native][keybinds] leaveCall failed",e);}})();"#.to_owned(),
        KeybindAction::PushToTalk => r#"(()=>{try{window.shelter.flux.dispatcher.dispatch({type:"PUSH_TO_TALK_STATE_CHANGE",isActive:true,isPriority:false});}catch(e){console.error("[Remcord native][keybinds] pushToTalk failed",e);}})();"#.to_owned(),
        KeybindAction::NavigateBack => r#"(()=>{try{history.back();}catch(e){console.error("[Remcord native][keybinds] navigateBack failed",e);}})();"#.to_owned(),
        KeybindAction::NavigateForward => r#"(()=>{try{history.forward();}catch(e){console.error("[Remcord native][keybinds] navigateForward failed",e);}})();"#.to_owned(),
        KeybindAction::OpenQuickCss => r#"(()=>{try{if(window.dispatchEvent(new CustomEvent("remcord-native:open-quickcss-editor",{cancelable:true,detail:{mode:"remcord"}}))!==false){alert("Could not open the QuickCSS editor: the Remcord settings plugin is not running.");}}catch(e){console.error("[Remcord native][keybinds] openQuickCss failed",e);}})();"#.to_owned(),
        KeybindAction::OpenSettings => {
            let path = serde_json::to_string(KEYBIND_OPEN_SETTINGS_PATH)
                .expect("a static settings path must serialize to JSON");
            format!(
                r#"(()=>{{try{{history.pushState({{ }},null,{path});window.dispatchEvent(new PopStateEvent("popstate",{{}}));}}catch(e){{console.error("[Remcord native][keybinds] openSettings failed",e);}}}})();"#
            )
        }
    }
}

/// Fixed settings path for the `openSettings` action, embedded in its
/// page script. Mirrors Electron's `navigateTo(window,
/// "/settings/remcord-settings")`; a test pins the script carries exactly
/// this path and no interpolation surface.
pub const KEYBIND_OPEN_SETTINGS_PATH: &str = "/settings/remcord-settings";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerator_grammar_parses_documented_shapes() {
        let parsed = parse_accelerator("Ctrl+Shift+M").unwrap();
        assert_eq!(parsed.modifiers, MOD_CONTROL | MOD_SHIFT);
        assert_eq!(parsed.vk, 0x4D);
        // Order-insensitive, case-insensitive.
        assert_eq!(parse_accelerator("shift+ctrl+m").unwrap(), parsed);
        assert_eq!(parse_accelerator("CommandOrControl+F7").unwrap().vk, 0x76);
        assert_eq!(parse_accelerator("Alt+Space").unwrap().vk, 0x20);
        assert_eq!(parse_accelerator("Super+Up").unwrap().modifiers, MOD_WIN);
        assert_eq!(parse_accelerator("Meta+ArrowDown").unwrap().vk, 0x28);
    }

    #[test]
    fn accelerator_grammar_rejects_hostile_shapes() {
        for bad in [
            "",
            "Ctrl",
            "Shift+Ctrl",
            "Ctrl+M+M",
            "Ctrl+Ctrl+M",
            "Ctrl+Foo",
            "Ctrl+Num0",
            "Ctrl+Shift+Alt+Win+X+Y",
            "Ctrl+",
            "+M",
            "javascript:alert(1)",
            "C:\\secret",
            "Ctrl+é",
        ] {
            assert!(parse_accelerator(bad).is_err(), "{bad}");
        }
        assert!(parse_accelerator(&"M".repeat(33)).is_err());
        // A bare key parses (Electron permits modifier-less accelerators);
        // holding it repeats the action, exactly like globalShortcut.
        assert_eq!(parse_accelerator("M").unwrap().modifiers, 0);
    }

    #[test]
    fn run_javascript_has_no_executable_shape() {
        assert!(KeybindAction::parse("runJavascript").is_none());
        for action in [
            "mute",
            "deafen",
            "leaveCall",
            "pushToTalk",
            "navigateBack",
            "navigateForward",
            "openQuickCss",
            "openSettings",
        ] {
            assert!(KeybindAction::parse(action).is_some(), "{action}");
        }
    }

    #[test]
    fn keybind_ids_use_the_closed_shape() {
        assert!(validate_keybind_id("abc-1_X").is_ok());
        for bad in ["", "../x", "a/b", "C:\\x", &"x".repeat(65)] {
            assert!(validate_keybind_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn action_scripts_are_fixed_templates_with_electron_shapes() {
        for action in [
            KeybindAction::Mute,
            KeybindAction::Deafen,
            KeybindAction::LeaveCall,
            KeybindAction::PushToTalk,
            KeybindAction::NavigateBack,
            KeybindAction::NavigateForward,
            KeybindAction::OpenQuickCss,
            KeybindAction::OpenSettings,
        ] {
            let script = keybind_action_script(action);
            assert!(!script.is_empty());
            // No interpolation surface: fixed templates only.
            assert!(!script.contains("request_id"));
            assert!(!script.contains("payload"));
        }
        let mute = keybind_action_script(KeybindAction::Mute);
        assert!(mute.contains("AUDIO_TOGGLE_SELF_MUTE"));
        let settings = keybind_action_script(KeybindAction::OpenSettings);
        assert!(settings.contains(KEYBIND_OPEN_SETTINGS_PATH));
        let quickcss = keybind_action_script(KeybindAction::OpenQuickCss);
        assert!(quickcss.contains("remcord-native:open-quickcss-editor"));
    }

    #[test]
    fn stored_run_javascript_entries_never_list() {
        // A hand-edited settings file carrying runJavascript (or an
        // unparsable accelerator) lists cleanly without it: the entry is
        // skipped, never executed.
        let raw = serde_json::json!([
            {"id": "evil", "accelerator": "Ctrl+E", "action": "runJavascript", "global": true, "enabled": true, "js": "alert(1)"},
            {"id": "ok1", "accelerator": "Ctrl+O", "action": "mute", "global": true, "enabled": true},
        ]);
        let entries: Vec<KeybindEntry> = raw
            .as_array()
            .unwrap()
            .iter()
            .filter_map(entry_from_json)
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "ok1");
    }
}

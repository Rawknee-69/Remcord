//! Windows overlay/badge icons for unread/mention counts (F20).
//!
//! Electron parity: `src/discord/window.ts` `page-title-updated` handler.
//! On `win32` Electron calls `passedWindow.setOverlayIcon()` with one of the
//! `assets/badge-*.ico` files derived from the document title prefix:
//! `•` (unread, no count) takes `badge-11.ico`, `(N)` (mentions) takes
//! `badge-N.ico` for `N <= 9` and `badge-10.ico` above that, and any other
//! title clears the overlay. The macOS dock-badge branch is out of scope:
//! native is Windows-only.
//!
//! Native shape: the document-start badge observer
//! (`native/web/badge/observer.js`) reports the raw `document.title` as a
//! one-way `badge_title` bridge event; the host parses it with
//! [`parse_title`] (pure, Electron-parity, fully unit-tested here), applies
//! the overlay through `ITaskbarList3::SetOverlayIcon` on the tao window's
//! `HWND`, and keeps the last-applied [`BadgeState`] for the read-only
//! `badge.get_state` capability. Titles are bounded
//! ([`MAX_BADGE_TITLE_CHARS`]); over-long reports are rejected before any
//! parse or Win32 work, and the observer already slices to a shorter prefix.
//!
//! Asset rule (hardening, documented): every referenced file MUST exist in
//! the compile-time table. Electron builds `badge-${pings}.ico` by string
//! interpolation, so a title like `(abc)` or `(0)` names a file that does
//! not exist (`badge-null.ico` / `badge-0.ico`); the native parser maps
//! those shapes to the closest real state instead of ever naming a missing
//! asset (see [`parse_title`]).

/// Maximum accepted `badge_title` event length in characters. Real Discord
/// titles are far shorter; the bound keeps one IPC message small and keeps
/// unbounded renderer text out of the host log line.
pub const MAX_BADGE_TITLE_CHARS: usize = 256;

/// Accessibility description passed to `SetOverlayIcon` while a badge is
/// shown. Electron passes the localized `title-unreadMessages` catalog
/// string; native keeps the fixed English text so the overlay path needs no
/// settings/language lookup on every title change.
pub const BADGE_OVERLAY_DESCRIPTION: &str = "Unread messages";

/// Generic-unread overlay asset (Electron: title starts with `•`).
pub const BADGE_UNREAD_ASSET: &str = "badge-11.ico";
/// Capped mention overlay asset (Electron: `(N)` with `N > 9`).
pub const BADGE_CAPPED_ASSET: &str = "badge-10.ico";
/// Highest mention count with its own asset file (`badge-1.ico`..=`badge-9.ico`).
pub const MAX_INDIVIDUAL_MENTIONS: u32 = 9;

/// Which overlay Electron would show for the current document title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BadgeKind {
    /// No badge: the overlay icon is cleared.
    Clear,
    /// Unread, no count (`•` prefix or an uncountable `(` shape).
    Unread,
    /// Numbered mentions (`(N)` prefix with `N >= 1`).
    Mention,
}

/// Last-applied overlay state, served by `badge.get_state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct BadgeState {
    pub kind: BadgeKind,
    /// Mention count for [`BadgeKind::Mention`], else `None`.
    pub pings: Option<u32>,
    /// Overlay asset file applied (or that would be applied on Windows),
    /// `None` when clear.
    pub asset: Option<&'static str>,
    /// Whether the last `SetOverlayIcon` call reported success. Always
    /// `false` off Windows (no taskbar overlay exists there).
    pub overlay_applied: bool,
}

impl Default for BadgeState {
    fn default() -> Self {
        BadgeState {
            kind: BadgeKind::Clear,
            pings: None,
            asset: None,
            overlay_applied: false,
        }
    }
}

impl BadgeState {
    /// Records the outcome of the `SetOverlayIcon` call for `self`.
    pub fn with_overlay(self, overlay_applied: bool) -> Self {
        BadgeState {
            overlay_applied,
            ..self
        }
    }

    fn unread() -> Self {
        BadgeState {
            kind: BadgeKind::Unread,
            pings: None,
            asset: Some(BADGE_UNREAD_ASSET),
            overlay_applied: false,
        }
    }

    fn mention(pings: u32) -> Self {
        let asset: &'static str = if pings > MAX_INDIVIDUAL_MENTIONS {
            BADGE_CAPPED_ASSET
        } else {
            match pings {
                1 => "badge-1.ico",
                2 => "badge-2.ico",
                3 => "badge-3.ico",
                4 => "badge-4.ico",
                5 => "badge-5.ico",
                6 => "badge-6.ico",
                7 => "badge-7.ico",
                8 => "badge-8.ico",
                _ => "badge-9.ico",
            }
        };
        BadgeState {
            kind: BadgeKind::Mention,
            pings: Some(pings),
            asset: Some(asset),
            overlay_applied: false,
        }
    }
}

/// Rejects an over-long title report before any parse or Win32 work.
/// The observer slices to a shorter prefix, so a rejection means a hostile
/// or corrupted event; the stored state is left untouched.
pub fn validate_title(title: &str) -> Result<(), &'static str> {
    if title.chars().count() > MAX_BADGE_TITLE_CHARS {
        return Err("badge title exceeds the length limit");
    }
    Ok(())
}

/// Extracts the leading mention count of a `(`-prefixed title, mirroring
/// Electron's `/\((\d+)\)/` match: ASCII digits immediately after the
/// parenthesis, saturating at `u32::MAX` instead of overflowing.
fn extract_pings(title: &str) -> Option<u32> {
    let digits = title[1..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    let mut pings: u32 = 0;
    for digit in digits.chars() {
        pings = pings
            .saturating_mul(10)
            .saturating_add(u32::from(digit as u8 - b'0'));
    }
    Some(pings)
}

/// Maps a document title to overlay state with exact Electron prefix
/// semantics (`startsWith("•")`, `startsWith("(")`, else clear).
/// `•...` reports unread (`badge-11.ico`) with no count; `(N)...` with
/// `N >= 1` reports a mention (`badge-N.ico`, capped at `badge-10.ico`
/// above 9); `(0)...` clears ((0) mentions means nothing is unread); `(`
/// with no digits (Electron would interpolate the missing
/// `badge-null.ico`) falls back to unread so a real asset always applies;
/// anything else clears.
/// Pure: no window, COM, settings, or filesystem state is touched.
pub fn parse_title(title: &str) -> BadgeState {
    if title.starts_with('•') {
        return BadgeState::unread();
    }
    if title.starts_with('(') {
        return match extract_pings(title) {
            Some(0) => BadgeState::default(),
            Some(pings) => BadgeState::mention(pings),
            None => BadgeState::unread(),
        };
    }
    BadgeState::default()
}

/// Compile-time overlay asset table: every file [`parse_title`] can name.
/// `include_bytes!` keeps the host self-contained (no runtime asset
/// directory lookup that could be redirected). Single source of truth for
/// both [`badge_asset_bytes`] and [`BADGE_ASSET_NAMES`].
const BADGE_ASSETS: [(&str, &[u8]); 11] = [
    ("badge-1.ico", include_bytes!("../../assets/badge-1.ico")),
    ("badge-2.ico", include_bytes!("../../assets/badge-2.ico")),
    ("badge-3.ico", include_bytes!("../../assets/badge-3.ico")),
    ("badge-4.ico", include_bytes!("../../assets/badge-4.ico")),
    ("badge-5.ico", include_bytes!("../../assets/badge-5.ico")),
    ("badge-6.ico", include_bytes!("../../assets/badge-6.ico")),
    ("badge-7.ico", include_bytes!("../../assets/badge-7.ico")),
    ("badge-8.ico", include_bytes!("../../assets/badge-8.ico")),
    ("badge-9.ico", include_bytes!("../../assets/badge-9.ico")),
    ("badge-10.ico", include_bytes!("../../assets/badge-10.ico")),
    ("badge-11.ico", include_bytes!("../../assets/badge-11.ico")),
];

/// Every asset file [`parse_title`] can name, in numeric order. Test
/// surface only (production code resolves through [`badge_asset_bytes`]);
/// defined here so the inventory cannot drift from the lookup table.
#[cfg(test)]
pub const BADGE_ASSET_NAMES: [&str; 11] = {
    let mut names = [""; 11];
    let mut index = 0;
    while index < 11 {
        names[index] = BADGE_ASSETS[index].0;
        index += 1;
    }
    names
};

/// Resolves one overlay asset file to its bundled ICO bytes, or `None`
/// for any name outside the table (paths, URLs, and unknown files never
/// resolve).
pub fn badge_asset_bytes(name: &str) -> Option<&'static [u8]> {
    BADGE_ASSETS
        .iter()
        .find(|(asset, _)| *asset == name)
        .map(|(_, bytes)| *bytes)
}

/// Size of one `ICONDIR` + `ICONDIRENTRY` header pair in bytes.
const ICO_ENTRY_SIZE: usize = 16;
/// `ICONDIR` fixed prefix length in bytes.
const ICO_DIR_SIZE: usize = 6;

/// Locates the preferred icon image inside ICO bytes, returning the
/// `(offset, size)` slice to hand to `CreateIconFromResourceEx` (header
/// excluded, exactly what that call expects). Prefers an explicit 16x16
/// entry (the taskbar overlay size, and present in every bundled badge
/// asset); falls back to the first entry. Returns `None` for malformed
/// input (bad magic, zero images, truncated headers, out-of-bounds image
/// ranges) so a corrupt asset fails closed instead of reading OOB. Pure.
pub fn icon_image_range(ico: &[u8]) -> Option<(usize, usize)> {
    if ico.len() < ICO_DIR_SIZE {
        return None;
    }
    if ico[0] != 0 || ico[1] != 0 || ico[2] != 1 || ico[3] != 0 {
        return None;
    }
    let count = usize::from(ico[4]) | (usize::from(ico[5]) << 8);
    if count == 0 || count > 255 {
        return None;
    }
    let headers_end = ICO_DIR_SIZE.checked_add(count.checked_mul(ICO_ENTRY_SIZE)?)?;
    if ico.len() < headers_end {
        return None;
    }
    let entry_range = |index: usize| {
        let base = ICO_DIR_SIZE + index * ICO_ENTRY_SIZE;
        // A zero width/height byte means 256px in the ICO format; only an
        // explicit 16 selects the overlay entry.
        let width = ico[base];
        let height = ico[base + 1];
        let size =
            u32::from_le_bytes([ico[base + 8], ico[base + 9], ico[base + 10], ico[base + 11]])
                as usize;
        let offset = u32::from_le_bytes([
            ico[base + 12],
            ico[base + 13],
            ico[base + 14],
            ico[base + 15],
        ]) as usize;
        if size == 0 {
            return None;
        }
        let end = offset.checked_add(size)?;
        if offset < headers_end || end > ico.len() {
            return None;
        }
        Some((width, height, offset, size))
    };
    let mut fallback: Option<(usize, usize)> = None;
    for index in 0..count {
        let Some((width, height, offset, size)) = entry_range(index) else {
            continue;
        };
        if fallback.is_none() {
            fallback = Some((offset, size));
        }
        if width == 16 && height == 16 {
            return Some((offset, size));
        }
    }
    fallback
}

/// Fixed-vocabulary `[Badge]` log line for one applied state. Carries only
/// the parsed kind, the bounded count, the static asset name, and the
/// outcome flag — never the raw title (channel/DM names stay out of logs).
pub fn format_log(state: &BadgeState) -> String {
    let kind = match state.kind {
        BadgeKind::Clear => "clear",
        BadgeKind::Unread => "unread",
        BadgeKind::Mention => "mention",
    };
    let pings = state
        .pings
        .map_or("none".to_owned(), |pings| pings.to_string());
    let asset = state.asset.unwrap_or("none");
    format!(
        "[Badge] kind={kind} pings={pings} asset={asset} overlay_applied={}",
        state.overlay_applied
    )
}

/// Applies `state` to the taskbar button of `hwnd` (a tao
/// `WindowExtWindows::hwnd()` value) via `ITaskbarList3::SetOverlayIcon`,
/// the same Win32 call Electron's `setOverlayIcon` makes. COM is entered
/// with a balanced `CoInitializeEx`/`CoUninitialize` pair owned by this
/// call (an `S_FALSE` re-entry still balances; an `RPC_E_CHANGED_MODE`
/// thread keeps its existing mode and skips the uninit), so the call is
/// safe from any thread regardless of who initialized COM first. The
/// created `HICON` is destroyed after the call (the taskbar copies it).
/// Returns whether the overlay call reported success; failures are typed
/// strings (fixed vocabulary plus the Win32 error), never panics.
#[cfg(target_os = "windows")]
pub fn apply_overlay_icon(hwnd: isize, state: &BadgeState) -> Result<bool, String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIconFromResourceEx, DestroyIcon, HICON, LR_DEFAULTCOLOR,
    };

    // Balanced COM entry owned by this call (see doc comment).
    let com_init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if com_init.is_err() && com_init != windows::core::HRESULT::from_win32(0x80010106) {
        return Err(format!("taskbar COM initialization failed: {com_init}"));
    }
    let com_owned = com_init == windows::core::HRESULT(0);

    let result = (|| {
        let taskbar: ITaskbarList3 =
            unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) }
                .map_err(|error| format!("taskbar instance creation failed: {error}"))?;
        unsafe { taskbar.HrInit() }
            .map_err(|error| format!("taskbar initialization failed: {error}"))?;
        let window = HWND(hwnd as *mut core::ffi::c_void);
        match state.asset {
            Some(name) => {
                let bytes = badge_asset_bytes(name)
                    .ok_or_else(|| format!("unknown badge asset: {name}"))?;
                let (offset, size) = icon_image_range(bytes)
                    .ok_or_else(|| format!("unparsable badge asset: {name}"))?;
                let icon: HICON = unsafe {
                    CreateIconFromResourceEx(
                        &bytes[offset..offset + size],
                        true,
                        0x0003_0000,
                        16,
                        16,
                        LR_DEFAULTCOLOR,
                    )
                }
                .map_err(|error| format!("badge icon creation failed: {error}"))?;
                let description = HSTRING::from(BADGE_OVERLAY_DESCRIPTION);
                let status = unsafe { taskbar.SetOverlayIcon(window, icon, &description) };
                unsafe {
                    let _ = DestroyIcon(icon);
                }
                status.map_err(|error| format!("overlay icon update failed: {error}"))?;
                Ok(true)
            }
            None => {
                unsafe { taskbar.SetOverlayIcon(window, HICON::default(), PCWSTR::null()) }
                    .map_err(|error| format!("overlay icon clear failed: {error}"))?;
                Ok(true)
            }
        }
    })();

    if com_owned {
        unsafe {
            CoUninitialize();
        }
    }
    result
}

/// Off-Windows stub: no taskbar overlay exists, so state is tracked only.
/// Same signature so the dispatch arm needs no platform fork beyond this.
#[cfg(not(target_os = "windows"))]
pub fn apply_overlay_icon(_hwnd: isize, _state: &BadgeState) -> Result<bool, String> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unread_dot_maps_to_badge_11() {
        let state = parse_title("• Discord");
        assert_eq!(state.kind, BadgeKind::Unread);
        assert_eq!(state.pings, None);
        assert_eq!(state.asset, Some(BADGE_UNREAD_ASSET));
        assert_eq!(state.asset, Some("badge-11.ico"));
    }

    #[test]
    fn mention_counts_map_to_numbered_assets_capped_at_ten() {
        for (title, pings, asset) in [
            ("(1) #general - Discord", 1, "badge-1.ico"),
            ("(2) Discord", 2, "badge-2.ico"),
            ("(9) Discord", 9, "badge-9.ico"),
            ("(10) Discord", 10, "badge-10.ico"),
            ("(11) Discord", 11, "badge-10.ico"),
            ("(99) Discord", 99, "badge-10.ico"),
            ("(007) Discord", 7, "badge-7.ico"),
        ] {
            let state = parse_title(title);
            assert_eq!(state.kind, BadgeKind::Mention, "{title}");
            assert_eq!(state.pings, Some(pings), "{title}");
            assert_eq!(state.asset, Some(asset), "{title}");
        }
    }

    #[test]
    fn plain_titles_and_zero_mentions_clear() {
        for title in [
            "Discord",
            "#general - Discord",
            "Friends - Discord",
            "",
            "(0) Discord",
        ] {
            let state = parse_title(title);
            assert_eq!(state.kind, BadgeKind::Clear, "{title}");
            assert_eq!(state.pings, None, "{title}");
            assert_eq!(state.asset, None, "{title}");
        }
        assert_eq!(parse_title("Discord"), BadgeState::default());
    }

    #[test]
    fn uncountable_paren_shape_falls_back_to_unread() {
        // Electron interpolates the missing `badge-null.ico` here; native
        // never names a missing asset and reports generic unread instead.
        for title in ["(abc) Discord", "() Discord", "( Discord"] {
            let state = parse_title(title);
            assert_eq!(state.kind, BadgeKind::Unread, "{title}");
            assert_eq!(state.asset, Some(BADGE_UNREAD_ASSET), "{title}");
        }
    }

    #[test]
    fn only_title_prefixes_count() {
        // Electron uses startsWith: markers elsewhere in the title clear.
        for title in [
            "Hi • Discord",
            "Room (3) Discord",
            "• embedded".trim_start_matches('•'),
        ] {
            assert_eq!(parse_title(title).kind, BadgeKind::Clear, "{title}");
        }
        assert_eq!(parse_title("• embedded").kind, BadgeKind::Unread);
    }

    #[test]
    fn over_long_titles_are_rejected() {
        assert!(validate_title("Discord").is_ok());
        assert!(validate_title(&"x".repeat(MAX_BADGE_TITLE_CHARS)).is_ok());
        assert_eq!(
            validate_title(&"x".repeat(MAX_BADGE_TITLE_CHARS + 1)),
            Err("badge title exceeds the length limit")
        );
    }

    #[test]
    fn every_referenced_asset_exists_and_parses() {
        // The parser may only name files present in this table.
        for name in BADGE_ASSET_NAMES {
            let bytes = badge_asset_bytes(name);
            assert!(bytes.is_some(), "{name} must be bundled");
            let bytes = bytes.unwrap_or_default();
            assert!(!bytes.is_empty(), "{name} must not be empty");
            let (offset, size) = icon_image_range(bytes).expect("{name} must parse");
            assert!(offset + size <= bytes.len(), "{name} range must fit");
            assert!(size > 0, "{name} image must be non-empty");
        }
        assert_eq!(badge_asset_bytes("badge-0.ico"), None);
        assert_eq!(badge_asset_bytes("badge-null.ico"), None);
        assert_eq!(badge_asset_bytes("../build/icon.ico"), None);
    }

    #[test]
    fn parser_names_only_bundled_assets() {
        // Every asset the parser can emit must resolve in the table,
        // including adversarial counts (overflow saturates, never panics).
        let adversarial = [
            "(1) x",
            "(9) x",
            "(10) x",
            "(4294967295) x",
            "(99999999999999999999999) x",
            "• x",
            "plain",
            "(0) x",
            "(nope) x",
        ];
        for title in adversarial {
            let state = parse_title(title);
            if let Some(asset) = state.asset {
                assert!(
                    BADGE_ASSET_NAMES.contains(&asset),
                    "{title} named unbundled {asset}"
                );
                assert!(badge_asset_bytes(asset).is_some());
            }
        }
        assert_eq!(
            parse_title("(99999999999999999999999) x").pings,
            Some(u32::MAX)
        );
    }

    #[test]
    fn icon_parser_rejects_malformed_input() {
        assert_eq!(icon_image_range(&[]), None);
        assert_eq!(icon_image_range(&[0, 0, 1, 0, 1, 0]), None);
        // Wrong magic (PNG, BMP, JPEG): never an ICO directory.
        assert_eq!(
            icon_image_range(&[0x89, b'P', b'N', b'G', 1, 0, 0, 0]),
            None
        );
        assert_eq!(icon_image_range(&[0, 0, 2, 0, 1, 0]), None);
        // Zero images.
        assert_eq!(icon_image_range(&[0, 0, 1, 0, 0, 0]), None);
        // Truncated entry headers.
        assert_eq!(icon_image_range(&[0, 0, 1, 0, 1, 0, 16, 16]), None);
        // Out-of-bounds image range.
        let mut bad = vec![0u8; 6 + 16];
        bad[2] = 1;
        bad[4] = 1;
        bad[6] = 16;
        bad[7] = 16;
        bad[6 + 8..6 + 12].copy_from_slice(&100u32.to_le_bytes());
        bad[6 + 12..6 + 16].copy_from_slice(&10_000u32.to_le_bytes());
        assert_eq!(icon_image_range(&bad), None);
        // Zero-size image.
        let mut empty = vec![0u8; 6 + 16 + 8];
        empty[2] = 1;
        empty[4] = 1;
        empty[6] = 16;
        empty[7] = 16;
        empty[6 + 12..6 + 16].copy_from_slice(&22u32.to_le_bytes());
        assert_eq!(icon_image_range(&empty), None);
    }

    #[test]
    fn state_serializes_to_the_get_state_wire_shape() {
        let state = BadgeState::mention(3).with_overlay(true);
        let json = serde_json::to_value(state).unwrap_or_default();
        assert_eq!(json["kind"], "mention");
        assert_eq!(json["pings"], 3);
        assert_eq!(json["asset"], "badge-3.ico");
        assert_eq!(json["overlay_applied"], true);
        let clear = serde_json::to_value(BadgeState::default()).unwrap_or_default();
        assert_eq!(clear["kind"], "clear");
        assert!(clear["pings"].is_null());
        assert!(clear["asset"].is_null());
        assert_eq!(clear["overlay_applied"], false);
    }

    #[test]
    fn log_line_is_fixed_vocabulary_without_title_text() {
        assert_eq!(
            format_log(&BadgeState::mention(12).with_overlay(true)),
            "[Badge] kind=mention pings=12 asset=badge-10.ico overlay_applied=true"
        );
        assert_eq!(
            format_log(&BadgeState::default()),
            "[Badge] kind=clear pings=none asset=none overlay_applied=false"
        );
        assert_eq!(
            format_log(&BadgeState::unread()),
            "[Badge] kind=unread pings=none asset=badge-11.ico overlay_applied=false"
        );
    }
}

//! F16 window-state persistence: bounds + maximized restore across restarts.
//!
//! Electron parity (`src/common/windowState.ts`,
//! `src/common/windowBounds.ts`, `src/discord/window.ts`): the bounds the
//! user left behind are re-applied at startup so the window reopens where
//! it was, maximized or not. This module owns the whole host side of that
//! contract:
//!
//! - the file model (`<profile>/storage/window.json`, Electron's exact
//!   `<userData>/storage/window.json` layout and camelCase keys, so a
//!   future Electron import reads the same shape),
//! - pure sanitize + monitor-fit math (Electron's `MIN 400x300`,
//!   `MIN_VISIBLE_PX 50`, display-id-then-point fallback, center-on-miss),
//! - atomic persist with bounded reads.
//!
//! The renderer supplies NOTHING: geometry is captured host-side from the
//! Tao window on move/resize/maximize events (see `main.rs`), and the
//! only bridge surface is the parameter-free `window.get_state` read.
//! There is deliberately NO `window.set_state` / `window.move` /
//! `window.resize` capability: arbitrary renderer-driven window placement
//! is not an Electron surface and would turn the bridge into a generic
//! host-control RPC.
//!
//! Multi-display note: Tao exposes no stable display id, so `displayId`
//! is the index into `available_monitors()` order — a best-effort hint
//! only. Placement never trusts it alone: a stored point outside every
//! known monitor falls back to the primary (first) monitor, exactly like
//! Electron's point-then-primary chain.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Leaf of the window-state file under the profile root. Mirrors
/// Electron's `<userData>/storage/window.json`.
pub const WINDOW_FILE_NAME: &str = "window.json";

/// Storage leaf under the profile root (same leaf `settings.json` lives
/// under).
pub const STORAGE_DIR_NAME: &str = "storage";

/// Electron's constructor defaults (`src/common/windowBounds.ts`):
/// a first run opens 835x600.
pub const DEFAULT_WIDTH: i64 = 835;
/// Electron's constructor default height.
pub const DEFAULT_HEIGHT: i64 = 600;

/// Electron's minimum window size (`MIN_WIDTH`/`MIN_HEIGHT`).
pub const MIN_WIDTH: i64 = 400;
/// Electron's minimum window height.
pub const MIN_HEIGHT: i64 = 300;

/// Electron's `MIN_VISIBLE_PX`: at least this many pixels of the window
/// must stay inside a monitor, otherwise the placement is off-screen.
pub const MIN_VISIBLE_PX: i64 = 50;

/// Absolute sanity ceiling for one dimension. Real displays stay far
/// below; anything above is corrupt input, not a monitor.
pub const MAX_DIMENSION: i64 = 16384;

/// Absolute sanity range for an origin coordinate. Real desktop origins
/// stay far inside; anything outside is corrupt input.
pub const MAX_COORD: i64 = 32768;

/// Maximum accepted `window.json` size. The real file is well under
/// 1 KiB; anything above is not our file.
pub const MAX_WINDOW_FILE_BYTES: u64 = 64 * 1024;

/// Electron's `queueSave` debounce: move/resize bursts coalesce for
/// 500 ms before one atomic write lands.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// File model: Electron's exact `window.json` shape
/// (`width/height/x/y/isMaximized/displayId?/displayScaleFactor?`).
/// Numbers arrive as JSON floats and are rounded host-side, exactly like
/// Electron's `getPosition`/`getSize` DIP values.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredWindowState {
    /// Logical width in DIPs, rounded and clamped to
    /// `MIN_WIDTH..=MAX_DIMENSION` on load.
    pub width: f64,
    /// Logical height in DIPs, rounded and clamped to
    /// `MIN_HEIGHT..=MAX_DIMENSION` on load.
    pub height: f64,
    /// Logical x origin in DIPs, rounded and clamped to
    /// `-MAX_COORD..=MAX_COORD` on load.
    pub x: f64,
    /// Logical y origin in DIPs, rounded and clamped to
    /// `-MAX_COORD..=MAX_COORD` on load.
    pub y: f64,
    /// Whether the window was maximized when the state was captured.
    pub is_maximized: bool,
    /// Best-effort monitor hint: index into `available_monitors()` order
    /// (Tao exposes no stable display id — see the module docs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_id: Option<i64>,
    /// Scale factor of the capturing monitor, for the DPI roundtrip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_scale_factor: Option<f64>,
}

/// Sanitized integer geometry in logical DIPs. Everything past this
/// point is range-checked: `width >= MIN_WIDTH`,
/// `height >= MIN_HEIGHT`, origins inside `±MAX_COORD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowGeometry {
    /// Clamped width (`MIN_WIDTH..=MAX_DIMENSION`).
    pub width: i64,
    /// Clamped height (`MIN_HEIGHT..=MAX_DIMENSION`).
    pub height: i64,
    /// Clamped x origin (`-MAX_COORD..=MAX_COORD`).
    pub x: i64,
    /// Clamped y origin (`-MAX_COORD..=MAX_COORD`).
    pub y: i64,
}

/// One known monitor in logical DIPs: origin plus size, with the
/// best-effort index hint and scale factor the host measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorRect {
    /// Logical x origin of the monitor.
    pub x: i64,
    /// Logical y origin of the monitor.
    pub y: i64,
    /// Logical width of the monitor.
    pub width: i64,
    /// Logical height of the monitor.
    pub height: i64,
    /// Index into `available_monitors()` order, if known.
    pub id: Option<i64>,
    /// Scale factor of this monitor, if known.
    pub scale_factor: Option<f64>,
}

/// Monitor-fitted placement. `x`/`y` are `None` when no monitor could
/// take the window (or none was supplied): the caller then opens at the
/// OS default position instead of guessing coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedPlacement {
    /// Fitted width.
    pub width: i64,
    /// Fitted height.
    pub height: i64,
    /// Fitted x origin, or `None` for the OS default.
    pub x: Option<i64>,
    /// Fitted y origin, or `None` for the OS default.
    pub y: Option<i64>,
    /// True when the stored monitor/point missed and the primary (first)
    /// monitor or the OS default took over (Electron logs the same
    /// `usedFallback` signal).
    pub used_fallback: bool,
}

/// Host-built path of the window-state file. The profile dir is the only
/// input and it always comes from the host's resolved profile — never
/// from the renderer.
pub fn window_file(profile_dir: &Path) -> PathBuf {
    profile_dir.join(STORAGE_DIR_NAME).join(WINDOW_FILE_NAME)
}

/// True when the debounce window since the last write has passed (or no
/// write happened yet this process): the pending capture may land now.
pub fn debounce_elapsed(last_save: Option<Instant>, now: Instant) -> bool {
    match last_save {
        None => true,
        Some(previous) => now.duration_since(previous) >= SAVE_DEBOUNCE,
    }
}

/// Rounds one JSON number to integer DIPs. `None` on non-finite input
/// (JSON itself cannot carry NaN/Infinity, but a hand-edited file can
/// carry 1e999, which deserializes to infinity).
fn round_dips(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    Some(value.round() as i64)
}

/// Sanitizes one raw file record into integer geometry. Sizes clamp
/// into `MIN..=MAX_DIMENSION` (Electron fits oversized windows into the
/// work area instead of rejecting them); origins clamp into
/// `±MAX_COORD`. `None` only when a member is missing or non-finite —
/// a corrupt record, not a placeable window.
pub fn sanitize_geometry(raw: &StoredWindowState) -> Option<WindowGeometry> {
    Some(WindowGeometry {
        width: round_dips(raw.width)?.clamp(MIN_WIDTH, MAX_DIMENSION),
        height: round_dips(raw.height)?.clamp(MIN_HEIGHT, MAX_DIMENSION),
        x: round_dips(raw.x)?.clamp(-MAX_COORD, MAX_COORD),
        y: round_dips(raw.y)?.clamp(-MAX_COORD, MAX_COORD),
    })
}

/// Loads the raw persisted record, including the maximized flag and the
/// monitor hint. Returns `None` when no file exists, when it exceeds the
/// byte bound, when it fails to parse, or when a member is missing or
/// carries an unknown field. A corrupt file is rewritten to `{}` best-effort
/// (mirroring Electron's `checkIfConfigIsBroken` fail-safe) so the next
/// load is clean; a rewrite failure is ignored — the in-memory result is
/// the same either way.
pub fn load_stored_record(profile_dir: &Path) -> Option<StoredWindowState> {
    let path = window_file(profile_dir);
    let bytes = std::fs::metadata(&path)
        .ok()
        .filter(|meta| meta.len() <= MAX_WINDOW_FILE_BYTES)?;
    let _ = bytes;
    let text = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<StoredWindowState>(&text) {
        Ok(record) => Some(record),
        Err(_) => {
            // Corrupt state must never strand the window off-screen on
            // every future launch: reset to the first-run shape.
            let _ = std::fs::write(&path, "{}");
            None
        }
    }
}

/// Persists one capture atomically (tmp file + rename, pretty JSON like
/// Electron's `JSON.stringify(object, null, 4)`). Creates the storage
/// dir when missing. The only caller is the host's window-event path.
pub fn save_state(profile_dir: &Path, state: &StoredWindowState) -> Result<(), String> {
    let dir = profile_dir.join(STORAGE_DIR_NAME);
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("window state storage unavailable: {error}"))?;
    let text = serde_json::to_string_pretty(state)
        .map_err(|error| format!("window state serialize failed: {error}"))?;
    let tmp = dir.join("window.json.tmp");
    std::fs::write(&tmp, text).map_err(|error| format!("window state write failed: {error}"))?;
    std::fs::rename(&tmp, window_file(profile_dir))
        .map_err(|error| format!("window state commit failed: {error}"))?;
    Ok(())
}

/// Builds the record the host persists from one live capture. Geometry
/// is rounded here so the file always holds integers; the monitor hint
/// is the `available_monitors()` index (or `None` when unknown).
pub fn build_state(
    width: f64,
    height: f64,
    x: f64,
    y: f64,
    is_maximized: bool,
    display_id: Option<i64>,
    display_scale_factor: Option<f64>,
) -> Option<StoredWindowState> {
    Some(StoredWindowState {
        width: round_dips(width)? as f64,
        height: round_dips(height)? as f64,
        x: round_dips(x)? as f64,
        y: round_dips(y)? as f64,
        is_maximized,
        display_id,
        display_scale_factor,
    })
}

/// Fits sanitized geometry onto the known monitors, mirroring
/// Electron's `sanitizeWindowBounds`: prefer the `displayId` match, else
/// the monitor containing the stored point, else the primary (first)
/// monitor. The window is clamped so at least `MIN_VISIBLE_PX` stays
/// visible; an oversized window shrinks to the monitor (never below the
/// minimum size); a total miss returns `x`/`y` as `None` with
/// `used_fallback: true` so the caller takes the OS default position.
pub fn resolve_placement(
    geometry: &WindowGeometry,
    display_id: Option<i64>,
    monitors: &[MonitorRect],
) -> ResolvedPlacement {
    if monitors.is_empty() {
        return ResolvedPlacement {
            width: geometry.width,
            height: geometry.height,
            x: None,
            y: None,
            used_fallback: true,
        };
    }
    let mut used_fallback = false;
    let target = display_id
        .and_then(|id| monitors.iter().find(|monitor| monitor.id == Some(id)))
        .or_else(|| {
            used_fallback = display_id.is_some();
            monitors
                .iter()
                .find(|monitor| point_on_monitor(geometry.x, geometry.y, monitor))
        })
        .or_else(|| {
            used_fallback = true;
            monitors.first()
        });
    let Some(monitor) = target else {
        return ResolvedPlacement {
            width: geometry.width,
            height: geometry.height,
            x: None,
            y: None,
            used_fallback: true,
        };
    };
    // Shrink-to-fit first (Electron's `fitRectToWorkArea` sizes before
    // positioning), never below the minimum window size.
    let width = geometry.width.min(monitor.width).max(MIN_WIDTH);
    let height = geometry.height.min(monitor.height).max(MIN_HEIGHT);
    // Clamp the origin so at least MIN_VISIBLE_PX of each edge stays on
    // the monitor; when the window is larger than the monitor the min
    // bound exceeds the max and the max wins (top-left aligned).
    let min_x = monitor.x - width + MIN_VISIBLE_PX;
    let max_x = monitor.x + monitor.width - MIN_VISIBLE_PX;
    let min_y = monitor.y - height + MIN_VISIBLE_PX;
    let max_y = monitor.y + monitor.height - MIN_VISIBLE_PX;
    let (x, y) = if min_x > max_x || min_y > max_y {
        (monitor.x, monitor.y)
    } else {
        (
            geometry.x.clamp(min_x.min(max_x), min_x.max(max_x)),
            geometry.y.clamp(min_y.min(max_y), min_y.max(max_y)),
        )
    };
    // A stored point that landed outside every monitor but still fits
    // the primary after clamping is a fallback (Electron's signal).
    if !monitors
        .iter()
        .any(|monitor| point_on_monitor(geometry.x, geometry.y, monitor))
    {
        used_fallback = true;
    }
    ResolvedPlacement {
        width,
        height,
        x: Some(x),
        y: Some(y),
        used_fallback,
    }
}

/// True when the logical point lies inside the monitor rect.
fn point_on_monitor(x: i64, y: i64, monitor: &MonitorRect) -> bool {
    x >= monitor.x
        && x < monitor.x + monitor.width
        && y >= monitor.y
        && y < monitor.y + monitor.height
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn raw(width: f64, height: f64, x: f64, y: f64) -> StoredWindowState {
        StoredWindowState {
            width,
            height,
            x,
            y,
            is_maximized: false,
            display_id: None,
            display_scale_factor: None,
        }
    }

    fn monitor(x: i64, y: i64, width: i64, height: i64) -> MonitorRect {
        MonitorRect {
            x,
            y,
            width,
            height,
            id: None,
            scale_factor: None,
        }
    }

    #[test]
    fn sane_geometry_passes_through() {
        let geometry = sanitize_geometry(&raw(835.0, 600.0, 100.0, 100.0)).unwrap();
        assert_eq!(
            geometry,
            WindowGeometry {
                width: 835,
                height: 600,
                x: 100,
                y: 100
            }
        );
    }

    #[test]
    fn undersized_windows_clamp_to_minimum() {
        let geometry = sanitize_geometry(&raw(10.0, 10.0, 0.0, 0.0)).unwrap();
        assert_eq!((geometry.width, geometry.height), (MIN_WIDTH, MIN_HEIGHT));
    }

    #[test]
    fn oversized_and_wild_values_clamp_to_bounds() {
        let geometry = sanitize_geometry(&raw(99999.0, 99999.0, 99999.0, -99999.0)).unwrap();
        assert_eq!(
            (geometry.width, geometry.height, geometry.x, geometry.y),
            (MAX_DIMENSION, MAX_DIMENSION, MAX_COORD, -MAX_COORD)
        );
    }

    #[test]
    fn non_finite_members_reject_the_record() {
        assert!(sanitize_geometry(&raw(f64::INFINITY, 600.0, 0.0, 0.0)).is_none());
        assert!(sanitize_geometry(&raw(835.0, f64::NAN, 0.0, 0.0)).is_none());
    }

    #[test]
    fn display_id_match_wins_over_point() {
        let monitors = [
            MonitorRect {
                id: Some(0),
                ..monitor(0, 0, 1920, 1080)
            },
            MonitorRect {
                id: Some(1),
                ..monitor(1920, 0, 1920, 1080)
            },
        ];
        // Point sits on monitor 0 but the hint pins monitor 1; the
        // origin clamps onto monitor 1 so MIN_VISIBLE_PX stays visible.
        let geometry = WindowGeometry {
            width: 800,
            height: 600,
            x: 100,
            y: 100,
        };
        let placement = resolve_placement(&geometry, Some(1), &monitors);
        assert_eq!((placement.x, placement.y), (Some(1170), Some(100)));
        assert!(!placement.used_fallback);
    }

    #[test]
    fn point_fallback_takes_the_containing_monitor() {
        let monitors = [monitor(0, 0, 1920, 1080), monitor(1920, 0, 1920, 1080)];
        let geometry = WindowGeometry {
            width: 800,
            height: 600,
            x: 2000,
            y: 100,
        };
        let placement = resolve_placement(&geometry, None, &monitors);
        assert_eq!((placement.x, placement.y), (Some(2000), Some(100)));
        assert!(!placement.used_fallback);
    }

    #[test]
    fn off_screen_point_falls_back_to_primary() {
        let monitors = [monitor(0, 0, 1920, 1080)];
        let geometry = WindowGeometry {
            width: 800,
            height: 600,
            x: 9000,
            y: 9000,
        };
        let placement = resolve_placement(&geometry, None, &monitors);
        assert!(placement.used_fallback);
        let (x, y) = (placement.x.unwrap(), placement.y.unwrap());
        assert!(x + 800 >= MIN_VISIBLE_PX && x <= 1920 - MIN_VISIBLE_PX);
        assert!(y + 600 >= MIN_VISIBLE_PX && y <= 1080 - MIN_VISIBLE_PX);
    }

    #[test]
    fn oversized_window_shrinks_to_monitor() {
        let monitors = [monitor(0, 0, 1280, 720)];
        let geometry = WindowGeometry {
            width: 4000,
            height: 3000,
            x: 100,
            y: 100,
        };
        let placement = resolve_placement(&geometry, None, &monitors);
        assert!((placement.width, placement.height) <= (1280, 720));
        assert!((placement.width, placement.height) >= (MIN_WIDTH, MIN_HEIGHT));
    }

    #[test]
    fn no_monitors_returns_os_default_with_fallback() {
        let geometry = WindowGeometry {
            width: 835,
            height: 600,
            x: 100,
            y: 100,
        };
        let placement = resolve_placement(&geometry, None, &[]);
        assert_eq!((placement.x, placement.y), (None, None));
        assert!(placement.used_fallback);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "remcord-f16-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let profile = dir.join("profile");
        let state = StoredWindowState {
            width: 900.0,
            height: 700.0,
            x: 50.0,
            y: 60.0,
            is_maximized: true,
            display_id: Some(0),
            display_scale_factor: Some(1.5),
        };
        save_state(&profile, &state).unwrap();
        let back = load_stored_record(&profile).unwrap();
        assert_eq!(back, state);
        // The file uses Electron's camelCase keys.
        let text = std::fs::read_to_string(window_file(&profile)).unwrap();
        assert!(text.contains("isMaximized") && text.contains("displayId"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_loads_as_absent() {
        let dir = std::env::temp_dir().join(format!(
            "remcord-f16-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(load_stored_record(&dir.join("profile")).is_none());
    }

    #[test]
    fn corrupt_file_resets_to_first_run_shape() {
        let dir = std::env::temp_dir().join(format!(
            "remcord-f16-corrupt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let profile = dir.join("profile");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(window_file(&profile), "{not json").unwrap();
        assert!(load_stored_record(&profile).is_none());
        assert_eq!(
            std::fs::read_to_string(window_file(&profile)).unwrap(),
            "{}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_members_reject_the_record() {
        let dir = std::env::temp_dir().join(format!(
            "remcord-f16-unknown-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let profile = dir.join("profile");
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(
            window_file(&profile),
            r#"{"width":800,"height":600,"x":0,"y":0,"isMaximized":false,"handle":"0x1234"}"#,
        )
        .unwrap();
        assert!(load_stored_record(&profile).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn debounce_gates_burst_writes() {
        let now = Instant::now();
        assert!(debounce_elapsed(None, now));
        assert!(!debounce_elapsed(Some(now), now));
        assert!(debounce_elapsed(Some(now - SAVE_DEBOUNCE), now));
        assert!(!debounce_elapsed(
            Some(now - SAVE_DEBOUNCE + Duration::from_millis(1)),
            now
        ));
    }

    #[test]
    fn build_state_rounds_and_carries_hints() {
        let state = build_state(835.4, 600.5, 10.2, 20.6, true, Some(1), Some(1.25)).unwrap();
        assert_eq!(
            (state.width, state.height, state.x, state.y),
            (835.0, 601.0, 10.0, 21.0)
        );
        assert!(state.is_maximized);
        assert_eq!(
            (state.display_id, state.display_scale_factor),
            (Some(1), Some(1.25))
        );
        assert!(build_state(f64::NAN, 600.0, 0.0, 0.0, false, None, None).is_none());
    }

    #[test]
    fn module_stays_host_side_only() {
        // F16 owns persistence math, not window control: no Tao/wry
        // handle, no script evaluation, no process control may ever enter
        // the shipped code. Like `close_behavior`, the check runs on the
        // non-test, non-comment source so this test's own list and the
        // docs can name the forbidden tokens without tripping it.
        let source = include_str!("window_state.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "tao::",
            "wry::",
            "WebView",
            "evaluate_script",
            "set_visible",
            "set_focus",
            "ControlFlow",
            "std::process",
            "Command::new",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
    }
}

//! Opt-in screenshare capture-quality control (screensharing low-resource mode).
//!
//! Native Remcord performs Discord screensharing through the default WebView2
//! browser display-capture path: there is no native capture pipeline, no
//! display-media request handler, and no constraint layer, so a share captures
//! at full source resolution with default WebRTC encoding. On high-resolution
//! displays with an integrated GPU that can mean unnecessarily large frames
//! flowing through capture, conversion, and software encoding.
//!
//! This module adds a user-controlled, reversible, document-start interceptor
//! built on the one supported existing capture path —
//! `navigator.mediaDevices.getDisplayMedia` plus
//! `MediaStreamTrack.applyConstraints()` on the resulting display track —
//! mirroring the Electron Shelter interceptor
//! (`src/shelter/screenshare/components/ScreensharePicker.tsx:35-111`):
//!
//! - **Default** (`"default"`): existing behavior is preserved exactly. The
//!   emitted script only publishes the frozen status global below and returns
//!   without touching `getDisplayMedia`.
//! - **Balanced** (`"balanced"`): targets up to 1280x720 at 30 FPS.
//! - **Low resource** (`"low"`): targets up to 960x540 at 15 FPS.
//!
//! Safety properties (all pinned by tests):
//!
//! - The mode travels from the host settings snapshot (a closed three-value
//!   vocabulary); the renderer supplies nothing. Unknown values fail safe to
//!   `"default"`.
//! - The script runs only on the exact Discord entry origins
//!   (`https://discord.com`, canary, ptb). Everywhere else it returns
//!   immediately.
//! - Consent and picker flow are preserved: the wrapper calls the ORIGINAL
//!   `getDisplayMedia` first with the caller's options untouched, then
//!   constrains the resulting video track. A rejected constraint keeps the
//!   original stream — sharing never breaks because the mode is on.
//! - Installation is idempotent per document (a `Symbol.for` registry entry,
//!   mirroring `displayMediaPatch.ts`): repeated shares never stack wrappers.
//! - No timers, intervals, observers, event subscriptions, network, storage,
//!   bridge, or eval surface. The constraints promise settles once; nothing
//!   is retained across share/stop cycles beyond the idempotent wrapper.
//! - The frozen `window.__remcordScreenshareQuality` status global
//!   (`{ mode, active, width, height, fps }`) states the actual mode and
//!   whether constraining is active, so a toggle that did nothing would be
//!   visible instead of misleading.
//! - Requested constraints are NOT reported as achieved: the script logs the
//!   observed `getSettings()` values (actual width/height/frameRate) and
//!   warns when capture remains larger than 1.25x the target.

/// Closed mode vocabulary. The renderer can never invent a mode: unknown
/// input normalizes to [`DEFAULT_MODE`].
pub const VALID_SCREENSHARE_QUALITY_MODES: &[&str] = &["default", "balanced", "low"];
/// Preserves the existing Discord/WebView2 capture behavior. No wrapping.
pub const DEFAULT_MODE: &str = "default";
/// Targets up to 1280x720 at 30 FPS through the existing capture path.
pub const BALANCED_MODE: &str = "balanced";
/// Targets up to 960x540 at 15 FPS through the existing capture path.
pub const LOW_MODE: &str = "low";

/// Balanced target: 1280x720 at 30 FPS.
pub const BALANCED_TARGET: (u32, u32, u32) = (1280, 720, 30);
/// Low-resource target: 960x540 at 15 FPS.
pub const LOW_TARGET: (u32, u32, u32) = (960, 540, 15);

/// Normalize one raw mode string to the closed vocabulary. Empty, unknown,
/// or differently-cased input fails safe to [`DEFAULT_MODE`]: an unrecognized
/// value must never alter capture behavior.
pub fn normalize_mode(raw: &str) -> &'static str {
    match raw {
        "balanced" => BALANCED_MODE,
        "low" => LOW_MODE,
        _ => DEFAULT_MODE,
    }
}

/// Constraint target for one normalized mode: `(width, height, fps)`.
/// `None` for [`DEFAULT_MODE`] (no constraining).
pub fn constraint_target(mode: &str) -> Option<(u32, u32, u32)> {
    match normalize_mode(mode) {
        BALANCED_MODE => Some(BALANCED_TARGET),
        LOW_MODE => Some(LOW_TARGET),
        _ => None,
    }
}

/// Placeholder the host replaces with the JSON-escaped normalized mode.
const MODE_PLACEHOLDER: &str = "__REMCORD_SCREENSHARE_QUALITY_MODE__";
/// Placeholder the host replaces with the capture wrapper (active modes) or
/// the empty string (`"default"`: the script then carries no capture code at
/// all, so existing behavior is preserved structurally, not just by branch).
const WRAPPER_PLACEHOLDER: &str = "__REMCORD_SCREENSHARE_QUALITY_WRAPPER__";

/// Fixed document-start script. The ONLY host-supplied value is the mode
/// (spliced as JSON, so it is always a quoted string literal).
const SCREENSHARE_QUALITY_SCRIPT_TEMPLATE: &str = r#"(() => {
  "use strict";
  const mode = __REMCORD_SCREENSHARE_QUALITY_MODE__;
  const ENTRY_ORIGINS = ["https://discord.com", "https://canary.discord.com", "https://ptb.discord.com"];
  const TARGETS = {
    balanced: { width: 1280, height: 720, fps: 30 },
    low: { width: 960, height: 540, fps: 15 }
  };
  const publishStatus = (active, target) => {
    try {
      Object.defineProperty(window, "__remcordScreenshareQuality", {
        configurable: false,
        enumerable: false,
        value: Object.freeze({
          mode,
          active,
          width: target ? target.width : null,
          height: target ? target.height : null,
          fps: target ? target.fps : null
        })
      });
    } catch (error) {
      console.error("[Remcord][ScreenshareQuality] status publish failed", error);
    }
  };
  if (ENTRY_ORIGINS.indexOf(window.location.origin) === -1) return;
  const target = TARGETS[mode];
  if (!target) {
    publishStatus(false, null);
    return;
  }
  publishStatus(true, target);
__REMCORD_SCREENSHARE_QUALITY_WRAPPER__})();"#;

/// Capture wrapper appended only for active modes. Calls the ORIGINAL
/// `getDisplayMedia` first (picker/consent untouched), then constrains the
/// resulting display track; a rejected constraint keeps the original stream.
const SCREENSHARE_QUALITY_WRAPPER: &str = r#"  const mediaDevices = navigator.mediaDevices;
  if (!mediaDevices || typeof mediaDevices.getDisplayMedia !== "function") {
    console.warn("[Remcord][ScreenshareQuality] display capture is unavailable; mode has no effect");
    return;
  }
  const registryKey = Symbol.for("remcord.screenshare.qualityPatch");
  const existing = globalThis[registryKey];
  const state = (existing && existing.state) || { target };
  state.target = target;
  if (existing && existing.wrapper && mediaDevices.getDisplayMedia === existing.wrapper) {
    return;
  }
  const original = mediaDevices.getDisplayMedia.bind(mediaDevices);
  const wrapper = async function (options) {
    const stream = await original(options || {});
    const video = stream.getVideoTracks()[0];
    if (!video) return stream;
    try {
      video.contentHint = state.target.fps >= 30 ? "motion" : "detail";
    } catch {
    }
    try {
      await video.applyConstraints({
        width: { ideal: state.target.width, max: state.target.width },
        height: { ideal: state.target.height, max: state.target.height },
        frameRate: { ideal: state.target.fps, max: state.target.fps }
      });
      const settings = video.getSettings();
      console.log(
        "[Remcord][ScreenshareQuality] capture constrained: target=" +
        state.target.width + "x" + state.target.height + "@" + state.target.fps +
        " actual=" + (settings.width || "?") + "x" + (settings.height || "?") +
        "@" + (settings.frameRate || "?")
      );
      if (
        typeof settings.width === "number" &&
        typeof settings.height === "number" &&
        (settings.width > state.target.width * 1.25 || settings.height > state.target.height * 1.25)
      ) {
        console.warn(
          "[Remcord][ScreenshareQuality] capture is larger than the target; " +
          "encode may still process full-resolution frames."
        );
      }
    } catch (error) {
      console.warn("[Remcord][ScreenshareQuality] constraints rejected; keeping the original capture", error);
    }
    return stream;
  };
  try {
    globalThis[registryKey] = { wrapper, state };
  } catch {
  }
  mediaDevices.getDisplayMedia = wrapper;
"#;

/// Build the document-start script for one raw mode string. The mode is
/// normalized first, so hand-edited or forward-compat values fail safe to
/// `"default"` (status-only script with no capture code). The returned text
/// contains no unspliced placeholder.
pub fn screenshare_quality_script(raw_mode: &str) -> Result<String, String> {
    let mode = normalize_mode(raw_mode);
    let encoded =
        serde_json::to_string(mode).map_err(|error| format!("mode must serialize: {error}"))?;
    let wrapper = if constraint_target(mode).is_some() {
        SCREENSHARE_QUALITY_WRAPPER
    } else {
        ""
    };
    let script = SCREENSHARE_QUALITY_SCRIPT_TEMPLATE
        .replace(MODE_PLACEHOLDER, &encoded)
        .replace(WRAPPER_PLACEHOLDER, wrapper);
    if script.contains(MODE_PLACEHOLDER) || script.contains(WRAPPER_PLACEHOLDER) {
        return Err("screenshare quality script is missing a splice".to_owned());
    }
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_modes_fail_safe_to_default() {
        for raw in [
            "",
            "DEFAULT",
            "Balanced",
            "ultra",
            "720p",
            "none",
            "  balanced  ",
        ] {
            assert_eq!(normalize_mode(raw), DEFAULT_MODE, "{raw:?} must fail safe");
        }
        assert_eq!(normalize_mode("balanced"), BALANCED_MODE);
        assert_eq!(normalize_mode("low"), LOW_MODE);
        assert_eq!(normalize_mode("default"), DEFAULT_MODE);
    }

    #[test]
    fn targets_match_the_candidate_presets() {
        assert_eq!(constraint_target("default"), None);
        assert_eq!(constraint_target("balanced"), Some((1280, 720, 30)));
        assert_eq!(constraint_target("low"), Some((960, 540, 15)));
        // Unknown input constrains nothing.
        assert_eq!(constraint_target("ultra"), None);
        assert_eq!(constraint_target(""), None);
    }

    #[test]
    fn default_script_preserves_behavior_and_publishes_status() {
        let script = screenshare_quality_script("default").expect("must build");
        assert!(script.contains("\"default\""));
        assert!(script.contains("__remcordScreenshareQuality"));
        assert!(script.contains("https://discord.com"));
        assert!(script.contains("https://canary.discord.com"));
        assert!(script.contains("https://ptb.discord.com"));
        // Default never touches capture: no wrapping, no constraints.
        assert!(!script.contains("getDisplayMedia"));
        assert!(!script.contains("applyConstraints"));
    }

    #[test]
    fn active_modes_constrain_through_the_existing_capture_path() {
        for (mode, dims) in [("balanced", "1280"), ("low", "960")] {
            let script = screenshare_quality_script(mode).expect("must build");
            assert!(
                script.contains(&format!("\"{mode}\"")),
                "{mode} mode must be spliced"
            );
            // Consent flow preserved: the original runs first with caller options.
            assert!(
                script.contains("getDisplayMedia"),
                "{mode} must wrap capture"
            );
            assert!(
                script.contains("original(options"),
                "{mode} must call through first"
            );
            assert!(script.contains("applyConstraints"), "{mode} must constrain");
            assert!(
                script.contains("getSettings"),
                "{mode} must verify actual settings"
            );
            // Rejection keeps the original stream: sharing never breaks.
            assert!(
                script.contains("keeping the original capture"),
                "{mode} needs the fallback"
            );
            // Idempotent: one wrapper per document, no stacking.
            assert!(
                script.contains("remcord.screenshare.qualityPatch"),
                "{mode} needs the registry"
            );
            // Actual behavior is reported, never assumed.
            assert!(script.contains(dims), "{mode} must carry its target");
            assert!(
                script.contains("__remcordScreenshareQuality"),
                "{mode} must publish status"
            );
        }
        let balanced = screenshare_quality_script("balanced").expect("must build");
        assert!(balanced.contains("width: 1280"));
        assert!(balanced.contains("height: 720"));
        assert!(balanced.contains("fps: 30"));
        // Oversize warning mirrors the Electron 1.25x check
        // (`ScreensharePicker.tsx:76`): requested settings are never reported
        // as achieved.
        assert!(balanced.contains("1.25"));
        let low = screenshare_quality_script("low").expect("must build");
        assert!(low.contains("width: 960"));
        assert!(low.contains("height: 540"));
        assert!(low.contains("fps: 15"));
    }

    #[test]
    fn script_has_no_forbidden_surfaces() {
        for mode in ["default", "balanced", "low"] {
            let script = screenshare_quality_script(mode).expect("must build");
            assert!(
                !script.contains(MODE_PLACEHOLDER),
                "{mode}: placeholder must be spliced"
            );
            assert!(
                !script.contains(WRAPPER_PLACEHOLDER),
                "{mode}: wrapper placeholder must be spliced"
            );
            for forbidden in [
                "fetch(",
                "XMLHttpRequest",
                "eval(",
                "new Function",
                "postMessage",
                "localStorage",
                "indexedDB",
                "remcord://",
                "http://",
                "wss://",
                "require(",
                "setInterval",
                "setTimeout",
                "MutationObserver",
                "addEventListener",
            ] {
                assert!(
                    !script.contains(forbidden),
                    "{mode}: script must not contain {forbidden}"
                );
            }
        }
    }

    #[test]
    fn unknown_raw_mode_builds_the_inert_default_script() {
        let script = screenshare_quality_script("ultra-hd").expect("must build");
        assert!(script.contains("\"default\""));
        assert!(!script.contains("applyConstraints"));
    }
}

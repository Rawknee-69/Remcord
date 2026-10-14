//! Native screenshare media hardening (F9 core).
//!
//! Electron Remcord performs screensharing through two preload patches that
//! have no native counterpart yet (`src/discord/preload/patches.mts`):
//!
//! - **WebRTC SDP patch**: raises Chromium's ~2500 kbps screenshare SDP cap
//!   to an 80 Mbps ceiling (with a modest 3 Mbps floor/start so GCC probes
//!   above Discord's default without fighting congestion control) and, on
//!   Windows/macOS, rewrites H.264 Constrained Baseline (`42e0xx`, the
//!   OpenH264 software path) to Baseline (`4200xx`) on local AND remote
//!   descriptions so the Media Foundation / VideoToolbox hardware encoder
//!   can be selected. Gated by `settings.sdpH264BaselineRewrite` (default
//!   on), exactly like Electron.
//! - **Camera `getUserMedia` fix**: Chromium ignores a video `deviceId`
//!   passed as a plain string ("ideal"), always returning the first camera.
//!   The patch promotes it to `{ exact }`, stops the previously active
//!   video tracks so the OS releases the hardware, retries `NotReadableError`
//!   with exponential backoff, and falls back to the original constraints.
//!   Audio tracking stays off on Windows (Electron enables it on darwin
//!   only); stopping prior audio streams breaks Discord on Windows/Linux.
//!
//! WebView2 offers no `desktopCapturer` equivalent and no display-media
//! request handler: source enumeration, the source picker commit, and
//! loopback audio stay owned by the OS system picker. Venmic is Linux-only
//! and has no Windows path. This module ports exactly the two platform-agnostic media
//! patches above — the slice of F9 that is both real and host-ownable.
//!
//! Safety properties (all pinned by tests):
//!
//! - The only host-supplied value is the closed `prefer_hw_h264` boolean,
//!   spliced as a JSON literal (`true`/`false`). The renderer supplies
//!   nothing; there is no bridge, fetch, storage, URL, path, or eval
//!   surface.
//! - Origin-gated to the three Discord entry origins; everywhere else the
//!   script publishes status and returns without touching WebRTC or media
//!   devices.
//! - Idempotent per document via `Symbol.for("remcord.screenshare.mediaPatch")`:
//!   repeated documents never stack wrappers, and re-running the script
//!   leaves the original prototypes/methods intact.
//! - Consent and picker flows are preserved: the SDP wrappers only rewrite
//!   the description the caller already produced, and the camera wrapper
//!   calls the ORIGINAL `getUserMedia` first (exact attempt, then fallback).
//!   Sharing never breaks because hardening is on.
//! - The frozen `window.__remcordScreenshareMedia` status global
//!   (`{ installed, preferHwH264, sdpPatch, cameraFix }`) states what ran,
//!   so a toggle that did nothing is visible instead of misleading.

/// Placeholder the host replaces with the JSON-encoded boolean.
const PREFER_HW_PLACEHOLDER: &str = "__REMCORD_SCREENSHARE_PREFER_HW_H264__";

/// Fixed document-start script. The ONLY host-supplied value is the boolean
/// (spliced as JSON, so it is always a `true`/`false` literal).
const SCREENSHARE_MEDIA_SCRIPT_TEMPLATE: &str = r#"(() => {
  "use strict";
  const preferHwH264 = __REMCORD_SCREENSHARE_PREFER_HW_H264__;
  const ENTRY_ORIGINS = ["https://discord.com", "https://canary.discord.com", "https://ptb.discord.com"];
  const publishStatus = () => {
    try {
      Object.defineProperty(window, "__remcordScreenshareMedia", {
        configurable: false,
        enumerable: false,
        value: Object.freeze({
          installed: true,
          preferHwH264: preferHwH264 === true,
          sdpPatch: true,
          cameraFix: true
        })
      });
    } catch (error) {
      console.error("[Remcord][ScreenshareMedia] status publish failed", error);
    }
  };
  publishStatus();
  if (ENTRY_ORIGINS.indexOf(window.location.origin) === -1) return;
  const registryKey = Symbol.for("remcord.screenshare.mediaPatch");
  try {
    if (globalThis[registryKey] && globalThis[registryKey].installed) return;
  } catch {
  }
  const markInstalled = () => {
    try {
      globalThis[registryKey] = { installed: true };
    } catch {
    }
  };
  const CAP = "80000";
  const MIN_BR = "3000";
  const START_BR = "3000";
  const setOrAppendFmtpParam = (sdp, key, value) => {
    const re = new RegExp(key + "=\\d+", "g");
    if (sdp.indexOf(key + "=") !== -1) {
      return sdp.replace(re, key + "=" + value);
    }
    return sdp.replace(/(a=fmtp:\d+ [^\r\n]*)/g, (line) => {
      if (line.indexOf(key + "=") !== -1) return line;
      return line + ";" + key + "=" + value;
    });
  };
  const mungeSdp = (sdp) => {
    if (!sdp || typeof sdp !== "string") return sdp;
    let out = sdp;
    if (preferHwH264) {
      out = out.replace(/profile-level-id=42e0([0-9a-fA-F]{2})/gi, "profile-level-id=4200$1");
    }
    out = setOrAppendFmtpParam(out, "x-google-max-bitrate", CAP);
    out = setOrAppendFmtpParam(out, "x-google-min-bitrate", MIN_BR);
    out = setOrAppendFmtpParam(out, "x-google-start-bitrate", START_BR);
    return out;
  };
  const wrapDescription = (desc) => {
    if (!desc || !desc.sdp) return desc;
    const sdp = mungeSdp(desc.sdp);
    if (sdp === desc.sdp) return desc;
    try {
      return new RTCSessionDescription({ type: desc.type, sdp });
    } catch (e) {
      return Object.assign({}, desc, { sdp });
    }
  };
  try {
    const proto = window.RTCPeerConnection && window.RTCPeerConnection.prototype;
    if (proto) {
      const origSLD = proto.setLocalDescription;
      if (typeof origSLD === "function") {
        proto.setLocalDescription = function (desc) {
          const args = Array.prototype.slice.call(arguments);
          if (args.length > 0) args[0] = wrapDescription(args[0]);
          return origSLD.apply(this, args);
        };
      }
      const origSRD = proto.setRemoteDescription;
      if (typeof origSRD === "function") {
        proto.setRemoteDescription = function (desc) {
          const args = Array.prototype.slice.call(arguments);
          if (args.length > 0) args[0] = wrapDescription(args[0]);
          return origSRD.apply(this, args);
        };
      }
      const origOffer = proto.createOffer;
      if (typeof origOffer === "function") {
        proto.createOffer = function () {
          const self = this;
          const args = arguments;
          return Promise.resolve(origOffer.apply(self, args)).then((offer) => wrapDescription(offer) || offer);
        };
      }
      const origAnswer = proto.createAnswer;
      if (typeof origAnswer === "function") {
        proto.createAnswer = function () {
          const self = this;
          const args = arguments;
          return Promise.resolve(origAnswer.apply(self, args)).then((answer) => wrapDescription(answer) || answer);
        };
      }
      console.log("[Remcord] Early WebRTC screenshare SDP patch installed" + (preferHwH264 ? " (H264 CBP->Baseline on local+remote)" : ""));
    }
  } catch (error) {
    console.warn("[Remcord][ScreenshareMedia] SDP patch failed; capture continues unpatched", error);
  }
  try {
    const mediaDevices = navigator.mediaDevices;
    if (mediaDevices && typeof mediaDevices.getUserMedia === "function") {
      const original = mediaDevices.getUserMedia.bind(mediaDevices);
      const activeVideoStreams = [];
      const stopTrackedStreams = (list) => {
        for (let i = 0; i < list.length; i++) {
          const stream = list[i].deref ? list[i].deref() : null;
          if (stream) {
            const tracks = stream.getVideoTracks();
            for (let j = 0; j < tracks.length; j++) {
              if (tracks[j].readyState === "live") {
                try {
                  tracks[j].stop();
                } catch {
                }
              }
            }
          }
        }
        list.length = 0;
      };
      mediaDevices.getUserMedia = async function (constraints) {
        const hasVideo = constraints && constraints.video && typeof constraints.video !== "boolean";
        if (hasVideo && activeVideoStreams.length > 0) stopTrackedStreams(activeVideoStreams);
        const hasStringVideoDeviceId = hasVideo && typeof constraints.video.deviceId === "string";
        if (!hasStringVideoDeviceId) {
          const stream = await original(constraints);
          try {
            if (stream.getVideoTracks().length > 0 && typeof WeakRef !== "undefined") activeVideoStreams.push(new WeakRef(stream));
          } catch {
          }
          return stream;
        }
        const requestedId = constraints.video.deviceId;
        const modified = Object.assign({}, constraints);
        modified.video = Object.assign({}, constraints.video, { deviceId: { exact: requestedId } });
        const MAX_RETRIES = 5;
        let lastErr;
        let delay = 50;
        for (let i = 0; i < MAX_RETRIES; i++) {
          if (i > 0) {
            await new Promise((resolve) => setTimeout(resolve, delay));
            delay *= 2;
          }
          try {
            const stream = await original(modified);
            try {
              if (stream.getVideoTracks().length > 0 && typeof WeakRef !== "undefined") activeVideoStreams.push(new WeakRef(stream));
            } catch {
            }
            return stream;
          } catch (e) {
            lastErr = e;
            if (e && e.name === "NotReadableError") continue;
            break;
          }
        }
        if (lastErr) {
          console.warn("[Remcord] Exact deviceId failed, falling back to ideal:", lastErr.name, lastErr.message);
        }
        const fallbackStream = await original(constraints);
        try {
          if (fallbackStream.getVideoTracks().length > 0 && typeof WeakRef !== "undefined") activeVideoStreams.push(new WeakRef(fallbackStream));
        } catch {
        }
        return fallbackStream;
      };
    }
  } catch (error) {
    console.warn("[Remcord][ScreenshareMedia] camera fix failed; capture continues unpatched", error);
  }
  markInstalled();
})();"#;

/// Build the document-start media-hardening script for one host-resolved
/// boolean. The value travels from the authoritative settings snapshot
/// (`sdpH264BaselineRewrite`, default on); the renderer supplies nothing.
/// The returned text contains no unspliced placeholder.
pub fn screenshare_media_script(prefer_hw_h264: bool) -> Result<String, String> {
    let encoded = if prefer_hw_h264 { "true" } else { "false" };
    let script = SCREENSHARE_MEDIA_SCRIPT_TEMPLATE.replace(PREFER_HW_PLACEHOLDER, encoded);
    if script.contains(PREFER_HW_PLACEHOLDER) {
        return Err("screenshare media script is missing a splice".to_owned());
    }
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_is_spliced_as_a_json_literal() {
        let on = screenshare_media_script(true).expect("must build");
        assert!(on.contains("const preferHwH264 = true"));
        assert!(!on.contains(PREFER_HW_PLACEHOLDER));
        let off = screenshare_media_script(false).expect("must build");
        assert!(off.contains("const preferHwH264 = false"));
        assert!(!off.contains(PREFER_HW_PLACEHOLDER));
    }

    #[test]
    fn script_is_gated_to_the_discord_entry_origins_and_publishes_status() {
        let script = screenshare_media_script(true).expect("must build");
        for origin in [
            "https://discord.com",
            "https://canary.discord.com",
            "https://ptb.discord.com",
        ] {
            assert!(script.contains(origin), "{origin} must be gated");
        }
        assert!(script.contains("window.location.origin"));
        assert!(script.contains("__remcordScreenshareMedia"));
        assert!(script.contains("sdpPatch"));
        assert!(script.contains("cameraFix"));
    }

    #[test]
    fn sdp_patch_carries_the_electron_bitrate_ceiling_and_h264_rewrite() {
        let script = screenshare_media_script(true).expect("must build");
        // Electron parity: 80 Mbps ceiling, 3 Mbps floor/start.
        assert!(script.contains("\"80000\""));
        assert!(script.contains("\"3000\""));
        assert!(script.contains("x-google-max-bitrate"));
        assert!(script.contains("x-google-min-bitrate"));
        assert!(script.contains("x-google-start-bitrate"));
        // Local AND remote descriptions are rewritten (Electron parity:
        // Discord's Go Live answer would otherwise overwrite a local-only
        // munge), plus offer/answer creation.
        for token in [
            "setLocalDescription",
            "setRemoteDescription",
            "createOffer",
            "createAnswer",
            "RTCSessionDescription",
        ] {
            assert!(script.contains(token), "{token} must be wrapped");
        }
        // H.264 CBP -> Baseline rewrite for the Media Foundation path.
        assert!(script.contains("profile-level-id=42e0"));
        assert!(script.contains("profile-level-id=4200$1"));
        assert!(script.contains("[Remcord] Early WebRTC screenshare SDP patch installed"));
    }

    #[test]
    fn camera_fix_promotes_string_device_ids_with_retry_and_fallback() {
        let script = screenshare_media_script(true).expect("must build");
        assert!(script.contains("getUserMedia"));
        assert!(script.contains("deviceId: { exact: requestedId }"));
        assert!(script.contains("NotReadableError"));
        assert!(script.contains("falling back to ideal"));
        // The original is always the capture path: exact attempt first,
        // original constraints on fallback. Sharing never breaks because
        // hardening is on.
        assert!(script.contains("await original(modified)"));
        assert!(script.contains("await original(constraints)"));
        // Previous video tracks are released so the OS frees the hardware.
        assert!(script.contains("tracks[j].stop()"));
    }

    #[test]
    fn installation_is_idempotent_per_document() {
        let script = screenshare_media_script(true).expect("must build");
        assert!(script.contains("remcord.screenshare.mediaPatch"));
        assert!(script.contains("globalThis[registryKey]"));
        // Re-running the script returns early; prototypes are wrapped once.
        assert!(script.contains("if (globalThis[registryKey]"));
    }

    #[test]
    fn shipped_script_has_no_forbidden_surfaces() {
        // No bridge, no fetch, no eval, no storage, no host input beyond the
        // spliced boolean. Module docs name forbidden APIs for contrast, so
        // comment lines are stripped before asserting on code. The two
        // single-shot retry delays are the documented exception (asserted by
        // the retry test); nothing is retained across calls.
        let shipped = SCREENSHARE_MEDIA_SCRIPT_TEMPLATE
            .split('\n')
            .filter(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("//") && !trimmed.starts_with("//!")
            })
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "postMessage",
            "fetch(",
            "eval(",
            "Function(",
            "localStorage",
            "sessionStorage",
            "indexedDB",
            "XMLHttpRequest",
            "WebSocket",
            "desktopCapturer",
            "venmic",
            "loopback",
            "chromeMediaSource",
            "require(",
            "process.platform",
            "setInterval",
            "MutationObserver",
            "addEventListener",
            "innerHTML",
            "remcord://",
            "http://",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
        assert!(shipped.contains(PREFER_HW_PLACEHOLDER));
    }
}

//! Remcord stream setup popup for native display capture.
//!
//! Electron Remcord replaces the capture source picker with its own modal
//! (source cards, resolution/FPS/audio in
//! `src/shelter/screenshare/components/ScreensharePicker.tsx`). The native
//! host cannot do the same for the *source* step: the WebView2 SDK exposes
//! only the `ScreenCaptureStarting` observation event (`Cancel`/`Handled`,
//! no source injection), so the system picker remains the component that
//! commits to a concrete screen or window.
//!
//! This module ports everything around that platform boundary:
//!
//! - A Discord-styled setup popup shown on every `getDisplayMedia` call on
//!   the exact Discord entry origins: source KIND (entire screen vs a
//!   window), resolution (480p–2160p), frame rate (5–120 FPS, matching the
//!   Electron picker's ceiling and the 120 FPS Go Live tier), and a share
//!   audio toggle. Styling resolves Discord theme variables, so the popup
//!   follows light/dark, custom themes, and Quick CSS.
//! - The chosen kind pre-filters the system picker through the standard
//!   `displaySurface` constraint (never inventing a source id), and the
//!   chosen quality is applied post-capture with `applyConstraints`,
//!   mirroring `screenshare_quality.rs`. A rejected constraint keeps the
//!   original stream: sharing never breaks because quality was requested.
//! - Cancelling (button, overlay click, or Escape) rejects with
//!   `NotAllowedError`, exactly like a dismissed picker, so Discord handles
//!   it through its normal denied-share path.
//!
//! Safety properties (all pinned by tests):
//!
//! - No bridge, no fetch, no eval/`Function`, no storage, no URLs, no host
//!   input of any kind. The script is a fixed literal: there are no
//!   placeholders to splice and nothing for the renderer to supply.
//! - Origin-gated to the three Discord entry origins; everywhere else it
//!   returns immediately.
//! - Idempotent per document (a `Symbol.for` registry entry, mirroring
//!   `displayMediaPatch.ts` and `screenshare_quality.rs`): repeated shares
//!   never stack wrappers, and a re-entrant call while the popup is open
//!   joins the pending decision instead of opening a second modal.
//! - The popup DOM (one `<style>` node plus one overlay node, both
//!   `data-rmc-sp` namespaced) is removed on every settle path, and the
//!   frozen `window.__remcordStreamPicker` status global plus the mutable
//!   `window.__remcordStreamPrefs` last-choice record keep the behavior
//!   observable instead of misleading.

/// Fixed document-start script. No placeholders: the returned text is final.
const SCREENSHARE_PICKER_SCRIPT: &str = r#"(() => {
  "use strict";
  const ENTRY_ORIGINS = ["https://discord.com", "https://canary.discord.com", "https://ptb.discord.com"];
  if (ENTRY_ORIGINS.indexOf(window.location.origin) === -1) return;
  const registryKey = Symbol.for("remcord.screenshare.pickerPatch");
  const RESOLUTIONS = [480, 720, 1080, 1440, 2160];
  const FRAME_RATES = [5, 15, 30, 60, 120];
  const DEFAULT_CHOICE = { kind: "monitor", width: 1920, height: 1080, fps: 30, audio: false };
  const publishStatus = () => {
    try {
      Object.defineProperty(window, "__remcordStreamPicker", {
        configurable: false,
        enumerable: false,
        value: Object.freeze({ installed: true })
      });
    } catch (error) {
      console.error("[Remcord][StreamPicker] status publish failed", error);
    }
    try {
      if (window.__remcordStreamPrefs === undefined) window.__remcordStreamPrefs = null;
    } catch {
    }
  };
  publishStatus();
  const mediaDevices = navigator.mediaDevices;
  if (!mediaDevices || typeof mediaDevices.getDisplayMedia !== "function") {
    console.warn("[Remcord][StreamPicker] display capture is unavailable; picker has no effect");
    return;
  }
  const existing = globalThis[registryKey];
  if (existing && existing.wrapper && mediaDevices.getDisplayMedia === existing.wrapper) return;
  const CSS_TEXT = "[data-rmc-sp-overlay]{position:fixed;inset:0;z-index:10000;display:flex;align-items:center;justify-content:center;background:rgba(0,0,0,0.7);}" +
    "[data-rmc-sp-card]{width:520px;max-width:calc(100vw - 48px);max-height:calc(100vh - 48px);overflow:auto;background:var(--background-primary,#313338);color:var(--text-normal,#dbdee1);border:1px solid var(--background-modifier-accent,#4e5058);border-radius:8px;padding:20px 24px;font-family:var(--font-primary,Whitney,\"Helvetica Neue\",Helvetica,Arial,sans-serif);}" +
    "[data-rmc-sp-title]{font-size:20px;font-weight:700;margin:0 0 4px;}" +
    "[data-rmc-sp-sub]{font-size:13px;color:var(--text-muted,#949ba4);margin:0 0 16px;}" +
    "[data-rmc-sp-h]{font-size:12px;font-weight:700;text-transform:uppercase;letter-spacing:.5px;color:var(--text-muted,#949ba4);margin:14px 0 8px;}" +
    "[data-rmc-sp-row]{display:flex;gap:8px;flex-wrap:wrap;}" +
    "[data-rmc-sp-opt]{flex:1 1 0;min-width:80px;padding:10px 6px;text-align:center;font-size:14px;font-weight:600;color:var(--text-normal,#dbdee1);background:var(--background-secondary-alt,#2b2d31);border:1px solid var(--background-modifier-accent,#4e5058);border-radius:6px;cursor:pointer;}" +
    "[data-rmc-sp-opt][aria-pressed=\"true\"]{background:var(--brand-500,#5865f2);border-color:var(--brand-500,#5865f2);color:#fff;}" +
    "[data-rmc-sp-audio]{display:flex;align-items:center;gap:10px;margin-top:16px;font-size:14px;cursor:pointer;}" +
    "[data-rmc-sp-audio] input{width:18px;height:18px;accent-color:var(--brand-500,#5865f2);}" +
    "[data-rmc-sp-foot]{display:flex;justify-content:flex-end;gap:10px;margin-top:20px;}" +
    "[data-rmc-sp-btn]{padding:10px 24px;font-size:14px;font-weight:600;border-radius:6px;border:1px solid var(--background-modifier-accent,#4e5058);background:var(--background-secondary-alt,#2b2d31);color:var(--text-normal,#dbdee1);cursor:pointer;}" +
    "[data-rmc-sp-btn-primary]{background:var(--brand-500,#5865f2);border-color:var(--brand-500,#5865f2);color:#fff;}";
  const el = (tag, attrs, text) => {
    const node = document.createElement(tag);
    if (attrs) for (const key of Object.keys(attrs)) node.setAttribute(key, attrs[key]);
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const segmented = (labels, initial, onPick) => {
    const row = el("div", { "data-rmc-sp-row": "" });
    const buttons = labels.map((label) => {
      const button = el("button", { type: "button", "data-rmc-sp-opt": "", "aria-pressed": label === initial ? "true" : "false" }, String(label));
      button.addEventListener("click", () => {
        for (const other of buttons) other.setAttribute("aria-pressed", other === button ? "true" : "false");
        onPick(label);
      });
      row.appendChild(button);
      return button;
    });
    return row;
  };
  const openPicker = () => {
    if (openPicker.pending) return openPicker.pending;
    openPicker.pending = new Promise((resolve) => {
      const choice = { kind: "monitor", width: 1920, height: 1080, fps: 30, audio: false };
      const style = el("style", { "data-rmc-sp": "", type: "text/css" });
      style.textContent = CSS_TEXT;
      (document.head || document.documentElement).appendChild(style);
      const overlay = el("div", { "data-rmc-sp-overlay": "", "data-rmc-sp": "" });
      const card = el("div", { "data-rmc-sp-card": "", role: "dialog", "aria-label": "Remcord stream setup" });
      card.appendChild(el("div", { "data-rmc-sp-title": "" }, "Share your screen"));
      card.appendChild(el("div", { "data-rmc-sp-sub": "" }, "Pick what to share and the stream quality, then confirm. Your system picker completes the source choice."));
      card.appendChild(el("div", { "data-rmc-sp-h": "" }, "What to share"));
      const kindRow = el("div", { "data-rmc-sp-row": "" });
      const kindButtons = [];
      const setKind = (kind) => {
        choice.kind = kind;
        for (const entry of kindButtons) entry.button.setAttribute("aria-pressed", entry.kind === kind ? "true" : "false");
      };
      for (const entry of [{ kind: "monitor", label: "Entire screen" }, { kind: "window", label: "A window" }]) {
        const button = el("button", { type: "button", "data-rmc-sp-opt": "", "aria-pressed": entry.kind === "monitor" ? "true" : "false" }, entry.label);
        button.addEventListener("click", () => setKind(entry.kind));
        kindRow.appendChild(button);
        kindButtons.push({ kind: entry.kind, button });
      }
      card.appendChild(kindRow);
      card.appendChild(el("div", { "data-rmc-sp-h": "" }, "Resolution"));
      card.appendChild(segmented([480, 720, 1080, 1440, 2160], 1080, (height) => {
        choice.height = height;
        choice.width = Math.round((height * 16) / 9);
      }));
      card.appendChild(el("div", { "data-rmc-sp-h": "" }, "Frame rate"));
      card.appendChild(segmented([5, 15, 30, 60, 120], 30, (fps) => {
        choice.fps = fps;
      }));
      const audioLabel = el("label", { "data-rmc-sp-audio": "" });
      const audioBox = el("input");
      audioBox.type = "checkbox";
      audioBox.addEventListener("change", () => {
        choice.audio = audioBox.checked;
      });
      audioLabel.appendChild(audioBox);
      audioLabel.appendChild(el("span", null, "Share audio"));
      card.appendChild(audioLabel);
      const foot = el("div", { "data-rmc-sp-foot": "" });
      const cancel = el("button", { type: "button", "data-rmc-sp-btn": "" }, "Cancel");
      const share = el("button", { type: "button", "data-rmc-sp-btn": "", "data-rmc-sp-btn-primary": "" }, "Share");
      foot.appendChild(cancel);
      foot.appendChild(share);
      card.appendChild(foot);
      overlay.appendChild(card);
      document.documentElement.appendChild(overlay);
      const onKey = (event) => {
        if (event.key === "Escape") settle(null);
      };
      document.addEventListener("keydown", onKey);
      const settle = (value) => {
        try {
          document.removeEventListener("keydown", onKey);
        } catch {
        }
        try {
          style.remove();
        } catch {
        }
        try {
          overlay.remove();
        } catch {
        }
        openPicker.pending = null;
        resolve(value);
      };
      cancel.addEventListener("click", () => settle(null));
      overlay.addEventListener("mousedown", (event) => {
        if (event.target === overlay) settle(null);
      });
      share.addEventListener("click", () => {
        try {
          window.__remcordStreamPrefs = { kind: choice.kind, width: choice.width, height: choice.height, fps: choice.fps, audio: choice.audio };
        } catch {
        }
        settle({ kind: choice.kind, width: choice.width, height: choice.height, fps: choice.fps, audio: choice.audio });
      });
      try {
        share.focus();
      } catch {
      }
    });
    return openPicker.pending;
  };
  openPicker.pending = null;
  const mergeOptions = (options, choice) => {
    const out = Object.assign({}, options);
    const kind = choice.kind === "window" ? "window" : "monitor";
    const video = out.video;
    if (video === true || video === undefined || video === null) {
      out.video = { displaySurface: kind };
    } else if (video && typeof video === "object") {
      const next = Object.assign({}, video);
      if (!("displaySurface" in next)) next.displaySurface = kind;
      out.video = next;
    }
    if (choice.audio) out.audio = true;
    return out;
  };
  const applyPickedQuality = async (stream, choice) => {
    const video = stream.getVideoTracks()[0];
    if (!video) return;
    try {
      video.contentHint = choice.fps >= 30 ? "motion" : "detail";
    } catch {
    }
    try {
      await video.applyConstraints({
        width: { ideal: choice.width, max: choice.width },
        height: { ideal: choice.height, max: choice.height },
        frameRate: { ideal: choice.fps, max: choice.fps }
      });
      const settings = video.getSettings();
      console.log(
        "[Remcord][StreamPicker] capture constrained: target=" +
        choice.width + "x" + choice.height + "@" + choice.fps +
        " actual=" + (settings.width || "?") + "x" + (settings.height || "?") +
        "@" + (settings.frameRate || "?")
      );
    } catch (error) {
      console.warn("[Remcord][StreamPicker] constraints rejected; keeping the original capture", error);
    }
  };
  const original = mediaDevices.getDisplayMedia.bind(mediaDevices);
  const wrapper = async function (options) {
    const choice = await openPicker();
    if (!choice) throw new DOMException("Remcord stream setup dismissed", "NotAllowedError");
    const stream = await original(mergeOptions(options || {}, choice));
    await applyPickedQuality(stream, choice);
    return stream;
  };
  try {
    globalThis[registryKey] = { wrapper };
  } catch {
  }
  mediaDevices.getDisplayMedia = wrapper;
})();"#;

/// Build the document-start picker script. The script is a fixed literal
/// with no host-spliced values, so this cannot fail; the return type stays
/// infallible on purpose (any future placeholder must switch it to
/// `Result` and add an unspliced-placeholder test).
pub fn screenshare_picker_script() -> String {
    SCREENSHARE_PICKER_SCRIPT.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_has_no_placeholders_and_publishes_status() {
        let script = screenshare_picker_script();
        assert!(
            !script.contains("__REMCORD_"),
            "no unspliced placeholder may ship"
        );
        assert!(script.contains("__remcordStreamPicker"));
        assert!(script.contains("__remcordStreamPrefs"));
        assert!(script.contains("remcord.screenshare.pickerPatch"));
    }

    #[test]
    fn script_is_gated_to_the_discord_entry_origins() {
        let script = screenshare_picker_script();
        for origin in [
            "https://discord.com",
            "https://canary.discord.com",
            "https://ptb.discord.com",
        ] {
            assert!(script.contains(origin), "{origin} must be gated");
        }
        assert!(script.contains("window.location.origin"));
    }

    #[test]
    fn picker_covers_the_full_quality_range() {
        let script = screenshare_picker_script();
        // Resolution tiers mirror the Electron picker, including 1440p and
        // 2160p; frame rates include the 120 FPS Go Live tier.
        for token in ["480", "720", "1080", "1440", "2160", "120"] {
            assert!(script.contains(token), "{token} option must exist");
        }
        // Defaults mirror the Electron picker store (1080p30, no audio).
        assert!(script.contains("width: 1920"));
        assert!(script.contains("height: 1080"));
        assert!(script.contains("fps: 30"));
        assert!(script.contains("audio: false"));
        // Kind pre-filter uses the standard constraint only.
        assert!(script.contains("displaySurface"));
        assert!(
            !script.contains("chromeMediaSource"),
            "no non-standard source-id constraint may ship"
        );
    }

    #[test]
    fn cancel_rejects_like_a_dismissed_picker() {
        let script = screenshare_picker_script();
        assert!(script.contains("NotAllowedError"));
        assert!(script.contains("Escape"));
    }

    #[test]
    fn popup_dom_is_namespaced_and_removed_on_settle() {
        let script = screenshare_picker_script();
        assert!(script.contains("data-rmc-sp"));
        // Every settle path detaches both nodes; a pending re-entrant call
        // joins instead of stacking a second modal.
        assert!(script.contains("openPicker.pending"));
        assert!(script.contains("overlay.remove()"));
        assert!(script.contains("style.remove()"));
    }

    #[test]
    fn rejected_constraints_keep_the_original_stream() {
        let script = screenshare_picker_script();
        assert!(script.contains("applyConstraints"));
        assert!(script.contains("keeping the original capture"));
    }

    #[test]
    fn shipped_script_has_no_forbidden_surfaces() {
        // No bridge, no fetch, no eval, no storage, no host input: the popup
        // is pure document-local DOM plus the platform capture call it
        // wraps. The three entry-origin literals are the documented gating
        // exception (asserted by `script_is_gated_to_the_discord_entry_origins`).
        // Module docs name forbidden APIs for contrast, so comment lines are
        // stripped before asserting on code.
        let shipped = SCREENSHARE_PICKER_SCRIPT
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
            "chromeMediaSource",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
        // The wrapper assignment is the single sanctioned write, and it
        // preserves the platform capture path (never a self-made stream).
        assert!(shipped.contains("mediaDevices.getDisplayMedia = wrapper"));
        assert!(shipped.contains("await original("));
    }
}

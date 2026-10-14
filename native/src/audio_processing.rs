//! Microphone auto-gain-control kill switch (`disableAutogain`).
//!
//! Electron's setting is a no-op upstream (its `disableAutogain.js` asset
//! does not exist, so the preload fetch 404s). The native shell implements
//! it for real: when enabled, the document-start layer wraps
//! `getUserMedia` and forces `autoGainControl: false` on every requested
//! audio track, so Discord's WebRTC stack captures without AGC. When
//! disabled (the default) the wrapper is absent and behavior is exactly
//! current.
//!
//! Origin-gated to the three Discord entry origins, idempotent per
//! document, no bridge/fetch/storage surface — the renderer supplies
//! nothing.

/// Document-start script forcing `autoGainControl: false` on microphone
/// tracks. Wraps `navigator.mediaDevices.getUserMedia` once per document
/// (guarded by a well-known symbol): object-form audio constraints gain
/// `autoGainControl: false` alongside the author's settings, `true`
/// becomes `{ autoGainControl: false }`, and absent/false audio stays
/// untouched. Video-only and audio-less calls pass through byte-identical.
pub fn autogain_script() -> Result<String, serde_json::Error> {
    let script = r#"(function () {
        if (window.Symbol && window[Symbol.for("remcord.disableAutogain")]) {
            return;
        }
        if (window.Symbol) {
            try {
                Object.defineProperty(window, Symbol.for("remcord.disableAutogain"), {
                    value: true,
                    configurable: false,
                    writable: false,
                });
            } catch (e) {}
        }
        try {
            var mediaDevices = navigator.mediaDevices;
            if (!mediaDevices || typeof mediaDevices.getUserMedia !== "function") {
                return;
            }
            var originalGetUserMedia = mediaDevices.getUserMedia.bind(mediaDevices);
            mediaDevices.getUserMedia = function (constraints) {
                try {
                    if (constraints && typeof constraints === "object" && constraints.audio) {
                        var audio = constraints.audio;
                        if (audio === true) {
                            constraints = Object.assign({}, constraints, {
                                audio: { autoGainControl: false },
                            });
                        } else if (typeof audio === "object") {
                            constraints = Object.assign({}, constraints, {
                                audio: Object.assign({}, audio, { autoGainControl: false }),
                            });
                        }
                    }
                } catch (e) {}
                return originalGetUserMedia(constraints);
            };
        } catch (e) {}
    })();"#;
    // Fail closed on template drift: the script must contain its guard and
    // the constraint it enforces, and must never grow a fetch/eval surface.
    if !script.contains("remcord.disableAutogain") || !script.contains("autoGainControl") {
        return Err(serde::de::Error::custom(
            "autogain script lost its guard or constraint",
        ));
    }
    for forbidden in ["fetch(", "XMLHttpRequest", "eval(", "Function("] {
        if script.contains(forbidden) {
            return Err(serde::de::Error::custom(
                "autogain script grew a forbidden surface",
            ));
        }
    }
    Ok(script.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_forces_no_agc_without_network_or_eval_surface() {
        let script = autogain_script().expect("script must build");
        assert!(script.contains("remcord.disableAutogain"));
        assert!(script.contains("autoGainControl: false"));
        assert!(script.contains("getUserMedia"));
        for forbidden in ["fetch(", "XMLHttpRequest", "eval(", "Function("] {
            assert!(!script.contains(forbidden), "{forbidden} must not ship");
        }
    }
}

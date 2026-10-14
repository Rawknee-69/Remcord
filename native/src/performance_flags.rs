//! GPU/CPU performance presets for the WebView2 environment (Electron
//! `src/common/flags.ts` parity, Windows slice).
//!
//! Each preset is a closed table of Chromium switches plus
//! enable/disable-blink-feature lists. The renderer only ever selects the
//! preset NAME (`performanceMode`); every string below is compiled in, so no
//! renderer input can inject a switch. `dynamic` resolves by AC/battery
//! power state (Electron `powerMonitor.isOnBatteryPower()` parity). Unknown
//! or absent modes resolve to no preset (`none`), exactly like Electron's
//! `default:` arm.
//!
//! The shared WebRTC baseline (`webrtcHwCommon` + Windows Media Foundation
//! encode) layers on top whenever `hardwareAcceleration` is on
//! (`applyPlatformVideoStack` win32 parity); `hardwareAcceleration: false`
//! additionally appends `--disable-gpu` (`app.disableHardwareAcceleration()`
//! parity). All of this composes into the environment arguments once at
//! startup (restart-required model): the environment is immutable after
//! creation, so a flip needs a relaunch — same as Electron.

/// One closed Chromium switch set, mirroring `flags.ts Preset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Preset {
    switches: &'static [(&'static str, Option<&'static str>)],
    enable_features: &'static [&'static str],
    disable_features: &'static [&'static str],
}

const PERFORMANCE: Preset = Preset {
    switches: &[
        ("enable-gpu-rasterization", None),
        ("enable-zero-copy", None),
        ("ignore-gpu-blocklist", None),
        (
            "enable-hardware-overlays",
            Some("single-fullscreen,single-on-top,underlay"),
        ),
        ("force_high_performance_gpu", None),
    ],
    enable_features: &[
        "EnableDrDc",
        "CanvasOopRasterization",
        "BackForwardCache:TimeToLiveInBackForwardCacheInSeconds/300/should_ignore_blocklists/true/enable_same_site/true",
        "ThrottleDisplayNoneAndVisibilityHiddenCrossOriginIframes",
        "UseSkiaRenderer",
        "WebAssemblyLazyCompilation",
    ],
    disable_features: &["Vulkan"],
};

const BALANCED: Preset = Preset {
    switches: &[
        ("enable-gpu-rasterization", None),
        ("enable-zero-copy", None),
        ("ignore-gpu-blocklist", None),
    ],
    enable_features: &[
        "CanvasOopRasterization",
        "UseSkiaRenderer",
        "WebAssemblyLazyCompilation",
        "CalculateNativeWinOcclusion",
        "ThrottleDisplayNoneAndVisibilityHiddenCrossOriginIframes",
    ],
    disable_features: &["Vulkan"],
};

const MEMORY: Preset = Preset {
    switches: &[
        ("enable-low-end-device-mode", None),
        ("enable-low-res-tiling", None),
        ("process-per-site", None),
        ("renderer-process-limit", Some("2")),
        ("force_low_power_gpu", None),
        ("disk-cache-size", Some("67108864")),
        ("skia-resource-cache-limit-mb", Some("64")),
    ],
    enable_features: &[
        "CalculateNativeWinOcclusion",
        "TurnOffStreamingMediaCachingOnBattery",
    ],
    disable_features: &[],
};

const VOIP: Preset = Preset {
    switches: &[
        ("enable-gpu-rasterization", None),
        ("enable-zero-copy", None),
        ("ignore-gpu-blocklist", None),
        ("enable-accelerated-video-decode", None),
        ("force_high_performance_gpu", None),
        ("disable-background-timer-throttling", None),
        ("disable-renderer-backgrounding", None),
        ("disable-backgrounding-occluded-windows", None),
        ("enable-gpu-memory-buffer-video-frames", None),
    ],
    enable_features: &[
        "WebRtcHWDecoding",
        "WebRtcHWEncoding",
        "AcceleratedVideoDecoder",
        "AcceleratedVideoEncoder",
        "ZeroCopyDesktopCapture",
    ],
    disable_features: &["UseChromeOSDirectVideoDecoder"],
};

const LATENCY: Preset = Preset {
    switches: &[
        ("enable-gpu-rasterization", None),
        ("enable-zero-copy", None),
        ("ignore-gpu-blocklist", None),
        ("force_high_performance_gpu", None),
        (
            "enable-hardware-overlays",
            Some("single-fullscreen,single-on-top,underlay"),
        ),
        ("disable-background-timer-throttling", None),
        ("disable-renderer-backgrounding", None),
        ("disable-backgrounding-occluded-windows", None),
        ("disable-ipc-flooding-protection", None),
        ("disable-backing-store-limit", None),
    ],
    enable_features: &[
        "EnableDrDc",
        "CanvasOopRasterization",
        "UseSkiaRenderer",
        "WebAssemblyLazyCompilation",
    ],
    disable_features: &["Vulkan"],
};

const SMOOTH_SCREENSHARE: Preset = Preset {
    switches: &[
        ("enable-gpu-rasterization", None),
        ("enable-zero-copy", None),
        ("ignore-gpu-blocklist", None),
        ("enable-accelerated-video-decode", None),
        ("disable-background-timer-throttling", None),
        ("disable-renderer-backgrounding", None),
        (
            "enable-hardware-overlays",
            Some("single-fullscreen,single-on-top,underlay"),
        ),
        ("force_high_performance_gpu", None),
    ],
    enable_features: &[
        "EnableDrDc",
        "CanvasOopRasterization",
        "BackForwardCache:TimeToLiveInBackForwardCacheInSeconds/300/should_ignore_blocklists/true/enable_same_site/true",
        "ThrottleDisplayNoneAndVisibilityHiddenCrossOriginIframes",
        "UseSkiaRenderer",
        "WebAssemblyLazyCompilation",
        "WebRtcHWEncoding",
        "WebRtcHWDecoding",
        "AcceleratedVideoEncoder",
        "AcceleratedVideoDecoder",
        "ZeroCopyDesktopCapture",
    ],
    disable_features: &["Vulkan", "UseChromeOSDirectVideoDecoder"],
};

const BATTERY: Preset = Preset {
    switches: &[
        ("force_low_power_gpu", None),
        ("enable-low-end-device-mode", None),
        ("enable-low-res-tiling", None),
        ("process-per-site", None),
    ],
    enable_features: &[
        "TurnOffStreamingMediaCachingOnBattery",
        "CalculateNativeWinOcclusion",
    ],
    disable_features: &[],
};

/// Shared WebRTC baseline (`webrtcHwCommon` in `flags.ts`).
const WEBRTC_HW_COMMON: Preset = Preset {
    switches: &[
        ("ignore-gpu-blocklist", None),
        ("enable-zero-copy", None),
        ("enable-accelerated-video-decode", None),
        ("enable-gpu-memory-buffer-video-frames", None),
    ],
    enable_features: &[
        "WebRtcHWDecoding",
        "AcceleratedVideoDecoder",
        "ZeroCopyDesktopCapture",
        "CanvasOopRasterization",
    ],
    disable_features: &["UseChromeOSDirectVideoDecoder"],
};

/// Windows Media Foundation encode path (`winVideoEncode` in `flags.ts`).
const WIN_VIDEO_ENCODE: Preset = Preset {
    switches: &[("webrtc-hw-encoding", None), ("webrtc-hw-decoding", None)],
    enable_features: &[
        "WebRtcHWEncoding",
        "AcceleratedVideoEncoder",
        "PlatformH264CbpEncoding",
    ],
    disable_features: &[],
};

/// Resolve the base preset for one `performanceMode` value. `dynamic`
/// consults `on_battery`; unknown values (and `none`) yield no preset,
/// matching Electron's `default:` arm.
fn base_preset(mode: &str, on_battery: bool) -> Option<&'static Preset> {
    match mode {
        "dynamic" => Some(if on_battery { &BATTERY } else { &PERFORMANCE }),
        "performance" => Some(&PERFORMANCE),
        "balanced" => Some(&BALANCED),
        "battery" => Some(&BATTERY),
        "memory" => Some(&MEMORY),
        "voip" => Some(&VOIP),
        "latency" => Some(&LATENCY),
        "smoothScreenshare" => Some(&SMOOTH_SCREENSHARE),
        _ => None,
    }
}

/// True when the machine is currently on battery power (Electron
/// `powerMonitor.isOnBatteryPower()` parity for the `dynamic` preset).
/// Unknown/unreadable status fails safe to AC (`false`).
#[cfg(target_os = "windows")]
fn is_on_battery_power() -> bool {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    unsafe {
        let mut status = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut status).is_err() {
            return false;
        }
        // ACLineStatus: 0 = offline (battery), 1 = online, 255 = unknown.
        status.ACLineStatus == 0
    }
}

/// Non-Windows builds never run this host; fail safe to AC.
#[cfg(not(target_os = "windows"))]
fn is_on_battery_power() -> bool {
    false
}

/// Render one preset's switches plus feature lists as Chromium
/// `--switch[=value]` / `--enable-features=a,b` / `--disable-features=c`
/// arguments, deduplicated. Order is stable (preset order, features last)
/// so logs and the graphics report stay comparable across runs.
///
/// NOTE: feature lists merge into SINGLE comma-joined switches: Chromium
/// keeps only the last occurrence of a repeated switch, so emitting one
/// `--enable-features` per preset would silently drop all but the last.
fn render_preset(preset: &Preset, out: &mut Vec<String>) {
    for (name, value) in preset.switches {
        match value {
            Some(value) => out.push(format!("--{name}={value}")),
            None => out.push(format!("--{name}")),
        }
    }
    if !preset.enable_features.is_empty() {
        out.push(format!("@@enable-features:{}", preset.enable_features.join(",")));
    }
    if !preset.disable_features.is_empty() {
        out.push(format!(
            "@@disable-features:{}",
            preset.disable_features.join(",")
        ));
    }
    out.sort();
    out.dedup();
}

/// Collapse the `@@enable-features:` / `@@disable-features:` placeholders
/// left by [`render_preset`] into single comma-joined switches (plus the
/// plain switches, sorted and deduplicated).
fn collapse_feature_lists(args: &mut Vec<String>) {
    let mut enable: Vec<String> = Vec::new();
    let mut disable: Vec<String> = Vec::new();
    args.retain(|arg| {
        if let Some(list) = arg.strip_prefix("@@enable-features:") {
            enable.extend(list.split(',').map(str::to_owned));
            false
        } else if let Some(list) = arg.strip_prefix("@@disable-features:") {
            disable.extend(list.split(',').map(str::to_owned));
            false
        } else {
            true
        }
    });
    enable.sort();
    enable.dedup();
    disable.sort();
    disable.dedup();
    if !enable.is_empty() {
        args.push(format!("--enable-features={}", enable.join(",")));
    }
    if !disable.is_empty() {
        args.push(format!("--disable-features={}", disable.join(",")));
    }
    args.sort();
    args.dedup();
}

/// Resolve the full extra-argument string for one settings snapshot:
/// performance preset + platform video stack (gated by
/// `hardwareAcceleration`), `--disable-gpu` when acceleration is off,
/// and the scroll/cache toggles. Returns `""` when nothing applies.
/// Pure: no filesystem, no environment, no window work.
pub fn resolve_extra_args(
    performance_mode: &str,
    hardware_acceleration: bool,
    smooth_scroll: bool,
    auto_scroll: bool,
    disable_http_cache: bool,
) -> String {
    let mut args: Vec<String> = Vec::new();
    if let Some(base) = base_preset(performance_mode, is_on_battery_power()) {
        render_preset(base, &mut args);
    }
    if hardware_acceleration {
        let mut video: Vec<String> = Vec::new();
        render_preset(&WEBRTC_HW_COMMON, &mut video);
        render_preset(&WIN_VIDEO_ENCODE, &mut video);
        args.extend(video);
        args.sort();
        args.dedup();
    } else {
        // Electron `app.disableHardwareAcceleration()` parity.
        args.push("--disable-gpu".to_owned());
    }
    if !smooth_scroll {
        // Electron `disable-smooth-scrolling` parity (`src/main.ts`).
        args.push("--disable-smooth-scrolling".to_owned());
    }
    if auto_scroll {
        // Electron `MiddleClickAutoscroll` blink-feature parity.
        args.push("--enable-blink-features=MiddleClickAutoscroll".to_owned());
    }
    if disable_http_cache {
        // Electron `disable-http-cache` parity (`src/main.ts`).
        args.push("--disable-http-cache".to_owned());
    }
    collapse_feature_lists(&mut args);
    args.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_and_none_modes_resolve_to_no_preset() {
        for mode in ["none", "", "turbo", "PERFORMANCE"] {
            assert_eq!(base_preset(mode, false), None, "{mode} must have no preset");
            assert_eq!(base_preset(mode, true), None, "{mode} must have no preset");
        }
    }

    #[test]
    fn dynamic_follows_battery_state() {
        assert!(base_preset("dynamic", true).is_some());
        assert!(base_preset("dynamic", false).is_some());
    }

    #[test]
    fn named_modes_resolve() {
        for mode in [
            "performance",
            "balanced",
            "battery",
            "memory",
            "voip",
            "latency",
            "smoothScreenshare",
        ] {
            assert!(base_preset(mode, false).is_some(), "{mode} must resolve");
        }
    }

    #[test]
    fn defaults_produce_only_the_video_stack() {
        // hardwareAcceleration=true layers the WebRTC baseline + Windows
        // Media Foundation encode (Electron applyPlatformVideoStack parity)
        // even with performanceMode "none".
        let args = resolve_extra_args("none", true, true, false, false);
        assert!(args.contains("--enable-accelerated-video-decode"), "{args}");
        assert!(args.contains("WebRtcHWEncoding"), "{args}");
        assert!(!args.contains("--disable-gpu"), "{args}");
        // Full off (acceleration off, toggles at no-op poles, mode none)
        // still carries the explicit disable-gpu marker.
        let off = resolve_extra_args("none", false, true, false, false);
        assert_eq!(off, "--disable-gpu");
    }

    #[test]
    fn toggles_render_exact_switches() {
        let args = resolve_extra_args("none", false, false, true, true);
        for expected in [
            "--disable-gpu",
            "--disable-smooth-scrolling",
            "--enable-blink-features=MiddleClickAutoscroll",
            "--disable-http-cache",
        ] {
            assert!(args.contains(expected), "missing {expected} in {args}");
        }
    }

    #[test]
    fn performance_renders_gpu_switches() {
        let args = resolve_extra_args("performance", true, true, false, false);
        assert!(args.contains("--enable-gpu-rasterization"), "{args}");
        assert!(args.contains("--enable-features="), "{args}");
        assert!(args.contains("WebRtcHWEncoding"), "{args}");
    }

    #[test]
    fn feature_lists_collapse_to_single_switches() {
        // Chromium keeps only the last repeated switch: merged presets
        // must emit exactly one enable list and one disable list.
        let args = resolve_extra_args("performance", true, true, false, false);
        assert_eq!(
            args.match_indices("--enable-features=").count(),
            1,
            "{args}"
        );
        assert_eq!(
            args.match_indices("--disable-features=").count(),
            1,
            "{args}"
        );
        // No placeholder leaks into the environment string.
        assert!(!args.contains("@@"), "{args}");
    }
}

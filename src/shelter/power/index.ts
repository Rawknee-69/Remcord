const {
    util: { log },
    flux: { dispatcher },
} = shelter;

function track(payload: { event: string; properties: { enabled: string } }) {
    if (payload.event === "join_voice_channel") {
        window.legcord.power.setPowerSaving(true);
    } else if (payload.event === "leave_voice_channel") {
        window.legcord.power.setPowerSaving(false);
    }
}

function subscribePowerTracking() {
    log("Legcord Power Integration");
    dispatcher.subscribe("TRACK", track);
}

/** True when running under the native WebView2 shell (Phase C+). */
function isNativeWebView(): boolean {
    return (window.legcord as unknown as { boot?: { runtime?: unknown } })?.boot?.runtime === "webview2";
}

/** Minimal native settings shape for this caller. Full snapshot documented in `native/src/settings.rs`. */
interface NativePowerSettings {
    blockPowerSavingInVoiceChat?: unknown;
}

/**
 * Native WebView2 path: read `blockPowerSavingInVoiceChat` through the typed
 * async `settings.get_config` capability. Never blocks document startup; the
 * subscription is installed once the snapshot resolves.
 */
async function onLoadNative(): Promise<void> {
    const getConfig = (window.legcord as unknown as { settings: { getConfig: () => Promise<NativePowerSettings> } })
        .settings.getConfig;
    const settings = await getConfig();
    if (settings.blockPowerSavingInVoiceChat !== true) return;
    subscribePowerTracking();
}

export function onLoad() {
    if (isNativeWebView()) {
        void onLoadNative();
        return;
    }
    // Electron path (unchanged synchronous implementation).
    const settings = window.legcord.settings.getConfig();
    if (!settings.blockPowerSavingInVoiceChat) return;
    subscribePowerTracking();
}

export function onUnload() {
    dispatcher.unsubscribe("TRACK", track);
}

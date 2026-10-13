/**
 * Renderer-side Go Live stream-parameter control for the native shell.
 *
 * Why this exists: the document-start capture cap (`applyConstraints` on
 * the display track) is rejected by this WebView2 build
 * (`OverconstrainedError`, proven in the field), so Balanced/Low capped
 * nothing. Discord's own picker is absent in this client flow, so there is
 * no working quality control at all. This module ports the *sender-level*
 * half of the proven Electron technique
 * (`src/shelter/screenshare/index.tsx::patchStreamQuality`): it caps what
 * Discord encodes/sends (resolution, frame rate, bitrate ceiling) through
 * Discord's own stream parameters instead of the capture track.
 *
 * Deliberate differences from the Electron plugin (kept minimal on purpose):
 * - No `MEDIA_ENGINE_SET_GO_LIVE_SOURCE` dispatch interception: subscribe
 *   only (`MEDIA_ENGINE_VIDEO_SOURCE_QUALITY_CHANGED` + bounded reapplies),
 *   so Discord's dispatch flow is never rewritten.
 * - No codec preferences: the encoder (OpenH264 vs platform hardware) is
 *   negotiated by Discord/Chromium and never forced here.
 * - No venmic/audio handling: Electron-only concern.
 * - Restart-required model like the rest of this panel: the mode is read
 *   once at install (launch-time store) and the subscription is installed
 *   only for active modes. Default installs nothing.
 *
 * Fail-open throughout: missing flux, logged-out user, no local sender, or
 * misshapen connection objects all mean "do nothing" — sharing is never
 * affected. Requested maxima are caps Discord adapts *below* (bandwidth,
 * content); they are reported as targets, never as achieved settings.
 */

import type { ScreenshareQualityMode } from "./settings.js";

/**
 * Live plugin store (read lazily — never destructured at import — so test
 * harnesses and late shelter globals resolve the current object, and a
 * missing plugin degrades to Default instead of throwing at load).
 */
function storeSettings(): Record<string, unknown> | null {
    try {
        const shelterGlobal = (globalThis as unknown as { shelter?: { plugin?: { store?: unknown } } }).shelter;
        const settings = (shelterGlobal?.plugin?.store as { settings?: unknown } | undefined)?.settings;
        return isRecord(settings) ? settings : null;
    } catch {
        return null;
    }
}

const BITRATE_CEILING = 25_000_000;
const REAPPLY_DELAYS_MS = [1000, 3000, 8000] as const;

/** Outgoing-stream target per active mode (Default has none: untouched). */
export const STREAM_TARGETS: Record<
    Exclude<ScreenshareQualityMode, "default">,
    { width: number; height: number; fps: number }
> = {
    "ultra-low": { width: 256, height: 144, fps: 15 },
    low: { width: 960, height: 540, fps: 15 },
    balanced: { width: 1280, height: 720, fps: 30 },
    "full-hd": { width: 1920, height: 1080, fps: 30 },
    "quad-hd": { width: 2560, height: 1440, fps: 30 },
    "ultra-hd": { width: 3840, height: 2160, fps: 30 },
};

/** Discord-like Go Live targets (bits/s) by height and fps band (Electron parity). */
function targetBitrateFor(height: number, fps: number): number {
    const highFps = fps > 30;
    const table: Record<number, [number, number]> = {
        480: [2_500_000, 3_500_000],
        720: [4_000_000, 6_000_000],
        1080: [8_000_000, 10_000_000],
        1440: [12_000_000, 15_000_000],
        2160: [18_000_000, 25_000_000],
    };
    const nearest = Object.keys(table)
        .map(Number)
        .sort((a, b) => Math.abs(a - height) - Math.abs(b - height))[0];
    const [low, high] = table[nearest] ?? [4_000_000, 6_000_000];
    return Math.min(highFps ? high : low, BITRATE_CEILING);
}

function isRecord(value: unknown): value is Record<string, unknown> {
    return !!value && typeof value === "object";
}

type FluxShape = {
    dispatcher?: {
        subscribe?: (event: string, handler: (dispatch: unknown) => void) => void;
        unsubscribe?: (event: string, handler: (dispatch: unknown) => void) => void;
    };
    stores?: {
        UserStore?: { getCurrentUser?: () => unknown };
        MediaEngineStore?: { getMediaEngine?: () => unknown };
    };
};

function fluxShape(): FluxShape | null {
    try {
        const shelterGlobal = (globalThis as unknown as { shelter?: { flux?: unknown } }).shelter;
        const flux = shelterGlobal?.flux;
        return isRecord(flux) ? (flux as FluxShape) : null;
    } catch {
        return null;
    }
}

function currentMode(): ScreenshareQualityMode {
    try {
        // Single source of truth: any non-default id with a stream target
        // is active, so new presets need no branch updates here.
        const raw = storeSettings()?.screenshareQuality;
        return typeof raw === "string" && raw !== "default" && raw in STREAM_TARGETS
            ? (raw as ScreenshareQualityMode)
            : "default";
    } catch {
        return "default";
    }
}

function localStreamConnection(): Record<string, unknown> | null {
    try {
        const flux = fluxShape();
        const currentUserId = flux?.stores?.UserStore?.getCurrentUser?.() as { id?: unknown } | undefined;
        const engine = flux?.stores?.MediaEngineStore?.getMediaEngine?.() as { connections?: unknown } | undefined;
        if (typeof currentUserId?.id !== "string" || currentUserId.id.length === 0 || !engine) return null;
        const connections = engine.connections;
        const list: unknown[] = Array.isArray(connections) ? connections : [...(connections as Iterable<unknown>)];
        const found = list.find((entry) => isRecord(entry) && entry.streamUserId === currentUserId.id);
        return found && isRecord(found) ? found : null;
    } catch {
        return null;
    }
}

/** Cap RTP framerate / raise the bitrate ceiling without forcing scale (Electron parity). */
async function applySenderEncodeLimits(
    pc: { getSenders?: () => Array<Record<string, unknown>> },
    fps: number,
    maxBitrate: number,
): Promise<void> {
    try {
        const sender = pc
            .getSenders?.()
            .find((entry) => isRecord(entry?.track) && (entry.track as Record<string, unknown>).kind === "video") as
            | {
                  getParameters?: () => { encodings?: Array<Record<string, unknown>> };
                  setParameters?: (params: unknown) => Promise<void>;
              }
            | undefined;
        const params = sender?.getParameters?.();
        if (!params || typeof sender?.setParameters !== "function") return;
        if (!Array.isArray(params.encodings) || params.encodings.length === 0) {
            params.encodings = [{}];
        }
        for (const encoding of params.encodings) {
            if (!isRecord(encoding)) continue;
            // Raise whenever Discord/Chromium left a lower ceiling — never
            // fight intentional downscales by forcing scaleResolutionDownBy.
            const current = encoding.maxBitrate;
            if (typeof current !== "number" || current < maxBitrate) {
                encoding.maxBitrate = maxBitrate;
            }
            encoding.maxFramerate = fps;
        }
        await sender.setParameters(params);
    } catch {
        // Fail open: sender limits are best-effort.
    }
}

/**
 * Applies the active mode's stream parameters to the local Go Live
 * connection. Returns `true` when a connection was patched, `false` when
 * there was nothing to do (default mode, logged out, idle, or misshapen).
 * Never throws.
 */
export function patchScreenshareStream(reason: string): boolean {
    try {
        const mode = currentMode();
        if (mode === "default") return false;
        const target = STREAM_TARGETS[mode];
        const connection = localStreamConnection();
        if (!connection) return false;

        const targetBitrate = targetBitrateFor(target.height, target.fps);
        const bitrateMin = Math.round(targetBitrate * 0.8);
        const bitrateMax = Math.min(Math.round(targetBitrate * 1.2), BITRATE_CEILING);

        const streamParams = connection.videoStreamParameters;
        const params = Array.isArray(streamParams) ? streamParams[0] : undefined;
        if (isRecord(params)) {
            params.maxFrameRate = target.fps;
            params.maxResolution = { type: "fixed", width: target.width, height: target.height };
            params.maxPixelCount = target.width * target.height;
            params.maxBitrate = bitrateMax;
        }

        const qualityManager = connection.videoQualityManager;
        const golive =
            isRecord(qualityManager) && isRecord(qualityManager.goliveMaxQuality)
                ? (qualityManager.goliveMaxQuality as Record<string, unknown>)
                : undefined;
        if (golive) {
            golive.bitrateMin = bitrateMin;
            golive.bitrateMax = bitrateMax;
            golive.bitrateTarget = targetBitrate;
        }

        const pc = connection.pc ?? connection.peerConnection ?? connection._pc;
        if (isRecord(pc)) {
            void applySenderEncodeLimits(
                pc as { getSenders?: () => Array<Record<string, unknown>> },
                target.fps,
                bitrateMax,
            );
        }

        console.log(
            `[Remcord][ScreenshareStream] patched local stream (${reason}) to at most ${target.width}x${target.height}@${target.fps} (bitrate target ${(targetBitrate / 1_000_000).toFixed(1)} Mbps, max ${((bitrateMax as number) / 1_000_000).toFixed(1)} Mbps). Discord adapts below these maxima.`,
        );
        return true;
    } catch {
        return false;
    }
}

let reapplyTimers: ReturnType<typeof setTimeout>[] = [];
let reapplyGeneration = 0;

function clearReapplyTimers(): void {
    try {
        for (const timer of reapplyTimers) clearTimeout(timer);
    } catch {
        // Fail open.
    }
    reapplyTimers = [];
    reapplyGeneration++;
}

function scheduleReapplies(): void {
    clearReapplyTimers();
    const generation = reapplyGeneration;
    for (const delay of REAPPLY_DELAYS_MS) {
        try {
            const timer = setTimeout(() => {
                if (generation !== reapplyGeneration) return;
                if (!localStreamConnection()) return;
                patchScreenshareStream(`reapply@${delay}ms`);
            }, delay);
            reapplyTimers.push(timer);
        } catch {
            // Fail open: fewer reapplies, same safety.
        }
    }
}

function onStreamQualityChange(): void {
    if (!patchScreenshareStream("quality-changed")) return;
    scheduleReapplies();
}

function onStreamEnd(dispatch: unknown): void {
    try {
        if (!isRecord(dispatch)) return;
        const streamKey = dispatch.streamKey;
        if (typeof streamKey !== "string") return;
        const owner = streamKey.split(":").at(-1);
        const flux = fluxShape();
        const currentUserId = flux?.stores?.UserStore?.getCurrentUser?.() as { id?: unknown } | undefined;
        if (owner === currentUserId?.id) clearReapplyTimers();
    } catch {
        // Fail open.
    }
}

let installed = false;

/**
 * Installs the stream-parameter control when the launch-time mode is
 * active. Returns `true` when subscribed, `false` when there was nothing
 * to install (default mode or missing flux). Never throws; safe to call
 * from plugin `onLoad`. Pair with `uninstallScreenshareStreamControl`
 * from `onUnload`.
 */
export function installScreenshareStreamControl(): boolean {
    try {
        if (installed) return true;
        if (currentMode() === "default") return false;
        const dispatcher = fluxShape()?.dispatcher;
        if (typeof dispatcher?.subscribe !== "function") return false;
        dispatcher.subscribe("MEDIA_ENGINE_VIDEO_SOURCE_QUALITY_CHANGED", onStreamQualityChange);
        dispatcher.subscribe("STREAM_DELETE", onStreamEnd);
        installed = true;
        return true;
    } catch {
        return false;
    }
}

/** Removes the control's subscriptions and timers. Never throws. */
export function uninstallScreenshareStreamControl(): void {
    try {
        clearReapplyTimers();
        if (!installed) return;
        const dispatcher = fluxShape()?.dispatcher;
        if (typeof dispatcher?.unsubscribe === "function") {
            dispatcher.unsubscribe("MEDIA_ENGINE_VIDEO_SOURCE_QUALITY_CHANGED", onStreamQualityChange);
            dispatcher.unsubscribe("STREAM_DELETE", onStreamEnd);
        }
    } catch {
        // Fail open.
    } finally {
        installed = false;
    }
}

/** Test-only reset for the install-once flag (production code never calls this). */
export function resetScreenshareStreamControlForTests(): void {
    try {
        clearReapplyTimers();
    } catch {
        // Fail open.
    }
    installed = false;
}

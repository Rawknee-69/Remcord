export type DisplayMediaInterceptor = (
    original: MediaDevices["getDisplayMedia"],
    thisArg: MediaDevices,
    options: DisplayMediaStreamOptions,
) => Promise<MediaStream>;

type DisplayMediaPatchState = {
    original: MediaDevices["getDisplayMedia"];
    wrapper: MediaDevices["getDisplayMedia"];
    interceptor: DisplayMediaInterceptor;
};

const patchRegistryKey = Symbol.for("legcord.screenshare.getDisplayMediaPatch");

function getPatchRegistry(): WeakMap<MediaDevices, DisplayMediaPatchState> {
    const existing = Reflect.get(globalThis, patchRegistryKey) as unknown;
    if (existing === undefined) {
        const registry = new WeakMap<MediaDevices, DisplayMediaPatchState>();
        if (!Reflect.set(globalThis, patchRegistryKey, registry)) {
            throw new Error("Unable to register the Legcord screenshare patch");
        }
        return registry;
    }

    if (!(existing instanceof WeakMap)) {
        throw new TypeError("Invalid Legcord screenshare patch registry");
    }
    return existing as WeakMap<MediaDevices, DisplayMediaPatchState>;
}

function callOriginal(
    original: MediaDevices["getDisplayMedia"],
    thisArg: MediaDevices,
    options: DisplayMediaStreamOptions,
): Promise<MediaStream> {
    return original.call(thisArg, options);
}

export function installDisplayMediaPatch(mediaDevices: MediaDevices, interceptor: DisplayMediaInterceptor): void {
    const registry = getPatchRegistry();
    const current = mediaDevices.getDisplayMedia;
    const previous = registry.get(mediaDevices);

    if (previous && current === previous.wrapper) {
        previous.interceptor = interceptor;
        return;
    }

    if (previous) {
        previous.interceptor = callOriginal;
        registry.delete(mediaDevices);
    }

    let state: DisplayMediaPatchState;
    const wrapper: MediaDevices["getDisplayMedia"] = async function (this: MediaDevices, options = {}) {
        return state.interceptor(state.original, this, options);
    };
    state = { original: current, wrapper, interceptor };
    registry.set(mediaDevices, state);
    mediaDevices.getDisplayMedia = wrapper;
}

export function restoreDisplayMediaPatch(mediaDevices: MediaDevices): void {
    const registry = getPatchRegistry();
    const state = registry.get(mediaDevices);
    if (!state) return;

    state.interceptor = callOriginal;
    if (mediaDevices.getDisplayMedia === state.wrapper) {
        mediaDevices.getDisplayMedia = state.original;
    }
    registry.delete(mediaDevices);
}

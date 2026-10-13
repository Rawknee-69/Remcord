(() => {
    // biome-ignore lint/suspicious/noRedundantUseStrict: Injected as a classic document-start script.
    "use strict";

    // Document-start storage preservation shared by every client mod.
    // Discord can delete `window.localStorage` during SPA initialization; the
    // capture below keeps standard web storage reachable through
    // `Window.prototype` so mod bundles keep working on fast reloads.
    // Entry origins for the three release channels (stable/canary/ptb):
    // the channel setting selects which one the host navigates to, and this
    // script must initialize on any of them. Exact match only.
    if (
        location.origin !== "https://discord.com" &&
        location.origin !== "https://canary.discord.com" &&
        location.origin !== "https://ptb.discord.com"
    )
        return;
    const describe = (descriptor) => {
        if (!descriptor) return "missing";
        if (Object.hasOwn(descriptor, "value")) return `value:${typeof descriptor.value}`;
        return `getter:${typeof descriptor.get}`;
    };
    let localStorageApi;
    try {
        localStorageApi = typeof window.localStorage?.getItem;
    } catch (error) {
        localStorageApi = `blocked:${error?.name || "unknown"}`;
    }
    try {
        const nativeStorage = window.localStorage;
        if (nativeStorage && typeof Window !== "undefined" && Window.prototype) {
            if (!Object.getOwnPropertyDescriptor(Window.prototype, "localStorage")) {
                Object.defineProperty(Window.prototype, "localStorage", {
                    configurable: true,
                    enumerable: true,
                    get: () => nativeStorage,
                });
            }
        }
    } catch {}
    Object.defineProperty(window, "__remcordNativeStorageAtDocumentStart", {
        configurable: false,
        enumerable: false,
        value: Object.freeze({
            localStorageApi,
            windowDescriptor: describe(Object.getOwnPropertyDescriptor(window, "localStorage")),
            prototypeDescriptor: describe(
                Object.getOwnPropertyDescriptor(Object.getPrototypeOf(window), "localStorage"),
            ),
            storageConstructor: typeof Storage,
        }),
    });
})();

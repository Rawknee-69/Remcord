/// <reference path="../../../node_modules/@uwu/shelter-defs/dist/shelter-defs/rootdefs.d.ts" />

const {
    util: { log },
    flux: { storesFlat },
    ui: { showToast },
    observeDom,
} = shelter;

interface GalleryMeta {
    guildId: string;
    guildName: string;
    channelId: string;
    channelName: string;
}

interface PendingSave {
    at: number;
    urls: string[];
    meta: GalleryMeta;
}

let pending: PendingSave | null = null;
let pendingTimer: ReturnType<typeof setTimeout> | undefined;
let unobserveMenus: (() => void) | undefined;

const PENDING_TTL_MS = 2500;
const INJECTED_ATTR = "data-remcord-gallery";

function galleryBridge(): { addUrl: (url: string, meta: GalleryMeta) => Promise<unknown> } | undefined {
    const gallery = (window.remcord as unknown as { gallery?: unknown }).gallery as { addUrl?: unknown } | undefined;
    if (gallery && typeof gallery.addUrl === "function") {
        return gallery as { addUrl: (url: string, meta: GalleryMeta) => Promise<unknown> };
    }
    return undefined;
}

function isHttpUrl(url: string): boolean {
    return /^https?:\/\//i.test(url);
}

function currentMeta(): GalleryMeta {
    const meta: GalleryMeta = { guildId: "", guildName: "", channelId: "", channelName: "" };
    try {
        const guildId: string = storesFlat.SelectedGuildStore?.getGuildId?.() ?? "";
        const channelId: string = storesFlat.SelectedChannelStore?.getChannelId?.() ?? "";
        meta.guildId = guildId ?? "";
        meta.channelId = channelId ?? "";
        try {
            meta.guildName = (guildId && storesFlat.GuildStore?.getGuild?.(guildId)?.name) || "";
        } catch {}
        try {
            meta.channelName = (channelId && storesFlat.ChannelStore?.getChannel?.(channelId)?.name) || "";
        } catch {}
    } catch {}
    return meta;
}

function mediaUrlsIn(root: Element): string[] {
    const found: string[] = [];
    const push = (url: string | null | undefined) => {
        if (typeof url === "string" && isHttpUrl(url) && !found.includes(url)) found.push(url);
    };
    for (const img of root.querySelectorAll("img")) {
        push((img as HTMLImageElement).currentSrc || img.getAttribute("src"));
    }
    for (const video of root.querySelectorAll("video")) {
        push((video as HTMLVideoElement).currentSrc || video.getAttribute("src"));
        const source = video.querySelector("source");
        if (source) push(source.getAttribute("src"));
    }
    return found.slice(0, 10);
}

function setPending(urls: string[]): void {
    if (pendingTimer !== undefined) clearTimeout(pendingTimer);
    if (urls.length === 0) {
        pending = null;
        pendingTimer = undefined;
        return;
    }
    pending = { at: Date.now(), urls, meta: currentMeta() };
    pendingTimer = setTimeout(() => {
        pending = null;
        pendingTimer = undefined;
    }, PENDING_TTL_MS);
}

function takePending(): PendingSave | null {
    const p = pending;
    pending = null;
    if (pendingTimer !== undefined) {
        clearTimeout(pendingTimer);
        pendingTimer = undefined;
    }
    if (!p || Date.now() - p.at > PENDING_TTL_MS) return null;
    return p;
}

/**
 * Capture-phase recorder: Discord renders its own menu and stops the event,
 * so by the time any menu exists the original target is gone. We snapshot
 * what was right-clicked here and never interfere with the event itself.
 */
function onContextMenu(event: MouseEvent): void {
    try {
        if (!galleryBridge()) return;
        const target = event.target as Element | null;
        if (!target || typeof (target as Element).closest !== "function") return;
        const el = target as Element;

        const img = el.closest("img");
        if (img) {
            const src = (img as HTMLImageElement).currentSrc || img.getAttribute("src") || "";
            if (isHttpUrl(src)) {
                setPending([src]);
                return;
            }
        }
        const video = el.closest("video");
        if (video) {
            const v = video as HTMLVideoElement;
            const src =
                v.currentSrc || v.getAttribute("src") || video.querySelector("source")?.getAttribute("src") || "";
            if (isHttpUrl(src)) {
                setPending([src]);
                return;
            }
        }
        const anchor = el.closest("a[href]");
        if (anchor) {
            const href = anchor.getAttribute("href") || "";
            if (isHttpUrl(href)) {
                setPending([href]);
                return;
            }
        }
        // Message background: offer every image/video in its content and
        // accessories (avatars live outside those containers, so they are
        // never swept up).
        const message = el.closest('li[id^="chat-messages-"]');
        if (message) {
            const scopes: Element[] = [];
            for (const part of message.querySelectorAll('[id^="message-content-"], [id^="message-accessories-"]')) {
                scopes.push(part);
            }
            const urls: string[] = [];
            for (const scope of scopes.length > 0 ? scopes : [message]) {
                for (const url of mediaUrlsIn(scope)) {
                    if (!urls.includes(url)) urls.push(url);
                }
            }
            setPending(urls.slice(0, 10));
            return;
        }
        setPending([]);
    } catch {
        /* never break page input */
    }
}

/** Deepest leaf element holding direct text — Discord's menu-item label slot. */
function findLabelNode(item: Element): Text | null {
    let best: Text | null = null;
    let bestDepth = -1;
    const walker = document.createTreeWalker(item, NodeFilter.SHOW_TEXT);
    let node = walker.nextNode() as Text | null;
    while (node) {
        if (node.data.trim().length > 0) {
            let depth = 0;
            let parent: Node | null = node.parentNode;
            while (parent && parent !== item) {
                depth++;
                parent = parent.parentNode;
            }
            if (depth > bestDepth) {
                bestDepth = depth;
                best = node;
            }
        }
        node = walker.nextNode() as Text | null;
    }
    return best;
}

function closeMenu(): void {
    try {
        document.dispatchEvent(
            new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true, cancelable: true }),
        );
    } catch {
        /* menu stays open; harmless */
    }
}

function saveUrls(urls: string[], meta: GalleryMeta): void {
    const bridge = galleryBridge();
    if (!bridge) return;
    void Promise.all(urls.map((url) => bridge.addUrl(url, meta))).then(
        () =>
            showToast({
                title: urls.length === 1 ? "Saved to Gallery" : `Saved ${urls.length} items to Gallery`,
                duration: 2500,
            }),
        () => showToast({ title: "Could not save to Gallery", duration: 3000 }),
    );
    closeMenu();
}

function injectIntoMenu(menu: HTMLElement): void {
    try {
        if (menu.hasAttribute(INJECTED_ATTR)) return;
        const save = takePending();
        if (!save) return;
        menu.setAttribute(INJECTED_ATTR, "1");

        const template = menu.querySelector('[role="menuitem"]');
        if (!template) return;
        const labelNode = findLabelNode(template);
        if (!labelNode) return;

        const item = template.cloneNode(true) as HTMLElement;
        item.removeAttribute("id");
        item.setAttribute("aria-label", "Save to Gallery");
        const itemLabel = findLabelNode(item);
        if (!itemLabel) return;
        itemLabel.data = save.urls.length === 1 ? "Save to Gallery" : `Save to Gallery (${save.urls.length})`;

        item.addEventListener(
            "click",
            (event) => {
                event.stopPropagation();
                event.preventDefault();
                saveUrls(save.urls, save.meta);
            },
            { capture: true },
        );

        // Prominent but not first: ahead of the first separator when the
        // menu is grouped, otherwise appended at the end.
        const separator = menu.querySelector('[role="separator"]');
        if (separator && separator.parentNode === menu) {
            menu.insertBefore(item, separator);
        } else if (separator?.parentNode) {
            separator.parentNode.insertBefore(item, separator);
        } else {
            menu.appendChild(item);
        }
    } catch {
        /* never break Discord menus */
    }
}

export function onLoad() {
    log("Remcord Gallery context menu");
    document.addEventListener("contextmenu", onContextMenu, true);
    try {
        const handle = observeDom('[role="menu"]', (node) => injectIntoMenu(node as HTMLElement));
        unobserveMenus = () => handle();
    } catch {
        unobserveMenus = undefined;
    }
}

export function onUnload() {
    document.removeEventListener("contextmenu", onContextMenu, true);
    try {
        unobserveMenus?.();
    } catch {}
    unobserveMenus = undefined;
    pending = null;
    if (pendingTimer !== undefined) {
        clearTimeout(pendingTimer);
        pendingTimer = undefined;
    }
}

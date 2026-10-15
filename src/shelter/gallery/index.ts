/// <reference path="../../../node_modules/@uwu/shelter-defs/dist/shelter-defs/rootdefs.d.ts" />

const {
    util: { log },
    flux: { storesFlat },
    ui: { showToast },
    observeDom,
} = shelter;

// Self-contained kind/file-name helpers (mirroring
// `src/common/galleryUtils.ts`, which this plugin cannot import: lune
// bundles each plugin dir in isolation). Kept in sync by inspection;
// only used for popup button visibility and title prefill, never for
// storage decisions (the host derives the stored kind itself).
const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "svg", "ico", "apng"]);
const VIDEO_EXT = new Set(["mp4", "webm", "mov", "mkv", "avi", "m4v", "ogv"]);

function detectKind(url: string): "image" | "video" | "link" | "file" {
    let pathPart = "";
    try {
        pathPart = new URL(url).pathname.toLowerCase();
    } catch {
        pathPart = url.toLowerCase().split("?")[0];
    }
    const ext = pathPart.split(".").pop() ?? "";
    if (IMAGE_EXT.has(ext)) return "image";
    if (VIDEO_EXT.has(ext)) return "video";
    if (!url) return "file";
    return "link";
}

function fileNameFromUrl(url: string, fallback = "saved-file"): string {
    try {
        const parsed = new URL(url);
        const last = parsed.pathname.split("/").filter(Boolean).pop() ?? "";
        const clean = decodeURIComponent(last.split("?")[0]).slice(0, 128);
        return clean || fallback;
    } catch {
        return fallback;
    }
}

// Discord's effective appearance, read from the resolved
// `--background-primary` (stock dark/light or any custom theme). Keeps the
// popup's native controls and scrollbars on Discord's theme. Best-effort:
// falls back to the OS preference, never throws.
function discordTheme(): "light" | "dark" {
    try {
        const raw = getComputedStyle(document.documentElement)
            .getPropertyValue("--background-primary")
            .trim()
            .toLowerCase();
        let r = -1;
        let g = -1;
        let b = -1;
        const hex = raw.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/);
        if (hex) {
            const h = hex[1];
            const full =
                h.length === 3
                    ? h
                          .split("")
                          .map((c) => c + c)
                          .join("")
                    : h;
            r = parseInt(full.slice(0, 2), 16);
            g = parseInt(full.slice(2, 4), 16);
            b = parseInt(full.slice(4, 6), 16);
        } else {
            const rgb = raw.match(/^rgba?\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})/);
            if (rgb) {
                r = Number(rgb[1]);
                g = Number(rgb[2]);
                b = Number(rgb[3]);
            }
        }
        if (r >= 0 && g >= 0 && b >= 0 && r <= 255 && g <= 255 && b <= 255) {
            return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255 > 0.4 ? "light" : "dark";
        }
    } catch {}
    try {
        return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
    } catch {
        return "dark";
    }
}

interface GalleryMeta {
    guildId: string;
    guildName: string;
    channelId: string;
    channelName: string;
}

interface GalleryBridge {
    addUrl: (
        url: string,
        meta: GalleryMeta & { title?: string; note?: string; categories?: string[] },
    ) => Promise<unknown>;
    categories: () => Promise<unknown>;
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

function galleryBridge(): GalleryBridge | undefined {
    const gallery = (window.remcord as unknown as { gallery?: unknown }).gallery as
        | {
              addUrl?: unknown;
              categories?: unknown;
          }
        | undefined;
    if (gallery && typeof gallery.addUrl === "function" && typeof gallery.categories === "function") {
        return gallery as GalleryBridge;
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

const POPUP_STYLE_ID = "remcord-gallery-popup-style";

function ensurePopupStyles(): void {
    if (document.getElementById(POPUP_STYLE_ID)) return;
    const style = document.createElement("style");
    style.id = POPUP_STYLE_ID;
    style.textContent = `
.remcord-gal-overlay{position:fixed;inset:0;z-index:99999;display:flex;align-items:center;justify-content:center;background:rgba(0,0,0,.62);padding:24px;box-sizing:border-box}
.remcord-gal-dialog{width:min(420px,100%);max-height:calc(100vh - 48px);overflow-y:auto;background:var(--background-primary,#fff);color:var(--text-normal,#060607);border:1px solid var(--border-subtle,rgba(0,0,0,.15));border-radius:12px;box-shadow:var(--shadow-high,0 8px 32px rgba(0,0,0,.4));padding:16px;box-sizing:border-box;font-family:var(--font-primary,inherit);font-size:14px}
.remcord-gal-title{margin:0 0 4px;font-size:16px;font-weight:700}
.remcord-gal-sub{margin:0 0 12px;font-size:12px;color:var(--text-muted,#5c5e66);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.remcord-gal-label{display:block;margin:10px 0 4px;font-size:12px;font-weight:700;text-transform:uppercase;letter-spacing:.02em;color:var(--text-muted,#5c5e66)}
.remcord-gal-input,.remcord-gal-textarea,.remcord-gal-select{width:100%;box-sizing:border-box;padding:8px 10px;border:1px solid var(--input-border-default,rgba(0,0,0,.2));border-radius:8px;background:var(--input-background-default,transparent);color:var(--input-text-default,inherit);font:inherit;font-size:14px}
.remcord-gal-input:focus,.remcord-gal-textarea:focus,.remcord-gal-select:focus{outline:2px solid var(--brand-500,#5865f2);outline-offset:-1px}
.remcord-gal-textarea{min-height:64px;resize:vertical}
.remcord-gal-actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:14px}
.remcord-gal-btn{flex:1 1 auto;padding:8px 12px;border-radius:8px;border:1px solid transparent;font:inherit;font-size:14px;font-weight:600;cursor:pointer;white-space:nowrap}
.remcord-gal-btn-primary{background:var(--brand-500,#5865f2);color:#fff}
.remcord-gal-btn-primary:hover{filter:brightness(1.08)}
.remcord-gal-btn-secondary{background:var(--background-secondary-alt,rgba(0,0,0,.08));color:var(--text-normal,inherit);border-color:var(--border-subtle,rgba(0,0,0,.12))}
.remcord-gal-btn-secondary:hover{filter:brightness(1.1)}
.remcord-gal-btn-ghost{background:transparent;color:var(--text-muted,#5c5e66)}
`;
    document.head.appendChild(style);
}

async function copyText(text: string): Promise<boolean> {
    try {
        await navigator.clipboard.writeText(text);
        return true;
    } catch {}
    try {
        const area = document.createElement("textarea");
        area.value = text;
        area.setAttribute("readonly", "");
        area.style.position = "fixed";
        area.style.opacity = "0";
        document.body.appendChild(area);
        area.select();
        const ok = document.execCommand("copy");
        area.remove();
        return ok;
    } catch {
        return false;
    }
}

async function downloadUrl(url: string): Promise<void> {
    const res = await fetch(url, { credentials: "omit" });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const blob = await res.blob();
    const objectUrl = URL.createObjectURL(blob);
    try {
        const anchor = document.createElement("a");
        anchor.href = objectUrl;
        anchor.download = fileNameFromUrl(url);
        document.body.appendChild(anchor);
        anchor.click();
        anchor.remove();
    } finally {
        setTimeout(() => URL.revokeObjectURL(objectUrl), 10_000);
    }
}

function collectionOptions(): Promise<string[]> {
    const bridge = galleryBridge();
    if (!bridge) return Promise.resolve([]);
    return bridge
        .categories()
        .then((res) => {
            if (!Array.isArray(res)) return [];
            return (res as Array<Record<string, unknown>>)
                .map((c) => String(c.name ?? ""))
                .filter((name) => name.length > 0);
        })
        .catch(() => []);
}

/**
 * Save popup: title + collection + note, then Save to Gallery, Copy Link,
 * or Download (media only). Plain DOM so it works in every menu context
 * without a Solid root; styled with Discord vars so it follows the theme.
 */
function openSavePopup(save: PendingSave): void {
    try {
        closeMenu();
        ensurePopupStyles();
        const bridge = galleryBridge();
        if (!bridge) return;
        const kinds = save.urls.map((url) => detectKind(url));
        const downloadable = kinds.some((k) => k === "image" || k === "video");
        const singleTitle = save.urls.length === 1 ? fileNameFromUrl(save.urls[0], "") : "";

        const overlay = document.createElement("div");
        overlay.className = "remcord-gal-overlay";
        overlay.style.colorScheme = discordTheme();
        const dialog = document.createElement("div");
        dialog.className = "remcord-gal-dialog";
        dialog.setAttribute("role", "dialog");
        dialog.setAttribute("aria-label", "Save to Gallery");

        const heading = document.createElement("h2");
        heading.className = "remcord-gal-title";
        heading.textContent = save.urls.length === 1 ? "Save to Gallery" : `Save ${save.urls.length} items to Gallery`;
        const sub = document.createElement("p");
        sub.className = "remcord-gal-sub";
        sub.textContent = save.urls.length === 1 ? save.urls[0] : `${save.urls.length} items`;
        sub.title = save.urls.join("\n");

        const titleLabel = document.createElement("label");
        titleLabel.className = "remcord-gal-label";
        titleLabel.textContent = "Name";
        const titleInput = document.createElement("input");
        titleInput.className = "remcord-gal-input";
        titleInput.type = "text";
        titleInput.maxLength = 256;
        titleInput.placeholder = singleTitle || "Untitled";
        titleInput.value = singleTitle;

        const colLabel = document.createElement("label");
        colLabel.className = "remcord-gal-label";
        colLabel.textContent = "Collection";
        const colSelect = document.createElement("select");
        colSelect.className = "remcord-gal-select";
        const none = document.createElement("option");
        none.value = "";
        none.textContent = "No collection";
        colSelect.appendChild(none);
        void collectionOptions().then((names) => {
            for (const name of names.slice(0, 512)) {
                const opt = document.createElement("option");
                opt.value = name;
                opt.textContent = name;
                colSelect.appendChild(opt);
            }
        });

        const noteLabel = document.createElement("label");
        noteLabel.className = "remcord-gal-label";
        noteLabel.textContent = "Note — why are you saving this?";
        const noteInput = document.createElement("textarea");
        noteInput.className = "remcord-gal-textarea";
        noteInput.maxLength = 500;
        noteInput.placeholder = "Optional note…";

        const actions = document.createElement("div");
        actions.className = "remcord-gal-actions";
        const saveBtn = document.createElement("button");
        saveBtn.type = "button";
        saveBtn.className = "remcord-gal-btn remcord-gal-btn-primary";
        saveBtn.textContent = "Save to Gallery";
        const copyBtn = document.createElement("button");
        copyBtn.type = "button";
        copyBtn.className = "remcord-gal-btn remcord-gal-btn-secondary";
        copyBtn.textContent = "Copy Link";
        const cancelBtn = document.createElement("button");
        cancelBtn.type = "button";
        cancelBtn.className = "remcord-gal-btn remcord-gal-btn-ghost";
        cancelBtn.textContent = "Cancel";
        actions.append(saveBtn, copyBtn);
        let downloadBtn: HTMLButtonElement | null = null;
        if (downloadable) {
            downloadBtn = document.createElement("button");
            downloadBtn.type = "button";
            downloadBtn.className = "remcord-gal-btn remcord-gal-btn-secondary";
            downloadBtn.textContent = "Download";
            actions.append(downloadBtn);
        }
        actions.append(cancelBtn);

        dialog.append(heading, sub, titleLabel, titleInput, colLabel, colSelect, noteLabel, noteInput, actions);
        overlay.appendChild(dialog);
        document.body.appendChild(overlay);

        const close = () => overlay.remove();
        overlay.addEventListener("mousedown", (e) => {
            if (e.target === overlay) close();
        });
        overlay.addEventListener("keydown", (e) => {
            if (e.key === "Escape") {
                e.stopPropagation();
                close();
            }
        });
        cancelBtn.addEventListener("click", close);

        saveBtn.addEventListener("click", () => {
            const title = titleInput.value.trim();
            const note = noteInput.value.trim();
            const categories = colSelect.value ? [colSelect.value] : [];
            saveBtn.textContent = "Saving…";
            saveBtn.setAttribute("disabled", "");
            void Promise.all(
                save.urls.map((url) => bridge.addUrl(url, { ...save.meta, title, note, categories })),
            ).then(
                () => {
                    close();
                    showToast({
                        title:
                            save.urls.length === 1 ? "Saved to Gallery" : `Saved ${save.urls.length} items to Gallery`,
                        duration: 2500,
                    });
                },
                () => {
                    saveBtn.textContent = "Save to Gallery";
                    saveBtn.removeAttribute("disabled");
                    showToast({ title: "Could not save to Gallery", duration: 3000 });
                },
            );
        });

        copyBtn.addEventListener("click", () => {
            void copyText(save.urls.join("\n")).then((ok) => {
                close();
                showToast({ title: ok ? "Link copied" : "Could not copy the link", duration: 2500 });
            });
        });

        downloadBtn?.addEventListener("click", () => {
            void (async () => {
                try {
                    for (const url of save.urls) {
                        try {
                            await downloadUrl(url);
                        } catch {
                            window.open(url, "_blank", "noopener");
                        }
                    }
                    close();
                    showToast({ title: "Download started", duration: 2500 });
                } catch {
                    showToast({ title: "Could not download", duration: 3000 });
                }
            })();
        });

        titleInput.focus();
    } catch {
        /* never break Discord menus */
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

/**
 * Native Discord menu labels we upgrade to the save popup. English-only
 * (the plugin's own labels are English-only too); other locales keep
 * Discord's default behavior for these items.
 */
const NATIVE_COPY_LABEL = /^copy link$/i;
const NATIVE_SAVE_MEDIA_LABEL = /^save (image|video)$/i;

function hookNativeItem(item: Element, save: PendingSave, fallback: () => void): void {
    try {
        if (item.hasAttribute(INJECTED_ATTR)) return;
        item.setAttribute(INJECTED_ATTR, "hooked");
        item.addEventListener(
            "click",
            (event) => {
                // Take over the click synchronously so Discord's handler
                // never fires; the popup (or the fallback below) owns what
                // happens next, so the user's intent is never lost.
                event.stopPropagation();
                event.preventDefault();
                try {
                    openSavePopup(save);
                } catch {
                    fallback();
                }
            },
            { capture: true },
        );
    } catch {
        /* leave the native item untouched */
    }
}

function copyFallback(save: PendingSave): void {
    closeMenu();
    void copyText(save.urls.join("\n")).then((ok) => {
        showToast({ title: ok ? "Link copied" : "Could not copy the link", duration: 2500 });
    });
}

function injectIntoMenu(menu: HTMLElement): void {
    try {
        if (menu.hasAttribute(INJECTED_ATTR)) return;
        const save = takePending();
        if (!save) return;
        menu.setAttribute(INJECTED_ATTR, "1");

        // Upgrade Discord's own Copy Link / Save Image(/Video) entries to
        // the save popup: same click position, but the user gets Save to
        // Gallery (with name + note), Copy Link, and Download in one place.
        for (const candidate of menu.querySelectorAll('[role="menuitem"]')) {
            const label = (candidate.textContent ?? "").trim().toLowerCase();
            if (NATIVE_COPY_LABEL.test(label)) {
                hookNativeItem(candidate, save, () => copyFallback(save));
            } else if (NATIVE_SAVE_MEDIA_LABEL.test(label)) {
                hookNativeItem(candidate, save, () => openSavePopup(save));
            }
        }

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
                try {
                    openSavePopup(save);
                } catch {
                    saveUrls(save.urls, save.meta);
                }
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

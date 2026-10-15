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

// Discord's effective appearance: the first scope in the theme chain whose
// resolved `--background-primary` parses (themed container, app mount,
// anchor, body, document root). Falls back to the OS preference when no
// scope yields a color, so custom themes, custom CSS, and stock themes all
// resolve without hardcoding. Never throws.
function discordTheme(anchor: Element): "light" | "dark" {
    for (const scope of themeScopes(anchor)) {
        try {
            const background = getComputedStyle(scope).getPropertyValue("--background-primary");
            const parsed = parseThemeValue(background);
            if (parsed) return parsed;
            const color = parseRgb(background);
            if (color && color[3] >= 0.95) return luminance([color[0], color[1], color[2]]) > 0.4 ? "light" : "dark";
        } catch {}
    }
    try {
        return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
    } catch {
        return "dark";
    }
}

/**
 * Snapshots every custom property the active theme resolved, merged across
 * every scope that may carry them (first non-empty value wins per
 * variable). Custom CSS and custom themes define their own variables, so a
 * fixed list would miss them: enumerating computed styles captures whatever
 * the active theme (stock, custom theme, or raw custom CSS) actually
 * applied, plus an explicit backfill of the variables the popup CSS reads
 * for hosts without enumeration. Best-effort and capped — never throws.
 */
function snapshotThemeVars(anchor: Element): Array<[string, string]> {
    const out: Array<[string, string]> = [];
    const seen = new Set<string>();
    const pushVar = (name: string, value: string) => {
        if (!seen.has(name) && value) {
            seen.add(name);
            out.push([name, value]);
        }
    };
    for (const scope of themeScopes(anchor)) {
        try {
            const computed = getComputedStyle(scope);
            const count = computed.length;
            for (let index = 0; index < count && out.length < 512; index += 1) {
                const name = computed[index];
                if (typeof name !== "string" || !name.startsWith("--") || seen.has(name)) continue;
                try {
                    pushVar(name, computed.getPropertyValue(name).trim());
                } catch {}
            }
        } catch {}
        for (const name of POPUP_THEME_VARS) {
            if (seen.has(name)) continue;
            try {
                pushVar(name, getComputedStyle(scope).getPropertyValue(name).trim());
            } catch {}
        }
    }
    return out;
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

const PENDING_TTL_MS = 10_000;
const INJECTED_ATTR = "data-remcord-gallery";
const SAVE_ITEM_ATTR = "data-remcord-gallery-save";

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

/** Non-consuming read: menus re-render and re-inject, so injection must
 * never eat the pending save — only a real user action clears it. */
function peekPending(): PendingSave | null {
    if (!pending || Date.now() - pending.at > PENDING_TTL_MS) return null;
    return pending;
}

function clearPending(): void {
    pending = null;
    if (pendingTimer !== undefined) {
        clearTimeout(pendingTimer);
        pendingTimer = undefined;
    }
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
.remcord-gal-overlay{position:fixed;inset:0;z-index:99999;display:flex;align-items:center;justify-content:center;background:color-mix(in srgb,light-dark(CanvasText,Canvas) 62%,transparent);padding:24px;box-sizing:border-box}
.remcord-gal-dialog{width:min(420px,100%);max-height:calc(100vh - 48px);overflow-y:auto;background:var(--background-primary,Canvas);color:var(--text-normal,CanvasText);border:1px solid var(--border-subtle,currentColor);border-radius:12px;box-shadow:var(--shadow-high,none);padding:16px;box-sizing:border-box;font-family:var(--font-primary,inherit);font-size:14px}
.remcord-gal-title{margin:0 0 4px;font-size:16px;font-weight:700}
.remcord-gal-sub{margin:0 0 12px;font-size:12px;color:var(--text-muted,CanvasText);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.remcord-gal-label{display:block;margin:10px 0 4px;font-size:12px;font-weight:700;text-transform:uppercase;letter-spacing:.02em;color:var(--text-muted,CanvasText)}
.remcord-gal-input,.remcord-gal-textarea,.remcord-gal-select{width:100%;box-sizing:border-box;padding:8px 10px;border:1px solid var(--input-border-default,currentColor);border-radius:8px;background:var(--input-background-default,Field);color:var(--input-text-default,FieldText);font:inherit;font-size:14px}
.remcord-gal-input:focus,.remcord-gal-textarea:focus,.remcord-gal-select:focus{outline:2px solid var(--brand-500,var(--text-normal,CanvasText));outline-offset:-1px}
.remcord-gal-textarea{min-height:64px;resize:vertical}
.remcord-gal-actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:14px}
.remcord-gal-btn{flex:1 1 auto;padding:8px 12px;border-radius:8px;border:1px solid transparent;font:inherit;font-size:14px;font-weight:600;cursor:pointer;white-space:nowrap}
.remcord-gal-btn-primary{background:var(--brand-500,var(--text-normal,CanvasText));color:var(--text-normal,CanvasText)}
.remcord-gal-btn-primary:hover{filter:brightness(1.08)}
.remcord-gal-btn-secondary{background:var(--background-secondary-alt,ButtonFace);color:var(--text-normal,CanvasText);border-color:var(--border-subtle,currentColor)}
.remcord-gal-btn-secondary:hover{filter:brightness(1.1)}
.remcord-gal-btn-ghost{background:transparent;color:var(--text-muted,CanvasText)}
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
 * Discord variables the popup CSS reads. Copied as resolved values onto the
 * overlay (see openSavePopup), so the popup matches stock and custom themes
 * no matter where it attaches.
 */
const POPUP_THEME_VARS = [
    "--background-primary",
    "--background-secondary",
    "--background-secondary-alt",
    "--text-normal",
    "--text-muted",
    "--border-subtle",
    "--input-background-default",
    "--input-text-default",
    "--input-border-default",
    "--brand-500",
    "--shadow-high",
    "--font-primary",
];

/**
 * Elements that may carry Discord's theme variables, most authoritative
 * first. Menus (and the click anchor with them) can render outside the
 * themed subtree, where variable reads come back empty even though Discord
 * itself looks themed — so the themed container, app mount, anchor, body,
 * and document root are all tried in order. Covers stock themes, custom
 * themes, and raw custom CSS (which usually targets `:root`).
 */
function themeScopes(anchor: Element): Element[] {
    const scopes: Element[] = [];
    const push = (el: Element | null | undefined) => {
        if (el && !scopes.includes(el)) scopes.push(el);
    };
    try {
        push(document.querySelector(".theme-dark, .theme-light"));
    } catch {}
    try {
        push(document.getElementById("app-mount"));
    } catch {}
    push(anchor);
    try {
        if (document.body) push(document.body);
    } catch {}
    try {
        if (document.documentElement) push(document.documentElement);
    } catch {}
    return scopes;
}

/** Parse one resolved background value into an appearance, if possible. */
function parseThemeValue(raw: string): "light" | "dark" | null {
    const value = raw.trim().toLowerCase();
    let r = -1;
    let g = -1;
    let b = -1;
    const hex = value.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/);
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
        const rgb = value.match(/^rgba?\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})/);
        if (rgb) {
            r = Number(rgb[1]);
            g = Number(rgb[2]);
            b = Number(rgb[3]);
        }
    }
    if (r < 0 || g < 0 || b < 0 || r > 255 || g > 255 || b > 255) return null;
    return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255 > 0.4 ? "light" : "dark";
}

type RgbTriple = [number, number, number];

/** Parse one rendered color value into `[r, g, b, alpha]`. */
function parseRgb(value: string): [number, number, number, number] | null {
    const v = value.trim().toLowerCase();
    if (v === "transparent") return [0, 0, 0, 0];
    const hex = v.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/);
    if (hex) {
        const h = hex[1];
        const full =
            h.length === 3
                ? h
                      .split("")
                      .map((c) => c + c)
                      .join("")
                : h;
        const r = parseInt(full.slice(0, 2), 16);
        const g = parseInt(full.slice(2, 4), 16);
        const b = parseInt(full.slice(4, 6), 16);
        if (r >= 0 && g >= 0 && b >= 0 && r <= 255 && g <= 255 && b <= 255) return [r, g, b, 1];
        return null;
    }
    const m = v.match(/^rgba?\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})(?:\s*,\s*([\d.]+))?\)/);
    if (m) {
        const r = Number(m[1]);
        const g = Number(m[2]);
        const b = Number(m[3]);
        const a = m[4] === undefined ? 1 : Number(m[4]);
        if (r >= 0 && g >= 0 && b >= 0 && r <= 255 && g <= 255 && b <= 255 && a >= 0 && a <= 1) {
            return [r, g, b, a];
        }
    }
    // Chromium/WebView2 preserves modern CSS Color values (notably Discord's
    // current `oklab(...)` surfaces) in computed styles instead of
    // serializing them to rgb(). Let the browser parse and convert every CSS
    // color syntax it supports; this reads the actual painted color rather
    // than approximating Discord token names. The unattached canvas is clean,
    // and all DOM/canvas access is best-effort.
    try {
        if (typeof CSS !== "undefined" && !CSS.supports("color", v)) return null;
        const canvas = document.createElement("canvas");
        canvas.width = 1;
        canvas.height = 1;
        const context = canvas.getContext("2d", { willReadFrequently: true });
        if (!context) return null;
        context.clearRect(0, 0, 1, 1);
        context.fillStyle = v;
        context.fillRect(0, 0, 1, 1);
        const pixel = context.getImageData(0, 0, 1, 1).data;
        return [pixel[0], pixel[1], pixel[2], pixel[3] / 255];
    } catch {
        return null;
    }
}

function luminance([r, g, b]: RgbTriple): number {
    return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
}

function mix(a: RgbTriple, b: RgbTriple, t: number): string {
    const k = Math.min(1, Math.max(0, t));
    const c = (x: number, y: number) => Math.round(x + (y - x) * k);
    return `rgb(${c(a[0], b[0])}, ${c(a[1], b[1])}, ${c(a[2], b[2])})`;
}

/**
 * Rendered-surface sample: the first opaque `background-color` found
 * walking the theme scopes, plus the live menu item's computed foreground.
 * Used to backfill missing or incompatible palette tokens from what Discord
 * literally paints, including modern CSS color syntaxes.
 */
function sampleSurface(anchor: Element): { bg: RgbTriple; fg: RgbTriple } | null {
    let base: RgbTriple | null = null;
    for (const scope of themeScopes(anchor)) {
        try {
            const cs = getComputedStyle(scope);
            const bg = parseRgb(cs.getPropertyValue("background-color"));
            if (!bg || bg[3] < 0.95) continue;
            base = [bg[0], bg[1], bg[2]];
            break;
        } catch {}
    }
    if (!base) return null;

    // The popup opens from a live menu item. Its computed foreground is
    // already the theme's actual text color, even when the root surface has
    // no legacy `--text-normal` variable. Prefer that over synthesizing
    // white/black from the background.
    for (const scope of [anchor, ...themeScopes(anchor)]) {
        try {
            const fg = parseRgb(getComputedStyle(scope).getPropertyValue("color"));
            if (fg && fg[3] >= 0.95) return { bg: base, fg: [fg[0], fg[1], fg[2]] };
        } catch {}
    }
    return null;
}

/** Fill the popup palette from a rendered surface, replacing legacy color
 * aliases only when the active theme has no `--background-primary` token. */
function fillDerivedVars(
    vars: Array<[string, string]>,
    sample: { bg: RgbTriple; fg: RgbTriple },
    replacePalette: boolean,
): void {
    const indexOf = (name: string) => vars.findIndex(([seen]) => seen === name);
    const put = (name: string, value: string) => {
        const index = indexOf(name);
        if (index < 0) vars.push([name, value]);
        else if (replacePalette) vars[index][1] = value;
    };
    const { bg, fg } = sample;
    put("--background-primary", mix(bg, fg, 0));
    put("--text-normal", mix(fg, bg, 0));
    put("--header-primary", mix(fg, bg, 0));
    put("--input-text-default", mix(fg, bg, 0));
    put("--text-muted", mix(bg, fg, 0.5));
    put("--background-secondary", mix(bg, fg, 0.06));
    put("--background-secondary-alt", mix(bg, fg, 0.12));
    put("--input-background-default", mix(bg, fg, 0.07));
    put("--border-subtle", mix(bg, fg, 0.2));
    put("--input-border-default", mix(bg, fg, 0.22));
}

/**
 * Save popup: title + collection + note, then Save to Gallery, Copy Link,
 * or Download (media only). Plain DOM so it works in every menu context
 * without a Solid root.
 *
 * The overlay attaches to `document.body` (menu layers can clip fixed
 * positioning or unmount, which would hide the popup while it still blocks
 * input). It carries resolved theme variables and a surface-derived palette
 * so its appearance does not depend on inheriting Discord's theme subtree.
 */
function openSavePopup(save: PendingSave, anchor: Element): boolean {
    let overlay: HTMLDivElement | null = null;
    try {
        // Snapshot the resolved theme BEFORE closing the menu: the close
        // unmounts the menu, so reading afterwards would yield unresolved
        // variables. Every variable is copied onto the overlay itself
        // (below), so the popup matches stock themes, custom themes, and
        // raw custom CSS wherever it attaches. When variables resolve
        // nowhere (custom themes styling surfaces directly), fall back to
        // the rendered surface sample so no theme can strand the popup in
        // fallback colors.
        let scheme = discordTheme(anchor);
        const themeVars = snapshotThemeVars(anchor);
        const hasPrimarySurface = themeVars.some(([name]) => name === "--background-primary");
        const sample = sampleSurface(anchor);
        if (sample) {
            // A modern Discord theme can expose hundreds of computed
            // properties while no longer defining the old primary token.
            // In that case its legacy input/text aliases are not a coherent
            // popup palette, so derive those colors from the actual surface.
            // Otherwise retain copied values and only backfill missing ones.
            fillDerivedVars(themeVars, sample, !hasPrimarySurface);
            scheme = luminance(sample.bg) <= 0.4 ? "dark" : "light";
        }
        closeMenu();
        ensurePopupStyles();
        const bridge = galleryBridge();
        if (!bridge) return false;
        const kinds = save.urls.map((url) => detectKind(url));
        const downloadable = kinds.some((k) => k === "image" || k === "video");
        const singleTitle = save.urls.length === 1 ? fileNameFromUrl(save.urls[0], "") : "";

        const popupOverlay = document.createElement("div");
        overlay = popupOverlay;
        popupOverlay.className = "remcord-gal-overlay";
        popupOverlay.style.colorScheme = scheme;
        for (const [name, value] of themeVars) {
            try {
                popupOverlay.style.setProperty(name, value);
            } catch {}
        }
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
            try {
                for (const name of names.slice(0, 512)) {
                    const opt = document.createElement("option");
                    opt.value = name;
                    opt.textContent = name;
                    colSelect.appendChild(opt);
                }
            } catch {}
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
        popupOverlay.appendChild(dialog);
        // Attach to body so Discord's menu teardown cannot remove the dialog;
        // copied/derived variables make styling independent of ancestry.
        document.body.appendChild(popupOverlay);
        markPopup(true);
        // Close only after attaching so this overlay survives menu teardown.
        closeMenu();
        titleInput.focus();

        const close = () => {
            try {
                markPopup(false);
                popupOverlay.remove();
            } catch {
                markPopup(false);
                try {
                    popupOverlay.style.display = "none";
                } catch {}
            }
        };
        popupOverlay.addEventListener("mousedown", (e) => {
            try {
                if (e.target === popupOverlay) close();
            } catch {}
        });
        popupOverlay.addEventListener("keydown", (e) => {
            try {
                if (e.key === "Escape") {
                    e.stopPropagation();
                    close();
                }
            } catch {}
        });
        cancelBtn.addEventListener("click", () => {
            try {
                close();
            } catch {}
        });

        saveBtn.addEventListener("click", () => {
            try {
                const title = titleInput.value.trim();
                const note = noteInput.value.trim();
                const categories = colSelect.value ? [colSelect.value] : [];
                saveBtn.textContent = "Saving…";
                saveBtn.setAttribute("disabled", "");
                void Promise.all(
                    save.urls.map((url) => bridge.addUrl(url, { ...save.meta, title, note, categories })),
                ).then(
                    () => {
                        try {
                            close();
                            showToast({
                                title:
                                    save.urls.length === 1
                                        ? "Saved to Gallery"
                                        : `Saved ${save.urls.length} items to Gallery`,
                                duration: 2500,
                            });
                        } catch {}
                    },
                    () => {
                        try {
                            saveBtn.textContent = "Save to Gallery";
                            saveBtn.removeAttribute("disabled");
                            showToast({ title: "Could not save to Gallery", duration: 3000 });
                        } catch {
                            try {
                                saveUrls(save.urls, save.meta);
                            } catch {}
                        }
                    },
                );
            } catch {
                try {
                    saveUrls(save.urls, save.meta);
                } catch {}
            }
        });

        copyBtn.addEventListener("click", () => {
            try {
                void copyText(save.urls.join("\n")).then((ok) => {
                    try {
                        close();
                        showToast({ title: ok ? "Link copied" : "Could not copy the link", duration: 2500 });
                    } catch {}
                });
            } catch {
                try {
                    copyFallback(save);
                } catch {}
            }
        });

        downloadBtn?.addEventListener("click", () => {
            try {
                close();
                downloadAll(save.urls);
            } catch {
                try {
                    downloadFallback(save);
                } catch {}
            }
        });
        return true;
    } catch {
        try {
            overlay?.remove();
        } catch {}
        markPopup(false);
        return false;
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
 * Native Discord menu labels we upgrade to the save popup. Matched against
 * the item's label slot (deepest text node), not the whole item text, which
 * carries surrounding chrome. Strict full-match so lookalikes like
 * "Copy Message Link" keep their native behavior. English-only (the
 * plugin's own labels are English-only too); other locales keep Discord's
 * default behavior for these items.
 */
const NATIVE_COPY_LABEL = /^copy (link|image|video)$/;
const NATIVE_SAVE_MEDIA_LABEL = /^save (image|video)$/;

function nativeLabelOf(item: Element): string {
    // Any text node inside the item can hold the label (hints or adornments
    // may sit deeper than it); join them so the label is never masked.
    try {
        const parts: string[] = [];
        const walker = document.createTreeWalker(item, NodeFilter.SHOW_TEXT);
        let node = walker.nextNode() as Text | null;
        while (node) {
            const text = node.data
                .toLowerCase()
                .replace(/[\s_]+/g, " ")
                .trim();
            if (text) parts.push(text);
            node = walker.nextNode() as Text | null;
        }
        return parts.join(" | ");
    } catch {
        return "";
    }
}

function nativeLabelMatches(label: string, pattern: RegExp): boolean {
    return label.split(" | ").some((part) => pattern.test(part));
}

/** True while our popup is on screen: stray pointer/click events from the
 * dismissed menu (e.g. a click landing after the takeover) must not open a
 * second popup. Set before the overlay attaches, cleared on close. */
let popupOpen = false;

function markPopup(open: boolean): void {
    popupOpen = open;
}

/**
 * Single delegated takeover for every gallery menu action. One
 * document-level capture listener (pointerdown AND click) instead of
 * per-item listeners: Discord re-renders menu nodes on hover/focus, which
 * silently drops node-level listeners, but nothing can preempt or detach
 * a document-capture handler. Non-gallery items return untouched.
 */
function onDocMenuAction(event: Event): void {
    try {
        if (popupOpen) return;
        const target = event.target as Element | null;
        if (!target || typeof target.closest !== "function") return;
        const item = target.closest('[role="menuitem"]');
        if (!item) return;
        if (item.hasAttribute(SAVE_ITEM_ATTR)) {
            event.stopPropagation();
            event.preventDefault();
            const save = peekPending();
            if (!save) return;
            clearPending();
            try {
                if (!openSavePopup(save, item)) saveUrls(save.urls, save.meta);
            } catch {
                saveUrls(save.urls, save.meta);
            }
            return;
        }
        const save = peekPending();
        if (!save) return;
        const label = nativeLabelOf(item);
        if (nativeLabelMatches(label, NATIVE_COPY_LABEL)) {
            event.stopPropagation();
            event.preventDefault();
            clearPending();
            try {
                if (!openSavePopup(save, item)) copyFallback(save);
            } catch {
                copyFallback(save);
            }
        } else if (nativeLabelMatches(label, NATIVE_SAVE_MEDIA_LABEL)) {
            event.stopPropagation();
            event.preventDefault();
            clearPending();
            try {
                if (!openSavePopup(save, item)) downloadFallback(save);
            } catch {
                downloadFallback(save);
            }
        }
    } catch {
        /* never break Discord menus */
    }
}

function copyFallback(save: PendingSave): void {
    closeMenu();
    void copyText(save.urls.join("\n")).then((ok) => {
        showToast({ title: ok ? "Link copied" : "Could not copy the link", duration: 2500 });
    });
}

function downloadAll(urls: string[]): void {
    void (async () => {
        try {
            for (const url of urls) {
                try {
                    await downloadUrl(url);
                } catch {
                    window.open(url, "_blank", "noopener");
                }
            }
            showToast({ title: "Download started", duration: 2500 });
        } catch {
            showToast({ title: "Could not download", duration: 3000 });
        }
    })();
}

function downloadFallback(save: PendingSave): void {
    closeMenu();
    downloadAll(save.urls);
}

function injectIntoMenu(menu: HTMLElement): void {
    try {
        if (menu.hasAttribute(INJECTED_ATTR)) return;
        // Peek only: the menu may re-render and re-inject, and the pending
        // save belongs to the user's click action, not to injection.
        const save = peekPending();
        if (!save) return;
        menu.setAttribute(INJECTED_ATTR, "1");

        // Our own entry carries a marker; the delegated document handler
        // owns the click (node-level listeners don't survive menu
        // re-renders). Native Copy Link / Save Image entries need no
        // marking: the delegate matches their labels at click time.
        if (menu.querySelector(`[${SAVE_ITEM_ATTR}]`)) return;
        const template = menu.querySelector('[role="menuitem"]');
        if (!template) return;
        const labelNode = findLabelNode(template);
        if (!labelNode) return;

        const item = template.cloneNode(true) as HTMLElement;
        item.removeAttribute("id");
        item.setAttribute("aria-label", "Save to Gallery");
        item.setAttribute(SAVE_ITEM_ATTR, "1");
        const itemLabel = findLabelNode(item);
        if (!itemLabel) return;
        itemLabel.data = save.urls.length === 1 ? "Save to Gallery" : `Save to Gallery (${save.urls.length})`;

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
    document.addEventListener("pointerdown", onDocMenuAction, true);
    document.addEventListener("click", onDocMenuAction, true);
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
        document.removeEventListener("pointerdown", onDocMenuAction, true);
    } catch {}
    try {
        document.removeEventListener("click", onDocMenuAction, true);
    } catch {}
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

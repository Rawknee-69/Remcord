/** Pure gallery helpers shared by main + renderer + tests. No node/electron imports. */

export type GalleryKind = "image" | "video" | "link" | "file";

export interface GalleryCategory {
    name: string;
    color: string;
    createdAt: number;
}

export interface GalleryItem {
    id: string;
    kind: GalleryKind;
    /** Original remote URL (http/https) or empty for local-only imports. */
    sourceUrl: string;
    /** Human title (filename, link title, or user label). */
    title: string;
    /** User note: why the item was saved. Empty when unset. */
    note: string;
    fileName: string;
    /** Path relative to the gallery root (files/xxx). Empty for link-only items. */
    localFile: string;
    sizeBytes: number;
    mime: string;
    guildId: string;
    guildName: string;
    channelId: string;
    channelName: string;
    authorName: string;
    messageId: string;
    savedAt: number;
    categories: string[];
}

export interface GalleryFilters {
    query: string;
    kind: GalleryKind | "all";
    guild: string;
    channel: string;
    category: string;
    fromMs: number | null;
    toMs: number | null;
}

export const GALLERY_PAGE_SIZE = 60;

const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "svg", "ico", "apng"]);
const VIDEO_EXT = new Set(["mp4", "webm", "mov", "mkv", "avi", "m4v", "ogv"]);

export function detectGalleryKind(url: string, mime = ""): GalleryKind {
    const m = mime.toLowerCase();
    if (m.startsWith("image/")) return "image";
    if (m.startsWith("video/")) return "video";
    let path = "";
    try {
        path = new URL(url).pathname.toLowerCase();
    } catch {
        path = url.toLowerCase().split("?")[0];
    }
    const ext = path.split(".").pop() ?? "";
    if (IMAGE_EXT.has(ext)) return "image";
    if (VIDEO_EXT.has(ext)) return "video";
    if (!url) return "file";
    return "link";
}

export function fileNameFromUrl(url: string, fallback = "saved-file"): string {
    try {
        const u = new URL(url);
        const last = u.pathname.split("/").filter(Boolean).pop() ?? "";
        const clean = decodeURIComponent(last.split("?")[0]).slice(0, 128);
        return clean || fallback;
    } catch {
        return fallback;
    }
}

export function formatGalleryBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KB", "MB", "GB", "TB"];
    let v = bytes / 1024;
    let u = 0;
    while (v >= 1024 && u < units.length - 1) {
        v /= 1024;
        u++;
    }
    return `${v >= 100 ? Math.round(v) : Math.round(v * 10) / 10} ${units[u]}`;
}

export function formatGalleryDate(ms: number): string {
    try {
        return new Date(ms).toLocaleString();
    } catch {
        return "";
    }
}

export function galleryDayKey(ms: number): string {
    try {
        return new Date(ms).toISOString().slice(0, 10);
    } catch {
        return "unknown";
    }
}

export function normalizeCategoryName(name: string): string {
    return name.trim().replace(/\s+/g, " ").slice(0, 48);
}

export function isValidCategoryName(name: string): boolean {
    const n = normalizeCategoryName(name);
    return n.length >= 1 && n.length <= 48;
}

export function emptyGalleryFilters(): GalleryFilters {
    return { query: "", kind: "all", guild: "all", channel: "all", category: "all", fromMs: null, toMs: null };
}

export function filterGalleryItems(items: GalleryItem[], f: GalleryFilters): GalleryItem[] {
    const q = f.query.trim().toLowerCase();
    return items.filter((it) => {
        if (f.kind !== "all" && it.kind !== f.kind) return false;
        if (f.guild !== "all" && it.guildId !== f.guild && it.guildName !== f.guild) return false;
        if (f.channel !== "all" && it.channelId !== f.channel && it.channelName !== f.channel) return false;
        if (f.category !== "all" && !it.categories.includes(f.category)) return false;
        if (f.fromMs !== null && it.savedAt < f.fromMs) return false;
        if (f.toMs !== null && it.savedAt > f.toMs) return false;
        if (!q) return true;
        const hay =
            `${it.title} ${it.fileName} ${it.sourceUrl} ${it.guildName} ${it.channelName} ${it.authorName} ${it.categories.join(" ")}`.toLowerCase();
        return q.split(/\s+/).every((tok) => hay.includes(tok));
    });
}

export function sortGalleryItems(items: GalleryItem[], order: "newest" | "oldest"): GalleryItem[] {
    const copy = [...items];
    copy.sort((a, b) => (order === "newest" ? b.savedAt - a.savedAt : a.savedAt - b.savedAt));
    return copy;
}

export function paginateGalleryItems<T>(items: T[], page: number, pageSize: number): T[] {
    const start = Math.max(0, page) * Math.max(1, pageSize);
    return items.slice(start, start + pageSize);
}

export type GalleryGroupBy = "none" | "guild" | "channel" | "date" | "kind" | "category";

export function groupGalleryItems(
    items: GalleryItem[],
    groupBy: GalleryGroupBy,
): Array<{ key: string; label: string; items: GalleryItem[] }> {
    if (groupBy === "none") return [{ key: "all", label: "", items }];
    const map = new Map<string, { label: string; items: GalleryItem[] }>();
    for (const it of items) {
        let key = "";
        let label = "";
        switch (groupBy) {
            case "guild":
                key = it.guildName || "Unknown server";
                label = key;
                break;
            case "channel":
                key = it.channelName || "Unknown channel";
                label = key;
                break;
            case "date":
                key = galleryDayKey(it.savedAt);
                label = key;
                break;
            case "kind":
                key = it.kind;
                label = it.kind[0].toUpperCase() + it.kind.slice(1);
                break;
            case "category":
                if (it.categories.length === 0) {
                    key = "__uncategorized__";
                    label = "Uncategorized";
                } else {
                    for (const c of it.categories) {
                        const e = map.get(c) ?? { label: c, items: [] };
                        e.items.push(it);
                        map.set(c, e);
                    }
                    continue;
                }
                break;
        }
        const e = map.get(key) ?? { label, items: [] };
        e.items.push(it);
        map.set(key, e);
    }
    return [...map.entries()]
        .sort((a, b) => a[0].localeCompare(b[0]))
        .map(([key, v]) => ({ key, label: v.label, items: v.items }));
}

export function sanitizeGalleryId(id: string): boolean {
    return /^[a-zA-Z0-9_-]{1,64}$/.test(id);
}

export function createGalleryId(): string {
    return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

/**
 * Reads Discord's effective appearance from the resolved
 * `--background-primary` value (whatever theme — stock dark/light or any
 * custom theme — actually applied). Returns `"light"`/`"dark"` by relative
 * luminance, or `null` when the value is missing/unparseable so callers
 * can fall back to the OS preference. Pure and dependency-free.
 */
export function galleryThemeFromBackground(value: string): "light" | "dark" | null {
    const v = value.trim().toLowerCase();
    let r = -1;
    let g = -1;
    let b = -1;
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
        r = parseInt(full.slice(0, 2), 16);
        g = parseInt(full.slice(2, 4), 16);
        b = parseInt(full.slice(4, 6), 16);
    } else {
        const rgb = v.match(/^rgba?\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})/);
        if (rgb) {
            r = Number(rgb[1]);
            g = Number(rgb[2]);
            b = Number(rgb[3]);
        }
    }
    if (r < 0 || g < 0 || b < 0 || r > 255 || g > 255 || b > 255) return null;
    return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255 > 0.4 ? "light" : "dark";
}

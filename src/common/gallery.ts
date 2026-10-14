import {
    copyFileSync,
    cpSync,
    existsSync,
    mkdirSync,
    readdirSync,
    readFileSync,
    rmSync,
    statSync,
    writeFileSync,
} from "node:fs";
import path from "node:path";
import { app, net } from "electron";
import { getConfig, setConfig } from "./config.js";
import {
    createGalleryId,
    detectGalleryKind,
    fileNameFromUrl,
    type GalleryCategory,
    type GalleryItem,
    type GalleryKind,
    isValidCategoryName,
    normalizeCategoryName,
} from "./galleryUtils.js";

export type { GalleryCategory, GalleryItem, GalleryKind };
export { detectGalleryKind };

export interface GalleryMeta {
    guildId?: string;
    guildName?: string;
    channelId?: string;
    channelName?: string;
    authorName?: string;
    messageId?: string;
    title?: string;
    categories?: string[];
}

const INDEX_FILE = "gallery-index.json";
const FILES_DIR = "files";
const INDEX_VERSION = 1;

let indexCache: { items: GalleryItem[]; categories: GalleryCategory[] } | null = null;
let saveTimer: NodeJS.Timeout | null = null;
let diskUsageCache: { bytes: number; at: number } | null = null;

const CATEGORY_COLORS = ["#5865f2", "#57f287", "#fee75c", "#eb459e", "#ed4245", "#e3e5e8", "#f47fff", "#44ddff"];

export function getDefaultGalleryRoot(): string {
    return path.join(app.getPath("userData"), "gallery");
}

export function getGalleryRoot(): string {
    const configured = getConfig("galleryRoot");
    if (typeof configured === "string" && configured.trim().length > 0) return configured;
    return getDefaultGalleryRoot();
}

function indexPath(root: string): string {
    return path.join(root, INDEX_FILE);
}

function filesDir(root: string): string {
    return path.join(root, FILES_DIR);
}

export function ensureGalleryDirs(root?: string): string {
    const dir = root ?? getGalleryRoot();
    mkdirSync(dir, { recursive: true });
    mkdirSync(filesDir(dir), { recursive: true });
    return dir;
}

function safeFileName(name: string): string {
    const base =
        path
            .basename(name)
            .replace(/[<>:"/\\|?*\0]/g, "_")
            .slice(0, 120) || "file";
    return base;
}

export function loadGalleryIndex(): { items: GalleryItem[]; categories: GalleryCategory[] } {
    if (indexCache) return indexCache;
    const root = ensureGalleryDirs();
    const p = indexPath(root);
    try {
        if (!existsSync(p)) {
            indexCache = { items: [], categories: [] };
            return indexCache;
        }
        const raw = JSON.parse(readFileSync(p, "utf-8")) as {
            version?: number;
            items?: GalleryItem[];
            categories?: GalleryCategory[];
        };
        const items = Array.isArray(raw.items) ? raw.items.filter((i) => i && typeof i.id === "string") : [];
        const categories = Array.isArray(raw.categories)
            ? raw.categories.filter((c) => c && typeof c.name === "string")
            : [];
        // Snapshot guarantee: items keep stored guild/channel names forever,
        // even after leaving the server. Never drop them here.
        indexCache = { items, categories };
        return indexCache;
    } catch {
        indexCache = { items: [], categories: [] };
        return indexCache;
    }
}

function schedulePersist(): void {
    if (saveTimer) return;
    saveTimer = setTimeout(() => {
        saveTimer = null;
        persistGalleryIndex();
    }, 150);
}

export function persistGalleryIndex(): void {
    if (!indexCache) return;
    try {
        const root = ensureGalleryDirs();
        const tmp = `${indexPath(root)}.tmp`;
        writeFileSync(tmp, JSON.stringify({ version: INDEX_VERSION, ...indexCache }, null, 1), "utf-8");
        writeFileSync(indexPath(root), readFileSync(tmp, "utf-8"), "utf-8");
        try {
            rmSync(tmp, { force: true });
        } catch {}
    } catch (e) {
        console.error("[Gallery] persist failed:", e);
    }
}

function invalidateDiskUsage(): void {
    diskUsageCache = null;
}

function categoryColor(name: string, count: number): string {
    let h = 0;
    for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    return CATEGORY_COLORS[(h + count) % CATEGORY_COLORS.length];
}

export function listGalleryCategories(): GalleryCategory[] {
    return [...loadGalleryIndex().categories].sort((a, b) => a.name.localeCompare(b.name));
}

export function addGalleryCategory(
    name: string,
): { ok: true; category: GalleryCategory } | { ok: false; error: string } {
    const clean = normalizeCategoryName(name);
    if (!isValidCategoryName(clean)) return { ok: false, error: "INVALID_NAME" };
    const idx = loadGalleryIndex();
    if (idx.categories.some((c) => c.name.toLowerCase() === clean.toLowerCase())) {
        return { ok: false, error: "EXISTS" };
    }
    const category: GalleryCategory = {
        name: clean,
        color: categoryColor(clean, idx.categories.length),
        createdAt: Date.now(),
    };
    idx.categories.push(category);
    schedulePersist();
    return { ok: true, category };
}

export function renameGalleryCategory(oldName: string, newName: string): { ok: boolean; error?: string } {
    const clean = normalizeCategoryName(newName);
    if (!isValidCategoryName(clean)) return { ok: false, error: "INVALID_NAME" };
    const idx = loadGalleryIndex();
    const existing = idx.categories.find((c) => c.name === oldName);
    if (!existing) return { ok: false, error: "NOT_FOUND" };
    if (idx.categories.some((c) => c.name.toLowerCase() === clean.toLowerCase() && c.name !== oldName)) {
        return { ok: false, error: "EXISTS" };
    }
    existing.name = clean;
    for (const item of idx.items) {
        item.categories = item.categories.map((c) => (c === oldName ? clean : c));
    }
    schedulePersist();
    invalidateDiskUsage();
    return { ok: true };
}

export function removeGalleryCategory(name: string): { ok: boolean } {
    const idx = loadGalleryIndex();
    idx.categories = idx.categories.filter((c) => c.name !== name);
    for (const item of idx.items) {
        item.categories = item.categories.filter((c) => c !== name);
    }
    schedulePersist();
    return { ok: true };
}

export function listGalleryItems(): GalleryItem[] {
    return [...loadGalleryIndex().items].sort((a, b) => b.savedAt - a.savedAt);
}

export function getGalleryItem(id: string): GalleryItem | undefined {
    return loadGalleryIndex().items.find((i) => i.id === id);
}

function uniqueDiskName(dir: string, wanted: string): string {
    const name = safeFileName(wanted);
    if (!existsSync(path.join(dir, name))) return name;
    const ext = path.extname(name);
    const base = path.basename(name, ext).slice(0, 80);
    for (let n = 1; n < 1000; n++) {
        const candidate = `${base}-${n}${ext}`;
        if (!existsSync(path.join(dir, candidate))) return candidate;
    }
    return `${Date.now()}-${name}`;
}

async function downloadToGallery(
    url: string,
    dir: string,
    fileName: string,
): Promise<{ file: string; bytes: number; mime: string }> {
    const res = await net.fetch(url);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const mime = res.headers.get("content-type")?.split(";")[0].trim() || "application/octet-stream";
    const buf = Buffer.from(await res.arrayBuffer());
    const name = uniqueDiskName(dir, fileName);
    writeFileSync(path.join(dir, name), buf);
    return { file: name, bytes: buf.byteLength, mime };
}

export async function addGalleryUrl(
    url: string,
    meta: GalleryMeta = {},
): Promise<{ ok: true; item: GalleryItem } | { ok: false; error: string }> {
    const clean = url.trim().slice(0, 2048);
    if (!/^https?:\/\//i.test(clean)) return { ok: false, error: "INVALID_URL" };
    const idx = loadGalleryIndex();
    if (idx.items.some((i) => i.sourceUrl === clean)) {
        const dup = idx.items.find((i) => i.sourceUrl === clean)!;
        return { ok: true, item: dup };
    }
    const root = ensureGalleryDirs();
    const dir = filesDir(root);
    const kind = detectGalleryKind(clean);
    const title = (meta.title ?? "").slice(0, 256) || fileNameFromUrl(clean, clean.slice(0, 80));
    const item: GalleryItem = {
        id: createGalleryId(),
        kind,
        sourceUrl: clean,
        title,
        fileName: fileNameFromUrl(clean),
        localFile: "",
        sizeBytes: 0,
        mime: kind === "link" ? "text/uri-list" : "application/octet-stream",
        guildId: (meta.guildId ?? "").slice(0, 64),
        guildName: (meta.guildName ?? "").slice(0, 128),
        channelId: (meta.channelId ?? "").slice(0, 64),
        channelName: (meta.channelName ?? "").slice(0, 128),
        authorName: (meta.authorName ?? "").slice(0, 128),
        messageId: (meta.messageId ?? "").slice(0, 64),
        savedAt: Date.now(),
        categories: (meta.categories ?? []).map(normalizeCategoryName).filter(Boolean).slice(0, 12),
    };
    // Persist media bytes to disk so the item survives leaving the server /
    // CDN expiry. Links stay metadata-only (size 0) and cost no disk space.
    if (kind === "image" || kind === "video") {
        try {
            const dl = await downloadToGallery(clean, dir, item.fileName);
            item.localFile = `${FILES_DIR}/${dl.file}`;
            item.sizeBytes = dl.bytes;
            item.mime = dl.mime;
            item.fileName = dl.file;
        } catch (e) {
            console.warn("[Gallery] download failed, keeping link entry:", e);
            item.kind = "link";
            item.mime = "text/uri-list";
        }
    }
    idx.items.unshift(item);
    schedulePersist();
    invalidateDiskUsage();
    return { ok: true, item };
}

export function addGalleryLink(
    url: string,
    meta: GalleryMeta = {},
): { ok: true; item: GalleryItem } | { ok: false; error: string } {
    const clean = url.trim().slice(0, 2048);
    if (!/^https?:\/\//i.test(clean)) return { ok: false, error: "INVALID_URL" };
    const idx = loadGalleryIndex();
    const item: GalleryItem = {
        id: createGalleryId(),
        kind: "link",
        sourceUrl: clean,
        title: (meta.title ?? "").slice(0, 256) || fileNameFromUrl(clean, clean.slice(0, 80)),
        fileName: fileNameFromUrl(clean),
        localFile: "",
        sizeBytes: 0,
        mime: "text/uri-list",
        guildId: (meta.guildId ?? "").slice(0, 64),
        guildName: (meta.guildName ?? "").slice(0, 128),
        channelId: (meta.channelId ?? "").slice(0, 64),
        channelName: (meta.channelName ?? "").slice(0, 128),
        authorName: (meta.authorName ?? "").slice(0, 128),
        messageId: (meta.messageId ?? "").slice(0, 64),
        savedAt: Date.now(),
        categories: (meta.categories ?? []).map(normalizeCategoryName).filter(Boolean).slice(0, 12),
    };
    idx.items.unshift(item);
    schedulePersist();
    return { ok: true, item };
}

export function importGalleryFiles(
    absPaths: string[],
    meta: GalleryMeta = {},
): { ok: true; items: GalleryItem[] } | { ok: false; error: string } {
    const root = ensureGalleryDirs();
    const dir = filesDir(root);
    const idx = loadGalleryIndex();
    const made: GalleryItem[] = [];
    for (const abs of absPaths.slice(0, 100)) {
        try {
            const st = statSync(abs);
            if (!st.isFile()) continue;
            const base = safeFileName(path.basename(abs));
            const name = uniqueDiskName(dir, base);
            copyFileSync(abs, path.join(dir, name));
            const kind: GalleryKind = detectGalleryKind(`file://${base}`);
            const item: GalleryItem = {
                id: createGalleryId(),
                kind: kind === "link" ? "file" : kind,
                sourceUrl: "",
                title: (meta.title ?? "").slice(0, 256) || base,
                fileName: name,
                localFile: `${FILES_DIR}/${name}`,
                sizeBytes: st.size,
                mime: "application/octet-stream",
                guildId: (meta.guildId ?? "").slice(0, 64),
                guildName: (meta.guildName ?? "").slice(0, 128),
                channelId: (meta.channelId ?? "").slice(0, 64),
                channelName: (meta.channelName ?? "").slice(0, 128),
                authorName: (meta.authorName ?? "").slice(0, 128),
                messageId: "",
                savedAt: Date.now(),
                categories: (meta.categories ?? []).map(normalizeCategoryName).filter(Boolean).slice(0, 12),
            };
            idx.items.unshift(item);
            made.push(item);
        } catch (e) {
            console.warn("[Gallery] import failed:", abs, e);
        }
    }
    if (made.length === 0) return { ok: false, error: "NO_FILES" };
    schedulePersist();
    invalidateDiskUsage();
    return { ok: true, items: made };
}

export function updateGalleryItem(
    id: string,
    patch: Partial<Pick<GalleryItem, "title" | "categories">>,
): { ok: true; item: GalleryItem } | { ok: false; error: string } {
    const idx = loadGalleryIndex();
    const item = idx.items.find((i) => i.id === id);
    if (!item) return { ok: false, error: "NOT_FOUND" };
    if (typeof patch.title === "string") item.title = patch.title.slice(0, 256);
    if (Array.isArray(patch.categories)) {
        item.categories = patch.categories.map(normalizeCategoryName).filter(Boolean).slice(0, 12);
    }
    schedulePersist();
    return { ok: true, item };
}

export function removeGalleryItem(id: string): { ok: boolean } {
    const idx = loadGalleryIndex();
    const at = idx.items.findIndex((i) => i.id === id);
    if (at === -1) return { ok: false };
    const [removed] = idx.items.splice(at, 1);
    if (removed?.localFile) {
        try {
            const abs = path.join(getGalleryRoot(), removed.localFile);
            if (abs.startsWith(path.resolve(getGalleryRoot())) && existsSync(abs)) rmSync(abs, { force: true });
        } catch {}
    }
    schedulePersist();
    invalidateDiskUsage();
    return { ok: true };
}

export function bulkRemoveGalleryItems(ids: string[]): { removed: number } {
    const set = new Set(ids);
    const idx = loadGalleryIndex();
    const doomed = idx.items.filter((i) => set.has(i.id));
    idx.items = idx.items.filter((i) => !set.has(i.id));
    for (const item of doomed) {
        if (!item.localFile) continue;
        try {
            const abs = path.join(getGalleryRoot(), item.localFile);
            if (abs.startsWith(path.resolve(getGalleryRoot())) && existsSync(abs)) rmSync(abs, { force: true });
        } catch {}
    }
    schedulePersist();
    invalidateDiskUsage();
    return { removed: doomed.length };
}

/** Exact disk usage: index file + every stored media byte, measured from fs. */
export function getGalleryDiskUsage(): { bytes: number; files: number; items: number } {
    if (diskUsageCache && Date.now() - diskUsageCache.at < 2000) {
        return { bytes: diskUsageCache.bytes, files: countFiles(), items: loadGalleryIndex().items.length };
    }
    const root = ensureGalleryDirs();
    let bytes = 0;
    let files = 0;
    const walk = (dir: string) => {
        let entries: string[] = [];
        try {
            entries = readdirSync(dir);
        } catch {
            return;
        }
        for (const e of entries) {
            const abs = path.join(dir, e);
            try {
                const st = statSync(abs);
                if (st.isDirectory()) walk(abs);
                else {
                    bytes += st.size;
                    files++;
                }
            } catch {}
        }
    };
    walk(root);
    diskUsageCache = { bytes, at: Date.now() };
    return { bytes, files, items: loadGalleryIndex().items.length };
}

function countFiles(): number {
    const root = getGalleryRoot();
    try {
        return readdirSync(filesDir(root)).length;
    } catch {
        return 0;
    }
}

export function resolveGalleryFile(item: GalleryItem): string | null {
    if (!item.localFile) return null;
    const abs = path.join(getGalleryRoot(), item.localFile);
    const resolved = path.resolve(abs);
    if (!resolved.startsWith(path.resolve(getGalleryRoot()))) return null;
    return existsSync(resolved) ? resolved : null;
}

export function setGalleryRoot(next: string): { ok: true; root: string } | { ok: false; error: string } {
    const clean = next.trim();
    if (!clean || clean.length > 512) return { ok: false, error: "INVALID_PATH" };
    const resolved = path.resolve(clean);
    try {
        mkdirSync(resolved, { recursive: true });
        mkdirSync(path.join(resolved, FILES_DIR), { recursive: true });
    } catch {
        return { ok: false, error: "NOT_WRITABLE" };
    }
    const current = path.resolve(getGalleryRoot());
    if (resolved !== current) {
        // Migrate existing cache so switching disk/partition/folder keeps items.
        try {
            if (existsSync(indexPath(current))) {
                copyFileSync(indexPath(current), indexPath(resolved));
            }
            const fromFiles = path.join(current, FILES_DIR);
            const toFiles = path.join(resolved, FILES_DIR);
            if (existsSync(fromFiles)) {
                cpSync(fromFiles, toFiles, { recursive: true, force: true });
            }
        } catch (e) {
            console.warn("[Gallery] migration copy failed:", e);
        }
        indexCache = null;
        diskUsageCache = null;
        ensureGalleryDirs(resolved);
        loadGalleryIndex();
    }
    setConfig("galleryRoot", resolved);
    return { ok: true, root: resolved };
}

export function resetGalleryCache(): void {
    indexCache = null;
    diskUsageCache = null;
}

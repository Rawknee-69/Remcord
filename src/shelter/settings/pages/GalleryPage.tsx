import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
    filterGalleryItems,
    formatGalleryBytes,
    formatGalleryDate,
    type GalleryCategory,
    type GalleryGroupBy,
    type GalleryItem,
    galleryThemeFromBackground,
    groupGalleryItems,
    paginateGalleryItems,
    sortGalleryItems,
} from "../../../common/galleryUtils.js";
import { Dropdown } from "../components/Dropdown.jsx";
import { EmptyState } from "../components/EmptyState.jsx";
import { SettingsPageHeader } from "../components/SettingsPageHeader.jsx";
import { isNativeWebView } from "../settings.js";
import classes from "./GalleryPage.module.css";

const {
    ui: { Header, HeaderTags, Divider, Button, ButtonSizes, TextBox, openModal, openConfirmationModal, showToast },
    plugin: { store },
    flux,
} = shelter as unknown as {
    // biome-ignore lint/suspicious/noExplicitAny: shelter runtime bridge is untyped
    ui: Record<string, any>;
    // biome-ignore lint/suspicious/noExplicitAny: shelter plugin store is untyped
    plugin: { store: Record<string, any> };
    // biome-ignore lint/suspicious/noExplicitAny: shelter flux stores are untyped
    flux: { storesFlat: Record<string, any> };
};

const PAGE_STEP = 60;
const PREFS_KEY = "remcord:gallery:prefs";
const blobCache = new Map<string, string>();

/**
 * Discord's effective appearance: whatever theme — stock dark/light or any
 * custom theme — actually resolved `--background-primary` to. Read from an
 * element inside Discord's themed tree (the page root lives in the settings
 * modal; `document.documentElement` sits outside the theme classes where
 * the variables don't resolve). Falls back to the OS preference when
 * unreadable, so the page never hardcodes an appearance. Never throws.
 */
function detectDiscordTheme(page: Element | undefined): "light" | "dark" {
    // Menus and modals can render outside the themed subtree, so every
    // scope that may carry the variables is tried in order (themed
    // container, app mount, page root, body, document root).
    const scopes: Array<Element | undefined> = [];
    try {
        scopes.push(document.querySelector(".theme-dark, .theme-light") ?? undefined);
    } catch {}
    try {
        scopes.push(document.getElementById("app-mount") ?? undefined);
    } catch {}
    scopes.push(page);
    try {
        scopes.push(document.body);
    } catch {}
    try {
        scopes.push(document.documentElement);
    } catch {}
    for (const scope of scopes) {
        if (!scope) continue;
        try {
            const parsed = galleryThemeFromBackground(
                getComputedStyle(scope).getPropertyValue("--background-primary") ?? "",
            );
            if (parsed) return parsed;
        } catch {}
    }
    try {
        return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
    } catch {
        return "dark";
    }
}

function readPrefs(): Partial<{ groupBy: GalleryGroupBy; sort: "newest" | "oldest"; kind: string }> {
    try {
        return JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}");
    } catch {
        return {};
    }
}

function writePrefs(p: Record<string, unknown>): void {
    try {
        localStorage.setItem(PREFS_KEY, JSON.stringify(p));
    } catch {}
}

function shelterContext(): { guildId: string; guildName: string; channelId: string; channelName: string } {
    const out = { guildId: "", guildName: "", channelId: "", channelName: "" };
    try {
        const flat = flux?.storesFlat;
        const guildId: string = flat?.SelectedGuildStore?.getGuildId?.() ?? "";
        const channelId: string = flat?.SelectedChannelStore?.getChannelId?.() ?? "";
        out.guildId = guildId ?? "";
        out.channelId = channelId ?? "";
        try {
            out.guildName = (guildId && flat?.GuildStore?.getGuild?.(guildId)?.name) || "";
        } catch {}
        try {
            out.channelName = (channelId && flat?.ChannelStore?.getChannel?.(channelId)?.name) || "";
        } catch {}
    } catch {}
    return out;
}

function castItem(raw: Record<string, unknown>): GalleryItem {
    return {
        id: String(raw.id ?? ""),
        kind: (raw.kind as GalleryItem["kind"]) ?? "link",
        sourceUrl: String(raw.sourceUrl ?? ""),
        title: String(raw.title ?? ""),
        note: String(raw.note ?? ""),
        fileName: String(raw.fileName ?? ""),
        localFile: String(raw.localFile ?? ""),
        sizeBytes: Number(raw.sizeBytes ?? 0),
        mime: String(raw.mime ?? ""),
        guildId: String(raw.guildId ?? ""),
        guildName: String(raw.guildName ?? ""),
        channelId: String(raw.channelId ?? ""),
        channelName: String(raw.channelName ?? ""),
        authorName: String(raw.authorName ?? ""),
        messageId: String(raw.messageId ?? ""),
        savedAt: Number(raw.savedAt ?? 0),
        categories: Array.isArray(raw.categories) ? (raw.categories as string[]) : [],
    };
}

function castCategory(raw: Record<string, unknown>): GalleryCategory {
    return {
        name: String(raw.name ?? ""),
        color: String(raw.color ?? "#5865f2"),
        createdAt: Number(raw.createdAt ?? 0),
    };
}

function Thumb(props: { item: GalleryItem }) {
    const [blobUrl, setBlobUrl] = createSignal<string | null>(null);
    const needsBlob = () => !props.item.sourceUrl.startsWith("http");
    onMount(() => {
        let alive = true;
        if (needsBlob()) {
            const cached = blobCache.get(props.item.id);
            if (cached) {
                setBlobUrl(cached);
            } else {
                void window.remcord.gallery.getFile(props.item.id).then((res) => {
                    if (!alive) return;
                    if (res.ok) {
                        try {
                            const bin = atob(res.base64);
                            const bytes = new Uint8Array(bin.length);
                            for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
                            const url = URL.createObjectURL(
                                new Blob([bytes], { type: res.mime || "application/octet-stream" }),
                            );
                            if (blobCache.size > 120) {
                                const first = blobCache.keys().next().value;
                                if (first) {
                                    try {
                                        URL.revokeObjectURL(blobCache.get(first)!);
                                    } catch {}
                                    blobCache.delete(first);
                                }
                            }
                            blobCache.set(props.item.id, url);
                            setBlobUrl(url);
                        } catch {}
                    }
                });
            }
            onCleanup(() => {
                alive = false;
            });
        }
    });
    return (
        <Show
            when={props.item.kind !== "link" && props.item.kind !== "file"}
            fallback={
                <Show
                    when={props.item.sourceUrl}
                    fallback={<div class={classes.linkThumb}>{props.item.title || props.item.fileName || "File"}</div>}
                >
                    <div class={classes.linkThumb}>{props.item.sourceUrl}</div>
                </Show>
            }
        >
            <Show
                when={props.item.kind === "image"}
                fallback={
                    <video class={classes.thumbVideo} preload="none" controls src={blobUrl() ?? props.item.sourceUrl}>
                        <track kind="captions" />
                    </video>
                }
            >
                <img
                    class={classes.thumb}
                    loading="lazy"
                    decoding="async"
                    src={blobUrl() ?? props.item.sourceUrl}
                    alt={props.item.title}
                    onError={(e) => {
                        // CDN expiry fallback: try the disk-cached blob once.
                        if (!blobUrl() && props.item.localFile) {
                            void window.remcord.gallery.getFile(props.item.id).then((res) => {
                                if (res.ok) {
                                    try {
                                        const bin = atob(res.base64);
                                        const bytes = new Uint8Array(bin.length);
                                        for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
                                        const url = URL.createObjectURL(
                                            new Blob([bytes], { type: res.mime || "image/*" }),
                                        );
                                        blobCache.set(props.item.id, url);
                                        (e.currentTarget as HTMLImageElement).src = url;
                                    } catch {}
                                }
                            });
                        }
                    }}
                />
            </Show>
        </Show>
    );
}

function AddLinkModal(props: { close: () => void; onSaved: () => void }) {
    const [url, setUrl] = createSignal("");
    const [title, setTitle] = createSignal("");
    const [note, setNote] = createSignal("");
    const ctx = shelterContext();
    function save() {
        const u = url().trim();
        if (!/^https?:\/\//i.test(u)) {
            showToast({ title: "Invalid URL", content: "Paste a full http(s) URL.", duration: 3000 });
            return;
        }
        void window.remcord.gallery
            .addUrl(u, {
                title: title().trim(),
                note: note().trim(),
                guildId: ctx.guildId,
                guildName: ctx.guildName,
                channelId: ctx.channelId,
                channelName: ctx.channelName,
            })
            .then((res) => {
                if ((res as { ok?: boolean }).ok === false) {
                    showToast({
                        title: "Save failed",
                        content: String((res as { error?: string }).error ?? ""),
                        duration: 3000,
                    });
                    return;
                }
                props.onSaved();
                props.close();
            });
    }
    return (
        <ui.ModalRoot size={ui.ModalSizes.SMALL}>
            <ui.ModalHeader close={props.close}>Save to Gallery</ui.ModalHeader>
            <ui.ModalBody>
                <Header tag={HeaderTags.HeadingSM}>URL</Header>
                <TextBox
                    value={url()}
                    onInput={setUrl}
                    placeholder="https://cdn.discordapp.com/… / youtube link / any link"
                />
                <Divider mt mb />
                <Header tag={HeaderTags.HeadingSM}>Title (optional)</Header>
                <TextBox value={title()} onInput={setTitle} placeholder="My clip" />
                <Divider mt mb />
                <Header tag={HeaderTags.HeadingSM}>Note (optional)</Header>
                <TextBox value={note()} onInput={setNote} placeholder="Why are you saving this?" />
                <p class={classes.hint}>
                    Tagged with {ctx.guildName || "current server"} #{ctx.channelName || "current channel"} · snapshots
                    stay even if you leave the server.
                </p>
            </ui.ModalBody>
            <ui.ModalConfirmFooter confirmText="Save" onConfirm={save} close={props.close} />
        </ui.ModalRoot>
    );
}

// shelter ui accessed late (Modal components) to keep tree-shaking simple.
// biome-ignore lint/suspicious/noExplicitAny: shelter runtime bridge is untyped
const ui = (shelter as unknown as { ui: Record<string, any> }).ui;

// shelter `Button size=SMALL` is a fixed 60x32px box (see shelter-ui
// ButtonSizes): long labels overflow the button and collide with neighbours.
// Every gallery Button below passes this auto-width style (the CSS module
// enforces the same as a second layer) so labels size their buttons.
const BTN_AUTO = {
    "--shltr-btn-w": "auto",
    "--shltr-btn-h": "36px",
    "white-space": "nowrap",
    "flex-shrink": "0",
} as Record<string, string>;

export function GalleryPage() {
    const prefs = readPrefs();
    const [items, setItems] = createSignal<GalleryItem[]>([]);
    const [categories, setCategories] = createSignal<GalleryCategory[]>([]);
    const [usage, setUsage] = createSignal({ bytes: 0, files: 0, items: 0 });
    const [root, setRoot] = createSignal("");
    const [loading, setLoading] = createSignal(true);
    const [query, setQuery] = createSignal("");
    const [kind, setKind] = createSignal("all");
    const [guild, setGuild] = createSignal("all");
    const [channel, setChannel] = createSignal("all");
    const [category, setCategory] = createSignal("all");
    const [groupBy, setGroupBy] = createSignal<GalleryGroupBy>((prefs.groupBy as GalleryGroupBy) ?? "none");
    const [sort, setSort] = createSignal<"newest" | "oldest">((prefs.sort as "newest" | "oldest") ?? "newest");
    const [visible, setVisible] = createSignal(PAGE_STEP);
    const [selected, setSelected] = createSignal<Set<string>>(new Set());
    const [newCat, setNewCat] = createSignal("");
    const [assignCat, setAssignCat] = createSignal("");
    // Live Discord theme: re-read whenever the OS scheme flips or Discord
    // rewrites its root (theme switch / custom theme applied), so the
    // `data-gallery-theme` tokens and `color-scheme` always match Discord.
    // Reads from the page root (inside the themed settings modal) because
    // `document.documentElement` sits outside Discord's theme classes.
    let pageEl: HTMLDivElement | undefined;
    const [theme, setTheme] = createSignal<"light" | "dark">(detectDiscordTheme(undefined));

    let sentinel: HTMLDivElement | undefined;

    function refresh() {
        // The native shell serves the full gallery surface through
        // `window.remcord.gallery` (host-owned storage root plus the
        // gallery index), so no environment branch is needed here.
        setLoading(true);
        void window.remcord.gallery
            .list()
            .then((res) => {
                setItems((res.items as Record<string, unknown>[]).map(castItem));
                setCategories((res.categories as Record<string, unknown>[]).map(castCategory));
            })
            .catch(() => showToast({ title: "Gallery failed to load", duration: 2500 }))
            .finally(() => setLoading(false));
        void window.remcord.gallery
            .usage()
            .then(setUsage)
            .catch(() => {});
        void window.remcord.gallery
            .root()
            .then((r) => setRoot(r.root))
            .catch(() => {});
    }

    onMount(() => {
        refresh();
        setTheme(detectDiscordTheme(pageEl));
        const updateTheme = () => setTheme(detectDiscordTheme(pageEl));
        const media = window.matchMedia("(prefers-color-scheme: dark)");
        const onMedia = () => updateTheme();
        try {
            media.addEventListener("change", onMedia);
        } catch {}
        const observer = new MutationObserver(updateTheme);
        // Watch every likely theme carrier: the document root plus the app
        // mount and the themed wrapper (Discord applies `.theme-dark` /
        // `.theme-light` below `documentElement`, where the variables live).
        const carriers: Element[] = [document.documentElement];
        try {
            const mount = document.getElementById("app-mount");
            if (mount) carriers.push(mount);
            const themed = document.querySelector(".theme-dark, .theme-light");
            if (themed && !carriers.includes(themed)) carriers.push(themed);
        } catch {}
        for (const target of carriers) {
            try {
                observer.observe(target, {
                    attributes: true,
                    attributeFilter: ["class", "style", "data-theme"],
                });
            } catch {}
        }
        const io = new IntersectionObserver(
            (entries) => {
                if (entries.some((e) => e.isIntersecting)) setVisible((v) => v + PAGE_STEP);
            },
            { rootMargin: "600px" },
        );
        if (sentinel) io.observe(sentinel);
        onCleanup(() => {
            io.disconnect();
            observer.disconnect();
            try {
                media.removeEventListener("change", onMedia);
            } catch {}
        });
    });

    const guildOptions = createMemo(() => {
        const map = new Map<string, string>();
        for (const it of items()) {
            const key = it.guildId || it.guildName || "";
            if (key) map.set(key, it.guildName || "Unknown server");
        }
        return [...map.entries()].sort((a, b) => a[1].localeCompare(b[1]));
    });
    const channelOptions = createMemo(() => {
        const map = new Map<string, string>();
        for (const it of items()) {
            if (guild() !== "all" && it.guildId !== guild() && it.guildName !== guild()) continue;
            const key = it.channelId || it.channelName || "";
            if (key) map.set(key, it.channelName || "Unknown channel");
        }
        return [...map.entries()].sort((a, b) => a[1].localeCompare(b[1]));
    });

    const filtered = createMemo(() =>
        sortGalleryItems(
            filterGalleryItems(items(), {
                query: query(),
                kind: kind() as never,
                guild: guild(),
                channel: channel(),
                category: category(),
                fromMs: null,
                toMs: null,
            }),
            sort(),
        ),
    );

    // Flatten grouped order, then window — keeps the DOM bounded (<visible>
    // cards) no matter how large the library grows.
    const visibleRows = createMemo(() => {
        const groups = groupGalleryItems(filtered(), groupBy());
        const flat: Array<{ label: string; groupKey: string; item: GalleryItem }> = [];
        for (const g of groups) for (const item of g.items) flat.push({ label: g.label, groupKey: g.key, item });
        return { groups: paginateGalleryItems(flat, 0, visible()), total: flat.length };
    });

    const groupedVisible = createMemo(() => {
        const map = new Map<string, { label: string; items: GalleryItem[] }>();
        for (const row of visibleRows().groups) {
            const e = map.get(row.groupKey) ?? { label: row.label, items: [] };
            e.items.push(row.item);
            map.set(row.groupKey, e);
        }
        return [...map.entries()].map(([key, v]) => ({ key, ...v }));
    });

    function toggleSelect(id: string) {
        setSelected((prev) => {
            const next = new Set(prev);
            if (next.has(id)) next.delete(id);
            else next.add(id);
            return next;
        });
    }

    function deleteSelected() {
        const ids = [...selected()];
        if (ids.length === 0) return;
        openConfirmationModal({
            header: () => `Delete ${ids.length} item${ids.length === 1 ? "" : "s"}?`,
            body: () => "This removes them from the Gallery cache on disk. This cannot be undone.",
            type: "danger",
            confirmText: "Delete",
            cancelText: "Cancel",
        }).then(
            () => {
                void window.remcord.gallery.bulkRemove(ids).then((r) => {
                    showToast({ title: `Deleted ${r.removed} item(s)`, duration: 2500 });
                    setSelected(new Set());
                    refresh();
                });
            },
            () => {},
        );
    }

    function createCategory() {
        const name = newCat().trim();
        if (!name) return;
        void window.remcord.gallery.categoryAdd(name).then((res) => {
            if ((res as { ok?: boolean }).ok) {
                setNewCat("");
                refresh();
            } else {
                showToast({ title: "Category exists or invalid", duration: 2500 });
            }
        });
    }

    function assignCategoryToSelected() {
        const name = assignCat();
        if (!name || selected().size === 0) return;
        const ids = [...selected()];
        void Promise.all(ids.map((id) => window.remcord.gallery.update(id, { categories: [name] }))).then(() => {
            showToast({ title: `Tagged ${ids.length} item(s) as ${name}`, duration: 2500 });
            refresh();
        });
    }

    function openAddModal() {
        openModal(({ close }: { close: () => void }) => <AddLinkModal close={close} onSaved={refresh} />);
    }

    function importFiles() {
        // Native WebView2 serves the host multi-file picker plus bounded
        // ingestion through `gallery.importFiles`; Electron keeps the
        // original `importDialog` call.
        if (isNativeWebView()) {
            void window.remcord.gallery
                .importFiles()
                .then((res) => {
                    const summary = res as { imported?: number; skipped?: number };
                    if (typeof summary?.imported === "number" && summary.imported > 0) refresh();
                    showToast({
                        title: `Imported ${summary?.imported ?? 0} file(s)${(summary?.skipped ?? 0) > 0 ? `, ${summary.skipped} skipped` : ""}`,
                        duration: 3000,
                    });
                })
                .catch(() => showToast({ title: "Import failed", duration: 2500 }));
            return;
        }
        const ctx = shelterContext();
        void window.remcord.gallery.importDialog(ctx).then((res) => {
            if ((res as { ok?: boolean }).ok) refresh();
        });
    }

    const t = store.i18n ?? {};

    return (
        <div ref={pageEl} class={classes.page} data-gallery-theme={theme()}>
            <SettingsPageHeader
                title={t["gallery-title"] ?? "Gallery"}
                description={
                    t["gallery-desc"] ??
                    "Everything you save — images, videos, links — grouped by server, channel and date. Items stay here even if you leave the server."
                }
            />
            {/* Native WebView2 serves the full gallery surface through
                `window.remcord.gallery`, including the host-owned folder
                picker behind Choose folder. */}
            <div class={classes.storageBar}>
                <span class={classes.storagePath} title={root()}>
                    {root() || "…"} · {usage().items} items · {formatGalleryBytes(usage().bytes)} on disk
                </span>
                <div class={classes.storageActions}>
                    <button
                        type="button"
                        class={classes.miniBtn}
                        title="Choose the gallery storage folder (any disk / partition / folder)"
                        onClick={() =>
                            void window.remcord.gallery
                                .pickRoot()
                                .then((r) => {
                                    const res = r as { ok?: boolean; error?: string };
                                    if (res.ok) refresh();
                                    else if (res.error && res.error !== "CANCELLED")
                                        showToast({ title: "Could not change folder", duration: 2500 });
                                })
                                .catch(() => showToast({ title: "Could not change folder", duration: 2500 }))
                        }
                    >
                        Choose folder
                    </button>
                    <button type="button" class={classes.miniBtn} onClick={() => window.remcord.gallery.openRoot()}>
                        Open
                    </button>
                    <button type="button" class={classes.miniBtn} onClick={refresh}>
                        Refresh
                    </button>
                </div>
            </div>

            <section class={classes.section} aria-label="Filters and primary actions">
                <h2 class={classes.sectionTitle}>Filters</h2>
                <div class={classes.toolbar}>
                    <div class={classes.search}>
                        <TextBox
                            value={query()}
                            onInput={(v: string) => {
                                setQuery(v);
                                setVisible(PAGE_STEP);
                            }}
                            placeholder="Search title, server, channel…"
                            aria-label="Search gallery"
                        />
                    </div>
                    <div class={classes.filterRow}>
                        <Dropdown
                            class={classes.dd}
                            value={kind()}
                            onChange={(v) => {
                                setKind(v);
                                setVisible(PAGE_STEP);
                            }}
                            options={[
                                { label: "All types", value: "all" },
                                { label: "Images", value: "image" },
                                { label: "Videos", value: "video" },
                                { label: "Links", value: "link" },
                                { label: "Files", value: "file" },
                            ]}
                        />
                        <Dropdown
                            class={classes.dd}
                            limitHeight
                            maxHeight={280}
                            value={guild()}
                            onChange={(v) => {
                                setGuild(v);
                                setChannel("all");
                                setVisible(PAGE_STEP);
                            }}
                            options={[
                                { label: "All servers", value: "all" },
                                ...guildOptions().map(([id, name]) => ({ label: name, value: id })),
                            ]}
                        />
                        <Dropdown
                            class={classes.dd}
                            limitHeight
                            maxHeight={280}
                            value={channel()}
                            onChange={(v) => {
                                setChannel(v);
                                setVisible(PAGE_STEP);
                            }}
                            options={[
                                { label: "All channels", value: "all" },
                                ...channelOptions().map(([id, name]) => ({ label: name, value: id })),
                            ]}
                        />
                        <Dropdown
                            class={classes.dd}
                            limitHeight
                            maxHeight={280}
                            value={category()}
                            onChange={(v) => {
                                setCategory(v);
                                setVisible(PAGE_STEP);
                            }}
                            options={[
                                { label: "All collections", value: "all" },
                                ...categories().map((c) => ({ label: c.name, value: c.name })),
                            ]}
                        />
                        <Dropdown
                            class={classes.dd}
                            value={groupBy()}
                            onChange={(v) => {
                                setGroupBy(v as GalleryGroupBy);
                                writePrefs({ groupBy: v, sort: sort(), kind: kind() });
                            }}
                            options={[
                                { label: "No grouping", value: "none" },
                                { label: "Group by server", value: "guild" },
                                { label: "Group by channel", value: "channel" },
                                { label: "Group by date", value: "date" },
                                { label: "Group by type", value: "kind" },
                                { label: "Group by collection", value: "category" },
                            ]}
                        />
                        <Dropdown
                            class={classes.dd}
                            value={sort()}
                            onChange={(v) => {
                                setSort(v as "newest" | "oldest");
                                writePrefs({ groupBy: groupBy(), sort: v, kind: kind() });
                            }}
                            options={[
                                { label: "Newest first", value: "newest" },
                                { label: "Oldest first", value: "oldest" },
                            ]}
                        />
                    </div>
                    <div class={classes.toolbarActions}>
                        <Button size={ButtonSizes.SMALL} style={BTN_AUTO} onClick={openAddModal}>
                            Add link
                        </Button>
                        <Button size={ButtonSizes.SMALL} style={BTN_AUTO} onClick={importFiles}>
                            Import files
                        </Button>
                    </div>
                </div>
            </section>

            <section class={classes.section} aria-label="Collection management">
                <h2 class={classes.sectionTitle}>Collections</h2>
                <div class={classes.categoryRow}>
                    <div class={classes.catInput}>
                        <TextBox
                            value={newCat()}
                            onInput={setNewCat}
                            placeholder="New collection name…"
                            aria-label="New collection name"
                        />
                    </div>
                    <Button
                        size={ButtonSizes.SMALL}
                        style={BTN_AUTO}
                        onClick={createCategory}
                        aria-label="Create collection"
                        title="Create collection"
                    >
                        <span aria-hidden="true" class={classes.plusIcon}>
                            +
                        </span>
                    </Button>
                    <div class={classes.catList}>
                        <For each={categories()}>
                            {(c) => (
                                <span class={classes.catPill} style={{ background: c.color }} title={c.name}>
                                    {c.name}
                                    <button
                                        type="button"
                                        class={classes.miniBtn}
                                        title={`Delete ${c.name}`}
                                        aria-label={`Delete collection ${c.name}`}
                                        onClick={() =>
                                            void window.remcord.gallery.categoryRemove(c.name).then(() => refresh())
                                        }
                                    >
                                        ×
                                    </button>
                                </span>
                            )}
                        </For>
                    </div>
                </div>
            </section>

            <Show when={selected().size > 0}>
                <section class={classes.section} aria-label="Bulk actions">
                    <h2 class={classes.sectionTitle}>Bulk actions</h2>
                    <div class={classes.selectionBar}>
                        <span class={classes.selectionCount}>
                            {selected().size} selected of {visibleRows().total} matching
                        </span>
                        <Dropdown
                            class={classes.dd}
                            limitHeight
                            maxHeight={240}
                            value={assignCat() || "__none"}
                            onChange={(v) => setAssignCat(v === "__none" ? "" : v)}
                            options={[
                                { label: "Tag with collection…", value: "__none" },
                                ...categories().map((c) => ({ label: c.name, value: c.name })),
                            ]}
                        />
                        <div class={classes.selectionActions}>
                            <Button
                                size={ButtonSizes.SMALL}
                                style={BTN_AUTO}
                                onClick={assignCategoryToSelected}
                                disabled={!assignCat()}
                            >
                                Tag
                            </Button>
                            <Show
                                when={!isNativeWebView()}
                                fallback={
                                    <Button
                                        size={ButtonSizes.SMALL}
                                        style={BTN_AUTO}
                                        onClick={() =>
                                            void window.remcord.gallery
                                                .exportItems([...selected()])
                                                .then((res) => {
                                                    const summary = res as { exported?: number; skipped?: number };
                                                    showToast({
                                                        title: `Copied ${summary?.exported ?? 0} file(s)${(summary?.skipped ?? 0) > 0 ? `, ${summary.skipped} skipped` : ""}`,
                                                        duration: 3000,
                                                    });
                                                })
                                                .catch(() => showToast({ title: "Copy failed", duration: 2500 }))
                                        }
                                    >
                                        Copy to folder
                                    </Button>
                                }
                            >
                                <Button
                                    size={ButtonSizes.SMALL}
                                    style={BTN_AUTO}
                                    onClick={() => void window.remcord.gallery.exportItems([...selected()])}
                                >
                                    Copy to folder
                                </Button>
                            </Show>
                            <Button size={ButtonSizes.SMALL} style={BTN_AUTO} onClick={deleteSelected}>
                                Delete selected
                            </Button>
                            <Button size={ButtonSizes.SMALL} style={BTN_AUTO} onClick={() => setSelected(new Set())}>
                                Clear
                            </Button>
                            <Button
                                size={ButtonSizes.SMALL}
                                style={BTN_AUTO}
                                onClick={() => setSelected(new Set(filtered().map((i) => i.id)))}
                            >
                                Select all matching
                            </Button>
                        </div>
                    </div>
                </section>
            </Show>

            <Show
                when={loading()}
                fallback={
                    <Show
                        when={filtered().length > 0}
                        fallback={
                            <EmptyState
                                message="Nothing saved yet"
                                description="Right-click any image, video or link in Discord and choose “Save to Gallery”, or use Add link / Import files above."
                            />
                        }
                    >
                        <p class={classes.hint}>
                            Showing {visibleRows().groups.length} of {visibleRows().total} matching · right-click any
                            media in Discord → Save to Gallery
                        </p>
                        <For each={groupedVisible()}>
                            {(g) => (
                                <>
                                    <Show when={groupBy() !== "none"}>
                                        <Header tag={HeaderTags.HeadingSM} class={classes.groupTitle}>
                                            {g.label} ({g.items.length})
                                        </Header>
                                        <Divider mt mb />
                                    </Show>
                                    <div class={classes.grid}>
                                        <For each={g.items}>
                                            {(item) => {
                                                const isSel = () => selected().has(item.id);
                                                return (
                                                    <article
                                                        class={`${classes.card} ${isSel() ? classes.cardSelected : ""}`}
                                                    >
                                                        <label class={classes.checkRow}>
                                                            <input
                                                                type="checkbox"
                                                                checked={isSel()}
                                                                onChange={() => toggleSelect(item.id)}
                                                                aria-label={`Select ${item.title || item.fileName || item.id}`}
                                                            />
                                                            <span class={classes.checkDate}>
                                                                {formatGalleryDate(item.savedAt)}
                                                            </span>
                                                            <Show when={isSel()}>
                                                                <span class={classes.selectedBadge}>Selected</span>
                                                            </Show>
                                                        </label>
                                                        <div class={classes.thumbWrap}>
                                                            <Thumb item={item} />
                                                        </div>
                                                        <div class={classes.meta}>
                                                            <span
                                                                class={classes.title}
                                                                title={item.title || item.fileName}
                                                            >
                                                                {item.title || item.fileName || "Untitled"}
                                                            </span>
                                                            <Show when={item.note}>
                                                                <span class={classes.note} title={item.note}>
                                                                    {item.note}
                                                                </span>
                                                            </Show>
                                                            <span class={classes.sub}>
                                                                {(item.guildName || "External") +
                                                                    (item.channelName ? ` · #${item.channelName}` : "")}
                                                            </span>
                                                            <span class={classes.sub}>
                                                                {item.kind} ·{" "}
                                                                {item.sizeBytes > 0
                                                                    ? formatGalleryBytes(item.sizeBytes)
                                                                    : "link"}
                                                            </span>
                                                            <Show when={item.categories.length > 0}>
                                                                <span class={classes.tagRow}>
                                                                    <For each={item.categories}>
                                                                        {(cn) => {
                                                                            const col =
                                                                                categories().find((c) => c.name === cn)
                                                                                    ?.color ?? "#5865f2";
                                                                            return (
                                                                                <span
                                                                                    class={classes.catPill}
                                                                                    style={{ background: col }}
                                                                                    title={cn}
                                                                                >
                                                                                    {cn}
                                                                                </span>
                                                                            );
                                                                        }}
                                                                    </For>
                                                                </span>
                                                            </Show>
                                                        </div>
                                                        <div class={classes.cardActions}>
                                                            <Show when={item.sourceUrl}>
                                                                <button
                                                                    type="button"
                                                                    class={classes.miniBtn}
                                                                    title="Open original link"
                                                                    onClick={() =>
                                                                        window.open(
                                                                            item.sourceUrl,
                                                                            "_blank",
                                                                            "noopener",
                                                                        )
                                                                    }
                                                                >
                                                                    Open
                                                                </button>
                                                            </Show>
                                                            <button
                                                                type="button"
                                                                class={classes.miniBtn}
                                                                title="Copy file to a folder"
                                                                onClick={() =>
                                                                    void window.remcord.gallery.exportItems([item.id])
                                                                }
                                                            >
                                                                Copy
                                                            </button>
                                                            <button
                                                                type="button"
                                                                class={`${classes.miniBtn} ${classes.miniBtnDanger}`}
                                                                title="Delete this item"
                                                                onClick={() =>
                                                                    void window.remcord.gallery
                                                                        .remove(item.id)
                                                                        .then(() => refresh())
                                                                }
                                                            >
                                                                Delete
                                                            </button>
                                                        </div>
                                                    </article>
                                                );
                                            }}
                                        </For>
                                    </div>
                                </>
                            )}
                        </For>
                        <div ref={sentinel} class={classes.sentinel} />
                    </Show>
                }
            >
                <EmptyState message="Loading Gallery…" description="Reading the on-disk index." />
            </Show>
        </div>
    );
}

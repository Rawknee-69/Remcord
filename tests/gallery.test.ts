import assert from "node:assert/strict";
import { test } from "node:test";
import {
    detectGalleryKind,
    filterGalleryItems,
    formatGalleryBytes,
    type GalleryItem,
    galleryThemeFromBackground,
    groupGalleryItems,
    paginateGalleryItems,
    sortGalleryItems,
} from "../src/common/galleryUtils.js";

function item(over: Partial<GalleryItem>): GalleryItem {
    return {
        id: "a",
        kind: "image",
        sourceUrl: "https://cdn.discordapp.com/x.png",
        title: "pic",
        note: "",
        fileName: "x.png",
        localFile: "files/x.png",
        sizeBytes: 10,
        mime: "image/png",
        guildId: "g1",
        guildName: "Guild",
        channelId: "c1",
        channelName: "general",
        authorName: "user",
        messageId: "",
        savedAt: 1000,
        categories: [],
        ...over,
    };
}

test("detectGalleryKind classifies media vs links", () => {
    assert.equal(detectGalleryKind("https://x/y.png"), "image");
    assert.equal(detectGalleryKind("https://x/y.mp4"), "video");
    assert.equal(detectGalleryKind("https://x/watch?v=1"), "link");
    assert.equal(detectGalleryKind("https://x/y.png", "video/mp4"), "video");
});

test("formatGalleryBytes is exact at boundaries", () => {
    assert.equal(formatGalleryBytes(0), "0 B");
    assert.equal(formatGalleryBytes(1023), "1023 B");
    assert.equal(formatGalleryBytes(1024), "1 KB");
    assert.equal(formatGalleryBytes(1536), "1.5 KB");
});

test("filter keeps snapshots even when guild unknown and paginates", () => {
    const items = [item({ id: "1", guildName: "Old Guild" }), item({ id: "2", kind: "link", guildName: "" })];
    assert.equal(
        filterGalleryItems(items, {
            query: "old",
            kind: "all",
            guild: "all",
            channel: "all",
            category: "all",
            fromMs: null,
            toMs: null,
        }).length,
        1,
    );
    assert.equal(paginateGalleryItems(items, 1, 1).length, 1);
    assert.equal(sortGalleryItems(items, "oldest")[0].id, "1");
});

test("grouping supports server/channel/date/kind/category", () => {
    const items = [item({ id: "1", guildName: "A", categories: ["clips"] }), item({ id: "2", guildName: "B" })];
    assert.equal(groupGalleryItems(items, "guild").length, 2);
    assert.equal(groupGalleryItems(items, "category").length, 2);
    assert.equal(groupGalleryItems(items, "none")[0].items.length, 2);
});

test("galleryThemeFromBackground follows the resolved Discord background", () => {
    // Stock Discord dark/light plus custom-theme shapes.
    assert.equal(galleryThemeFromBackground("#313338"), "dark");
    assert.equal(galleryThemeFromBackground("#ffffff"), "light");
    assert.equal(galleryThemeFromBackground("#FFF"), "light");
    assert.equal(galleryThemeFromBackground("rgb(49, 51, 56)"), "dark");
    assert.equal(galleryThemeFromBackground("rgba(255,255,255,0.9)"), "light");
    assert.equal(galleryThemeFromBackground("  #282B30  "), "dark");
    // Missing or unparseable values fall back to null (caller uses the OS).
    assert.equal(galleryThemeFromBackground(""), null);
    assert.equal(galleryThemeFromBackground("transparent"), null);
    assert.equal(galleryThemeFromBackground("var(--x)"), null);
    assert.equal(galleryThemeFromBackground("not a color"), null);
});

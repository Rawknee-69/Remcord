import assert from "node:assert/strict";
import { test } from "node:test";
import { cssColorToHex, pickOverlayColor } from "../src/common/overlayColors.js";

test("cssColorToHex normalizes theme colors and rejects transparency", () => {
    assert.equal(cssColorToHex("#121214"), "#121214");
    assert.equal(cssColorToHex("#ABC"), "#aabbcc");
    assert.equal(cssColorToHex("rgb(18, 18, 20)"), "#121214");
    assert.equal(cssColorToHex("rgba(153, 170, 181, 1)"), "#99aab5");
    assert.equal(cssColorToHex("rgba(0, 0, 0, 0)"), null);
    assert.equal(cssColorToHex("rgba(0, 0, 0, 0.5)"), null);
    assert.equal(cssColorToHex("transparent"), null);
    assert.equal(cssColorToHex("red"), null);
});

test("pickOverlayColor takes the first opaque candidate", () => {
    assert.equal(pickOverlayColor(["rgba(0,0,0,0)", "rgb(30, 31, 34)"]), "#1e1f22");
    assert.equal(pickOverlayColor([null, undefined, "nope"]), "#121214");
});

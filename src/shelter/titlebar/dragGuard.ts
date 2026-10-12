/**
 * Native-shell top-band drag guard.
 *
 * The top-band drag listener turns empty pixels of the top ~64px into a
 * window caption. A mousedown there calls `preventDefault()` and starts a
 * native caption drag, whose OS move-loop swallows the mouse-up — so any
 * Discord control the guard fails to recognize LOSES its click. Discord's
 * guild rail (including the home/DM button) is built from `div`s with React
 * handlers (`role="treeitem"`, `data-list-item-id`), not `a`/`button`, so the
 * guard must recognize those plus a cursor backstop. Clicks always win over
 * dragging: unknown targets fail closed to "do not drag".
 */

/** Vertical extent of the draggable top band in CSS pixels. */
export const TOP_BAND_HEIGHT_PX = 64;

/**
 * Elements that must never start a caption drag. Covers the native controls,
 * standard interactive elements, and Discord's div-based navigation
 * (`treeitem` guild rail incl. the home/DM button, tabs, list containers)
 * plus embedded media/metadata elements whose clicks belong to the page.
 */
export const TOP_BAND_CLICKABLE_SELECTOR = [
    "#minimize",
    "#maximize",
    "#quit",
    "#remcordNavControls",
    "a",
    "button",
    "[role='button']",
    "[role='treeitem']",
    "[role='tree']",
    "[role='tab']",
    "[role='menuitem']",
    "[role='option']",
    "[role='listbox']",
    "[data-list-item-id]",
    "li",
    "input",
    "textarea",
    "select",
    "[role='textbox']",
    "[contenteditable='true']",
    "svg",
    "img",
    "video",
    "canvas",
    "iframe",
].join(",");

export type ClosestTarget = {
    closest(selectors: string): unknown;
};

/**
 * Pure drag decision: `true` means "hands off, this mousedown may be a
 * Discord interaction". Fails closed (returns `true`) for null targets,
 * missing `closest`, and matcher errors. A `pointer` cursor is Discord's own
 * "this is clickable" signal and backstops markup the selector does not name.
 */
export function shouldSkipTopBandDrag(target: ClosestTarget | null, cursor: string): boolean {
    if (!target || typeof target.closest !== "function") return true;
    try {
        if (target.closest(TOP_BAND_CLICKABLE_SELECTOR)) return true;
    } catch {
        return true;
    }
    return cursor === "pointer";
}

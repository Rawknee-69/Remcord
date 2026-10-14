/** Pure helpers for syncing Electron's native titlebar-overlay buttons with the Discord theme. */

const HEX6 = /^#[0-9a-fA-F]{6}$/;
const HEX3 = /^#[0-9a-fA-F]{3}$/;

/** Normalize `rgb()/rgba()/#rgb/#rrggbb` to lowercase `#rrggbb`. `null` when transparent or unparsable. */
export function cssColorToHex(input: string): string | null {
    const v = input.trim().toLowerCase();
    if (HEX6.test(v)) return v;
    if (HEX3.test(v)) return `#${v[1]}${v[1]}${v[2]}${v[2]}${v[3]}${v[3]}`;
    const m = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)(?:\s*,\s*([\d.]+))?\s*\)$/.exec(v);
    if (!m) return null;
    const alpha = m[4] === undefined ? 1 : Number(m[4]);
    if (!Number.isFinite(alpha) || alpha < 1) return null;
    const parts = [m[1], m[2], m[3]].map((n) => {
        const c = Math.max(0, Math.min(255, Math.round(Number(n))));
        return Number.isFinite(c) ? c.toString(16).padStart(2, "0") : null;
    });
    if (parts.some((p) => p === null)) return null;
    return `#${(parts as string[]).join("")}`;
}

/** True when `color` is a usable opaque `#rrggbb` overlay color. */
export function isUsableOverlayColor(color: string): boolean {
    return /^#[0-9a-f]{6}$/.test(color);
}

/** First opaque candidate wins; falls back to Remcord's historic defaults. */
export function pickOverlayColor(candidates: Array<string | null | undefined>): string {
    for (const c of candidates) {
        if (typeof c !== "string") continue;
        const hex = cssColorToHex(c);
        if (hex && isUsableOverlayColor(hex)) return hex;
    }
    return "#121214";
}

export function pickOverlaySymbolColor(candidates: Array<string | null | undefined>): string {
    for (const c of candidates) {
        if (typeof c !== "string") continue;
        const hex = cssColorToHex(c);
        if (hex && isUsableOverlayColor(hex)) return hex;
    }
    return "#99aab5";
}

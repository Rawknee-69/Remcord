/**
 * Custom JS/CSS bundle URL handling.
 *
 * Defaults are intentionally empty (unset): Remcord must never silently
 * request, download, or execute a bundle from another product's domain.
 * Users may supply their own URLs through the existing settings interface;
 * those values keep the previous validation contract (http/https only) and
 * the existing permission/integrity/error handling at the call sites.
 */

/** Unset default: no bundle URL is configured out of the box. */
export const DEFAULT_CUSTOM_JS_BUNDLE = "";

/** Unset default: no bundle URL is configured out of the box. */
export const DEFAULT_CUSTOM_CSS_BUNDLE = "";

/** Reserved, clearly non-resolving example value for UI placeholders. */
export const CUSTOM_BUNDLE_PLACEHOLDER = "https://example.invalid/custom-bundle.js";

/** True when no custom bundle URL has been configured by the user. */
export function isCustomBundleUnset(value: unknown): boolean {
    return typeof value !== "string" || value.trim().length === 0;
}

export type CustomBundleValidation = { ok: true } | { ok: false; reason: string };

/**
 * Validate a user-supplied custom bundle URL without fetching it.
 * Accepts only http(s) URLs; rejects empty input, malformed input, and
 * non-http(s) schemes. No network access is performed.
 */
export function validateCustomBundleUrl(value: unknown): CustomBundleValidation {
    if (typeof value !== "string" || value.trim().length === 0) {
        return { ok: false, reason: "empty" };
    }
    let parsed: URL;
    try {
        parsed = new URL(value.trim());
    } catch {
        return { ok: false, reason: "malformed" };
    }
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
        return { ok: false, reason: "unsupported-scheme" };
    }
    return { ok: true };
}

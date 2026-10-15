import { isNativeWebView } from "./settings.js";

/** GitHub username of the Remcord maintainer shown on the About page. */
export const MAINTAINER_GITHUB_USERNAME = "rawknee-69";

/** Discord snowflake of the maintainer. The friend button opens this profile. */
export const MAINTAINER_DISCORD_USER_ID = "778460273073848342";

export const MAINTAINER_GITHUB_URL = `https://github.com/${MAINTAINER_GITHUB_USERNAME}`;
export const MAINTAINER_SPONSOR_URL = `https://github.com/sponsors/${MAINTAINER_GITHUB_USERNAME}`;
export const MAINTAINER_AVATAR_URL = `${MAINTAINER_GITHUB_URL}.png`;
/** Opens the maintainer's profile in Discord, where visitors can send a friend request. */
export const MAINTAINER_FRIEND_URL = `https://discord.com/users/${MAINTAINER_DISCORD_USER_ID}`;

export const DISCORD_SERVER_INVITE = "https://discord.gg/JatCnhKufc";

/**
 * Opens an external link. `window.open` is handled by both shells: Electron
 * delegates to the OS browser and the Rust shell fast-paths https URLs to
 * `ExternalOpen` (`native/src/new_window.rs`), so no bridge call is needed.
 */
export function openExternal(url: string): void {
    window.open(url, "_blank");
}

/**
 * App version label. Reads the synchronous Electron value when present and
 * falls back to a static label natively (the Rust shell intentionally does
 * not shim `version`; only the async HeroUpdater capability carries it).
 * Never throws, so the page renders identically in both shells.
 */
export function getAppVersionLabel(): string {
    try {
        const version = (window as unknown as { remcord?: { version?: unknown } }).remcord?.version;
        if (typeof version === "string" && version.length > 0 && version !== "0.0.0") return `v${version}`;
    } catch {
        // Fall through to the dev label below.
    }
    return "Dev Build";
}

/** Human-readable runtime badge: native Rust shell vs Electron. */
export function getRuntimeLabel(): string {
    try {
        return isNativeWebView() ? "Native · Rust + WebView2" : "Electron";
    } catch {
        return "Electron";
    }
}

/**
 * Copies text via the renderer clipboard (no host surface needed).
 * Resolves true on success, false otherwise — callers toast accordingly.
 */
export async function copyAboutText(text: string): Promise<boolean> {
    try {
        await navigator.clipboard.writeText(text);
        return true;
    } catch {
        return false;
    }
}

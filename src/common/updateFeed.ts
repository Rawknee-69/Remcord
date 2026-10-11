/**
 * Remcord update-feed configuration.
 *
 * No Remcord release repository or hosted update feed exists yet, so there is
 * deliberately no default endpoint. Every consumer must treat a `null` feed
 * as "updates are not configured for this build": show the neutral message,
 * perform no network request, and never fall back to another product's feed.
 *
 * When a real Remcord feed is provisioned, configuring it is a deliberate
 * release task: set `UPDATE_FEED_URL` / `DOWNLOAD_PAGE_URL` to the verified
 * Remcord-owned endpoints in this module only.
 */

/** Remote `latest.json` feed. `null` = not configured (no request allowed). */
export const UPDATE_FEED_URL: string | null = null;

/** Download page opened by the update UI. `null` = not configured. */
export const DOWNLOAD_PAGE_URL: string | null = null;

/** True only when a real Remcord feed has been deliberately configured. */
export function isUpdateFeedConfigured(): boolean {
    return UPDATE_FEED_URL !== null && UPDATE_FEED_URL.length > 0;
}

export type UpdateCheckOutcome = { status: "unconfigured" } | { status: "update-available" } | { status: "up-to-date" };

/**
 * Compare a local version against already-fetched remote feed data.
 * Pure string comparison kept identical to the previous checker semantics
 * (dots stripped, lexicographic compare). Performs no I/O itself.
 */
export function compareVersions(localVersion: string, remoteVersion: string): UpdateCheckOutcome {
    const local = localVersion.replace(/\./g, "");
    const remote = remoteVersion.replace(/\./g, "");
    return remote > local ? { status: "update-available" } : { status: "up-to-date" };
}

export interface UpdateFeedFetcher {
    fetchJson: (url: string) => Promise<{ version: string }>;
}

/**
 * Check for updates against the configured feed.
 *
 * Fail-closed: when no feed is configured this returns `{ status:
 * "unconfigured" }` WITHOUT touching `fetcher` (no network request), so
 * callers can prove the unconfigured build never polls any endpoint.
 */
export async function checkForUpdates(localVersion: string, fetcher?: UpdateFeedFetcher): Promise<UpdateCheckOutcome> {
    const feedUrl = UPDATE_FEED_URL;
    if (!isUpdateFeedConfigured() || feedUrl === null) {
        return { status: "unconfigured" };
    }
    if (!fetcher) {
        return { status: "unconfigured" };
    }
    const data = await fetcher.fetchJson(feedUrl);
    return compareVersions(localVersion, data.version);
}

/** Neutral user-facing message for builds without a configured feed. */
export const UPDATE_UNCONFIGURED_MESSAGE =
    "Updates are not configured for this build. No update feed is available yet, so no check was performed.";

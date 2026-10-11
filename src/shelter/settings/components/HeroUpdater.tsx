import { createSignal } from "solid-js";
import classes from "./HeroUpdater.module.css";

const {
    ui: { Button, ButtonSizes, Header, HeaderTags, Text, ButtonColors },
} = shelter;

/**
 * Update-feed configuration.
 *
 * No Remcord release repository or hosted update feed exists yet, so there
 * is deliberately no endpoint here. The checker below stays fail-closed:
 * with no feed configured it reports "unconfigured" without issuing any
 * network request and never falls back to another product's feed.
 * Configuring a real Remcord feed later is a deliberate release task.
 */
export const UPDATE_FEED_URL: string | null = null;
export const DOWNLOAD_PAGE_URL: string | null = null;

export const UPDATE_UNCONFIGURED_MESSAGE =
    "Updates are not configured for this build. No update feed is available yet, so no check was performed.";

export function isUpdateFeedConfigured(): boolean {
    return UPDATE_FEED_URL !== null && UPDATE_FEED_URL.length > 0;
}

async function getLocalVersion(): Promise<string> {
    // Native WebView2 has no synchronous `window.remcord.version` read: use
    // the typed async `app.get_version` capability instead. Electron keeps
    // the original synchronous read below.
    if ((window.remcord as unknown as { boot?: { runtime?: unknown } })?.boot?.runtime === "webview2") {
        const native = (window.remcord as unknown as { native: { getVersion: () => Promise<string> } }).native;
        return await native.getVersion();
    }
    return window.remcord.version;
}

export type HeroUpdateStatus = "unconfigured" | "update-available" | "up-to-date";

export async function checkForUpdates(): Promise<HeroUpdateStatus> {
    if (!isUpdateFeedConfigured() || UPDATE_FEED_URL === null) {
        return "unconfigured";
    }
    const response = await fetch(UPDATE_FEED_URL);
    const data = await response.json();
    const remoteVersion = (data.version as string).replace(/\./g, ""); // easy to compare
    if (remoteVersion > (await getLocalVersion()).replace(/\./g, "")) {
        return "update-available";
    }
    return "up-to-date";
}

export const HeroUpdater = () => {
    const [checking, setChecking] = createSignal(false);
    const [status, setStatus] = createSignal<HeroUpdateStatus | null>(null);

    const handleCheck = async () => {
        setChecking(true);
        setStatus(null);
        const result = await checkForUpdates();
        setStatus(result);
        setChecking(false);
    };

    const handleDownload = () => {
        if (DOWNLOAD_PAGE_URL !== null) {
            window.open(DOWNLOAD_PAGE_URL, "_blank");
        }
    };

    return (
        <div class={classes.hero}>
            <Header tag={HeaderTags.H3}>Update Checker</Header>
            <Text>Check if a new version is available and download updates easily.</Text>
            {status() === null && (
                <Button onClick={handleCheck} class={classes.checkButton} disabled={checking()}>
                    Check for Updates
                </Button>
            )}

            {status() === "unconfigured" && <Text>{UPDATE_UNCONFIGURED_MESSAGE}</Text>}
            {status() === "update-available" && (
                <>
                    <Text>Update available!</Text>
                    <Button size={ButtonSizes.XLARGE} color={ButtonColors.RED} onClick={handleDownload}>
                        Open Download Page
                    </Button>
                </>
            )}
            {status() === "up-to-date" && <Text>Your app is up to date.</Text>}
        </div>
    );
};

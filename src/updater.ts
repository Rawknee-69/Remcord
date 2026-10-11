import { platform } from "node:os";
import type { AllPublishOptions } from "builder-util-runtime";
import { AppImageUpdater, DebUpdater, MacUpdater } from "electron-updater";

/**
 * Store updater provider configuration.
 *
 * No Remcord repository or update feed exists yet, so this is deliberately
 * unconfigured: `getUpdateProvider()` returns `null` and every platform path
 * below logs a neutral message instead of polling any endpoint.
 *
 * Platform coverage of this module:
 * - macOS (`darwin`): `MacUpdater` (GitHub provider when configured).
 * - Linux: `AppImageUpdater` + `DebUpdater` (GitHub provider when
 *   configured).
 * - Windows: NOT covered here — Windows ships NSIS packages with manual
 *   upgrade and has no auto-updater in this codebase. Nothing below claims
 *   otherwise.
 *
 * When a real Remcord distribution endpoint is provisioned, configuring it
 * is a deliberate release task: return the verified provider here. Until
 * then the feature stays explicitly unavailable (fail-closed).
 */
export function getUpdateProvider(): AllPublishOptions | null {
    return null;
}

/** True only when a real Remcord distribution endpoint is configured. */
export function isUpdateProviderConfigured(): boolean {
    return getUpdateProvider() !== null;
}

export const UPDATER_UNCONFIGURED_MESSAGE =
    "Automatic updates are not configured for this build. No Remcord update feed exists yet.";

function checkForUpdatesIfConfigured(): void {
    const provider = getUpdateProvider();
    if (provider === null) {
        console.log(`[Updater] ${UPDATER_UNCONFIGURED_MESSAGE}`);
        return;
    }

    if (platform() === "darwin") {
        const autoUpdater = new MacUpdater(provider);
        autoUpdater.checkForUpdatesAndNotify();
    }

    if (platform() === "linux") {
        const appimage = new AppImageUpdater(provider);
        const deb = new DebUpdater(provider);

        deb.on("update-downloaded", (_e) => {
            deb.quitAndInstall();
        });
        appimage.on("update-downloaded", (_e) => {
            appimage.quitAndInstall();
        });

        deb.checkForUpdatesAndNotify();
        appimage.checkForUpdatesAndNotify();
    }
}

checkForUpdatesIfConfigured();

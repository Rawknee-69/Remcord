import { createSignal, For, Show } from "solid-js";
import {
    commitRestoreNative,
    discardRestoreNative,
    inspectBackupNative,
    isNativeBackupAvailable,
    type NativeRestorePreview,
    normalizeNativeInspectResult,
    normalizeNativeRestoreResult,
    parseElectronRestoreResult,
    saveBackup,
} from "../settings.js";
import classes from "./BackupSection.module.css";

const {
    ui: {
        Button,
        Header,
        HeaderTags,
        ButtonSizes,
        Divider,
        openModal,
        openConfirmationModal,
        ModalRoot,
        ModalBody,
        ModalConfirmFooter,
        ModalSizes,
        ModalHeader,
        SwitchItem,
        showToast,
    },
    plugin: { store },
} = shelter;

type IncludeOptions = {
    remcordConfig: boolean;
    remcordThemesAndQuickCss: boolean;
    remcordExtensionPlugins: boolean;
    vencordModData: boolean;
    equicordModData: boolean;
    shelterModData: boolean;
    modBundles: boolean;
};

const defaultIncludes: IncludeOptions = {
    remcordConfig: true,
    remcordThemesAndQuickCss: true,
    remcordExtensionPlugins: true,
    vencordModData: true,
    equicordModData: true,
    shelterModData: true,
    modBundles: true,
};
function getLocalStoragePropertyDescriptor() {
    const iframe = document.createElement("iframe");
    document.head.append(iframe);
    const pd = Object.getOwnPropertyDescriptor(iframe.contentWindow, "localStorage");
    iframe.remove();
    return pd;
}

const pd = getLocalStoragePropertyDescriptor();
const localStorage = pd?.get?.call(window) ?? window.localStorage;

function BackupOptionsModal(props: { close: () => void; onConfirm: (includes: IncludeOptions) => void }) {
    const t = store.i18n;
    const [includes, setIncludes] = createSignal<IncludeOptions>({ ...defaultIncludes });

    function patch<K extends keyof IncludeOptions>(key: K, value: boolean) {
        setIncludes((prev) => ({ ...prev, [key]: value }));
    }

    function confirm() {
        props.onConfirm(includes());
        props.close();
    }

    const inc = includes;

    return (
        <ModalRoot size={ModalSizes.MEDIUM}>
            <ModalHeader close={props.close}>{t["backup-modalTitle"]}</ModalHeader>
            <ModalBody>
                <Header tag={HeaderTags.H5} class={classes.modalEyebrow}>
                    {t["splash-title"]}
                </Header>
                <SwitchItem hideBorder value={inc().remcordConfig} onChange={(v: boolean) => patch("remcordConfig", v)}>
                    {t["backup-includeRemcordConfig"]}
                </SwitchItem>
                <SwitchItem
                    hideBorder
                    value={inc().remcordThemesAndQuickCss}
                    onChange={(v: boolean) => patch("remcordThemesAndQuickCss", v)}
                >
                    {t["backup-includeRemcordThemes"]}
                </SwitchItem>
                <SwitchItem
                    hideBorder
                    value={inc().remcordExtensionPlugins}
                    onChange={(v: boolean) => patch("remcordExtensionPlugins", v)}
                >
                    {t["backup-includeRemcordPlugins"]}
                </SwitchItem>
                <Divider mt mb />
                <SwitchItem
                    hideBorder
                    value={inc().vencordModData}
                    onChange={(v: boolean) => patch("vencordModData", v)}
                >
                    {t["backup-includeVencord"]}
                </SwitchItem>
                <SwitchItem
                    hideBorder
                    value={inc().equicordModData}
                    onChange={(v: boolean) => patch("equicordModData", v)}
                >
                    {t["backup-includeEquicord"]}
                </SwitchItem>
                <SwitchItem
                    hideBorder
                    value={inc().shelterModData}
                    onChange={(v: boolean) => patch("shelterModData", v)}
                >
                    {t["backup-includeShelter"]}
                </SwitchItem>
                <SwitchItem hideBorder value={inc().modBundles} onChange={(v: boolean) => patch("modBundles", v)}>
                    {t["backup-includeModBundles"]}
                </SwitchItem>
            </ModalBody>
            <ModalConfirmFooter confirmText={t["backup-confirmBackup"]} onConfirm={confirm} close={props.close} />
        </ModalRoot>
    );
}

function formatPreviewBytes(bytes: number): string {
    if (!Number.isSafeInteger(bytes) || bytes < 0) return "0 B";
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Preview modal for one held staged restore: shows exactly what the commit
 * would change (sections, file count, sizes, mod payloads, entry listing)
 * before the user confirms. Confirming commits the held staged bytes;
 * closing or cancelling discards the hold without touching the profile.
 */
function BackupRestorePreviewModal(props: { close: () => void; preview: NativeRestorePreview; onConfirm: () => void }) {
    const t = store.i18n;
    const sectionLabels: Array<[keyof IncludeOptions, string]> = [
        ["remcordConfig", t["backup-includeRemcordConfig"]],
        ["remcordThemesAndQuickCss", t["backup-includeRemcordThemes"]],
        ["remcordExtensionPlugins", t["backup-includeRemcordPlugins"]],
        ["vencordModData", t["backup-includeVencord"]],
        ["equicordModData", t["backup-includeEquicord"]],
        ["shelterModData", t["backup-includeShelter"]],
        ["modBundles", t["backup-includeModBundles"]],
    ];
    const activeSections = () => sectionLabels.filter(([key]) => props.preview.includes[key]);
    const modPayloads = () =>
        [
            props.preview.hasVencordModData ? t["backup-includeVencord"] : null,
            props.preview.hasEquicordModData ? t["backup-includeEquicord"] : null,
            props.preview.hasShelterModData ? t["backup-includeShelter"] : null,
        ].filter((label): label is string => typeof label === "string" && label.length > 0);
    const shownEntries = () => props.preview.entries.slice(0, 100);
    const hiddenEntries = () => props.preview.entryCount - shownEntries().length;

    return (
        <ModalRoot size={ModalSizes.LARGE}>
            <ModalHeader close={props.close}>{t["backup-restoreConfirmHeader"]}</ModalHeader>
            <ModalBody>
                <Text tag={TextTags.textSM}>
                    {t["backup-restoreConfirmBody"]} {props.preview.entryCount} files (
                    {formatPreviewBytes(props.preview.uncompressedBytes)}).
                </Text>
                <Divider mt mb />
                <Header tag={HeaderTags.H5} class={classes.modalEyebrow}>
                    Sections
                </Header>
                <For each={activeSections()}>
                    {([, label]) => (
                        <div>
                            <Text tag={TextTags.textSM}>{label}</Text>
                        </div>
                    )}
                </For>
                <Show when={modPayloads().length > 0}>
                    <Divider mt mb />
                    <Header tag={HeaderTags.H5} class={classes.modalEyebrow}>
                        Mod settings
                    </Header>
                    <For each={modPayloads()}>
                        {(label) => (
                            <div>
                                <Text tag={TextTags.textSM}>{label}</Text>
                            </div>
                        )}
                    </For>
                </Show>
                <Divider mt mb />
                <Header tag={HeaderTags.H5} class={classes.modalEyebrow}>
                    Files
                </Header>
                <div style={{ "max-height": "220px", overflow: "auto" }}>
                    <For each={shownEntries()}>
                        {(entry) => (
                            <div>
                                <Text tag={TextTags.textSM}>{entry}</Text>
                            </div>
                        )}
                    </For>
                </div>
                <Show when={hiddenEntries() > 0}>
                    <Text tag={TextTags.textSM}>…and {hiddenEntries()} more.</Text>
                </Show>
            </ModalBody>
            <ModalConfirmFooter
                type="danger"
                confirmText={t["backup-restoreConfirm"]}
                cancelText={t["backup-restoreCancel"]}
                onConfirm={props.onConfirm}
                close={props.close}
            />
        </ModalRoot>
    );
}

function applyClientModsFromRestore(raw: unknown): void {
    if (!raw || typeof raw !== "object") return;
    const cm = raw as Record<string, unknown>;
    if (typeof cm.vencordLocalStorage === "string" && cm.vencordLocalStorage.length > 0) {
        localStorage.setItem("VencordSettings", cm.vencordLocalStorage);
    }
    if (typeof cm.equicordLocalStorage === "string" && cm.equicordLocalStorage.length > 0) {
        localStorage.setItem("EquicordSettings", cm.equicordLocalStorage);
    }
    if (cm.shelter && typeof cm.shelter === "object") {
        const shelterData = cm.shelter as { plugins?: unknown; enabledPlugins?: unknown };
        console.log("Restoring shelter plugins", shelterData);
        if (shelterData.plugins && typeof shelterData.plugins === "object") {
            for (const [id, data] of Object.entries(
                shelterData.plugins as Record<string, { src: string; update: boolean }>,
            )) {
                shelter.plugins.addRemotePlugin(id, data.src, data.update);
            }
        }
    }
}

export function BackupSection(props: { embedded?: boolean }) {
    const t = store.i18n;
    // Exactly one backup/restore operation at a time: guards double-click
    // duplicate dialogs and overlapping host operations. Released on every
    // completion, cancellation, and failure path below.
    const [busy, setBusy] = createSignal(false);

    function buildClientMods(includes: IncludeOptions) {
        const vencordSettings = localStorage.getItem("VencordSettings");
        const equicordSettings = localStorage.getItem("EquicordSettings");
        const shelterPlugins = shelter.plugins.installedPlugins();
        const shelterEnabledPlugins = shelter.plugins.loadedPlugins();

        return {
            vencordLocalStorage: includes.vencordModData ? vencordSettings : undefined,
            equicordLocalStorage: includes.equicordModData ? equicordSettings : undefined,
            shelter: includes.shelterModData
                ? { plugins: shelterPlugins, enabledPlugins: shelterEnabledPlugins }
                : undefined,
        };
    }

    async function runBackup(includes: IncludeOptions) {
        if (busy()) return;
        setBusy(true);
        try {
            const payload = {
                includes,
                clientMods: buildClientMods(includes),
            };
            // One call site for both shells: the native facade preserves the
            // Electron save signature and result shape exactly, so the
            // payload construction and toast behavior below are identical in
            // both. clientMods stay renderer-built; the output path is never
            // exposed.
            const result = await saveBackup(JSON.stringify(payload));
            if (result.ok) {
                showToast({
                    title: t["backup-successTitle"],
                    content: t["backup-successBody"],
                    duration: 4000,
                });
            } else if (result.error === "CANCELLED") {
                showToast({
                    title: t["backup-cancelledTitle"],
                    content: t["backup-cancelledBody"],
                    duration: 3000,
                });
            } else {
                showToast({
                    title: t["backup-failedTitle"],
                    content: result.error,
                    duration: 5000,
                });
            }
        } catch {
            showToast({
                title: t["backup-failedTitle"],
                content: t["backup-unknownError"],
                duration: 5000,
            });
        } finally {
            setBusy(false);
        }
    }

    function openBackupModal() {
        if (busy()) return;
        openModal(({ close }: { close: () => void }) => (
            <BackupOptionsModal
                close={close}
                onConfirm={(includes) => {
                    void runBackup(includes);
                }}
            />
        ));
    }

    async function commitHeldRestore() {
        // Exactly one path per commit. Native: the host commits the held
        // staged bytes and returns the bounded clientMods OBJECT (never
        // applied host-side). Electron: the existing JSON-string path.
        // Both converge on the shared outcome below; the renderer only
        // feeds clientMods into the existing application path, which it
        // already owns.
        const outcome = isNativeBackupAvailable()
            ? normalizeNativeRestoreResult(await commitRestoreNative())
            : parseElectronRestoreResult(await window.remcord.backup.restore());
        if (!outcome.ok) {
            if (outcome.error === "CANCELLED") {
                showToast({
                    title: t["backup-cancelledTitle"],
                    content: t["backup-cancelledBody"],
                    duration: 3000,
                });
                return;
            }
            showToast({
                title: t["backup-failedTitle"],
                content:
                    outcome.error === "INVALID"
                        ? t["backup-invalidFile"]
                        : outcome.error === "UNKNOWN"
                          ? t["backup-unknownError"]
                          : outcome.error,
                duration: 5000,
            });
            return;
        }
        applyClientModsFromRestore(outcome.clientMods);
        showToast({
            title: t["backup-restoreDoneTitle"],
            content: t["backup-restoreDoneBody"],
            duration: 6000,
        });
    }

    async function inspectAndPreview(releaseBusy: () => void): Promise<boolean> {
        // Native preview-then-commit: inspect stages and holds the archive
        // host-side and returns the preview; the preview modal confirms.
        // Returns true when the preview modal opened (it owns releaseBusy
        // from there: commit releases it on completion, and a preview
        // dismissal releases it after discarding the hold); every other
        // path releases before returning false. Confirming commits the
        // held bytes; anything else discards the hold without touching the
        // profile.
        let inspected: unknown;
        try {
            inspected = await inspectBackupNative();
        } catch {
            showToast({
                title: t["backup-failedTitle"],
                content: t["backup-unknownError"],
                duration: 5000,
            });
            releaseBusy();
            return false;
        }
        const outcome = normalizeNativeInspectResult(inspected);
        if ("canceled" in outcome && outcome.canceled === true) {
            showToast({
                title: t["backup-cancelledTitle"],
                content: t["backup-cancelledBody"],
                duration: 3000,
            });
            releaseBusy();
            return false;
        }
        if ("error" in outcome) {
            showToast({
                title: t["backup-failedTitle"],
                content: outcome.error,
                duration: 5000,
            });
            releaseBusy();
            return false;
        }
        if (!("preview" in outcome) || !outcome.preview) {
            showToast({
                title: t["backup-failedTitle"],
                content: t["backup-invalidFile"],
                duration: 5000,
            });
            releaseBusy();
            return false;
        }
        const preview = outcome.preview;
        openModal(({ close }: { close: () => void }) => (
            <BackupRestorePreviewModal
                close={() => {
                    // Closing the preview without confirming discards the
                    // hold: nothing was applied, nothing is kept.
                    void discardRestoreNative().finally(() => releaseBusy());
                    close();
                }}
                preview={preview}
                onConfirm={() => {
                    close();
                    void commitHeldRestore().finally(() => releaseBusy());
                }}
            />
        ));
        return true;
    }

    function restoreFromBackup() {
        if (busy()) return;
        setBusy(true);
        const releaseBusy = () => setBusy(false);
        // The first confirm is the generic destructive-action warning (same
        // in both shells). Natively the picker + preview follow; the commit
        // happens only from the preview modal, which owns busy() until it
        // settles.
        openConfirmationModal({
            header: () => t["backup-restoreConfirmHeader"],
            body: () => t["backup-restoreConfirmBody"],
            type: "danger",
            confirmText: t["backup-restoreConfirm"],
            cancelText: t["backup-restoreCancel"],
        }).then(
            async () => {
                let modalOwnsBusy = false;
                try {
                    if (isNativeBackupAvailable()) {
                        modalOwnsBusy = await inspectAndPreview(releaseBusy);
                        return;
                    }
                    const outcome = parseElectronRestoreResult(await window.remcord.backup.restore());
                    if (!outcome.ok) {
                        if (outcome.error === "CANCELLED") {
                            showToast({
                                title: t["backup-cancelledTitle"],
                                content: t["backup-cancelledBody"],
                                duration: 3000,
                            });
                            return;
                        }
                        showToast({
                            title: t["backup-failedTitle"],
                            content:
                                outcome.error === "INVALID"
                                    ? t["backup-invalidFile"]
                                    : outcome.error === "UNKNOWN"
                                      ? t["backup-unknownError"]
                                      : outcome.error,
                            duration: 5000,
                        });
                        return;
                    }
                    applyClientModsFromRestore(outcome.clientMods);
                    showToast({
                        title: t["backup-restoreDoneTitle"],
                        content: t["backup-restoreDoneBody"],
                        duration: 6000,
                    });
                } catch {
                    showToast({
                        title: t["backup-failedTitle"],
                        content: t["backup-unknownError"],
                        duration: 5000,
                    });
                } finally {
                    if (!modalOwnsBusy) setBusy(false);
                }
            },
            () => {
                setBusy(false);
            },
        );
    }

    return (
        <div class={props.embedded ? classes.embedded : classes.item}>
            <Show when={!props.embedded}>
                <Header class={classes.title} tag={HeaderTags.H3}>
                    {t["backup-pageTitle"]}
                </Header>
                <div class={classes.note}>{t["backup-pageSubtitle"]}</div>
            </Show>
            <div class={classes.actions}>
                <Button onClick={openBackupModal} size={ButtonSizes.LARGE} disabled={busy()}>
                    {t["backup-createBackup"]}
                </Button>
                <Button onClick={restoreFromBackup} size={ButtonSizes.LARGE} type="danger" disabled={busy()}>
                    {t["backup-restore"]}
                </Button>
            </div>
            <Show when={!props.embedded}>
                <Divider />
            </Show>
        </div>
    );
}

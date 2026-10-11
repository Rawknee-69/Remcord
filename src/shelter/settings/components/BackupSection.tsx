import { createSignal, Show } from "solid-js";
import {
    isNativeBackupAvailable,
    normalizeNativeRestoreResult,
    parseElectronRestoreResult,
    restoreBackupNative,
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

    function restoreFromBackup() {
        if (busy()) return;
        setBusy(true);
        openConfirmationModal({
            header: () => t["backup-restoreConfirmHeader"],
            body: () => t["backup-restoreConfirmBody"],
            type: "danger",
            confirmText: t["backup-restoreConfirm"],
            cancelText: t["backup-restoreCancel"],
        }).then(
            async () => {
                try {
                    // Exactly one path per restore. Native: the host owns the
                    // picker, validation, staging, commit, and rollback, and
                    // returns the bounded clientMods OBJECT (never applied
                    // host-side). Electron: the existing JSON-string path.
                    // Both converge on the shared outcome below; the renderer
                    // only feeds clientMods into the existing application
                    // path, which it already owns.
                    const outcome = isNativeBackupAvailable()
                        ? normalizeNativeRestoreResult(await restoreBackupNative())
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
                } catch {
                    showToast({
                        title: t["backup-failedTitle"],
                        content: t["backup-unknownError"],
                        duration: 5000,
                    });
                } finally {
                    setBusy(false);
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

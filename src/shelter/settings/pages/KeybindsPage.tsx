import { For, Show } from "solid-js";
import type { Keybind } from "../../../@types/keybind.js";
import { EmptyState } from "../components/EmptyState.jsx";
import { KeybindCard } from "../components/KeybindCard.jsx";
import { KeybindMaker } from "../components/KeybindMaker.jsx";
import { SettingsPageHeader } from "../components/SettingsPageHeader.jsx";
import { isNativeWebView } from "../settings.js";
import classes from "./KeybindsPage.module.css";

const {
    plugin: { store },
    ui: { Button, ButtonSizes, openModal, Text, TextTags },
} = shelter;

export function KeybindsPage() {
    function addNewKeybind() {
        // The native shell exposes no keybind capability; the Add action is
        // disabled natively, and this guards the empty-state action too.
        if (isNativeWebView()) return;
        openModal(({ close }: { close: () => void }) => <KeybindMaker close={close} />);
    }

    const t = store.i18n;
    const keybinds = () => (store.settings.keybinds as Keybind[]) ?? [];

    return (
        <>
            <SettingsPageHeader title={t["keybinds-pageTitle"]} description={t["keybinds-pageDesc"]}>
                <Text tag={TextTags.textSM} class={classes.tip}>
                    {t["keybind-globalNote"]}
                </Text>
            </SettingsPageHeader>
            <div class={classes.toolbar}>
                <Button
                    size={ButtonSizes.LARGE}
                    disabled={isNativeWebView()}
                    title={isNativeWebView() ? "Keybinds are not available in the native Remcord app" : undefined}
                    onClick={addNewKeybind}
                >
                    {t["keybind-addKeybind"]}
                </Button>
            </div>
            <Show when={isNativeWebView()}>
                <div
                    style={{
                        background: "rgba(255, 255, 210, 0.85)",
                        border: "1px solid #e6c200",
                        color: "#2d2100",
                        padding: "12px 16px",
                    }}
                >
                    Keybinds are not available in the native Remcord app yet. Existing keybinds are listed below but
                    cannot be changed here.
                </div>
            </Show>
            <Show when={window.remcord.platform === "linux"}>
                <div
                    style={{
                        background: "rgba(255, 255, 210, 0.85)",
                        border: "1px solid #e6c200",
                        color: "#2d2100",
                        padding: "12px 16px",
                    }}
                >
                    Adding keybinds on Linux is recommended to be done through the system settings instead of Remcord.
                    This is because some keybinds may not work properly due to how Linux handles global shortcuts. If
                    you want to add a keybind through Remcord, please make sure to test it first.
                </div>
            </Show>
            <Show
                when={keybinds().length > 0}
                fallback={
                    <EmptyState
                        message={t["keybinds-empty"]}
                        description={t["keybinds-emptyDesc"]}
                        actionLabel={t["keybind-addKeybind"]}
                        onAction={addNewKeybind}
                    />
                }
            >
                <For each={keybinds()}>
                    {(keybind: Keybind) => <KeybindCard keybind={keybind} disabled={isNativeWebView()} />}
                </For>
            </Show>
        </>
    );
}

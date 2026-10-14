import { createSignal } from "solid-js";
import type { Keybind } from "../../../@types/keybind.js";
import { isNativeKeybindAvailable, refreshSettings, removeKeybindNative, toggleKeybindNative } from "../settings.js";
import classes from "./KeybindCard.module.css";

const {
    ui: { IconBin, Header, Switch, HeaderTags },
    plugin: { store },
} = shelter;

export const KeybindCard = (props: { keybind: Keybind; disabled?: boolean }) => {
    const [switchState, setSwitchState] = createSignal(props.keybind.enabled);

    function toggleKeybind(state: boolean) {
        if (props.disabled) return;
        // Native WebView2 flips through the async capability with an
        // optimistic store update (reverted when the host refuses);
        // Electron keeps the original fire-and-forget send.
        if (isNativeKeybindAvailable()) {
            const keybinds = ((store.settings.keybinds as Keybind[] | undefined) ?? []).slice();
            const entry = keybinds.find((keybind) => keybind.id === props.keybind.id);
            const previous = entry?.enabled ?? props.keybind.enabled;
            if (entry) entry.enabled = state;
            store.settings.keybinds = keybinds;
            setSwitchState(state);
            void toggleKeybindNative(props.keybind.id).then(
                (result) => {
                    const confirmed = result.ok && typeof result.enabled === "boolean" ? result.enabled : previous;
                    if (entry) entry.enabled = confirmed;
                    setSwitchState(confirmed);
                },
                () => {
                    if (entry) entry.enabled = previous;
                    setSwitchState(previous);
                },
            );
            return;
        }
        setSwitchState(state);
        window.remcord.settings.toggleKeybind(props.keybind.id);
        refreshSettings();
        console.log(store.settings.keybinds);
    }
    function removeKeybind() {
        if (props.disabled) return;
        // Native WebView2 removes through the async capability with an
        // optimistic store update (restored when the host refuses);
        // Electron keeps the original fire-and-forget send.
        if (isNativeKeybindAvailable()) {
            const keybinds = ((store.settings.keybinds as Keybind[] | undefined) ?? []).slice();
            const at = keybinds.findIndex((keybind) => keybind.id === props.keybind.id);
            if (at === -1) return;
            const [removed] = keybinds.splice(at, 1);
            store.settings.keybinds = keybinds;
            void removeKeybindNative(props.keybind.id).then(
                (result) => {
                    if (!result.ok) {
                        store.settings.keybinds = [...((store.settings.keybinds as Keybind[]) ?? []), removed];
                    }
                },
                () => {
                    store.settings.keybinds = [...((store.settings.keybinds as Keybind[]) ?? []), removed];
                },
            );
            return;
        }
        window.remcord.settings.removeKeybind(props.keybind.id);
        refreshSettings();
        console.log(store.settings.keybinds);
    }
    return (
        <div class={classes.card} style={props.disabled ? { opacity: "0.55" } : undefined}>
            <div class={classes.info}>
                <Header tag={HeaderTags.H2}>
                    {store.i18n[`keybind-${props.keybind.action}`] ?? props.keybind.action}
                </Header>
                <Header class={classes.eyebrow} tag={HeaderTags.EYEBROW}>
                    {props.keybind.accelerator}
                </Header>
            </div>
            <div class={classes.btnContainer}>
                <button
                    title={
                        (store.i18n["keybind-delete"] as string | undefined) ??
                        (props.disabled ? "Keybinds are not available in the native Remcord app" : undefined)
                    }
                    type="button"
                    disabled={props.disabled}
                    onClick={removeKeybind}
                    class={classes.btn}
                >
                    <IconBin />
                </button>
            </div>
            <div class={classes.switch}>
                <Switch checked={switchState()} disabled={props.disabled} onChange={toggleKeybind} />
            </div>
        </div>
    );
};

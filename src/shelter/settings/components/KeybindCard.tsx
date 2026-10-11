import { createSignal } from "solid-js";
import type { Keybind } from "../../../@types/keybind.js";
import { refreshSettings } from "../settings.js";
import classes from "./KeybindCard.module.css";

const {
    ui: { IconBin, Header, Switch, HeaderTags },
    plugin: { store },
} = shelter;

export const KeybindCard = (props: { keybind: Keybind; disabled?: boolean }) => {
    const [switchState, setSwitchState] = createSignal(props.keybind.enabled);

    function toggleKeybind(state: boolean) {
        // The native shell exposes no keybind capability: never call into
        // the missing `window.remcord.settings.*keybind` members.
        if (props.disabled) return;
        setSwitchState(state);
        window.remcord.settings.toggleKeybind(props.keybind.id);
        refreshSettings();
        console.log(store.settings.keybinds);
    }
    function removeKeybind() {
        if (props.disabled) return;
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

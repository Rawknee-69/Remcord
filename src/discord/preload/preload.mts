import type { RemcordWindow } from "../../@types/remcordWindow.js";
import "./bridge.js";
import "./mods/shelter.mjs";
import "./mods/vencord.mjs";
import "./mods/equicord.mjs";
import "./mods/custom.mjs";
import "./plugins.mjs";
import "./patches.mjs";
import "./newTitlebar.mjs";
import "./titlebar.mjs";
import "./themes.js";
import "./inviteBackButton.mjs";

console.log("Remcord");
window.localStorage.setItem("hideNag", "true");
declare global {
    interface Window {
        remcord: RemcordWindow;
    }
}

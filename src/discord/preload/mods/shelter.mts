const { ipcRenderer, webFrame } = require("electron");

import type { ModBundle } from "../../../@types/ModBundle.js";

const requiredPlugins: Record<string, [string, { isVisible: boolean; allowedActions: Record<string, true> }]> = {
    "remcord-arrpc": ["remcord://plugins/rpc/", { isVisible: false, allowedActions: {} }],
    "remcord-settings": ["remcord://plugins/settings/", { isVisible: false, allowedActions: {} }],
    "remcord-power": ["remcord://plugins/power/", { isVisible: false, allowedActions: {} }],
    "remcord-screenshare": ["remcord://plugins/screenshare/", { isVisible: false, allowedActions: {} }],
    "remcord-titlebar": ["remcord://plugins/titlebar/", { isVisible: false, allowedActions: {} }],
};
if (process.platform === "darwin") {
    requiredPlugins["remcord-touchbar"] = [
        "remcord://plugins/touchbar/",
        { isVisible: true, allowedActions: { toggle: true } },
    ];
}
async function inject() {
    try {
        await ipcRenderer.invoke("getShelterBundle").then(async (bundle: ModBundle) => {
            if (bundle?.enabled) {
                await webFrame.executeJavaScript(`(()=>{
                const SHELTER_INJECTOR_PLUGINS = ${JSON.stringify(requiredPlugins)};
                ${bundle.js}
            })()`);
            }
        });
    } catch (e) {
        console.error(e);
    }
}
inject();

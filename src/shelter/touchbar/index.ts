/// <reference path="../../../node_modules/@uwu/shelter-defs/dist/shelter-defs/rootdefs.d.ts" />

const {
    util: { log },
    flux: { dispatcher, storesFlat },
} = shelter;

function updateVoiceState() {
    const { mute, deaf } = storesFlat.MediaEngineStore.getSettings();
    log(`[Touchbar] Setting voice state: mute: ${mute}, deaf: ${deaf}`);
    window.remcord.touchbar.setVoiceState(mute, deaf);
}

function track(payload: { event: string; properties: { enabled: string } }) {
    if (payload.event === "join_voice_channel") {
        window.remcord.touchbar.setVoiceTouchbar(true);
    } else if (payload.event === "leave_voice_channel") {
        window.remcord.touchbar.setVoiceTouchbar(false);
    }
}

function getAllGuilds() {
    const guilds = storesFlat.GuildStore.getGuildIds();
    const array = [];
    for (let index = 0; index < guilds.length; ++index) {
        const guildID = guilds[index];
        array.push(`${guildID}/${storesFlat.GuildStore.getGuild(guildID).icon}`);
    }
    return array;
}

let guildImportTimeout: ReturnType<typeof setTimeout> | undefined;

export function onLoad() {
    if (window.remcord.platform === "darwin") {
        log("Remcord Touchbar Integration");
        updateVoiceState();
        dispatcher.subscribe("TRACK", track);
        dispatcher.subscribe("AUDIO_TOGGLE_SELF_MUTE", updateVoiceState);
        dispatcher.subscribe("AUDIO_TOGGLE_SELF_DEAF", updateVoiceState);
        guildImportTimeout = setTimeout(() => {
            guildImportTimeout = undefined;
            window.remcord.touchbar.importGuilds(getAllGuilds());
        }, 5000);
    }
}

export function onUnload() {
    dispatcher.unsubscribe("TRACK", track);
    dispatcher.unsubscribe("AUDIO_TOGGLE_SELF_MUTE", updateVoiceState);
    dispatcher.unsubscribe("AUDIO_TOGGLE_SELF_DEAF", updateVoiceState);
    if (guildImportTimeout !== undefined) {
        clearTimeout(guildImportTimeout);
        guildImportTimeout = undefined;
    }
}

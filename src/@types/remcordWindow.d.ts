import type { Node } from "@vencord/venmic";
import type { Game, GameList, ProcessInfo } from "arrpc";
import type { IPCSources } from "../shelter/screenshare/components/SourceCard.tsx";
import type { Keybind } from "./keybind.js";
import type { Settings } from "./settings.js";
import type { ThemeManifest } from "./themeManifest.js";

export interface RemcordPluginInfo {
    id: string;
    name: string;
    version: string;
    description?: string;
    author?: string;
    enabled: boolean;
    compatible: boolean;
    compatibilityMessage?: string;
    compatibleVersions: string[];
    hasMain: boolean;
    hasPreload: boolean;
    hasRenderer: boolean;
}

export interface RemcordWindow {
    window: {
        show: () => void;
        hide: () => void;
        minimize: () => void;
        maximize: () => void;
        unmaximize: () => void;
        /** Native shell only: begin a frameless-chrome drag. Absent in Electron. */
        drag?: () => void;
        /** Native shell only: flip borderless fullscreen (F11). Absent in Electron. */
        toggleFullscreen?: () => void;
        quit: () => void;
        maximized: () => boolean;
        isNormal: () => boolean;
    };
    electron: string;
    getLang: (toGet: string) => Promise<string>;
    version: string;
    platform: string;
    osRelease: string;
    restart: () => void;
    translations: string;
    settings: {
        getConfig: () => Readonly<Settings>;
        setConfig: <K extends keyof Settings>(object: K, toSet: Settings[K]) => void;
        openStorageFolder: () => void;
        openThemesFolder: () => void;
        openCustomIconDialog: () => void;
        copyDebugInfo: () => void;
        copyGPUInfo: () => void;
        openWebRTCInternals: () => void;
        openGPUInfo: () => void;
        setLang(lang: string): () => void;
        addKeybind: (keybind: Keybind) => void;
        toggleKeybind: (id: string) => void;
        removeKeybind: (id: string) => void;
    };
    touchbar: {
        setVoiceTouchbar: (state: boolean) => void;
        setVoiceState: (mute: boolean, deafen: boolean) => void;
        importGuilds: (array: Array<string>) => void;
    };
    power: {
        setPowerSaving: (state: boolean) => void;
        isPowerSavingEnabled: () => boolean;
    };
    screenshare: {
        getSources: (
            callback: (event: Electron.IpcRendererEvent, sources: Array<IPCSources>, ...args: unknown[]) => void,
        ) => void;
        start: (id: string, name: string, audio: boolean) => void;
        venmicStart: (include: Node[]) => Promise<boolean>;
        venmicSystemStart: (exclude: Node[]) => Promise<boolean>;
        venmicList: () => Promise<
            { ok: true; targets: Node[]; hasPipewirePulse: boolean } | { ok: false; isGlibCxxOutdated: boolean }
        >;
        venmicStop: () => Promise<void>;
    };
    themes: {
        install: (url: string) => Promise<unknown>;
        openImportPicker: () => void;
        /** Electron-only picker completion event; absent in the native shell. */
        onThemesChanged?: (callback: (summary: { imported: number; failed: number }) => void) => () => void;
        uninstall: (id: string) => void;
        set: (id: string, state: boolean) => void;
        getThemes: () => Readonly<ThemeManifest[]>;
        refresh: () => Readonly<ThemeManifest[]>;
        openQuickCss: () => void;
        edit: (id: string) => void;
        folder: (id: string) => void;
        importQuickCss: (css: string) => void;
        disableQuickCss: () => void;
        enableQuickCss: () => void;
    };
    rpc: {
        listen: (msg: {
            activity: {
                assets: { large_image: string | null; small_image: string | null };
                application_id: number;
                name: string;
            };
        }) => void;
        getProcessList: () => ProcessInfo[];
        refreshProcessList: () => void;
        addDetectable: (e: Game) => void;
        removeDetectable: (id: string) => void;
        getDetectables: () => GameList;
        getBlacklist: () => DetectedGame[];
        blacklistGame: (name: string, id: number) => void;
        unblacklistGame: (id: number) => void;
    };
    backup: {
        save(data: string): Promise<{ ok: true } | { ok: false; error: string }>;
        restore(): Promise<string>;
    };
    gallery: {
        list: () => Promise<{ items: Array<Record<string, unknown>>; categories: Array<Record<string, unknown>> }>;
        addUrl: (url: string, meta?: Record<string, unknown>) => Promise<Record<string, unknown>>;
        addLink: (url: string, meta?: Record<string, unknown>) => Promise<Record<string, unknown>>;
        importDialog: (meta?: Record<string, unknown>) => Promise<Record<string, unknown>>;
        remove: (id: string) => Promise<{ ok: boolean }>;
        bulkRemove: (ids: string[]) => Promise<{ removed: number }>;
        update: (id: string, patch: Record<string, unknown>) => Promise<Record<string, unknown>>;
        categories: () => Promise<Array<Record<string, unknown>>>;
        categoryAdd: (name: string) => Promise<Record<string, unknown>>;
        categoryRename: (oldName: string, newName: string) => Promise<Record<string, unknown>>;
        categoryRemove: (name: string) => Promise<unknown>;
        usage: () => Promise<{ bytes: number; files: number; items: number }>;
        root: () => Promise<{ root: string }>;
        pickRoot: () => Promise<Record<string, unknown>>;
        openRoot: () => void;
        exportItems: (ids: string[]) => Promise<Record<string, unknown>>;
        getFile: (
            id: string,
        ) => Promise<{ ok: true; base64: string; mime: string; fileName: string } | { ok: false; error: string }>;
    };
    plugins: {
        list: () => Promise<RemcordPluginInfo[]>;
        setEnabled: (id: string, enabled: boolean) => Promise<{ ok: boolean }>;
        reload: (id: string) => Promise<{ ok: boolean }>;
        openFolder: () => void;
    };
    /** Plugin storage API. Requires user to enable "Extended plugin abilities" in Remcord settings. */
    fs: {
        writeFile: (
            pluginId: string,
            relativePath: string,
            data: string,
        ) => Promise<{ ok: true } | { ok: false; error: string }>;
        readFile: (
            pluginId: string,
            relativePath: string,
        ) => Promise<{ ok: true; data: string } | { ok: false; error: string }>;
    };
}

export interface DetectedGame {
    name: string;
    id: number;
}

export interface RemcordRPC {
    lastDetectedGames: DetectedGame[];
    onLastDetectedUpdate: ((list: DetectedGame[]) => void) | null;
    listen: (msg: {
        activity: {
            assets: { large_image: string | null; small_image: string | null };
            application_id: number;
            name: string;
        };
    }) => void;
}

declare global {
    interface Window {
        remcordRPC?: RemcordRPC;
    }
}

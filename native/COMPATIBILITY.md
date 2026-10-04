# Native WebView2 compatibility boundary

This document records the existing Electron renderer/preload contract and the intentionally narrow native bridge foundation. The Electron sources remain the behavioral specification. Nothing here claims that the Rust shell is Legcord-compatible.

“Boot required” means necessary to open Discord in a bare WebView, not necessary to preserve every existing Legcord feature. Injection and the renderer bridge are also tested in this prototype even though bare Discord can load without them.

| Electron capability | Current Legcord call sites / contract | WebView2 equivalent | Migration difficulty | Boot required? | Can defer? |
| --- | --- | --- | --- | --- | --- |
| `BrowserWindow` | [`src/discord/window.ts`](../src/discord/window.ts) `createWindow`; [`src/discord/ipc.ts`](../src/discord/ipc.ts) diagnostics windows; setup, splash and CSS editor windows | Native HWND plus `CoreWebView2Controller`; Tao owns the prototype window | High: bounds, titlebar, visibility, multi-window and lifecycle policies | Yes: one window | Multi-window, custom chrome and diagnostics can defer |
| `webContents` | `window.ts` owns navigation, reload, title/focus/maximize; `ipc.ts` sends native updates | `CoreWebView2` events and controller | High: event coverage differs and needs one owner | Yes: navigation/view | Most parity behaviors can defer |
| `executeJavaScript` | `window.ts` focus/maximize state; `common/dom.ts`; `common/keybindActions.ts`; `rpcProcess.ts`; preload mod files | `CoreWebView2.ExecuteScriptAsync` | Medium: async errors and document/frame targeting | Not for bare Discord | Yes; bootstrap and prototype probes are present |
| Preload / early script | `window.ts` configures `discord/preload.mjs`; `discord/preload/plugins.mts` and `preload/mods/*.mts` load client mods | `AddScriptToExecuteOnDocumentCreatedAsync` | High: registration must precede navigation; main-frame vs iframe scope must be explicit | No for bare Discord; yes for current mod startup | Not for the required mod ecosystem; prototype registers a main-frame script |
| `contextBridge` | [`src/discord/preload/bridge.ts`](../src/discord/preload/bridge.ts) exposes `window.legcord` | No direct equivalent; a document-start facade over `WebMessageReceived` | High: current facade is broad and synchronous APIs must be redesigned | No for bare Discord | Broad Legcord bridge can defer; this prototype exposes only the documented snapshot and typed ping |
| `webFrame` | `window.ts` frame-created YouTube injection; `preload/plugins.mts`; `preload/mods/{custom,equicord,shelter,vencord}.mts` | Frame-aware WebView2 scripting / `CoreWebView2Frame` | High: frame lifetime and cross-origin behavior differ | No | Yes, except where an enabled mod specifically needs frame injection |
| `insertCSS` | `preload/mods/{custom,equicord,vencord}.mts`; `preload/themes.ts`; [`src/common/themes.ts`](../src/common/themes.ts) Quick CSS/themes | `AddScriptToExecuteOnDocumentCreatedAsync` or `ExecuteScriptAsync` creating managed `<style>` elements | Medium: style identity/removal and per-document reapplication | No | Yes for full themes; prototype injects and checks a CSS custom property |
| `ipcMain` | [`src/discord/ipc.ts`](../src/discord/ipc.ts) registers bundle/config/theme/plugin/RPC/backup handlers; `screenshare.ts`, Venmic and other main modules register IPC | `WebMessageReceived` with a versioned, explicitly dispatched Rust command enum | Very high: hundreds of exposed calls and sync IPC need contract-by-contract migration | No for bare Discord; yes for existing mod feature parity | Yes; prototype supports only ping plus the documented acknowledgement, CSS, renderer-error, and bootstrap-metrics events |
| `ipcRenderer` | `bridge.ts`, `preload/themes.ts`, `preload/plugins.mts`, mod preloads | `window.chrome.webview.postMessage` / `WebMessageReceived` | Very high: subscriptions, events, invokes, and synchronous calls need async typed equivalents | No for bare Discord | Yes, beyond the tested demo bridge |
| `session.webRequest` | `window.ts` blocks telemetry/local WebSockets, rewrites YouTube Referer; `main.ts` applies proxy; CSP extension mutates policy | `WebResourceRequested`, filters, response replacement, WebView2 proxy configuration | High: request coverage and response/CSP manipulation require backend-specific validation | No | Yes; not part of first boot |
| Permissions | [`src/main.ts`](../src/main.ts) `setPermissionRequestHandler`; `screenshare.ts` display-media handler | `PermissionRequested`, media-capture and OS permission APIs | High: microphone/camera/display capture and consent must be tested separately | No for login; yes for voice/video | Yes |
| Downloads | Repository search found no custom `will-download`/download-item handler in the active source. Existing behavior is therefore the Electron/Chromium default unless another integration registers one at runtime. | `DownloadStarting` plus explicit destination/consent/completion handling | Medium-high: first characterize default behavior before choosing save/overwrite policy | No | Yes |
| Popups / windows | `window.ts` `setWindowOpenHandler`: `about:blank`, Discord stream popout, ICS blob, external links | `NewWindowRequested` with explicit allow/deny and a window registry | High: popup ownership, external URL policy, and blob behavior | No | Yes; prototype logs and denies popups |
| Screen sharing | [`src/discord/screenshare.ts`](../src/discord/screenshare.ts), `window.ts`, Shelter screen-share integration | WebView2 media permission/capture APIs plus Windows capture/audio services | Very high: `desktopCapturer`, loopback audio and source picker need native adapters | No for boot; critical for call parity | Yes, but must be qualified before claiming voice/video parity |
| Clipboard | `window.ts` context-menu image copy; `ipc.ts` debug/text clipboard | Win32 clipboard APIs and narrow WebView2 clipboard permission | Medium: image encoding, user gesture and permission behavior | No | Yes |
| Tray | [`src/discord/tray.ts`](../src/discord/tray.ts); setup tray in `src/setup/tray.ts` | Native Windows notification-area icon and callbacks | Medium: lifecycle, icon updates, menu state | No | Yes |
| Native menus | [`src/discord/menu.ts`](../src/discord/menu.ts); `window.ts` context menu | Win32 menus / WebView2 context-menu event | Medium-high: application, page and accessibility behavior | No | Yes |
| Window state | `window.ts` saves/restores bounds and maximize/minimize/focus state; [`src/common/windowState.ts`](../src/common/windowState.ts) persists it | Tao/Win32 window events plus an isolated native config store | Medium | No | Yes |
| Navigation / reload events | `window.ts` loads stable/canary/PTB Discord, handles `window.open`, reloads unresponsive pages; `main.ts` handles renderer-process loss | `NavigationStarting`, `NavigationCompleted`, process-failure events, `CoreWebView2.Reload` | Medium-high: origin policy and process recovery differ | Yes: initial navigation; reload events are prototype-validated only on target | Feature-complete loss/recovery policy can defer |
| Protocol handling | [`src/protocol.ts`](../src/protocol.ts) registers `legcord://`; mods and theme assets use local resources | `SetVirtualHostNameToFolderMapping` or `WebResourceRequested` | High: local path authorization, MIME and CSP behavior | No for bare Discord; yes for current local assets/mods | Yes |
| Updater | [`src/updater.ts`](../src/updater.ts), enabled by config; `main.ts` restart/relaunch | A Rust updater/signature/rollback service | High: release channels, signing and install layout | No | Yes |
| arRPC | [`src/rpc.ts`](../src/rpc.ts) starts arRPC; [`src/discord/rpcProcess.ts`](../src/discord/rpcProcess.ts) scans processes and forwards activity | Rust WebSocket service plus Windows process enumeration; typed page event | High: behavior and detection parity | No | Yes |
| Client mods / renderer injection | `discord/extensions/modloader.ts`; `discord/preload/mods/*.mts`; `discord/preload/plugins.mts`; `common/themes.ts` | Document-start script registry, post-load execution and managed styles | High: preserve ordering, update/failure behavior, CSP and reload lifecycle | No for bare Discord; yes for existing modified-client behavior | The general registry can defer beyond prototype; early-script and CSS mechanics are demonstrated |

## Prototype boundary

`native/src/main.rs` is deliberately independent of the existing Electron implementation. It uses the fixed DEV profile `E:\Legcord.webview-dev-user-data`, visits `https://discord.com/app`, limits top-level navigation to HTTPS Discord hosts, accepts bridge messages only from the exact `https://discord.com` origin, and has no general-purpose host invocation command. Electron features above must not be treated as implemented just because the browser can display Discord.

## Bridge protocol v1

The transport is Wry's `window.ipc.postMessage` / WebView2 web-message callback. The Rust side is authoritative and validates the origin, JSON shape, version, message size, request ID, and every payload before dispatch. Structs and tagged enums deny unknown fields. The envelope is either a request or a one-way event; it cannot contain both.

| Direction | Exact message |
| --- | --- |
| Renderer request | `{"version":1,"request_id":"legcord-<document-time>-<sequence>","command":{"type":"ping","payload":{"nonce":"..."}}}` |
| Native response | `{"version":1,"request_id":"...","ok":true,"payload":{"nonce":"...","message":"pong"},"error":null}` |
| Renderer event | `{"version":1,"event":{"type":"round_trip_ack","payload":{"nonce":"..."}}}` |

The only command is `ping`; the events are `round_trip_ack`, `css_probe`, `renderer_error`, and `bootstrap_metrics`. Request IDs are limited to 128 ASCII alphanumeric, `-`, or `_` characters, may not be reused within a document, and are capped at 4096 tracked IDs per document. The request/response contract is asynchronous. The renderer correlates responses by ID, ignores unknown IDs and wrong-version responses, and rejects outstanding promises on timeout or `pagehide`. Native request state is reset on a new document. Unknown versions, commands, events, malformed payloads, duplicate IDs, oversized messages, and untrusted origins are rejected and logged; no unknown request is dispatched.

Limits: a bridge message is at most 8 KiB; a ping nonce is non-empty and at most 128 bytes; CSS probe text is at most 128 bytes; renderer error kind is at most 64 bytes and its message at most 2 KiB; measured bootstrap time must be finite and in `[0, 60000]` ms. There are no arbitrary channels, host method names, path arguments, eval commands, or filesystem operations.

### Implemented `window.legcord`

The exact facade is `window.legcord = Object.freeze({ boot, native })`, installed in the document-start script only when `location.origin === "https://discord.com"`.

| Property | Kind | Behavior |
| --- | --- | --- |
| `window.legcord.boot.runtime` | immutable boot snapshot | `"webview2"` |
| `window.legcord.boot.platform` | immutable boot snapshot | `"win32"`; the native prototype is Windows-only |
| `window.legcord.boot.protocol_version` | immutable boot snapshot | `1` |
| `window.legcord.boot.capabilities` | immutable boot snapshot | `["bridge.ping"]` |
| `window.legcord.native.ping(nonce)` | async native request | Returns `{ nonce, message: "pong" }`; rejects malformed input, transport errors, native errors, timeout, or document disposal |
| `window.legcord.native.supports(name)` | renderer-local | Boolean membership test against the frozen boot capability list |
| `window.legcord.native.requireCapability(name)` | renderer-local | Returns `true` when present; otherwise throws with `code === "UNSUPPORTED_CAPABILITY"` |
| `window.legcord.native.reportCss(value)` | typed one-way diagnostic event | Sends the native prototype's CSS injection probe result; not a general CSS API |

No existing Electron-facing `window.legcord` methods are implemented by this initial facade. Boot snapshot values are native runtime metadata only; they do not impersonate Legcord config, Legcord version, OS release, translations, or Electron version.

### Classification of the existing renderer contract

The table lists the exposed `LegcordWindow` groups from `src/@types/legcordWindow.d.ts` and the behavior actually wired by `src/discord/preload/bridge.ts`. Existing API calls must not be redirected to an untyped generic bridge.

| API / caller | Sync or async; arguments / return | Side effects and boot-critical status | Migration decision |
| --- | --- | --- | --- |
| `window` — preload titlebar and Shelter titlebar | `show/hide/minimize/maximize/unmaximize/quit(): void`; `maximized()/isNormal(): boolean`; no arguments | Native window lifecycle; custom titlebar is not needed to open Discord | Deferred native commands; dynamic state is asynchronous or event-cached, never sync IPC |
| `settings.getConfig()` — Shelter settings/power/titlebar | Synchronous; no arguments; full `Settings` object | Reads mutable app settings; not needed for bare Discord, but current UI assumes synchronous state | Populate an initial documented config snapshot only if native settings support is added; migrate updates to async |
| `settings.setConfig(key,value)` — Shelter settings | Synchronous; key/value; handler returns `undefined` after persisting | Mutates config; not boot-critical | Async validated native request; migrate return type/callers |
| `settings.addKeybind(keybind)`, `toggleKeybind(id)`, `removeKeybind(id)` | Fire-and-forget; typed keybind or ID; `void` | Mutate settings and refresh global keybinds | Deferred |
| `settings.openStorageFolder/openThemesFolder/openCustomIconDialog/copyDebugInfo/copyGPUInfo/openWebRTCInternals/openGPUInfo` | Fire-and-forget; no args; `void` | Opens filesystem locations/dialogs/internal pages or writes clipboard | Deferred; host-owned dialogs/paths only |
| `settings.setLang(lang)` | Fire-and-forget; language string; `void` (type declaration says `() => void`, implementation sends without returning) | Changes app language | Deferred async request |
| `settings.dumpFlags()` | Synchronous; no args; applied flags object | Logs and returns runtime flags; not boot-critical | Async diagnostics request or native-local equivalent |
| `touchbar.setVoiceTouchbar(state)`, `setVoiceState(mute,deafen)`, `importGuilds(guilds)` — Shelter touchbar | Fire-and-forget; boolean(s) / string array; `void` | Mutates Touch Bar or voice state / imports guilds | Deferred; Touch Bar is platform-specific |
| `power.setPowerSaving(state)`, `isPowerSavingEnabled()` — Shelter power | Fire-and-forget boolean / synchronous boolean read | Mutates/reads native power-saving behavior | Deferred; mutable/dynamic read becomes async or cached event |
| `electron`, `version`, `platform`, `osRelease`, `translations` — preload and Shelter | Synchronous properties; respectively Electron version, Legcord version, OS platform/release, raw translation JSON | Snapshot-like values; platform/version/translation are read at preload boot | Only true native runtime metadata is in this prototype snapshot. Electron and Legcord version, OS release, and translations are not substituted with invented values |
| `getLang(key)` — invite-back preload and UI | Async; translation key string; `Promise<string>` | Reads one localized string; not boot-critical | Async typed request if localization is ported |
| `screenshare.getSources(callback)`, `start(source,name,audio)`, `venmicStart(include)`, `venmicSystemStart(exclude)`, `venmicList()`, `venmicStop()` — Shelter screenshare | Event subscription callback / fire-and-forget / async promises; source IDs, names, booleans, venmic node arrays; returns callback event, void, or `Promise` values | Capture source selection, native media and audio routing | Deferred; no source IDs, paths, or media authority exposed in this milestone |
| `themes.install(url)`, `uninstall(id)`, `edit(id)`, `getThemes()`, `refresh()`, `openImportPicker()`, `set(id,state)`, `folder(id)`, `openQuickCss()`, `importQuickCss(css)`, `enableQuickCss()`, `disableQuickCss()` | Mixed; install is declared void but implementation returns an async invoke; theme list/refresh are sync arrays; others fire-and-forget | Reads/updates theme files, opens dialogs, edits CSS; not bare-boot-critical | Snapshot only for a proven initial immutable listing; mutations/import are async and host-owned; full theme handling deferred |
| `rpc.refreshProcessList()`, `getProcessList()`, `addDetectable(game)`, `removeDetectable(id)`, `getDetectables()`, `getBlacklist()`, `blacklistGame(name,id)`, `unblacklistGame(id)` — Shelter registered games/RPC | Mixed fire-and-forget, synchronous process/list/cache reads, synchronous blacklist mutation | Reads process list, edits detectable/blacklist state, emits `rpcBlacklistUpdated`; not boot-critical | Process/detectable reads become async or locally cached; blacklist state uses typed events and async mutations |
| `backup.save(data)`, `restore()` — settings | Async; serialized backup string / no args; promise result | Opens save/open dialogs and reads/writes archive contents | Deferred native user-mediated file picker; renderer must not supply a filesystem path |
| `plugins.list()`, `setEnabled(id,enabled)`, `reload(id)`, `openFolder()` — plugins page | Async list/toggle/reload; fire-and-forget folder open | Enumerates and changes runtime plugins | Deferred; no plugin loader is part of the prototype |
| `fs.writeFile(pluginId,relativePath,data)`, `readFile(pluginId,relativePath)` — plugin storage | Async; plugin ID, relative path, text; structured success/error result | Reads/writes scoped plugin storage, gated by user setting and path containment | Deferred; never pass renderer-controlled paths to Rust |
| `window.legcordRPC` — `src/discord/rpcProcess.ts` / Shelter RPC | Renderer-side API; receives activity and notifies consumers | Page-world RPC data delivery; not a `LegcordWindow` member | Deferred arRPC integration |
| `window.legcord.channel` — `src/splash/redirect.html` | Synchronous string property | Selects redirect channel on splash page | Not part of the Discord document contract; splash is deferred |

### Direct Electron dependencies in Discord preload

These are separate from `window.legcord` and are required before the current preload/modules can run. All remain unimplemented and are not loaded in the native prototype.

| Caller | Electron dependency and contract | Migration |
| --- | --- | --- |
| `preload/bridge.ts` | `require("electron")`, `contextBridge.exposeInMainWorld`, `ipcRenderer.send/sendSync/invoke/on/removeListener`; defines the entire broad facade above and an RPC blacklist cache | Replace caller contract in deliberate TS migration; never polyfill `ipcRenderer` |
| `preload/plugins.mts` | `ipcRenderer.invoke` for preload/renderer runtime plugin entries and source; `webFrame.executeJavaScript`; dynamic `new Function` plugin evaluation | Entire loader deferred; script sources must not be introduced as generic host RPC |
| `preload/mods/{vencord,shelter,equicord,custom}.mts` | `ipcRenderer.invoke` for user bundle reads; `webFrame.executeJavaScript/insertCSS`; Shelter additionally uses `process.platform` | All four mod loaders deferred; none executes in native |
| `preload/themes.ts` | `ipcRenderer.on("addTheme"/"removeTheme")`; inserts/removes DOM styles | Deferred typed theme events and style lifecycle |
| `preload/titlebar.mts` | `ipcRenderer.sendSync` for OS/config/window state and `send` for window actions | Deferred titlebar; config boot values would need a native snapshot and dynamic state async |
| `preload/newTitlebar.mts` | `ipcRenderer.sendSync` for windowStyle/transparency/OS; `process.platform` | Deferred; no native chrome settings are asserted |
| `preload/patches.mts` | `ipcRenderer.sendSync` for display version and two settings; `process.platform`; DOM injection and timing | Requires a deliberate preload settings/version migration; not loaded |
| `preload/inviteBackButton.mts` | Async `ipcRenderer.invoke("getLang")`, DOM/page-world history patches | Deferred; translation lookup may later be a typed async request |
| all preloads above | `webFrame`, `contextBridge`, Node `require`, `process`, `ipcRenderer` are Electron-only globals, not WebView globals | Explicit adapters or TS migrations per feature; no ambient Electron emulation |
| `src/setup/preload.mts` | `contextBridge`, `ipcRenderer.send/sendSync/invoke`; exposes `window.setup` with restart, OS, settings save, and localization calls | Separate setup document; deferred |
| `src/splash/preload.mts` | `contextBridge`, `ipcRenderer.send/sendSync/invoke`; exposes `window.internal` with restart, app/splash metadata, mods, localization, and splash completion | Separate splash document; deferred |
| `src/cssEditor/preload.mts` | `contextBridge`, synchronous CSS read and fire-and-forget CSS write; exposes `window.cssEditor` | Separate editor document and file-backed state; deferred |

### Complete `sendSync` migration table

There are 35 call sites in the current repository search, grouped below by the contract they implement. The listed Electron handlers in `src/discord/ipc.ts` establish return values and side effects; values are not inferred from the TypeScript casts alone.

| Purpose / channels | Callers and returned data | Classification | Migration |
| --- | --- | --- | --- |
| Boot environment: `getOS`, `getOSRelease`, `get-app-version`, `displayVersion` | `bridge.ts`; `titlebar.mts`; `newTitlebar.mts`; `patches.mts`. Returns `process.platform`, `os.release()`, `getVersion()`, `getDisplayVersion()` | Boot snapshot values; safe only if sourced from native runtime/app metadata | Snapshot actual native values; retain separate Legcord app version only when native product version is authoritative. No Electron value is fabricated |
| Boot config: `getConfig("windowStyle"|"transparency"|"minimizeToTray"|"sdpH264BaselineRewrite"|"disableAutogain")` | `bridge.ts`, `titlebar.mts`, `newTitlebar.mts`, `patches.mts`. Returns configured typed value; setter elsewhere persists and returns `undefined` | Boot config reads, but current settings can be mutable | Include only proven immutable initial settings in a versioned native snapshot; migrate mutations to async and refresh cache/events |
| Window state: `win-isMaximized`, `win-isNormal` | `bridge.ts`, `titlebar.mts`. Returns current booleans | Dynamic cached-state candidate, not a boot constant | Native state events/local cache or async query; never a sync message |
| Full settings: `getEntireConfig` | `bridge.ts`; returns full `Settings` object | Cached-state candidate with mutable settings | Async initial read/cache plus events; keep secrets and unsupported fields out |
| Settings mutation: `setConfig(key,value)` | `bridge.ts`; updates config and returns `undefined` synchronously | Mutable operation disguised as sync result | Async command; update TS API/callers to await or optimistically cache |
| Localization: `getTranslations` | `bridge.ts`; returns raw translation JSON | Read-only snapshot candidate, but locale can change | Boot snapshot only for selected locale and immutable copy; language mutation remains async/evented |
| Theme data: `getThemes`, `refreshThemesCache` | `bridge.ts`; returns theme manifest array; refresh invalidates cache then reads it | Filesystem/plugin/theme reads plus mutable cache invalidation | Async typed theme-list request; local cache with explicit refresh event/operation |
| RPC state: `getRpcBlacklist`, `getProcessList`, `getDetectables` | `bridge.ts`; returns blacklist entries, process list, game list; blacklist cache subscribes to `rpcBlacklistUpdated` | Cached state reads and potentially changing process data | Local event-updated cache or async reads; do not block the renderer |
| RPC mutations: `blacklistGame(name,id)`, `unblacklistGame(id)` | `bridge.ts`; mutates blacklist, returns new entries, broadcasts update | Mutable operations with a synchronous result | Async request/response and typed change event |
| Diagnostics: `dumpFlags` | `bridge.ts`; logs and returns applied flags object | Read-only but non-boot-critical | Async diagnostics command or native computation |
| Setup: `setup-getOS` | `src/setup/preload.mts`; returns `process.platform` used for setup chrome | Boot snapshot, separate setup document | Deferred setup shell |
| Splash: `get-app-version`, `splash-isDev`, `splash-isMicrosoftStore`, `splash-clientmod` | `src/splash/preload.mts`; returns app version and splash flags/mod selection strings | Boot snapshot with other preload-only effects | Deferred splash; snapshot only after its native behavior is in scope |
| CSS editor: `editor-getCSS` | `src/cssEditor/preload.mts`; returns current CSS string | Filesystem read; mutable editor state | Async editor document contract; editor deferred |

`sendSync` calls are not replaced in `src/`. The boot snapshot in this milestone contains only `runtime`, `platform`, `protocol_version`, and the `bridge.ping` capability; it is not a partial `Settings` snapshot. No current sync caller is wired to it.

### `send`, `invoke`, and event contracts

| Preload caller / channels | Arguments and return | Side effects / migration |
| --- | --- | --- |
| `bridge.ts` window actions: `win-show`, `win-hide`, `win-minimize`, `win-maximize`, `win-unmaximize`, `win-quit`, `restart` | No args; `void` | Native window/restart actions; deferred explicit commands |
| `bridge.ts` settings actions: `addKeybind(keybind)`, `toggleKeybind(id)`, `removeKeybind(id)`, `openStorageFolder`, `setLang(lang)`, `openThemesFolder`, `openCustomIconDialog`, `copyDebugInfo`, `copyGPUInfo`, `openWebRTCInternals`, `openGPUInfo` | Typed keybind/id/lang or no args; `void` | Mutations, OS shell/dialog/clipboard/internal-page effects; deferred with user intent/validation |
| `bridge.ts` Touch Bar/power: `setVoiceTouchbar(state)`, `setVoiceState(mute,deafen)`, `importGuilds(guilds)`, `setPowerSaving(state)` | Boolean(s), string array; `void` | Native voice/window-control settings; deferred |
| `bridge.ts` screenshare: `startScreenshare(source,name,audio)`; `getSources` event subscription | String source ID/name/boolean; `void`; source event gives IDs, names, thumbnails, optional icon | Native capture and source authority; deferred |
| `bridge.ts` theme changes: `uninstallTheme(id)`, `editTheme(id)`, `openImportPicker`, `setThemeEnabled(id,enabled)`, `openThemeFolder(id)`, `openQuickCss`, `importQuickCss(css)`, `enableQuickCss`, `disableQuickCss` | IDs, enabled boolean, CSS string, or no args; `void` | Theme files, dialogs, editor windows and CSS injection; deferred host-controlled file handling |
| `bridge.ts` RPC changes: `refreshProcessList`, `addDetectable(game)`, `removeDetectable(id)` | Game/ID or no args; `void` | Process scanning / game config mutations; deferred |
| `bridge.ts` plugin folder: `openRuntimePluginsFolder` | No args; `void` | Opens a native path selected by host; deferred |
| `bridge.ts` async channels: `getLang`, `installBDTheme`, `backupSave`, `backupRestore`, `plugins:list`, `plugins:set-enabled`, `plugins:reload`, `pluginWriteFile`, `pluginReadFile` | Strings/IDs/booleans/serialized data; promise result. Backup and plugin-storage results are structured; theme install resolves `undefined` in its handler despite a preload cast of `Promise<null>` | Async but not safe to expose wholesale: dialogs, arbitrary third-party plugin code, and scoped filesystem access. Deferred per typed capability |
| mod preloads: `getShelterBundle`, `getVencordBundle`, `getEquicordBundle`, `getCustomBundle`; plugins runtime entries/scripts | No args or plugin target/ID; promises yield bundle text/flags or runtime entries/source | Filesystem reads plus executable JS/CSS; not boot-critical for Discord, but required for modified-client parity. Entire loader deferred |
| `themes.ts`: `addTheme(name,css)`, `removeTheme(name)`; bridge RPC blacklist update | Event payloads are name/CSS or entry arrays; listener returns unsubscribe for blacklist only | Updates renderer styles/cache; future bridge must use narrow typed subscriptions with lifecycle cleanup |
| `inviteBackButton.mts`: `getLang(key)` | Translation key; promise resolves string or undefined | Async localized text read; deferred typed locale request |

## Explicit capability ownership

The document-start script owns facade creation, local response correlation, `pagehide` promise cleanup and renderer-side diagnostics. Rust owns the versioned protocol schema, origin gate, request-ID uniqueness, command dispatch, responses, isolated persistent profile, and all host capabilities. The native profile remains exactly `E:\Legcord.webview-dev-user-data`. The renderer supplies no path, channel name, host method, executable source, or filesystem operation.

### Deferred and not tested

Filesystem and plugin storage; updater; tray; native menus; downloads; permissions; screen capture; WebRTC/native media; `webRequest`; protocol handlers; arRPC; popup windows; full mod loader; Vencord, Shelter, Equicord, custom bundles; runtime plugins; existing broad `window.legcord` APIs; all direct Electron preload globals. These are explicitly not implemented or tested by this foundation. Discord loading/runtime behavior outside the bridge smoke probe is not evidence of Legcord compatibility.

### Validation status

The Rust protocol and JavaScript facade have independent tests; runtime validation must separately confirm Discord loads, the facade is present, the snapshot is immutable, a facade ping returns through Rust, unsupported capabilities reject safely, and a reload creates a fresh document-local facade. Bootstrap size, document-start execution time, request/response latency, and bridge message counts are logged. Allocation counts are not measured. No polling, observers, timers for communication, per-frame messages, or JANK optimization is used.

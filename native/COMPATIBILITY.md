# Native WebView2 compatibility boundary

This document records the existing Electron renderer/preload contract and the intentionally narrow native bridge. The Electron sources remain the behavioral specification. Native WebView2 now executes three real client mods — Vencord, Shelter, and Equicord — through one shared typed substrate; nothing here claims that the Rust shell is Legcord-compatible, and each mod's own feature surface (settings, plugins, themes) is still largely deferred.

“Boot required” means necessary to open Discord in a bare WebView, not necessary to preserve every existing Legcord feature. Injection and the renderer bridge are also tested in this prototype even though bare Discord can load without them.

| Electron capability | Current Legcord call sites / contract | WebView2 equivalent | Migration difficulty | Boot required? | Can defer? |
| --- | --- | --- | --- | --- | --- |
| `BrowserWindow` | [`src/discord/window.ts`](../src/discord/window.ts) `createWindow`; [`src/discord/ipc.ts`](../src/discord/ipc.ts) diagnostics windows; setup, splash and CSS editor windows | Native HWND plus `CoreWebView2Controller`; Tao owns the prototype window | High: bounds, titlebar, visibility, multi-window and lifecycle policies | Yes: one window | Multi-window and diagnostics can defer; custom chrome implemented (P0.1) |
| `webContents` | `window.ts` owns navigation, reload, title/focus/maximize; `ipc.ts` sends native updates | `CoreWebView2` events and controller | High: event coverage differs and needs one owner | Yes: navigation/view | Most parity behaviors can defer |
| `executeJavaScript` | `window.ts` focus/maximize state (replaced by typed `window_state` event P0.1); `common/dom.ts`; `common/keybindActions.ts`; `rpcProcess.ts`; preload mod files | `CoreWebView2.ExecuteScriptAsync` | Medium: async errors and document/frame targeting | Not for bare Discord | Yes; bootstrap and prototype probes are present |
| Preload / early script | `window.ts` configures `discord/preload.mjs`; `discord/preload/preload.mts` imports `mods/{shelter,vencord,equicord,custom}.mts` | `AddScriptToExecuteOnDocumentCreatedAsync` | High: registration must precede navigation; main-frame vs iframe scope must be explicit | No for bare Discord; yes for mod startup | Mod adapters are main-frame/document-start; other preload modules defer |
| `contextBridge` | [`src/discord/preload/bridge.ts`](../src/discord/preload/bridge.ts) exposes `window.legcord` | No direct equivalent; a document-start facade over `WebMessageReceived` | High: current facade is broad and synchronous APIs must be redesigned | No for bare Discord | Broad Legcord bridge can defer; this prototype exposes the snapshot, ping, and the typed registry-scoped bundle operation |
| `webFrame` | `window.ts` frame-created YouTube injection; `preload/plugins.mts`; `preload/mods/{custom,equicord,shelter,vencord}.mts` | Frame-aware WebView2 scripting / `CoreWebView2Frame` | High: frame lifetime and cross-origin behavior differ | No | Yes, except where an enabled mod specifically needs frame injection |
| `insertCSS` | `preload/mods/vencord.mts` and `preload/mods/equicord.mts` inject their `browser.css` after bundle execution (`preload/mods/shelter.mts` injects none); `preload/themes.ts` and [`src/common/themes.ts`](../src/common/themes.ts) manage unrelated themes | Native host runs a fixed style-element insertion script with stable identity, in the same document and after the mod JS — only for descriptors that declare a stylesheet | Medium: CSP, style order, removal/reapplication must be qualified | No | Vencord + Equicord CSS are implemented, Shelter has none; other mod CSS defers |
| `ipcMain` | [`src/discord/ipc.ts`](../src/discord/ipc.ts) registers `getShelterBundle`, `getVencordBundle`, `getEquicordBundle`, `getCustomBundle` plus bundle/config/theme/plugin/RPC/backup handlers; `screenshare.ts`, Venmic and other main modules register IPC | `WebMessageReceived` with a versioned, explicitly dispatched Rust command enum | Very high: hundreds of exposed calls and sync IPC need contract-by-contract migration | No for bare Discord; yes for existing mod feature parity | Native supports `ping`, the registry-scoped `mod_get_bundle` operation, window chrome commands (`window_show`/`hide`/`minimize`/`maximize`/`unmaximize`/`quit`), typed lifecycle/error events, and `window_state` host→renderer event; other handlers defer |
| `ipcRenderer` | The Vencord/Shelter/Equicord preloads use only `invoke("getVencordBundle"/"getShelterBundle"/"getEquicordBundle")`; `bridge.ts`, `preload/plugins.mts`, and other mod preloads use separate APIs | `window.ipc.postMessage` / `WebMessageReceived` with one exact mod-bundle request contract | Very high: subscriptions, events, invokes, and synchronous calls need async typed equivalents | No for bare Discord | All three mod bundle startups are bridged through one shared registry-scoped capability; all other IPC surfaces defer |
| `session.webRequest` | `window.ts` blocks telemetry/local WebSockets, rewrites YouTube Referer; `main.ts` applies proxy; CSP extension mutates policy | `WebResourceRequested`, filters, response replacement, WebView2 proxy configuration | High: request coverage and response/CSP manipulation require backend-specific validation | No | Yes; not part of first boot |
| Permissions | [`src/main.ts`](../src/main.ts) `setPermissionRequestHandler`; `screenshare.ts` display-media handler | `PermissionRequested`, media-capture and OS permission APIs | High: microphone/camera/display capture and consent must be tested separately | No for login; yes for voice/video | Yes |
| Downloads | Repository search found no custom `will-download`/download-item handler in the active source. Existing behavior is therefore the Electron/Chromium default unless another integration registers one at runtime. | `DownloadStarting` plus explicit destination/consent/completion handling | Medium-high: first characterize default behavior before choosing save/overwrite policy | No | Yes |
| Popups / windows | `window.ts` `setWindowOpenHandler`: `about:blank`, Discord stream popout, ICS blob, external links | `NewWindowRequested` classification (`native/src/new_window.rs`) with one managed single-slot child, host-owned `ShellExecuteW` external open, deny-by-default | High: popup ownership, external URL policy, and blob behavior | No | Implemented (P0.4); downloads stay deferred |
| Screen sharing | [`src/discord/screenshare.ts`](../src/discord/screenshare.ts), `window.ts`, Shelter screen-share integration | WebView2 media permission/capture APIs plus Windows capture/audio services | Very high: `desktopCapturer`, loopback audio and source picker need native adapters | No for boot; critical for call parity | Yes, but must be qualified before claiming voice/video parity |
| Clipboard | `window.ts` context-menu image copy; `ipc.ts` debug/text clipboard | Win32 clipboard APIs and narrow WebView2 clipboard permission | Medium: image encoding, user gesture and permission behavior | No | Yes |
| Tray | [`src/discord/tray.ts`](../src/discord/tray.ts); setup tray in `src/setup/tray.ts` | Native Windows notification-area icon and callbacks | Medium: lifecycle, icon updates, menu state | No | Yes |
| Native menus | [`src/discord/menu.ts`](../src/discord/menu.ts); `window.ts` context menu | Win32 menus / WebView2 context-menu event | Medium-high: application, page and accessibility behavior | No | Yes |
| Window state | `window.ts` saves/restores bounds and maximize/minimize/focus state; [`src/common/windowState.ts`](../src/common/windowState.ts) persists it | Tao/Win32 window events plus an isolated native config store | Medium | No | Yes |
| Navigation / reload events | `window.ts` loads stable/canary/PTB Discord, handles `window.open`, reloads unresponsive pages; `main.ts` handles renderer-process loss | `NavigationStarting`, `NavigationCompleted`, process-failure events, `CoreWebView2.Reload` | Medium-high: origin policy and process recovery differ | Yes: initial navigation; reload events are prototype-validated only on target | Feature-complete loss/recovery policy can defer |
| Protocol handling | [`src/protocol.ts`](../src/protocol.ts) registers `legcord://`; mods and theme assets use local resources | WebView2 environment scheme registration + `WebResourceRequested` (P0.2: all six namespaces served, CSP extended narrowly) | High: local path authorization, MIME and CSP behavior | No for bare Discord; yes for current local assets/mods | Splash/setup/editor windows can defer; the protocol itself is implemented |
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
| Renderer request (mod bundle capability) | `{"version":1,"request_id":"...","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord"}}}` |
| Renderer request (settings capability) | `{"version":1,"request_id":"...","command":{"type":"settings_get_config","payload":{}}}` |
| Renderer request (settings mutation capability) | `{"version":1,"request_id":"...","command":{"type":"settings_set_block_power_saving","payload":{"value":true}}}` |
| Renderer request (settings set capability) | `{"version":1,"request_id":"...","command":{"type":"settings_set","payload":{"key":"mods","value":["vencord"]}}}` |
| Renderer request (localization capability) | `{"version":1,"request_id":"...","command":{"type":"localization_get_lang","payload":{"key":"invite-goBackToApp"}}}` |
| Renderer request (app version capability) | `{"version":1,"request_id":"...","command":{"type":"app_get_version","payload":{}}}` |
| Renderer request (plugin storage read) | `{"version":1,"request_id":"...","command":{"type":"plugin_storage_read","payload":{"plugin_id":"my-plugin","relative_path":"cache/note.txt"}}}` |
| Renderer request (plugin storage write) | `{"version":1,"request_id":"...","command":{"type":"plugin_storage_write","payload":{"plugin_id":"my-plugin","relative_path":"cache/note.txt","data":"..."}}}` |
| Renderer request (bundle discovery) | `{"version":1,"request_id":"...","command":{"type":"bundles_list","payload":{}}}` |
| Renderer request (backup save) | `{"version":1,"request_id":"...","command":{"type":"backup_save","payload":{"includes":{...7 booleans...},"clientMods":{...bounded envelope...}}}}` |
| Renderer request (backup restore) | `{"version":1,"request_id":"...","command":{"type":"backup_restore","payload":{}}}` |
| Renderer request (window show) | `{"version":1,"request_id":"...","command":{"type":"window_show","payload":{}}}` |
| Renderer request (window hide) | `{"version":1,"request_id":"...","command":{"type":"window_hide","payload":{}}}` |
| Renderer request (window minimize) | `{"version":1,"request_id":"...","command":{"type":"window_minimize","payload":{}}}` |
| Renderer request (window maximize) | `{"version":1,"request_id":"...","command":{"type":"window_maximize","payload":{}}}` |
| Renderer request (window unmaximize) | `{"version":1,"request_id":"...","command":{"type":"window_unmaximize","payload":{}}}` |
| Renderer request (window quit) | `{"version":1,"request_id":"...","command":{"type":"window_quit","payload":{}}}` |
| Renderer request (folder reveal) | `{"version":1,"request_id":"...","command":{"type":"folders_open","payload":{"location":"themes"}}}` |
| Renderer request (theme folder open) | `{"version":1,"request_id":"...","command":{"type":"folders_open_theme","payload":{"id":"my-theme"}}}` |
| Renderer request (theme install) | `{"version":1,"request_id":"...","command":{"type":"theme_install_code","payload":{"css":"/** @name Midnight Harbor */ :root {}"}}}` |
| Renderer request (theme CSS write) | `{"version":1,"request_id":"...","command":{"type":"theme_write_css","payload":{"id":"my-theme","css":":root {}"}}}` |
| Renderer request (theme file import) | `{"version":1,"request_id":"...","command":{"type":"theme_import_files","payload":{}}}` |
| Renderer request (theme enable flip) | `{"version":1,"request_id":"...","command":{"type":"theme_set_enabled","payload":{"id":"my-theme","enabled":true}}}` |
| Ping response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"ping","payload":{"nonce":"...","message":"pong"}},"error":null}` |
| Mod bundle response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"mod_bundle","payload":{"mod_id":"vencord","bundle_version":"...","bundle_bytes":1,"css_bytes":1,"request_latency_ms":1.0,"retrieval_ms":1.0,"execution_ms":1.0,"css_injection_ms":1.0,"marker":"vencord-bootstrap-executed","document_execution_count":1}},"error":null}` |
| Settings response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"settings_config","payload":{"settings":{...},"source":"file","payload_bytes":2048}},"error":null}` |
| Settings mutation response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"settings_mutation","payload":{"block_power_saving_in_voice_chat":true,"source":"file","payload_bytes":2048}},"error":null}` |
| Settings set response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"settings_set","payload":{"key":"mods","value":["vencord"],"source":"file","payload_bytes":2048}},"error":null}` |
| Localization response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"localization_string","payload":{"value":"Go back to the Discord app","lang":"en-US"}},"error":null}` |
| App version response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"app_version","payload":{"version":"1.3.0"}},"error":null}` |
| Plugin storage read response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"plugin_storage_read","payload":{"data":"...","payload_bytes":38}},"error":null}` |
| Plugin storage write response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"plugin_storage_write","payload":{"payload_bytes":38}},"error":null}` |
| Bundle inventory response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"bundles_list","payload":{"bundles":[{"name":"vencord.js","present":true,"bytes":698272},{"name":"vencord.css","present":true,"bytes":41335},{"name":"equicord.js","present":false,"bytes":0},{"name":"equicord.css","present":false,"bytes":0},{"name":"shelter.js","present":true,"bytes":235850},{"name":"custom.js","present":false,"bytes":0},{"name":"custom.css","present":false,"bytes":0}]}},"error":null}` |
| Backup save response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"backup_saved","payload":{"canceled":false,"bytes":1217955}},"error":null}` |
| Backup restore response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"backup_restored","payload":{"canceled":false,"client_mods":{"vencordLocalStorage":"...","shelter":{...}}}},"error":null}` |
| Window ack response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"window_ack","payload":{"command":"window_show"}},"error":null}` |
| Window state event | `{"version":1,"event":{"type":"window_state","payload":{"focused":true,"maximized":false}}}` |
| Folder reveal response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"folders_opened","payload":{"location":"themes"}},"error":null}` |
| Theme folder open response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"theme_folder_opened","payload":{"id":"my-theme"}},"error":null}` |
| Theme install response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"theme_installed","payload":{"id":"Midnight-Harbor-BD","enabled":false,"css_bytes":128}},"error":null}` |
| Theme CSS write response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"theme_written","payload":{"id":"my-theme","enabled":true,"css_bytes":32}},"error":null}` |
| Theme import response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"theme_imported","payload":{"canceled":false,"imported":1,"failed":0}},"error":null}` |
| Theme enable response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"theme_enabled","payload":{"id":"my-theme","enabled":true}},"error":null}` |
| Renderer event | `{"version":1,"event":{"type":"round_trip_ack","payload":{"nonce":"..."}}}` |

The commands are `ping`, `mod_get_bundle`, `settings_get_config`, `settings_set_block_power_saving`, `settings_set`, `localization_get_lang`, `app_get_version`, `plugin_storage_read`, `plugin_storage_write`, `bundles_list`, `backup_save`, `backup_restore`, `folders_open`, `folders_open_theme`, `theme_install_code`, `theme_write_css`, `theme_import_files`, and `theme_set_enabled`; events include `round_trip_ack`, `css_probe`, `renderer_error`, `bootstrap_metrics`, the typed Vencord CSS/bootstrap/adapter/response acknowledgements, and the DEV-only `settings_probe`/`get_lang_probe`/`settings_mutation_probe`/`settings_set_probe`/`app_version_probe`/`plugin_storage_probe`/`backup_save_probe`/`backup_restore_probe` diagnostics (emitted only by the `LEGCORD_NATIVE_SETTINGS_SMOKE=1` / `LEGCORD_NATIVE_GETLANG_SMOKE=1` / `LEGCORD_NATIVE_SETTINGS_MUTATION_SMOKE=1` / `LEGCORD_NATIVE_SETTINGS_SET_SMOKE=1` / `LEGCORD_NATIVE_APP_VERSION_SMOKE=1` / `LEGCORD_NATIVE_PLUGIN_STORAGE_SMOKE=1` probes, never by shipped adapters). Request IDs are limited to 128 ASCII alphanumeric, `-`, or `_` characters, may not be reused within a document, and are capped at 4096 tracked IDs per document. The request/response contract is asynchronous. The renderer correlates responses by ID, ignores unknown IDs and wrong-version responses, and rejects outstanding promises on timeout or `pagehide`. Native request state and Vencord's one-start/one-execution guard reset on a new document. Unknown versions, commands, events, malformed payloads, duplicate IDs, oversized messages, and untrusted origins are rejected and logged; no unknown request is dispatched.

## Vencord execution boundary

### Exact existing Electron chain

1. During app startup, `src/main.ts` awaits `Promise.all([fetchMods(), initializePluginSystem()])` before proceeding to `createWindow()`.
2. `src/discord/extensions/modloader.ts` reads the configured `mods` list, resolves Vencord's fixed release URLs from `src/common/consts.ts`, and downloads `browser.js`/`browser.css` to `app.getPath("userData")`. For configured mods it uses `forEach(async ...)`; those Vencord download/cache operations are not awaited by `fetchMods()`, so the window/preload can race the first download.
3. `src/discord/window.ts` creates a sandboxed BrowserWindow (`nodeIntegration: false`, `contextIsolation: true`) with `discord/preload.mjs`, then navigates to the selected Discord channel.
4. `src/discord/preload/preload.mts` imports `bridge.ts`, then the mod preloads including `mods/vencord.mts`. `vencord.mts` starts `inject()` immediately.
5. `inject()` invokes `ipcRenderer.invoke("getVencordBundle")`. `src/discord/ipc.ts` returns `enabled: getConfig("mods").includes("vencord")` and reads only `${app.getPath("userData")}/vencord.js` and `vencord.css`.
6. When enabled, the preload awaits `webFrame.executeJavaScript(bundle.js)` and then calls `webFrame.insertCSS(bundle.css!)`. This executes in the Discord page world; there is no additional Vencord init call, plugin-entry lookup, CSS removal key, or success marker. The preload catches/logs failures.

The Vencord integration's exact required Electron surface is therefore the module's Electron `require`, `ipcRenderer.invoke("getVencordBundle")`, `webFrame.executeJavaScript`, and `webFrame.insertCSS`. The main-process handler's filesystem read and modloader's download/write use Node `fs`/`path` plus `app.getPath("userData")`; they are not renderer filesystem APIs. `contextBridge` and `process.versions.electron` belong to the separately imported broad `bridge.ts`, not to `vencord.mts`. `plugins.mts` separately uses `ipcRenderer`, `webFrame`, and `new Function` to run user-authored runtime plugins; that dynamic execution is not a Vencord dependency. The Vencord standalone browser bundle is universal browser code, not an Electron preload shim.

### Native capability and adapter

The document-start Vencord adapter in `native/web/mods/vencord/adapter.js` is a thin consumer of the shared factory in `native/web/mod-substrate/mod-bootstrap.js`. It declares only Vencord's identity and typed wire vocabulary (capability name, expected marker, result/ack/error event names, state property names) and auto-requests one explicit `native.getModBundle("vencord")` call per Discord document. It can only send `mod_get_bundle` with the single logical id `"vencord"`. Rust validates the id shape, resolves the `VENCORD_MOD` descriptor from `KNOWN_MODS` by exact match, and runs the generic substrate pipeline: schema, exact stable Discord origin, request ID, per-document duplicate guard, and fixed bundle release sources. The substrate downloads only the descriptor's fixed JS/CSS release assets, allows only that descriptor's HTTPS redirect hosts, enforces byte limits, and caches only the fixed file names under `E:\Legcord.webview-dev-user-data` plus an in-memory `ModCache`. Renderer input cannot select a URL, path, filename, channel, command, or executable source.

## Scoped mod bundle capability (Phase D)

One shared, typed, registry-backed capability now serves every client mod: `mod_get_bundle` / capability `mod.get_bundle` / facade `native.getModBundle(modId)`. It replaces the previous mod-specific `mod_vencord_get_bundle` command (byte-for-byte identical Vencord behavior, no Vencord behavior changed).

### Electron bundle contracts (traced, unchanged)

`src/discord/ipc.ts` registers four handlers that read ONLY fixed `<userData>` file names through `ifExistsRead` (`existsSync` + `readFileSync(path, "utf-8")`, returning `undefined` when absent, no size limit):

| Handler | Files read | Returned shape | CSS separate | Executable JS | Caller |
| --- | --- | --- | --- | --- | --- |
| `getVencordBundle` | `vencord.js`, `vencord.css` | `{ js, css, enabled }` | yes (`insertCSS`) | yes (`executeJavaScript`) | `src/discord/preload/mods/vencord.mts` |
| `getShelterBundle` | `shelter.js` | `{ js, enabled: true }` | no (wraps source in an injector IIFE with a fixed `SHELTER_INJECTOR_PLUGINS` map) | yes | `src/discord/preload/mods/shelter.mts` |
| `getEquicordBundle` | `equicord.js`, `equicord.css` | `{ js, css, enabled }` | yes | yes | `src/discord/preload/mods/equicord.mts` |
| `getCustomBundle` | `custom.js`, `custom.css` (only when enabled) | `{ js, css, enabled }` | optional | yes | `src/discord/preload/mods/custom.mts` |

`enabled` comes from `getConfig("mods").includes(<mod>)` (Shelter hardcodes `true`). `src/discord/extensions/modloader.ts` (`fetchMods` → `cacheCheck`/`downloadMod`, sources in `src/common/consts.ts`) downloads the release assets into the same fixed names. Runtime plugin source (`plugins.mts` → `plugins:get-runtime-entries`/`get-runtime-script`) is a SEPARATE logical resource: per-plugin `manifest.json` + entry scripts executed with `new Function`, gated by plugin manifests. It is NOT part of this capability.

### Logical bundle registry (host-controlled)

| Logical id | JS source | CSS source | JS/CSS cache names | Size limits | Identity proof | Redirect hosts | Enablement | State |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `vencord` | `github.com/Vendicated/Vencord` devbuild `browser.js` | same release `browser.css` | `vencord.js` / `vencord.css` | 32 MiB / 8 MiB | build banner `// Vencord `, ≤64 chars | `github.com`, `release-assets.githubusercontent.com` | `settings.mods` | **validated end-to-end** |
| `shelter` | `raw.githubusercontent.com/uwu/shelter-builds/main/shelter.js` | **none** | `shelter.js` | 32 MiB | upstream source sentinels + host `sha256-…` digest (no banner is published) | `raw.githubusercontent.com` | always (Electron hardcodes `enabled: true`) | **validated end-to-end** |
| `equicord` | `github.com/Equicord/Equicord` latest `browser.js` | same release `browser.css` | `equicord.js` / `equicord.css` | 32 MiB / 8 MiB | build banner `// Equicord `, ≤64 chars | `github.com`, `release-assets.githubusercontent.com` | `settings.mods` | **validated end-to-end** |
| `custom` | user-provided build artifact | optional | `custom.js` / `custom.css` | — | — | — | — | deferred (no authoritative host-controlled source; never registerable as-is) |

Every descriptor value lives in the Rust `ModDescriptor` registry (`KNOWN_MODS` in `main.rs`); `mod_substrate.rs` stays mod-free. The three registered mods differ in **shape**, not in machinery — the descriptor's optional dimensions carry the difference and the substrate stays generic.

### Per-mod shape (traced, not assumed)

| Dimension | Vencord | Shelter | Equicord |
| --- | --- | --- | --- |
| CSS | yes (`insertCSS`) | **none** — `getShelterBundle` returns only `js`, no stylesheet exists upstream, no `insertCSS` in its preload | **yes**, explicitly (`//NOTE - Equicord requires CSS.`) |
| Page-world global | `Vencord` (sync) | `shelter` (**async** — assigned after Discord's `#app-mount` has children) + requires a `plugins` member | `Vencord` (sync; Equicord is a Vencord fork and publishes that global) |
| Settings key | `VencordSettings` | **none** — Shelter uses IndexedDB (`dbstore`), never `localStorage`; the snapshot reports `not_applicable` rather than inventing a key | `EquicordSettings` |
| Host wrapper | none | Electron's `SHELTER_INJECTOR_PLUGINS` IIFE is **deliberately not reproduced**: it depends on the `legcord://` protocol and the preload plugin loader, both unimplemented natively, and Shelter skips the injector when the global is absent | none |
| Enablement | `settings.mods` | unconditional (Electron hardcodes `enabled: true`) | `settings.mods` |

"Ships no CSS" is a first-class state: `ModDescriptor.css` is `Option<ModCssSource>`, validation refuses a stylesheet a descriptor did not declare and refuses a missing one it did, and the host runs no CSS stage at all (no empty `<style>`, no CSS result event, `css_bytes`/`css_injection_ms` both `null` in the response).

Adding a fourth client mod is still one descriptor plus one thin adapter — no protocol, facade, or substrate change.

### Protocol and validation

```text
renderer native.getModBundle("vencord")
  → requireCapability("mod.get_bundle") or sync UNSUPPORTED_CAPABILITY throw
  → client charset/bound check [A-Za-z0-9._-]{1,64} (TypeError before dispatch)
  → {"type":"mod_get_bundle","payload":{"mod_id":"vencord"}}
     (no path/url/file/source/channel/method member: deny_unknown_fields)
  → host: validate_mod_bundle_id (same charset + 64 B cap)
  → host: exact KNOWN_MODS lookup, else UNKNOWN_MOD
  → per-document one-start guard, else DUPLICATE_BOOTSTRAP
  → substrate: fixed release retrieval + validation + caches
  → typed mod_bundle response (metadata only)
```

Rejected explicitly (tests assert each): `../…`, `..\…`, `C:\…`, `//server/share/…`, `https://…`, `javascript:…`, empty, whitespace, and >64 B ids; any extra payload member including `url`, `path`, `file`, `source`, `channel`, `method`; unknown command names (`invoke`, `mod_vencord_get_bundle`). Charset-valid but unregistered ids (`custom`, `vencord.js`, `Vencord`, `SHELTER`, `EQUICORD`, `shelter/../x`, `equicord/foo`) parse and are refused by the host with `UNKNOWN_MOD` — no file is read and no source is substituted.

Artifact validation is per descriptor and never invents a marker:

- **Banner-identity mods** (Vencord, Equicord) require the exact `// <Name> ` first-line prefix plus a non-empty ≤64-byte version; missing, over-long, or foreign banners are refused.
- **Header-less mods** (Shelter) require **every** upstream sentinel to be present verbatim. Shelter's published artifact begins directly with its IIFE and carries no banner, so a banner check would be a fabricated requirement; instead the identity is its own initialization sentinels (`shelter is initializing...`, `shelter is initialized. took: `), none of which appears in Vencord's or Equicord's bundle, plus a host-computed `sha256-…` content digest reported as the bundle version. The digest is exact identity, not a pin: any byte change yields a different version, and the same bytes always yield the same one.
- All mods: bounded size, non-empty, valid UTF-8. A CSS declaration is required or refused as a unit (never a half-present stylesheet).
- Startup fails fast on any descriptor that could not be served safely (missing capability, id the request validator rejects, duplicate id, no adapter, or a value that could not be spliced into a generated script).

### Response shape

`mod_bundle` carries exactly ten bounded fields: `mod_id`, `bundle_version`, `bundle_bytes`, `css_bytes`, `request_latency_ms`, `retrieval_ms`, `execution_ms`, `css_injection_ms`, `marker`, `document_execution_count`. No bundle source, URL, path, filename, cache layout, or profile directory ever crosses the bridge (asserted in Rust tests). `mod_id` is the registry echo, so a mod-agnostic response cannot be mistaken for another mod's bundle — each adapter validates both `mod_id` and its own marker. For a mod that ships no stylesheet, `css_bytes` and `css_injection_ms` are **both** `null`; the renderer-side schema requires the pair to agree, so "ships no CSS" is never confused with "shipped an empty stylesheet".

### Cache behavior

One two-tier substrate cache, reused unchanged by all three mods (no per-mod cache was added): durable disk cache at fixed names under `E:\Legcord.webview-dev-user-data` (`vencord.js`/`vencord.css`, `shelter.js`, `equicord.js`/`equicord.css`), plus an in-memory `ModCache` keyed by mod id that avoids re-reading/validating across document generations of one run. Retrieval runs on a worker thread (`legcord-<id>-bundle`) and returns to the event loop through the typed `ModBundleReady` event with a document generation stamp; a result from a superseded generation is discarded. Cache hit reports `retrieval_ms=0.000`.

### Mod-agnostic lifecycle events

The Vencord-named wire events (`vencord_css_result`, `vencord_bootstrap_result`, `vencord_adapter_error`, `vencord_response_ack`) are **kept byte-identical** because Vencord's contract is already validated. New consumers use a parallel mod-agnostic family — `mod_css_result`, `mod_bootstrap_result`, `mod_adapter_error`, `mod_response_ack` — each carrying an explicit `mod_id`. Both families are normalized into the **same** host handlers, so there is exactly one implementation of the CSS gate, the bootstrap verdict, the duplicate guard, the marker check, and the response for every registered mod.

Each handler re-checks identity twice: the claimed `mod_id` must resolve in the registry, **and** it must equal the mod that owns the correlated request. One mod's adapter therefore cannot satisfy another mod's CSS gate or report another's verdict. The host-generated CSS script echoes its own descriptor's `mod_id`, so a renderer cannot claim that stylesheet belonged to a different mod. Unregistered ids on any event are logged as `unknown_mod` and ignored.

### Adapter enablement (host-controlled)

Adapter injection is decided entirely by Rust: each descriptor's own traced Electron policy (`SettingsMods` for Vencord/Equicord → `settings.mods` contains the id; `AlwaysEnabled` for Shelter, matching Electron's hardcoded `enabled: true`) applied to the authoritative `storage/settings.json` snapshot, plus an optional DEV-only allowlist (`LEGCORD_NATIVE_DIAGNOSTIC_MODS`) that can only **remove** mods. Corrupt settings fail closed to "no settings-gated mods". The renderer participates in none of this. One `[Mod:Registry] mod='<id>' adapter_enabled=<bool> enabling=<policy>` line per registered mod makes the decision auditable.

### Vencord, Shelter, and Equicord integration

All three adapters are thin identity declarations over the same factory: capability name, their own `getModBundle("<id>")` call, expected marker, wire event names, state property names, and the two identity checks (`mod_id` echo + own marker). Execution, CSS, readiness, markers, storage snapshot, duplicate protection, and page-world semantics live entirely in the substrate. Vencord's execution/CSS/marker behavior is unchanged; its adapter differs only in using the legacy event names and no `modId` echo.

### Remaining mod work

1. Custom mods stay blocked until a host-controlled authoritative source exists; there is deliberately no renderer-supplied path/URL for it.
2. Shelter's Legcord plugin surface (`SHELTER_INJECTOR_PLUGINS`, the `legcord://` protocol, the preload plugin loader, `new Function`) is a separate logical resource and remains deferred — which is why its Electron injector wrapper is deliberately not reproduced.
3. Runtime plugin loading (`plugins.mts`, `new Function`, per-plugin manifests) is also deferred and unrelated to bundle registration.
4. Running Vencord and Equicord together is a configuration conflict (both publish the `Vencord` page-world global), not a host defect; enablement follows `settings.mods` exactly as Electron does.

To avoid transferring a multi-megabyte script back through the renderer and to preserve page-world execution despite Discord's script CSP, the capability returns a typed metadata result only after Rust retrieves and invokes that fixed source via Wry/WebView2 `ExecuteScriptAsync` in the main document. The existing Vencord source is run as a direct classic script, not wrapped in an IIFE or `eval`; no generic script-execution method is exposed. After execution, a descriptor-parameterized verifier checks for the `globalThis.Vencord` object. Only then does the adapter's non-enumerable `window.__legcordNativeVencord` marker become `executed`.

CSS uses a descriptor-parameterized host-generated script to create/update one `<style id="legcord-native-vencord-css">` after bundle execution. It appends to the document's current head/root, reports a typed result, and replaces rather than duplicates that style if reused. Each full reload receives a fresh adapter, pending-request map, marker, style element, and native document generation; stale worker completions are discarded. The substrate permits only one request and one successful marker per mod per document.

Failure logs distinguish `bundle_retrieval`, `adapter_initialization`, `script_execution`, `css_injection`, and renderer `uncaught`/`unhandled_rejection`/`console.error` reports. The host remains alive on these failures. The native response contains byte counts, bundle header version, request/retrieval/execution/CSS timings, and the marker; it deliberately does not return source text to JavaScript.

## Bundle discovery capability (Phase D Step 1)

The first backups slice migrates ONLY bundle-file discovery — the `BUNDLE_FILES` half of `src/common/backup.ts` — to a read-only typed capability `backup.list_bundles` (wire `bundles_list`, facade `window.legcord.backup.listBundles()`). No save/restore, no zip handling, no dialogs, no writes, no folder opening, no theme install/edit. There is no migrated `src/` caller (the Electron backup UI keeps calling `save`/`restore`; a native branch would be fabrication), so this slice establishes the allowlist + root-scoping contract that a future export slice will build on.

### Electron chain (unchanged)

`src/common/backup.ts:73-81` defines exactly 7 fixed names at `<userData>/` (`vencord.js`, `vencord.css`, `equicord.js`, `equicord.css`, `shelter.js`, `custom.js`, `custom.css`). `buildBackupZipBuffer` checks each with `addFileIfExists` (existence check; missing files silently skipped, never placeholders), reads whole files unbounded, and stores them as `bundles/<name>`; `applyBackupFromMap` restores them with recursive `mkdir` plus silent overwrite, gated only by `includes.modBundles` and exact allowlist equality. `backupSave` takes one JSON string (`{includes, clientMods}` — the `clientMods` blob is renderer-built from `localStorage`/Shelter and never touches host files) and writes only through a host-mediated save dialog; `backupRestore` takes no arguments and reads only through a host-mediated open dialog. `native/tests/electron-compat.test.mjs` asserts these handlers, the bridge pair, the declared `backup` contract, and the absence of any native backup branch in `src/` are unchanged.

### Native chain

```text
renderer await window.legcord.backup.listBundles()
  → requireCapability("backup.list_bundles") or sync UNSUPPORTED_CAPABILITY throw
  → parameter-free {"type":"bundles_list","payload":{}} request
     (path/url/include_*/name/method members host-rejected by deny_unknown_fields)
  → Rust bundles::describe_bundle_files(<DEV profile>/, fixed BUNDLE_FILES only)
     (host-built paths; symlink_metadata, never followed; missing or
      non-regular file → present:false, bytes:0; other IO errors fail closed
      as BUNDLES_UNAVAILABLE with no path in the message)
  → validated bundles_list { bundles: [{name, present, bytes}] × 7 } → resolved entries
```

Semantic parity with Electron (asserted by comparing the canonical definitions, not by paraphrase): same 7 bundle names, same ordering, same presence semantics for regular files, same `<name>` bundle path naming. `custom.js`/`custom.css` are inventoried exactly like the other five names but stay inventory-only and non-executable — no descriptor registers the `custom` id, so presence can never become execution.

Deliberate Electron differences (hardening, not regressions): metadata only (`{name, present, bytes}` — contents, absolute/profile paths, URLs, and path-bearing errors never cross); symlinks/junctions/directories report absent instead of being followed (Electron's `readFileSync` would follow a planted link and embed outside bytes into a backup); no upper byte bound on reported sizes (nothing is allocated from metadata); non-`NotFound` IO errors fail the whole inventory closed instead of guessing absence. The facade enforces the exact 7-name sequence, `absent ⇒ bytes 0`, and rejects any entry carrying `path`/`url`/`data`.

## Backup save capability (Phase D4-1)

The second backups slice migrates ONLY the save half — `buildBackupZipBuffer` plus the `showSaveDialog` + write path of `backupSave` — to a typed capability `backup.save` (wire `backup_save`, facade `window.legcord.backup.save(payloadString)` preserving the existing Electron JSON-string signature, so `BackupSection.tsx` is untouched). Restore (`applyBackupFromMap` / `backupRestore`) is out of scope for that slice (see the restore section below) and BackupSection UI wiring stays deferred to D4-3. There is no new `src/` caller in either slice: a native branch in the Electron backup UI would be fabrication, so DEV-only probes (`LEGCORD_NATIVE_BACKUP_SAVE_SMOKE=1`, `LEGCORD_NATIVE_BACKUP_RESTORE_SMOKE=1`) exercise the real capabilities instead.

### Electron chain (unchanged)

`src/common/backup.ts::buildBackupZipBuffer(payload, paths)` builds `{ version: 1, createdAt: new Date().toISOString(), appVersion: app.getVersion(), includes, clientMods }` (only flagged clientMods keys carried; unflagged stay absent; flagged-but-missing values become `null`), then collects `data/storage/settings.json`, `data/quickCss.css` + `data/themes/**`, `data/plugins/**` (disk `extensions/`) + `data/runtime-plugins/**` (disk `plugins/`) + `data/plugin-storage/**`, and `bundles/<name>` for the 7 fixed names — manifest first, whole-file unbounded reads, filesystem order, missing files/dirs silently skipped. `src/discord/ipc.ts::backupSave` parses one JSON string, builds the buffer, shows `dialog.showSaveDialog` (translated title `backup-dialogSave-title`, `defaultPath: Documents/legcord-backup-YYYY-MM-DD.zip`, one `ZIP/.zip` filter), returns `{ ok: false, error: "CANCELLED" }` on dismissal, appends `.zip` when missing, and writes with `writeFileSync`. `native/tests/electron-compat.test.mjs` asserts every handler, bridge mapping, and declared contract above is byte-identical.

### Native chain

```text
renderer await window.legcord.backup.save(jsonString)
  → requireCapability("backup.save") or sync UNSUPPORTED_CAPABILITY throw
  → client exact-shape check ({includes, clientMods} only; 7 booleans;
     3 known clientMods keys; per-string ≤ 1 MiB; TypeError before dispatch)
  → {"type":"backup_save","payload":{"includes":{...},"clientMods":{...}}}
     (any path/filename/archive/source/dialog/root member host-rejected)
  → dedicated STA worker (COM once per thread): validate envelope
     → synthesize manifest (version 1, UTC ISO-8601 createdAt, appVersion
        from the existing app-version owner) → collect bounded entries
        (sorted order) → build deflate ZIP (zip 2.x, manifest first)
        → IFileSaveDialog (translated title, ZIP filter, UTC-date default
        filename, .zip enforced) → sibling-temp + rename write
  → validated backup_saved { canceled, bytes } (byte count only, never a path)
  → facade maps to the Electron shape: {ok:true} | {ok:false, error:"CANCELLED"}
     | {ok:false, error:<typed code>} (rejections mapped, never unhandled)
```

Response echo discipline differs from other capabilities by necessity: the renderer needs Electron's `{ok, error?}` shape, so the facade resolves (never rejects) all host outcomes into it. Transport/capability misuse still rejects or throws synchronously before dispatch.

UI wiring (D4-3): `BackupSection.tsx` calls one shared `saveBackup(payloadString)` helper for both shells — the native facade preserves the Electron save signature AND result shape, so payload construction (`includes` + renderer-built `clientMods`), the success/cancelled/failed toasts, and the silent `CANCELLED` control flow are identical in both. A `busy` signal disables both buttons while an operation runs (no duplicate dialogs, no overlapping host operations) with try/finally release plus a catch → generic-error toast, closing the pre-existing unhandled-rejection hole on transport failure.

Deliberate Electron differences (hardening, not regressions): typed includes + bounded clientMods (per-string ≤ 1 MiB, total ≤ 4 MiB, shelter ≤ 2 MiB with depth ≤ 32 / nodes ≤ 65536 — renderer input is never trusted); per-file ≤ 1 MiB except the bundle allowlist (≤ 32 MiB, the mod-substrate retrieval ceiling — real artifacts like `equicord.js` are multi-megabyte); total uncompressed ≤ 64 MiB; entries ≤ 4096; overflow fails closed as `BACKUP_TOO_LARGE`, never a truncated archive; links/junctions/non-regular files skipped, never followed; sorted traversal (deterministic archives; Electron uses filesystem order); nothing created for backup purposes (a missing `quickCss.css` is NOT created, unlike the Quick CSS loader); no `defaultPath` (the host sets the filename only); temp-file + rename write (Electron writes in place); typed path-free errors instead of raw messages; renderer receives `{canceled, bytes}` semantics only.

Known issue (fixed in P0.4): `compatibility.js` discovery sent `backup_list_bundles` while the protocol expects `bundles_list`. The facade now sends `bundles_list`; the old string stays host-rejected. D4-1 did not exercise listBundles and left the one-line fix out of scope.

## Backup restore capability (Phase D4-2)

The third backups slice migrates ONLY the restore half — `applyBackupFromMap` plus the `showOpenDialog` + read path of `backupRestore` — to a typed capability `backup.restore` (wire `backup_restore`, facade `window.legcord.backup.restore()` with no arguments, so `BackupSection.tsx` is untouched). The native restore consumes the SAME Electron-compatible ZIP format the save path produces (and valid Electron-produced archives that avoid unsafe/unbounded behavior), while intentionally hardening every unsafe filesystem/archive behavior. BackupSection UI wiring (D4-3) stays deferred: there is no new `src/` caller, so a DEV-only probe (`LEGCORD_NATIVE_BACKUP_RESTORE_SMOKE=1`) exercises the real capability instead.

### Electron chain (unchanged)

`src/common/backup.ts::applyBackupFromMap(map, paths)` parses `manifest.json` (`version === 1` is the ONLY check), then iterates the whole `Map<string, Buffer>` in insertion order with INLINE writes (no staging, no transaction): `data/storage/settings.json` (only when `rest === "settings.json"`) is fed through `setConfigBulk` (SHALLOW merge into the live cache, then write — existing/unknown keys survive, archive keys win); `data/quickCss.css` verbatim; `data/themes/**`, `data/plugins/**` (disk `extensions/`), `data/runtime-plugins/**` (disk `plugins/`), `data/plugin-storage/**` verbatim through `resolvePathUnderBaseDir` (zip-slip guard — unsafe entries are SKIPPED with a console warning while earlier entries stay committed, so a hostile archive leaves a half-restored profile); `bundles/<name>` only on EXACT allowlist equality (unknown names fall through). Include flags gate each section (present-but-disabled entries skipped); unknown names fall through silently; overwrites are silent; symlinks/junctions are followed (`writeFileSync`); duplicates collapse to last-wins in the `Map`. `src/discord/ipc.ts::backupRestore` takes no arguments, reads the picked file unbounded with `readFileSync`, and returns the JSON string `{ ok: true, clientMods }` with `clientMods` VERBATIM from the manifest. `BackupSection.tsx::applyClientModsFromRestore` writes `VencordSettings`/`EquicordSettings` into `localStorage` and calls `shelter.plugins.addRemotePlugin(id, src, update)` per shelter entry (URL fetch/execution happens renderer-side). `native/tests/electron-compat.test.mjs` asserts every handler, bridge mapping, declared contract, and renderer applier above is byte-identical, and that no `src/` caller references the native restore path.

### Native chain

```text
renderer await window.legcord.backup.restore()
  → requireCapability("backup.restore") or sync UNSUPPORTED_CAPABILITY throw
  → parameter-free {"type":"backup_restore","payload":{}} request
     (any archive path/filename/url/filter/dialog member host-rejected)
  → dedicated STA worker (COM once per thread): host-owned IFileOpenDialog
     (translated backup-dialogOpen-title, one ZIP filter, single-select,
      FOS_FILEMUSTEXIST|FOS_FORCEFILESYSTEM) → bounded archive read
     (no-follow, ≤ 65 MiB, races re-checked)
     → validate the ENTIRE archive (manifest, every entry name + routing,
        sizes, counts, duplicates, collisions) → stage outside the profile
     → pre-commit destination check (no writes) → commit with rollback
  → validated backup_restored { canceled, client_mods } (envelope only,
     never a path) → facade maps to { ok: true, clientMods }
     | { ok: false, error: "CANCELLED" } | { ok: false, error: <typed code> }
     (rejections mapped, never unhandled)
```

Response echo discipline matches save by necessity: the renderer needs Electron's `{ok, error?}` shape, so the facade resolves (never rejects) all host outcomes into it, including timeouts (`REQUEST_TIMEOUT`). Transport/capability misuse still rejects or throws synchronously before dispatch.

### Archive validation (whole archive before any profile mutation)

- Input: picked file ≤ 65 MiB (64 MiB payload budget + container slack); manifest ≤ ~4 MiB (save-request expression); entry count ≤ 4096; total uncompressed ≤ 64 MiB; normal file ≤ 1 MiB; bundle file ≤ 32 MiB (save parity for real build artifacts); entry names ≤ 1024 B, components ≤ 128 B, depth ≤ 32. Overflow fails the whole restore as `BACKUP_TOO_LARGE`.
- Entry names: `\` normalized to `/` first (Electron parity), then the whole archive is rejected on absolute/drive/UNC paths, NUL/controls, `:` (drive + ADS), empty/`.`/`..` components. Directories are skipped (Electron parity — they create nothing).
- Duplicates/collisions fail the whole restore as `INVALID_ARCHIVE` (Electron's silent `Map` last-wins is deliberately NOT reproduced): exact duplicate names, case-insensitive (Windows) collisions, and separator-normalization collisions.
- Manifest: exact `version: 1`, exact seven include flags, clientMods with exactly the three known keys validated through the SAME bounded envelope as save (per-string ≤ 1 MiB, total ≤ 4 MiB, shelter ≤ 2 MiB, depth ≤ 32, nodes ≤ 65536). Unknown top-level metadata is ignored (forward compatibility — it is never routed anywhere). Missing/malformed manifest fails closed.
- Closed routing (never a generic extract): `data/storage/settings.json` only (other storage names ignored, Electron parity); `data/quickCss.css` exact; `data/themes/<id>/<rel>` (id via `validate_theme_id` + device-stem rule, rest via `validate_relative_path`; loose files directly under `themes/` validated as relative paths — the saver collects those too); `data/plugins/<rel>` (disk `extensions/`, relative-path only — no id architecture exists there); `data/runtime-plugins/<rel>` (disk `plugins/`, runtime-plugin dir charset `[A-Za-z0-9._-]` + device-stem rule, identity stays loader-owned); `data/plugin-storage/<plugin_id>/<rel>` (strict `validate_plugin_id` + `validate_relative_path`); `bundles/<name>` exact allowlist equality only (unknown names ignored, never profile-root writes). Unknown top-level sections ignored (Electron fall-through parity).
- Include flags are authoritative: present-but-disabled sections are skipped (Electron `continue` parity); absent-but-enabled sections restore nothing.
- Reads are streaming (`take(bound + 1)` per entry into staging files): the uncompressed archive is never fully buffered in memory, unlike Electron's whole-`Map` model.

### Staging / commit / rollback

- Staging lives in a unique host temp dir (`legcord-backup-restore-<pid>-<nanos>-<ctr>`), never inside `themes/`/`plugins/`/`plugin-storage/`/`extensions/`. Removed on success AND failure — no temp residue either way.
- Commit phase 1 (no writes): every destination pre-checked — symlinks and non-file occupants fail BEFORE any byte lands. Phase 2: parents created by the host, canonical containment verified per parent against the canonical section root (a junction planted AT a section root resolves outside and fails closed), existing files backed up to rollback state (existing files over 64 MiB fail closed instead of being buffered), then per-file atomic unique-temp + same-directory rename.
- Settings is a STAGED SHALLOW MERGE (Electron `setConfigBulk` parity — archive keys win, existing/unknown keys survive, 2-space pretty JSON like the native settings mutation with identical semantics, merged document capped at the 256 KiB settings bound). A missing/corrupt live file merges from `{}` (Electron corrupt-cache fallback parity — restore is a recovery path). Archive settings must still be a JSON object; wrong-typed values are accepted verbatim with repair deferred to the existing startup check, exactly as in Electron.
- On any commit failure, already-replaced files are restored from rollback state and newly created files are removed. This is per-file atomicity plus best-effort multi-file recovery — NOT a cross-filesystem transaction, and it is documented as such (never claimed as "atomic restore").
- Overwrite semantics preserved (silent replace, deterministic repeat, unrelated files untouched); Quick CSS restores bytes verbatim (UTF-8 classification stays loader-owned); themes restore `manifest.json` verbatim so the archive's own `enabled` value rides along (the host never flips enablement); `custom.js`/`custom.css` restore as DATA ONLY (never executed).

### clientMods boundary

The host NEVER applies clientMods: no `localStorage` manipulation, no `addRemotePlugin`, no fetching, no script execution. It returns the filtered, bounded envelope for the existing renderer mechanism. Oversized/malformed/wrong-typed envelopes fail the whole restore instead of crossing the bridge unbounded.

### UI wiring (D4-3)

`BackupSection.tsx` branches restore on `isNativeBackupAvailable()` (existing `isNativeWebView` environment check AND `backup.save`/`backup.restore` function presence — an Electron `window.legcord.backup` also carries those names, so the environment check selects the shape): native takes the OBJECT result through `normalizeNativeRestoreResult`, Electron takes the existing JSON-string path through `parseElectronRestoreResult`, and both converge on one shared outcome (`{ok:true, clientMods}` | `{ok:false, error}`) with the pre-existing toasts — success + restart-required message, silent `CANCELLED`, `INVALID` → invalid-file toast, `UNKNOWN`/typed code → failed toast. The existing `applyClientModsFromRestore` (localStorage writes + `addRemotePlugin`) is untouched and stays the renderer's job; the new helpers add no storage/plugin/fetch surface of their own (asserted). One degenerate-input difference from the old inline parse: `{}` (or another truthy-shaped non-`ok:true` document, which neither host ever sends) now reports invalid-file instead of an erroneous success toast. The confirmation modal and restart message are unchanged; no reload is forced (the existing UI never reloaded).

### Intentional Electron/native differences (hardening, not regressions)

Whole-archive rejection instead of skip-with-warning (no half-restored profile); duplicate/collision rejection instead of last-wins; strict section validators instead of zip-slip-only; no-follow link refusal at read, stage, and commit instead of silent following; staged commit with rollback instead of inline writes; bounded everything instead of unbounded `Map`/reads; typed path-free errors instead of raw messages; renderer receives the filtered envelope only. Format-compatible in both directions for archives that avoid unsafe/unbounded behavior (proven: native save output round-trips; an independently constructed archive restores; archives exploiting unsafe shapes are rejected as expected).

Known issue (fixed in P0.4): `compatibility.js` discovery sent `backup_list_bundles` while the protocol expects `bundles_list`. Neither D4-1 nor D4-2 exercised listBundles; the one-line facade fix landed with the popup slice.

## Popup and external navigation (P0.4)

P0.4 migrates ONLY the `setWindowOpenHandler` decision surface (`src/discord/window.ts:214-264`) to a dedicated module `native/src/new_window.rs` plus one managed single-slot child owned by the event loop. No downloads (`DownloadStarting` stays deferred), no tray/screenshare/power/crash/instance/benchmark work, no generic navigation framework, no generic shell API, no multi-window registry.

### Electron chain (unchanged)

Exact order: `url === "about:blank"` → allow (default options; comment cites the Vencord/Equicord QuickCSS popup); `isDiscordIcsBlobUrl(url)` → allow hidden (`show:false`); `isDiscordPopoutUrl(url)` → allow with `alwaysOnTop: getConfig("popoutPiP")`; raw `startsWith("https:"/"http:"/"mailto:")` → `shell.openExternal` + deny; `ignoreProtocolWarning` → `shell.openExternal` + deny; otherwise a modal Yes/No dialog (raw URL interpolated) whose checkbox persists `ignoreProtocolWarning`, answered asynchronously while the handler synchronously denies. No sanitizer runs on any external path. Renderer `window.open` callers in `src/` are all external `https:` links (About/support/HeroUpdater/download URL) — they take the external-open path, never a child.

### Native chain

```text
NewWindowRequested(url)  [wry handler; renderer size/position ignored]
  → decide(url) in new_window.rs (parsed, ordered like Electron)
  → AllowAboutBlank | AllowIcs | AllowDiscordPopout
      → WebView2 popup denied (Handled=true), NativeEvent::PopupRequested
        queued → event loop creates/focuses the single managed child
  → ExternalOpen (https/http with parsed host, mailto)
      → open_external_url: ShellExecuteW("open", url), no command line,
        no interpreter, no HWND/PID → WebView2 popup denied, parent stays
  → ConfirmExternal (unknown custom, e.g. discord://, tel:) → denied + log
      (no Electron modal, no sticky bypass, no new settings key)
  → Deny (javascript:/data:/file:/non-ICS blob:/malformed) → denied + log,
      never reaches the shell
```

Child (`create_popup_child`, `main.rs`): fixed 800x600 Tao window, default decorations, `alwaysOnTop` only from the host-read `popoutPiP` setting and only for the Discord popout; `about:blank` visible, ICS hidden, popout visible. Same `WebContext` + same adopted `ICoreWebView2Environment` (shared profile/session/cookies/storage), same document-start facade, same bridge origin gate (exact `https://discord.com`), same `legcord://` registration (per-WebView filters re-registered), same P0.3 permission policy, Discord-clamped navigation (ICS children additionally allow the ICS blob shape), nested popups unconditionally denied. Verified mechanism (A): wry's `Create{webview}` path needs an `EventLoopWindowTarget` the handler lacks, so approved popups deny at that layer and re-create on the event loop; raw `SetNewWindow`/deferral machinery is wry-internal and untouched. Repeated requests focus the live child (no window stacking); child `CloseRequested` clears the slot; parent shutdown drops it.

### Intentional Electron/native differences (hardening, not regressions)

Parsed classification instead of `startsWith` (`https:evil`, `https:///x` never fast-path — raw `://` authority required); popout requires exact host + no explicit port (even `:443`) + no userinfo (`evil@discord.com`, `discord.com:444` rejected; Electron's hostname-only check accepts both); ICS inner origin likewise port/userinfo-checked; `javascript:`/`data:`/`file:`/non-ICS `blob:` always deny (Electron dialogs them and can sticky-allow everything afterwards); unknown schemes deny instead of the modal (safest accepted policy; documented); `window.open` opener linkage is not preserved (managed child is a fresh navigation — the QuickCSS popup flow that needs `window.opener` is a known limitation); ICS has no download manager (hidden child only, parity unclaimed until runtime proves the flow).

### Bundles list fix (rides with P0.4)

`native/web/compatibility.js::listBundles` sent `backup_list_bundles`; the protocol (`protocol.rs::BridgeCommand::BundlesList`, `rename_all="snake_case"`) only ever accepted `bundles_list`, and the response validator already expected `bundles_list`. One-line facade correction; the old string stays a host-side deserialization error (new `protocol.rs` test pins the rejection). No backup format/inventory/save/restore/Electron change. `window.legcord.backup.listBundles()` now round-trips to the 7-entry `{name, present, bytes}` inventory.

## Folder opening capability (Phase D Step 2)

Four caller-backed folder behaviors migrate to two typed capabilities under `window.legcord.folders`: `folders.open` (wire `folders_open`, facade `folders.open(location)`) reveals exactly one of three fixed roots, and `folders.open_theme` (wire `folders_open_theme`, facade `folders.openTheme(id)`) opens exactly one validated theme directory. No generic open(path), no dialogs, no pickers, no file-in-editor paths, no theme install/edit.

### Electron chain (unchanged)

| Renderer caller | Channel (`send`, return ignored) | Host call | Input |
| --- | --- | --- | --- |
| `settings.openStorageFolder` (`SettingsPage.tsx`) | `openStorageFolder` (`ipc.ts:350`) | `shell.showItemInFolder(<userData>/storage/)` | none (fixed) |
| `settings.openThemesFolder` (`ThemesPage.tsx`) | `openThemesFolder` (`ipc.ts:159` **and** `:353` — registered twice, both fire identically) | `shell.showItemInFolder(<userData>/themes/)` | none (fixed) |
| `plugins.openFolder` (`PluginsPage.tsx`) | `openRuntimePluginsFolder` (`ipc.ts:359`) | `shell.showItemInFolder(<userData>/plugins/)` | none (fixed) |
| `themes.folder(id)` (`ThemesCard.tsx`) | `openThemeFolder` (`ipc.ts:201`) | `shell.openPath(path.join(themesPath, id))` | raw theme id, **no validation** (`../` escapes, absolute wins in `path.join`) |

`openPluginsFolder` (`extensions/`) and `openCrashesFolder` have no renderer caller and no bridge entry — deferred. `openQuickCss`/`editTheme` open files in an editor, `openImportPicker`/`openCustomIconDialog` are picker dialogs, `openExternal` is browser navigation — all separate milestones. `native/tests/electron-compat.test.mjs` asserts every handler, bridge mapping, and declared `void` contract above is byte-identical, including the double `openThemesFolder` registration and the raw `path.join`.

### Native chain

```text
renderer await window.legcord.folders.open("themes")
  → requireCapability("folders.open") or sync UNSUPPORTED_CAPABILITY throw
  → client enum check (storage|themes|plugins, TypeError before dispatch)
  → {"type":"folders_open","payload":{"location":"themes"}}
     (unknown variants + any path/url/selector member host-rejected)
  → Rust folders::ensure_folder_root (create-if-missing, canonical containment)
  → reveal_folder: SHOpenFolderAndSelectItems(parent, child) — parent opens
     with the root selected (showItemInFolder-equivalent, direct shell API)
  → validated folders_opened { location } (echo must equal the request)

renderer await window.legcord.folders.openTheme("my-theme")
  → requireCapability("folders.open_theme") or sync UNSUPPORTED_CAPABILITY throw
  → client id check ([A-Za-z0-9._-]{1,64}, never ./.., TypeError before dispatch)
  → {"type":"folders_open_theme","payload":{"id":"my-theme"}}
  → Rust folders::resolve_theme_folder (themes::validate_theme_id, host join,
     canonical containment; missing → UNKNOWN_THEME, never created)
  → open_folder: ShellExecuteW("open", dir) — Explorer opens inside the
     folder (openPath-equivalent for a directory, direct shell API)
  → validated theme_folder_opened { id } (echo must equal the request)
```

Reveal and open are different shell operations and are implemented, logged (`op=reveal` vs `op=open`), and tested as such — never described as one operation just because both show Explorer. Neither constructs a command line; both dispatch the verb and return immediately without blocking on the opened application. Non-Windows builds fail closed (`FOLDERS_UNSUPPORTED_PLATFORM`); Windows WebView2 is the target platform.

Deliberate Electron differences (hardening, not regressions): closed enum instead of fixed calls scattered across channels (same three roots, one capability); theme ids validated with the exact `themes.rs` rule instead of raw `path.join`; canonical containment with planted links failing closed as `OUTSIDE_ROOT` instead of followed; errors are typed (`INVALID_THEME_ID`, `UNKNOWN_THEME`, `OUTSIDE_ROOT`, `REVEAL_FAILED`, `OPEN_FAILED`, `FOLDERS_UNAVAILABLE`) with no paths in messages instead of silent no-ops; responses echo the validated target. Shell display strings use the plain Win32 form: containment is verified on the canonical path, but the `\\?\` prefix is stripped for the shell call because verb handlers such as `SHParseDisplayName` reject it with `E_INVALIDARG` (found during DEV runtime validation).

### Settings UI wiring (Phase D Step 2 follow-up)

The four settings-UI actions dispatch to the native capabilities when the WebView2 shell grants them, keeping the Electron `send` path otherwise. Shared helpers in `src/shelter/settings/settings.ts` (`revealFolderNative("storage" | "themes" | "plugins")`, `openThemeFolderNative(id)`) reuse the existing `isNativeWebView()` check plus a namespace function-presence gate — no second detection mechanism — and return `true` when native handled the action, so each call site executes exactly one path. The theme id passes through verbatim; validation stays host-owned. Rejections are swallowed after a path-free `console.warn` (fire-and-forget parity with Electron's silent `send`, but never an unhandled rejection and never a path in the UI). Native-only surface stays in local minimal interfaces per the `setBlockPowerSavingInVoiceChat`/`getVersion` precedent, so `legcordWindow.d.ts` is unchanged. Wired callers: `SettingsPage.tsx` (storage), `ThemesPage.tsx` (themes), `PluginsPage.tsx` toolbar + empty-state (plugins), `ThemesCard.tsx` per-theme folder (id). The main IPC handlers, bridge mappings, and declared `void` contracts are untouched.

## Theme install capability (Phase D Step 3a)

`themes.install_code` (wire `theme_install_code`, facade `window.legcord.themes.installFromCode(css)`) installs one BetterDiscord theme from CSS text under the host-controlled themes root. No URL, no path, no filename, no id, no dialog, no fetch: the host derives the id from `@name`, fixes `src.css`, and keeps the theme disabled. No `src/` caller is branched — the Electron theme UI keeps calling `install`/`edit`/`openImportPicker`.

### Electron chain (unchanged)

`src/common/themes.ts::installTheme(linkOrPath)` fetches `http(s)` URLs with global `fetch` (arbitrary hosts, no allowlist) or reads local files, then `installThemeFromCode` parses with `parseBDManifest` (`/**` required; `name` defaults to `"null"`; `enabled: false`; `import` stops the scan), derives `` `${name.replace(" ", "-")}-BD` `` (first space only), and silently overwrites `<themes>/<id>/manifest.json` + `src.css` (also storing `updateSrc` and `supportsLegcordTitlebar`). `editTheme(id)` reads the manifest with raw string interpolation and opens the manifest-named file in the system editor or the internal Monaco editor; `openImportPicker` feeds a filtered file dialog into the same installer. `native/tests/electron-compat.test.mjs` asserts all of this is byte-identical.

### Native chain

```text
renderer await window.legcord.themes.installFromCode(css)
  → requireCapability("themes.install_code") or sync UNSUPPORTED_CAPABILITY throw
  → client non-empty/≤1 MiB check (TypeError before dispatch)
  → {"type":"theme_install_code","payload":{"css":"..."}}
     (url/path/filename/id/destination members host-rejected by deny_unknown_fields)
  → Rust themes::install_theme_from_code (parse → derive → contain → atomic write)
  → validated theme_installed { id, enabled: false, css_bytes } → resolved metadata
```

`parse_bd_manifest` mirrors the JS `/@([^ ]*) (.*)/g` scan byte-for-byte (verified against node: mid-line matches, `@import` break, newline-swallowed keys, kept `\r`, last-wins duplicates). The id is `<first-space-dashed name>-BD` validated by `validate_theme_id`; a missing/empty/evil name is `INVALID_NAME` (Electron's `"null-BD"` fallback deliberately not reproduced). The manifest is the Electron shape minus `updateSrc` (no URL input in this slice; key order normalized — JSON-semantically identical, both loaders order-insensitive) with `enabled: false` always (fresh BD installs start disabled; the native loader skips them until an enable path exists — current migration limitation, not a silent flip). Both files land via sibling-temp + same-directory rename; the existing live watcher observes them and drives the managed CSS lifecycle (verified: a disabled install produces no renderer work; enabling later is a separate slice). Transport note: `theme_install_code` is the single documented exception to the 8 KiB message bound (up to 1 MiB + envelope slack, CSS re-checked at parse); every other command keeps the legacy bound — including the 1 MiB-capable plugin-storage write, whose effective bound therefore stays 8 KiB (pre-existing behavior, unchanged by this slice).

Deliberate Electron differences: validated/rejected names instead of `null-BD`; 1 MiB CSS cap and 128 B/4 KiB header caps (Electron unbounded); atomic writes (Electron in place); no remote fetch, no file dialog, no editor launch; shell display uses the plain Win32 form only where the shell is involved (not in this slice).

## Theme CSS write capability (Phase D Step 3b)

`themes.write_css` (wire `theme_write_css`, facade `window.legcord.themes.writeCss(id, css)`) replaces one theme's CSS text. The renderer supplies only the id and the new CSS: the destination is the manifest-declared file resolved through the filename allowlist, and `manifest.json` is never modified (enablement and metadata preserved byte-for-byte, mirroring Electron's editor save path). No URL, no path, no filename, no destination, no enablement flag, no editor/process launch, no dialog. No `src/` caller is branched.

### Electron edit chain (unchanged)

`editTheme(id)` reads the manifest with raw string interpolation (missing/malformed manifests throw unhandled) and opens the manifest-named file in the system editor (`shell.openPath`, traversal-capable) or the internal Monaco window (`openCssEditor`, which reads/writes the file directly with no ack). `native/tests/electron-compat.test.mjs` asserts this implementation is byte-identical.

### Native chain

```text
renderer await window.legcord.themes.writeCss("my-theme", css)
  → requireCapability("themes.write_css") or sync UNSUPPORTED_CAPABILITY throw
  → client id/css checks (TypeError before dispatch)
  → {"type":"theme_write_css","payload":{"id":"my-theme","css":"..."}}
     (path/filename/url/destination/enabled members host-rejected)
  → Rust themes::write_theme_css (validate id → dir must exist → bounded
     manifest read → filename allowlist → canonical containment → unique-temp
     atomic replace; manifest untouched)
  → validated theme_written { id, enabled, css_bytes } → resolved metadata
```

The write goes through the canonical directory plus one validated component; a pre-planted link at the destination is refused outright (stricter than follow-then-contain); the temp file is uniquely named with create-new semantics so it cannot be pre-planted either. A missing directory is `UNKNOWN_THEME` (never created); a missing/malformed/oversized manifest is `INVALID_MANIFEST`; an evil manifest filename is `INVALID_FILENAME`. The existing live watcher observes the replacement and drives the managed CSS lifecycle in place (no re-append, no duplication). The bounded-CSS transport exception covers this second code-carrying command (comment updated, bound unchanged).

Deliberate Electron differences (hardening, not regressions): validated ids instead of raw interpolation; manifest filenames allowlisted instead of followed raw (Electron's `../planted.css` equivalent is rejected); 1 MiB cap; atomic unique-temp replacement instead of in-place write; typed path-free errors instead of silent/throwing behavior.

## Theme import picker capability (Phase D Step 3c)

`themes.import_files` (wire `theme_import_files`, facade `window.legcord.themes.importFiles()`) opens the host-owned native file picker and installs every selected file through the SAME shared installer as `themes.install_code` (`themes::install_parsed_theme` — exactly one installation implementation underneath both; no parser/manifest/validation/bound/write logic duplicated). No `src/` caller is branched.

### Electron picker chain (unchanged)

`ipcMain.on("openImportPicker")` (`src/discord/ipc.ts:162`): `dialog.showOpenDialog` with no `defaultPath`, translated `title`/`buttonLabel`/`filters[].name` (`dialog-importTheme-title/button/discordStyles/allFiles`), `properties: ["openFile", "multiSelections"]`, filters `scss`/`css` then `*`; on success `for (const file of result.filePaths) { installTheme(file); }` — each file dispatched WITHOUT awaiting the previous one; cancellation returns silently; `installTheme` reads local files unbounded as UTF-8 (or fetches arbitrary remote URLs). `native/tests/electron-compat.test.mjs` asserts all of this is byte-identical.

### Native chain

```text
renderer await window.legcord.themes.importFiles()
  → requireCapability("themes.import_files") or sync UNSUPPORTED_CAPABILITY throw
  → no client checks possible (no input); 120 s long-operation timeout
  → {"type":"theme_import_files","payload":{}}
     (path/filename/url/directory/filter/option members host-rejected)
  → dedicated STA worker (COM once per thread): IFileOpenDialog
     (translated title + Import label, styles filter first, all-files
     second, multi-select, no default path) → GetResults in order
  → per file: link refusal → 1 MiB pre-check → capped read + post re-check
     → UTF-8 → parse_bd_manifest → install_parsed_theme (sequential,
     continue-after-failure) → validated theme_imported summary
```

The dialog is the Windows Common Item Dialog (`IFileOpenDialog` with `SetTitle`/`SetOkButtonLabel`/`SetFileTypes`/`FOS_ALLOWMULTISELECT` + `GetResults`), never the legacy `GetOpenFileNameW` path and never a third-party dialog dependency — entirely host-side on a named worker (`legcord-theme-import-sta`) so the WebView/UI thread never blocks. The renderer sends exactly `{}`. The bounded summary `{ canceled, imported, failed }` carries counts only: no ids, arrays, paths, or OS error text. Cancellation (dismissed dialog or empty result) resolves `{ canceled: true }` — never a rejection, toast, or console error.

Deliberate Electron differences: sequential in-order processing instead of un-awaited dispatch (deterministic, bounded memory; documented); symlinks/junctions/reparse points refused outright instead of followed; 1 MiB + UTF-8 + race re-check instead of unbounded reads; no remote URL fetch and no network surface; theme id exclusively from `@name` through `validate_theme_id` (the source filename never becomes the id). Watcher composition is unchanged: the existing live watcher observes the atomic writes and drives the managed CSS lifecycle (a disabled fresh install correctly yields zero renderer work).

## Theme enable flip capability (Phase D Step 3d-1)

`themes.set_enabled` (wire `theme_set_enabled`, facade `window.legcord.themes.setEnabled(id, enabled)`) flips one theme's `manifest.enabled` flag. The renderer supplies only the id and the boolean: the manifest path is derived from the validated id under the host-controlled themes root, and ONLY `enabled` changes — every other field, known or unknown, is preserved by editing the parsed JSON document in place. No `src/` caller is branched.

### Electron toggle chain (unchanged)

`ThemesCard.toggleTheme` → bridge `themes.set(id, state)` (`ipcRenderer.send("setThemeEnabled")`, fire-and-forget) → `setThemeEnabled(id, enabled)`: re-reads the manifest, and only when the state differs pushes `addTheme(id, css)` / `removeTheme(id)` per window; persists ONLY `manifest.enabled`; then the UI re-reads the list via sync `getThemes`. Switches are per-theme and independent: multiple themes may be enabled simultaneously, nothing disables another theme, and no reload occurs. `native/tests/electron-compat.test.mjs` asserts all of this is byte-identical.

### Native chain

```text
renderer await window.legcord.themes.setEnabled("my-theme", true)
  → requireCapability("themes.set_enabled") or sync UNSUPPORTED_CAPABILITY throw
  → client id/boolean checks (TypeError before dispatch)
  → {"type":"theme_set_enabled","payload":{"id":"my-theme","enabled":true}}
     (path/filename/manifest/css/url/destination members host-rejected)
  → Rust themes::set_theme_enabled (validate id → dir must exist →
     canonical containment → bounded no-follow manifest read → object with
     bool-or-absent enabled → no-op when already equal → else flip only
     `enabled` + atomic unique-temp replace)
  → validated theme_enabled { id, enabled } (echo must equal the request)
```

The flip never touches styles directly: the existing live watcher observes the manifest rewrite, re-reads the snapshot, and the managed CSS lifecycle injects/updates/removes the theme's style node (exactly one per theme) with cascade order repair. Same-state requests return without rewriting, so no watcher event fires and no CSS work follows. Manifest links are refused by the no-follow read; directory links resolving outside fail closed; unknown themes and malformed/oversized manifests are typed path-free errors.

Deliberate Electron differences (hardening, not regressions): validated ids instead of raw interpolation; canonical containment with planted links failing closed; 64 KiB manifest cap; atomic unique-temp replacement instead of in-place write; typed errors instead of silent/throwing behavior; a wrong-typed stored `enabled` is rejected rather than trusted.

## Themes UI wiring (Phase D Step 3d-2 UI)

The `ThemesPage`/`ThemesCard` renderer UI dispatches to the already-proven native capabilities. No new filesystem logic, no new theme-management capability, no second CSS path: the host watcher keeps owning CSS application, and the UI keeps owning only inventory display.

### Dispatch table (exactly one path per action)

| Action | Native (capability present) | Electron (capability absent) |
| --- | --- | --- |
| Page inventory | `loadNativeThemes()` over `themes.listThemes()`, scroll-preserving `store.themes` replace | `refreshThemes()` over sync `getThemes()` |
| Manual refresh | same native reload | `window.legcord.themes.refresh()` |
| Toggle | `setNativeThemeEnabled(id, state)` over `themes.setEnabled(id, state)` + reconcile reload | `window.legcord.themes.set(id, state)` + sync `refreshThemes()` |
| Import | `importNativeThemes()` over `themes.importFiles()` + reload when `imported > 0` | `window.legcord.themes.openImportPicker()` |
| Theme folder | `openThemeFolderNative(id)` (unchanged) | `window.legcord.themes.folder(id)` (unchanged) |
| URL install | hidden in native mode | `window.legcord.themes.install(url)` |
| Edit / uninstall | hidden in native mode (no placeholder RPC) | unchanged Electron calls |
| Update from URL | hidden (native inventory never carries `updateSrc`) | unchanged |
| Quick CSS switch / open | disabled in native mode | unchanged |

Gating reuses the existing `isNativeWebView()` check plus per-operation function-presence checks (`isNativeThemeListAvailable`, `isNativeThemeToggleAvailable`, `isNativeThemeImportAvailable`, `isNativeThemesMode`) in `src/shelter/settings/settings.ts`. No window-type, user-agent, Electron, or WebView2 detection was added. Native list entries validate to `{ id, name, author, description, version, enabled, theme: "" }` (`theme` is type-required but never consumed by the card; no CSS/filesystem path is invented). Malformed entries are skipped, never crashing the settings page.

### State flow

Initial mount loads asynchronously (`onMount` + a synchronous `refreshThemes()` only on the Electron path, so first paint never blocks on the bridge); toggle applies an optimistic switch, awaits the host flip, then reconciles from a fresh `listThemes()` (reverting on failure); import reloads only when the summary reports `imported > 0` (cancellation and host failures stay silent with a path-free warning, never an unhandled rejection). No polling, no retry loops. The native watcher — not React — applies/removes CSS; one toggle performs exactly one `setEnabled` plus one inventory reload, never native + Electron together. `native/tests/electron-compat.test.mjs` asserts every Electron string above is still present and that no `install_code`/`write_css`/generic `invoke`/`sendSync` surface entered `src/`.

### Deferred (explicitly not migrated)

Edit, system/Monaco editor, uninstall, remote URL install/update, Quick CSS management, `updateSrc` carriage. The UI hides/disables these in native mode instead of inventing RPCs for them.

## Shelter loader probe (3d-2 unblock probe)

A DEV-only probe (`LEGCORD_NATIVE_LOADER_PROBE=1`, typed `shelter_loader_probe` event) verifies the exact upstream `shelter.plugins` entry points a future native settings-plugin loader needs: `addLocalPlugin`, `startPlugin`, `stopPlugin`, `removePlugin` must exist as functions, and the URL-based `ensureLoaderPlugin` must be absent (it is omitted from upstream's public plugins object, so no URL-selected loading surface is reachable). The probe then round-trips one synthetic plugin with host-fixed id/source/manifest through add/start/stop/remove and verifies no residue remains, so document reloads never duplicate it (a leftover id is removed first). No settings code executes, nothing is fetched, no renderer input is accepted anywhere in the chain, and the report carries only stage strings and booleans. The single eval in the chain is upstream Shelter's own plugin executor over the fixed bytes. This probe deliberately does NOT load the real settings plugin: executing it still requires the sync boot snapshot (`getConfig`/`getThemes`/translations) plus the `ts-out` build artifact, which remain scoped future work (see the 3d-2 runtime-blocker record).

## Reusable mod substrate (Phase B)

`native/src/mod_substrate.rs` owns every behavior identical for all client mods; it names no mod. The typed adapter contract is `ModDescriptor`: stable `id`, boot `capability`, fixed JS URL and cache file, an `Option<ModCssSource>` stylesheet contract, allowed redirect hosts, JS byte limit, `ModArtifactIdentity`, page-world global name and required member, `PageGlobalReadiness`, `ModEnabling`, success marker, an `Option<&str>` settings key, bootstrap DOM event name, storage-context global name, and log tag. `main.rs` registers `VENCORD_MOD`, `SHELTER_MOD`, and `EQUICORD_MOD` in `KNOWN_MODS`; lookup is by exact id only.

Three of those fields are policy enums rather than strings, because three real artifacts differ in kind and forcing them into one shape would create meaningless values:

- `ModArtifactIdentity::{BuildHeader, SourceSentinels}` — publish a banner or not.
- `PageGlobalReadiness::{Synchronous, Awaited{ready_global, timeout_ms, poll_ms}}` — publish the global during bundle evaluation or after an async init.
- `ModEnabling::{AlwaysEnabled, SettingsMods}` — gate on `settings.mods` or not.
- `css: Option<ModCssSource>` and `settings_key: Option<&str>` — ship a stylesheet / use `localStorage`, or genuinely do not.

`validate_descriptor` is a fail-fast structural check that every registered descriptor passes at startup and in tests: names spliced into generated JavaScript must be plain identifiers or bounded safe tokens, cache file names must be bare names, and readiness bounds must be a coarse bounded poll. A malformed descriptor can never reach a script builder.

The substrate additionally owns `ModCache` (in-memory validated bundles keyed by mod id), `DocumentModLifecycle` (document generation plus per-mod one-start/one-execution guards cleared on navigation), `ModHostMetrics` (per-mod request/bootstrap/duplicate counters), and the fixed script builders for CSS insertion, storage-context snapshot, execution verification, and the async page-global trap.

`native/web/mod-substrate/` mirrors the split: `storage-preservation.js` keeps `Window.prototype.localStorage` reachable after Discord deletes its own property (shared by every mod, covered by a dedicated regression test); `mod-bootstrap.js` exposes `window.__legcordNativeModSubstrate.createModAdapter(config)`, which owns origin gating, once-per-document loading, request correlation, bootstrap verdict handling, typed result reporting, and adapter error reporting. Mod adapters supply only identity plus hooks (`requestBundle`, `validateResponse`, `resultExtra`, `resultPayloadExtra`, `responseExtra`), plus an optional `modId` that is echoed on every typed host event when the mod-agnostic event family is used.

`native/web/mods/testmod/adapter.js` remains a test-only synthetic consumer. It is never included in the document-start script, never touches Discord UI, and never submits account state. It exists to exercise the substrate against a shape that differs from both real consumers (banner identity, stylesheet, synchronous global, `localStorage` settings key, settings-gated) — the opposite combination from Shelter.

## Settings capability (Phase C)

The first real mod-facing `window.legcord` capability migrates `settings.getConfig()` from synchronous Electron IPC to typed async WebView2 IPC. Exactly one real caller is migrated: the Shelter power plugin's `blockPowerSavingInVoiceChat` read (`src/shelter/power/index.ts`). All other `settings.getConfig()` callers (Shelter settings, titlebar) keep the unchanged synchronous Electron implementation.

### Electron chain (unchanged)

`bridge.ts` exposes `getConfig: () => ipcRenderer.sendSync("getEntireConfig")`; `ipc.ts` answers `getEntireConfig` with `getEntireConfig()` (the cached `Settings` object from `<userData>/storage/settings.json`). Neither file is modified by Phase C; `native/tests/electron-compat.test.mjs` asserts the synchronous contract.

### Native chain

Renderer `window.legcord.settings.getConfig()` → parameter-free `settings_get_config` request → Rust `settings::load_settings` reads the host-built `<DEV profile>/storage/settings.json` into the typed `SettingsSnapshot` (`native/src/settings.rs`, Electron-compatible camelCase keys, defaults mirroring `src/common/config.ts`) → validated `settings_config` response → renderer schema check → resolved snapshot object. Missing file returns compiled-in defaults (`source: "defaults"`, nothing is written); corrupt file returns typed `SETTINGS_UNAVAILABLE`; unknown future keys are ignored; wrong field types are rejected. There is no `settings.setConfig`, no key argument, no path argument, and no generic invocation.

The facade resolves the snapshot object directly (same shape as Electron's return), never blocks document startup, and uses the shared pending map: correlation by request ID, timeout rejection, and `pagehide` disposal. The Shelter power plugin branches on `window.legcord.boot.runtime === "webview2"`: native uses `await getConfig()` (subscription installed after resolve), Electron keeps the direct synchronous call.

## Settings mutation capability (Phase C-3)

The write half of the settings capability migrates ONE real mutation — `blockPowerSavingInVoiceChat` — from synchronous Electron IPC to a typed async WebView2 capability. Exactly one real caller is migrated: the Shelter power-panel switch (`src/shelter/settings/pages/SettingsPage.tsx` via `setBlockPowerSavingInVoiceChat` in `src/shelter/settings/settings.ts`). There is deliberately NO generic `setConfig(key, value)` on the native bridge.

### Why this setting

- It is the same key the already-migrated Shelter power read path consumes (`src/shelter/power/index.ts` reads only `blockPowerSavingInVoiceChat`), so read + write now cover one coherent capability pair.
- Boolean type → strictest possible value validation; no enum/string parsing risk.
- Real Shelter UI caller exists (power-panel `SwitchItem`); the Electron branch of that caller is preserved byte-for-byte.
- No window-chrome/restart-critical side effects in the native shell (Electron marks restart-required; the native path persists and updates local store without recreating any window).

### Electron chain (unchanged)

`bridge.ts` exposes `setConfig: (key, value) => ipcRenderer.sendSync("setConfig", key, value)`; `ipc.ts` answers `setConfig` with `setConfig(key, value)` (in-memory cache update + full-file `writeFileSync` of `<userData>/storage/settings.json`) and returns `undefined` synchronously. No validation, no return value, no change events, no re-read; callers (notably Shelter `settings.ts`, which updates `store.settings` locally first) fire-and-forget. None of these files are modified by Phase C-3; `native/tests/electron-compat.test.mjs` asserts the synchronous contract.

### Native chain

Renderer `await window.legcord.settings.setBlockPowerSavingInVoiceChat(value)` → `requireCapability("settings.set_block_power_saving_in_voice_chat")` → bool-only `settings_set_block_power_saving` request (non-boolean values rejected client-side; key/path/object members rejected host-side by `deny_unknown_fields`) → Rust `settings::apply_block_power_saving` loads the authoritative settings (missing file → defaults; corrupt file → typed `SETTINGS_UNAVAILABLE`, never overwritten), applies ONLY the allowlisted field (unknown future keys preserved via JSON-document merge), persists atomically (sibling temp file + same-directory rename), and returns the validated `settings_mutation { block_power_saving_in_voice_chat, source: "file", payload_bytes }` response → renderer schema check → resolved boolean. The migrated Shelter caller assigns the host-confirmed value into `store.settings` (scroll-preserving, in place); a rejected mutation leaves local state untouched. Host dispatches bridge requests on one event-loop thread, so mutations serialize with no locks; rapid sequential writes were tested to leave deterministic valid state with last-write-wins.

## Settings set capability (POST-D4 settings milestone)

The closed `settings.set` allowlist migrates the remaining REQUIRED settings mutations — the ones whose native Settings UI controls are enabled and would otherwise throw on the missing generic `setConfig`. The allowlist is exactly four keys, derived from actual callsites (`src/shelter/settings/settings.ts`, `SettingsPage.tsx`, `SupportBanner.tsx`):

| Key | Value | Why allowlisted |
| --- | --- | --- |
| `mods` | `ValidMods[]` (`vencord`/`equicord`/`custom`/`shelter`, ≤8) | Mod toggles (`toggleMod`) are enabled natively; the mod registry resolves adapters from `settings.mods`. Same mutual-exclusion shape Electron builds. Takes effect on next launch (restart-required model). |
| `supportBannerDismissed` | `bool` | Banner is rendered natively; its dismiss button would otherwise throw. Pure renderer UI state. |
| `showExperimentalPluginMenu` | `bool` | Reachable toggle; read by the settings plugin at section registration from the boot snapshot. Takes effect on next launch. |
| `extendedPluginAbilities` | `bool` | Reachable toggle; persisted for Electron parity (it gates `pluginWriteFile`/`pluginReadFile` there). The native plugin-storage capabilities are already capability-gated, so this carries no host side effect beyond persistence. |

Every other `Settings` field stays out: window chrome/tray/startup keys are ELECTRON-ONLY (no native window shell consumes them yet); proxy/bundle/icon/argument keys are UNSAFE from the renderer (URL/path/CLI-switch/proxy injection); voice/capture/RPC/keybind keys are DEFERRED with their milestones (`keybinds` additionally carries arbitrary renderer JS); `quickCss`/`csp` are owned by their own native capabilities and the QuickCSS switch is disabled in the native Themes UI, so it has no native caller. `blockPowerSavingInVoiceChat` keeps its existing dedicated capability and is NOT duplicated here.

### Native chain

Renderer `await window.legcord.settings.set(key, value)` → `requireCapability("settings.set")` → client allowlist check (unknown keys, key↔variant mismatches, unknown mod ids rejected before dispatch) → `settings_set` request → host re-validates via `settings::validate_settings_set` (parse-time and dispatch-time) → Rust `settings::apply_settings_set` loads the authoritative settings (missing file → defaults; corrupt file → typed `SETTINGS_UNAVAILABLE`, never overwritten), merges ONLY the allowlisted field into the JSON document (unknown future keys preserved), persists atomically (sibling temp file + same-directory rename), and returns the validated `settings_set { key, value, source: "file", payload_bytes }` response (key echo must equal the request) → renderer schema check → resolved `{ key, value }`. Migrated callers (`toggleMod`, banner dismiss, the two switches) assign the host-confirmed value into `store.settings` in place with scroll preservation; a rejected mutation leaves local state untouched. Live `setLang` and `restart` need no new code: they already exist as narrow one-way events (`settings_boot_set_lang` → `localization::apply_lang` with charset/length validation and atomic write; `settings_boot_restart` → document reload, explicitly NOT Electron relaunch).

## Localization capability (Phase C-2)

The second mod-facing `window.legcord` capability migrates the already-async `getLang(key)` contract to the typed bridge. Exactly one caller is migrated: the invite back-button preload's `invite-goBackToApp` label lookup (`src/discord/preload/inviteBackButton.mts`). No language mutation, no new format, no duplicated database.

### Electron chain (unchanged)

`bridge.ts` exposes `getLang: async (toGet) => await ipcRenderer.invoke("getLang", toGet)`; `ipc.ts` handles `getLang` with `getLang(toGet)` from `src/common/lang.ts`: in-memory cached catalog for the `<userData>/storage/lang.json` selection (default `en-US`, two-letter codes normalized to `xx-XX`), selected catalog with `en-US` fallback merge, and `data[key] ?? ""` — a missing key returns `""`, never rejects. The invite caller itself uses the raw invoke plus `|| "Go back to the Discord app"`; history patching and the JANK-02 interval removal are untouched.

### Native chain

Renderer `window.legcord.getLang(key)` → `requireCapability("localization.get_lang")` → key-only `localization_get_lang` request (empty/oversized/non-string keys rejected client-side and host-side) → Rust `localization::lookup` over the compile-time-embedded `assets/lang/*.json` catalogs (via `native/build.rs`) with the identical selection/fallback/empty-string rules, selection re-read per request from `<DEV profile>/storage/lang.json` → validated `localization_string { value, lang }` → resolved string. The invite caller branches on `boot.runtime === "webview2"`; Electron keeps the raw invoke byte-for-byte.

### Required dependency categories

- **Needed to execute Vencord:** one standalone Vencord browser JS bundle, the matching browser CSS bundle, the page-world execution mechanism, and fixed CSS insertion after the JS. The native bridge transports only a typed request/result, not bundle data. (Shelter needs only its JS bundle; Equicord needs JS plus CSS.)
- **Optional Legcord features:** existing `userData` location, cache/ref update policy, broad `window.legcord` API, the runtime-plugin loader, themes, RPC, and all other `ipcRenderer` channels. (Mod enable/disable is honored natively, but only to decide adapter injection — see "Adapter enablement".)
- **Electron-only replacements in this slice:** `app.getPath` plus exact local bundle reads are replaced by the isolated DEV cache and fixed URL retrieval; `ipcRenderer.invoke` by the one typed operation; `webFrame.executeJavaScript` by fixed-source Wry main-world execution; `webFrame.insertCSS` by the one identified style element.
- **Deferred:** ambient `require("electron")`, `window.require`, arbitrary `native.invoke`, arbitrary `eval`/host source execution, `contextBridge` parity, `process` parity, arbitrary file reads, config migration, generic plugin scripts, custom bundles, and each mod's full settings/update UX.

Limits: a bridge message is at most 8 KiB; a ping nonce is non-empty and at most 128 bytes; CSS probe text is at most 128 bytes; renderer error kind is at most 64 bytes and its message at most 2 KiB; measured bootstrap time must be finite and in `[0, 60000]` ms. There are no arbitrary channels, host method names, path arguments, eval commands, or filesystem operations.

## App version capability (Phase C-4)

The fourth and final Phase C mod-facing `window.legcord` capability migrates the synchronous `window.legcord.version` read to the typed async bridge. Exactly one caller is migrated: the Shelter update checker's local-version comparison (`src/shelter/settings/components/HeroUpdater.tsx`). AboutPopup's render-time version label keeps the unchanged synchronous Electron read.

### Why this contract

Every remaining `window.legcord` read was traced before choosing. `rpc.getBlacklist` sits on the hot RPC `listen()` path and inside synchronous Solid memos, carries an unbounded array plus mutation/event pairing, and is filesystem-backed. `translations` is a whole-catalog payload. `platform`/`osRelease` callers are synchronous render conditions. `isPowerSavingEnabled` has no renderer caller and needs an OS power API. `dumpFlags` has no caller. `getDetectables`/`getProcessList` are unbounded OS-scanned arrays. The version read is the smallest safe migration: one short string, a compile-time source, an already-async caller, no parameters, no filesystem, no native UI dependency.

### Electron chain (unchanged)

`bridge.ts` exposes `version: ipcRenderer.sendSync("get-app-version", "app-version")`; `ipc.ts` answers `get-app-version` with `getVersion()` from `src/common/version.ts` (`isDev` → `"0.0.0"`, otherwise `app.getVersion()` from `package.json`). The declared `LegcordWindow.version: string` contract is untouched; `native/tests/electron-compat.test.mjs` asserts all three.

### Native chain

Renderer `await window.legcord.native.getVersion()` → `requireCapability("app.get_version")` → parameter-free `app_get_version` request (any channel/key/path member rejected by `deny_unknown_fields`) → Rust `app::current_app_version()` (the `package.json` version embedded at compile time by `native/build.rs`; debug builds report `"0.0.0"` exactly like Electron `isDev`) → validated `app_version { version }` response (non-empty, at most 64 UTF-8 bytes, both sides) → resolved string. There is no runtime source to be missing or corrupt: a missing or malformed `package.json` version fails the build, and the host validates the embedded constant at startup. The HeroUpdater caller branches on `boot.runtime === "webview2"`; Electron keeps the direct `window.legcord.version` read. No `version` field, `sendSync`, generic invoke, or filesystem capability exists on the native facade.

### Implemented `window.legcord`

The exact facade is `window.legcord = Object.freeze({ boot, native, settings, getLang, pluginStorage })`, installed in the document-start script only when `location.origin === "https://discord.com"`.

| Property | Kind | Behavior |
| --- | --- | --- |
| `window.legcord.boot.runtime` | immutable boot snapshot | `"webview2"` |
| `window.legcord.boot.platform` | immutable boot snapshot | `"win32"`; the native prototype is Windows-only |
| `window.legcord.boot.protocol_version` | immutable boot snapshot | `1` |
| `window.legcord.boot.capabilities` | immutable boot snapshot | `["bridge.ping", "mod.get_bundle", "settings.get_config", "settings.set_block_power_saving_in_voice_chat", "settings.set", "localization.get_lang", "app.get_version", "plugin_storage.read", "plugin_storage.write"]` |
| `window.legcord.native.ping(nonce)` | async native request | Returns `{ nonce, message: "pong" }`; rejects malformed input, transport errors, native errors, timeout, or document disposal |
| `window.legcord.native.getModBundle(modId)` | registry-scoped async request (one logical id) | Resolves the registered mod's bundle operation for `vencord`, `shelter`, or `equicord`; typed size/version/timing/marker metadata only after the host executes the bundle and, for the mods that declare one, applies CSS. Rejects unregistered ids with `UNKNOWN_MOD`, malformed ids before dispatch, timeout, or document disposal |
| `window.legcord.native.getVencordBundle()` | thin Vencord convenience wrapper over `getModBundle("vencord")` | Same response, fixed id |
| `window.legcord.native.supports(name)` | renderer-local | Boolean membership test against the frozen boot capability list |
| `window.legcord.native.requireCapability(name)` | renderer-local | Returns `true` when present; otherwise throws with `code === "UNSUPPORTED_CAPABILITY"` |
| `window.legcord.native.reportCss(value)` | typed one-way diagnostic event | Sends the native prototype's CSS injection probe result; not a general CSS API |
| `window.legcord.settings.getConfig()` | explicit settings-only async request (Phase C) | Requires the `settings.get_config` capability (`UNSUPPORTED_CAPABILITY` otherwise). Sends the parameter-free `settings_get_config` command and resolves the validated `Settings` snapshot object (Electron-compatible keys). Rejects malformed snapshots, `SETTINGS_UNAVAILABLE` host failures, timeout (default 5 s), or document disposal. No synchronous facade exists |
| `window.legcord.settings.setBlockPowerSavingInVoiceChat(value)` | explicit allowlisted settings-mutation async request (Phase C-3) | Requires the `settings.set_block_power_saving_in_voice_chat` capability (`UNSUPPORTED_CAPABILITY` otherwise). Sends exactly one boolean (`settings_set_block_power_saving`, non-boolean client-rejected) and resolves the host-confirmed applied value from the validated `settings_mutation` response (`source` must be `"file"`). Rejects malformed responses, `SETTINGS_UNAVAILABLE` host failures (missing/unreadable/corrupt file, write failure), timeout (default 5 s), or document disposal. No generic `setConfig` exists on the native facade |
| `window.legcord.settings.set(key, value)` | explicit closed-allowlist settings-set async request (POST-D4) | Requires the `settings.set` capability (`UNSUPPORTED_CAPABILITY` otherwise). Sends exactly one allowlisted key (`mods`, `support_banner_dismissed`, `show_experimental_plugin_menu`, `extended_plugin_abilities`; unknown keys client-rejected) plus its bounded value (`ValidMods`-only array ≤8, or boolean matching the key; mismatches client-rejected) as `settings_set` and resolves `{ key, value }` from the validated `settings_set` response (key echo must equal the request, `source` must be `"file"`). Rejects malformed responses, `SETTINGS_UNAVAILABLE`/`INVALID_SETTINGS_SET` host failures, timeout (default 5 s), or document disposal. No generic `setConfig`, no paths, no URLs, no objects |
| `window.legcord.getLang(key)` | explicit localization-only async request (Phase C-2) | Requires the `localization.get_lang` capability (`UNSUPPORTED_CAPABILITY` otherwise). Sends exactly one bounded key (`localization_get_lang`, non-empty, at most 128 UTF-8 bytes; client- and host-validated) and resolves the localized string — `""` for a missing key, exactly like Electron. Rejects malformed responses, host failures, timeout (default 5 s), or document disposal. No synchronous facade exists |
| `window.legcord.native.getVersion()` | explicit app-version async request (Phase C-4) | Requires the `app.get_version` capability (`UNSUPPORTED_CAPABILITY` otherwise). Sends the parameter-free `app_get_version` command and resolves the application version string (`"0.0.0"` on dev builds, otherwise the `package.json` version, exactly like Electron `getVersion()`). Rejects empty/oversized versions (at most 64 UTF-8 bytes), host failures, timeout (default 5 s), or document disposal. No synchronous `version` field exists on the native facade |
| `window.legcord.pluginStorage.readFile(pluginId, relativePath)` | explicit scoped plugin-storage read (Phase D) | Requires the `plugin_storage.read` capability (`UNSUPPORTED_CAPABILITY` otherwise). Sends exactly a logical plugin id (`[A-Za-z0-9_-]{1,64}`, client- and host-validated) plus a logical relative path (non-empty, at most 256 UTF-8 bytes client-side; full traversal/drive/UNC/device/quirks policy host-side) as `plugin_storage_read` and resolves the file text. Rejects malformed responses (data must be a string within 1 MiB with matching byte count), typed host failures (`NOT_FOUND` for missing files like Electron, `INVALID_PATH`/`INVALID_PLUGIN_ID`, `EXTENSION_DISABLED`, `SETTINGS_UNAVAILABLE`, `TOO_LARGE`, `READ_FAILED`), timeout (default 5 s), or document disposal. No paths, roots, or metadata cross the boundary |
| `window.legcord.pluginStorage.writeFile(pluginId, relativePath, data)` | explicit scoped plugin-storage write (Phase D) | Requires the `plugin_storage.write` capability (`UNSUPPORTED_CAPABILITY` otherwise). Sends the same logical addressing plus UTF-8 text `data` (at most 1 MiB, client- and host-validated) as `plugin_storage_write` and resolves the persisted byte count. Same rejection taxonomy as the read (`WRITE_FAILED` instead of `READ_FAILED`). No generic `window.legcord.fs` object exists anywhere on the native facade |

No other existing Electron-facing `window.legcord` methods are implemented by this facade. Boot snapshot values are native runtime metadata only; they do not impersonate Legcord config, OS release, translations, or Electron version. The one exception is the app version itself, which is the authoritative `package.json` value (or `"0.0.0"` on dev builds), not an invented substitute.

No other existing Electron-facing `window.legcord` methods are implemented by this facade. Boot snapshot values are native runtime metadata only; they do not impersonate Legcord config, Legcord version, OS release, translations, or Electron version.

### Classification of the existing renderer contract

The table lists the exposed `LegcordWindow` groups from `src/@types/legcordWindow.d.ts` and the behavior actually wired by `src/discord/preload/bridge.ts`. Existing API calls must not be redirected to an untyped generic bridge.

| API / caller | Sync or async; arguments / return | Side effects and boot-critical status | Migration decision |
| --- | --- | --- | --- |
| `window` — preload titlebar and Shelter titlebar | `show/hide/minimize/maximize/unmaximize/quit(): void`; `maximized()/isNormal(): boolean`; no arguments | Native window lifecycle; custom titlebar is not needed to open Discord | Deferred native commands; dynamic state is asynchronous or event-cached, never sync IPC |
| `settings.getConfig()` — Shelter settings/power/titlebar | Synchronous; no arguments; full `Settings` object | Reads mutable app settings; not needed for bare Discord, but current UI assumes synchronous state | Populate an initial documented config snapshot only if native settings support is added; migrate updates to async |
| `settings.setConfig(key,value)` — Shelter settings | Synchronous; key/value; handler returns `undefined` after persisting | Mutates config; not boot-critical | Phase C-3 ports exactly one mutation (`blockPowerSavingInVoiceChat` via the bool-only `settings_set_block_power_saving` command and `settings.set_block_power_saving_in_voice_chat` capability, atomic host-controlled persist, host-confirmed value back). POST-D4 adds the closed `settings.set` allowlist (`settings_set` command, `settings.set` capability) for the four REQUIRED keys (`mods`, `supportBannerDismissed`, `showExperimentalPluginMenu`, `extendedPluginAbilities`) with per-key typed values, key echo, and the same atomicity/confirmed-value contract. All other keys keep the sync Electron implementation; no generic native writer exists |
| `settings.addKeybind(keybind)`, `toggleKeybind(id)`, `removeKeybind(id)` | Fire-and-forget; typed keybind or ID; `void` | Mutate settings and refresh global keybinds | Deferred |
| `settings.openStorageFolder/openThemesFolder/openCustomIconDialog/copyDebugInfo/copyGPUInfo/openWebRTCInternals/openGPUInfo` | Fire-and-forget; no args; `void` | Opens filesystem locations/dialogs/internal pages or writes clipboard | Deferred; host-owned dialogs/paths only |
| `settings.setLang(lang)` | Fire-and-forget; language string; `void` (type declaration says `() => void`, implementation sends without returning) | Changes app language | Deferred async request |
| `settings.dumpFlags()` | Synchronous; no args; applied flags object | Logs and returns runtime flags; not boot-critical | Async diagnostics request or native-local equivalent |
| `touchbar.setVoiceTouchbar(state)`, `setVoiceState(mute,deafen)`, `importGuilds(guilds)` — Shelter touchbar | Fire-and-forget; boolean(s) / string array; `void` | Mutates Touch Bar or voice state / imports guilds | Deferred; Touch Bar is platform-specific |
| `power.setPowerSaving(state)`, `isPowerSavingEnabled()` — Shelter power | Fire-and-forget boolean / synchronous boolean read | Mutates/reads native power-saving behavior | Deferred; mutable/dynamic read becomes async or cached event |
| `electron`, `platform`, `osRelease`, `translations` — preload and Shelter | Synchronous properties; respectively Electron version, OS platform/release, raw translation JSON | Snapshot-like values; platform/version/translation are read at preload boot | Only true native runtime metadata is in this prototype snapshot. Electron version, OS release, and translations are not substituted with invented values |
| `getLang(key)` — invite-back preload and UI | Async; translation key string; `Promise<string>` | Reads one localized string; not boot-critical | Phase C-2 ports exactly one key path (`invite-goBackToApp` via `localization.get_lang`, empty-string missing-key parity, embedded catalogs). No language mutation; other callers deferred |
| `version` — Shelter update checker and About popup | Synchronous string property | Snapshot-like value read at preload boot; update comparison is already async | Phase C-4 ports exactly one read path (HeroUpdater's local-version comparison via `app.get_version`, dev `"0.0.0"` parity, compile-time `package.json` source). AboutPopup's render-time label keeps the sync Electron read |
| `screenshare.getSources(callback)`, `start(source,name,audio)`, `venmicStart(include)`, `venmicSystemStart(exclude)`, `venmicList()`, `venmicStop()` — Shelter screenshare | Event subscription callback / fire-and-forget / async promises; source IDs, names, booleans, venmic node arrays; returns callback event, void, or `Promise` values | Capture source selection, native media and audio routing | Deferred; no source IDs, paths, or media authority exposed in this milestone |
| `themes.install(url)`, `uninstall(id)`, `edit(id)`, `getThemes()`, `refresh()`, `openImportPicker()`, `set(id,state)`, `folder(id)`, `openQuickCss()`, `importQuickCss(css)`, `enableQuickCss()`, `disableQuickCss()` | Mixed; install is declared void but implementation returns an async invoke; theme list/refresh are sync arrays; others fire-and-forget | Reads/updates theme files, opens dialogs, edits CSS; not bare-boot-critical | Snapshot only for a proven initial immutable listing; mutations/import are async and host-owned; full theme handling deferred |
| `rpc.refreshProcessList()`, `getProcessList()`, `addDetectable(game)`, `removeDetectable(id)`, `getDetectables()`, `getBlacklist()`, `blacklistGame(name,id)`, `unblacklistGame(id)` — Shelter registered games/RPC | Mixed fire-and-forget, synchronous process/list/cache reads, synchronous blacklist mutation | Reads process list, edits detectable/blacklist state, emits `rpcBlacklistUpdated`; not boot-critical | Process/detectable reads become async or locally cached; blacklist state uses typed events and async mutations |
| `backup.save(data)`, `restore()` — settings | Async; serialized backup string / no args; promise result | Opens save/open dialogs and reads/writes archive contents | Native `backup.save` + `backup.restore` (host-mediated pickers, bounded validate/stage/commit); BackupSection UI wiring deferred |
| `plugins.list()`, `setEnabled(id,enabled)`, `reload(id)`, `openFolder()` — plugins page | Async list/toggle/reload; fire-and-forget folder open | Enumerates and changes runtime plugins | Deferred; no plugin loader is part of the prototype |
| `fs.writeFile(pluginId,relativePath,data)`, `readFile(pluginId,relativePath)` — plugin storage | Async; plugin ID, relative path, text; structured success/error result | Reads/writes scoped plugin storage, gated by user setting and path containment | Phase D ports exactly this pair as typed `plugin_storage.read`/`plugin_storage.write` capabilities under `window.legcord.pluginStorage` (strict id/path validation, canonical containment, 1 MiB bounds, atomic writes, no `window.legcord.fs`). There are no in-repo callers, so no `src/` caller is branched; a DEV-only smoke probe exercises the exact public contract |
| `window.legcordRPC` — `src/discord/rpcProcess.ts` / Shelter RPC | Renderer-side API; receives activity and notifies consumers | Page-world RPC data delivery; not a `LegcordWindow` member | Deferred arRPC integration |
| `window.legcord.channel` — `src/splash/redirect.html` | Synchronous string property | Selects redirect channel on splash page | Not part of the Discord document contract; splash is deferred |

### Direct Electron dependencies in Discord preload

These are separate from `window.legcord`. Only the exact integration operations in `preload/mods/{vencord,shelter,equicord}.mts` have a native replacement; other current preload modules remain unimplemented and are not loaded.

| Caller | Electron dependency and contract | Migration |
| --- | --- | --- |
| `preload/bridge.ts` | `require("electron")`, `contextBridge.exposeInMainWorld`, `ipcRenderer.send/sendSync/invoke/on/removeListener`; defines the broad Electron facade and RPC blacklist cache. Also reads `process.versions.electron` | Only the narrow native boot facade exists; broad `contextBridge`, `process`, and IPC behaviors remain deferred |
| `preload/plugins.mts` | `ipcRenderer.invoke` for preload/renderer runtime plugin entries and source; `webFrame.executeJavaScript`; dynamic `new Function` plugin evaluation | Entire loader deferred; script sources must not be introduced as generic host RPC |
| `preload/mods/vencord.mts` | `require("electron")`; `ipcRenderer.invoke("getVencordBundle")`; `webFrame.executeJavaScript(bundle.js)` in page world, then `webFrame.insertCSS(bundle.css)`. No `contextBridge`, `process`, or direct filesystem API in this module | Replaced by the registry-scoped `mod_get_bundle` command and native one-purpose bundle/CSS injection; nothing is polyfilled |
| `preload/mods/{shelter,equicord}.mts` | `ipcRenderer.invoke` for local bundle reads; `webFrame.executeJavaScript`; Equicord also `webFrame.insertCSS`; Shelter additionally wraps the source in a host-built `SHELTER_INJECTOR_PLUGINS` IIFE and reads `process.platform` | Replaced by the same registry-scoped `mod_get_bundle` command plus native bundle/CSS injection — one descriptor and one thin adapter each. Shelter's injector wrapper is deliberately not reproduced (the `legcord://` protocol and preload plugin loader are deferred, and Shelter skips the injector when that global is absent) |
| `preload/mods/custom.mts` | `ipcRenderer.invoke("getCustomBundle")` for a user-supplied local artifact; `webFrame.executeJavaScript` and optional `insertCSS` | Deferred by design: there is no authoritative host-controlled source for a custom mod, and no renderer-supplied path or URL is permitted |
| `preload/themes.ts` | `ipcRenderer.on("addTheme"/"removeTheme")`; inserts/removes DOM styles | Deferred typed theme events and style lifecycle |
| `preload/titlebar.mts` | `ipcRenderer.sendSync` for OS/config/window state and `send` for window actions | Deferred titlebar; config boot values would need a native snapshot and dynamic state async |
| `preload/newTitlebar.mts` | `ipcRenderer.sendSync` for windowStyle/transparency/OS; `process.platform` | Deferred; no native chrome settings are asserted |
| `preload/patches.mts` | `ipcRenderer.sendSync` for display version and two settings; `process.platform`; DOM injection and timing | Requires a deliberate preload settings/version migration; not loaded |
| `preload/inviteBackButton.mts` | Async `ipcRenderer.invoke("getLang")`, DOM/page-world history patches | Deferred; translation lookup may later be a typed async request |
| all preloads above | `webFrame`, `contextBridge`, Node `require`, `process`, `ipcRenderer` are Electron-only globals, not WebView globals | Explicit adapters or TS migrations per feature; the three client-mod adapters are implemented with no ambient Electron emulation |
| `src/setup/preload.mts` | `contextBridge`, `ipcRenderer.send/sendSync/invoke`; exposes `window.setup` with restart, OS, settings save, and localization calls | Separate setup document; deferred |
| `src/splash/preload.mts` | `contextBridge`, `ipcRenderer.send/sendSync/invoke`; exposes `window.internal` with restart, app/splash metadata, mods, localization, and splash completion | Separate splash document; deferred |
| `src/cssEditor/preload.mts` | `contextBridge`, synchronous CSS read and fire-and-forget CSS write; exposes `window.cssEditor` | Separate editor document and file-backed state; deferred |

### Complete `sendSync` migration table

There are 35 call sites in the current repository search, grouped below by the contract they implement. The listed Electron handlers in `src/discord/ipc.ts` establish return values and side effects; values are not inferred from the TypeScript casts alone.

| Purpose / channels | Callers and returned data | Classification | Migration |
| --- | --- | --- | --- |
| Boot environment: `getOS`, `getOSRelease`, `get-app-version`, `displayVersion` | `bridge.ts`; `titlebar.mts`; `newTitlebar.mts`; `patches.mts`. Returns `process.platform`, `os.release()`, `getVersion()`, `getDisplayVersion()` | Boot snapshot values; safe only if sourced from native runtime/app metadata | Snapshot actual native values; retain separate Legcord app version only when native product version is authoritative. No Electron value is fabricated. Phase C-4 ports exactly one app-version read path (HeroUpdater via async `app.get_version`, compile-time `package.json` source); all other boot-environment reads keep the sync Electron implementation |
| Boot config: `getConfig("windowStyle"|"transparency"|"minimizeToTray"|"sdpH264BaselineRewrite"|"disableAutogain")` | `bridge.ts`, `titlebar.mts`, `newTitlebar.mts`, `patches.mts`. Returns configured typed value; setter elsewhere persists and returns `undefined` | Boot config reads, but current settings can be mutable | Include only proven immutable initial settings in a versioned native snapshot; migrate mutations to async and refresh cache/events |
| Window state: `win-isMaximized`, `win-isNormal` | `bridge.ts`, `titlebar.mts`. Returns current booleans | Dynamic cached-state candidate, not a boot constant | Native state events/local cache or async query; never a sync message |
| Full settings: `getEntireConfig` | `bridge.ts`; returns full `Settings` object | Cached-state candidate with mutable settings | Phase C migrates exactly one read path: the Shelter power `blockPowerSavingInVoiceChat` read is served natively by the async `settings.get_config` capability (validated snapshot, no mutation). All other readers keep the sync Electron implementation |
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
| mod preloads: `getShelterBundle`, `getVencordBundle`, `getEquicordBundle`, `getCustomBundle`; plugins runtime entries/scripts | No args or plugin target/ID; promises yield bundle text/flags or runtime entries/source | Vencord, Shelter, and Equicord all route through the shared `mod.get_bundle` capability described above; custom bundles (no host-controlled source) and runtime plugin entries remain deferred |
| `themes.ts`: `addTheme(name,css)`, `removeTheme(name)`; bridge RPC blacklist update | Event payloads are name/CSS or entry arrays; listener returns unsubscribe for blacklist only | Updates renderer styles/cache; future bridge must use narrow typed subscriptions with lifecycle cleanup |
| `inviteBackButton.mts`: `getLang(key)` | Translation key; promise resolves string or undefined | Async localized text read; deferred typed locale request |

## Plugin storage capability (Phase D)

The first filesystem-scoped native capability migrates the public third-party plugin-storage pair (`window.legcord.fs.writeFile`/`readFile` over `pluginWriteFile`/`pluginReadFile`) to typed async WebView2 capabilities under `window.legcord.pluginStorage`. No generic filesystem RPC exists: the renderer supplies only a logical plugin id plus a logical relative path, and the host builds the single root `<profile>/plugin-storage/<plugin_id>/` itself. Themes, Quick CSS, bundles, backups, folder opening, pickers, directory listing, and arbitrary paths remain deferred.

### Electron chain (unchanged)

`bridge.ts` exposes `fs.writeFile/readFile` as `ipcRenderer.invoke("pluginWriteFile"/"pluginReadFile", pluginId, relativePath, data?)`; `ipc.ts` gates both on `getConfig("extendedPluginAbilities")` (`EXTENSION_DISABLED` otherwise), sanitizes the id (`[^a-zA-Z0-9_-]` stripped, 64 chars, `"default"` fallback), confines the path lexically (`resolve(base, normalize(rel))`, reject `..`/absolute), creates parent directories recursively, and performs unbounded UTF-8 reads/writes (`NOT_FOUND` for missing files). There are no in-repository callers — this is a public third-party plugin API — so no `src/` caller is branched in this milestone; `native/tests/electron-compat.test.mjs` asserts the Electron contract is byte-identical and that no `src/` file references the native `pluginStorage` path.

## Theme injection (Phase D)

Static enabled-theme injection at document startup: the native host discovers `<profile>/themes/<id>/`, reads `manifest.json`, honors `enabled`, resolves the manifest-provided CSS filename under a strict allowlist, and pushes each CSS through the shared managed-CSS lifecycle (`mod_substrate::managed_css_script` / `managed_css_remove_script`) in sorted-id order. There is deliberately NO `themes.getCSS()` bridge, no new protocol command, and no new boot capability: the earlier pull-style read investigation was rejected because current themes are host → renderer pushes with no native renderer caller, and the renderer never receives the absolute theme root.

### Electron chain (unchanged)

`src/common/themes.ts::buildThemeCssCache` iterates the themes folder, reads each `manifest.json` (`getThemeManifest`, mtime-cached), defaults a missing `enabled` via a legacy `disabled.txt` check, and caches `readFileSync(`${themePath}/${manifest.theme}`)` — a raw string join with no filename validation, so `../` and absolute values escape the theme directory. `injectThemesMain` rebuilds the cache at startup and, on `did-finish-load`, pushes Quick CSS first when enabled (`initQuickCss` → `addTheme "legcord-quick-css"` with a 300 ms-debounced file watcher) then one `addTheme (id, css)` per cached theme in discovery order. The renderer (`src/discord/preload/themes.ts` over `src/common/dom.ts::addTheme`) creates one `<style id=name>` per push (duplicate-guarded by id) and `removeTheme` deletes by id. `setThemeEnabled` re-reads/pushes or removes per window and persists `manifest.enabled`. The theme-list watcher (`startThemeWatcher`, 500 ms debounce) refreshes only the manifest list cache, not CSS. Vencord CSS travels separately via `webFrame.insertCSS` at preload time (before `did-finish-load`), so Electron cascade order is Vencord CSS → Quick CSS → themes. `native/tests/electron-compat.test.mjs` asserts this pipeline is byte-identical and that no `themes.getCSS()` pull exists on either side.

### Native chain

On every trusted `PageLoadFinished`, the host runs `themes::load_enabled_themes` (sorted by id; per-theme typed skip reasons so one bad theme never blocks others), removes managed styles for previously injected ids that left the enabled set (idempotent, usually a fresh-document no-op), then evaluates one `managed_css_script(style_id, css)` per theme. Style identity is `legcord-native-theme-<id>` (namespaced; Electron reuses the raw id). No renderer request, response, capability, or path crosses the boundary; a DEV-only `LEGCORD_NATIVE_THEMES_SMOKE=1` observation script reports managed node ids/bytes plus head order and Vencord coexistence through the evaluate callback (never the bridge).

Deliberate Electron differences (hardening, not regressions): ids validated (`[A-Za-z0-9._-]{1,64}`, never joined raw); manifest CSS filenames must be bare `*.css` without separators/drive/UNC/device/quirks (the exact `../planted.css` shape Electron would honor is rejected as `invalid_filename`); lexical single-component safety plus canonical file containment (reads go through the canonical path; planted symlinks/junctions resolve outside the root and are rejected); manifests ≤ 64 KiB and CSS ≤ 1 MiB (Electron unbounded); deterministic sorted-id order instead of discovery order; missing `enabled` defaults to enabled (the legacy `disabled.txt` migration path is not reproduced); empty CSS is skipped rather than pushed. Live watching, install/edit, and folder opening remain deferred; Quick CSS is now implemented below.

## Quick CSS injection (Phase D)

Static Quick CSS startup injection: when the authoritative settings flag is enabled, the native host reads its own fixed `<profile>/quickCss.css` and pushes it through the same shared managed-CSS lifecycle, emitted BEFORE themes to preserve Electron's cascade. There is deliberately NO `quickCss.get()` bridge, no new protocol command, and no new boot capability: like themes, Quick CSS is a host → renderer push with no native renderer caller, and the renderer never receives the file path.

### Electron chain (unchanged)

`src/common/themes.ts::initQuickCss` resolves `<userData>/quickCss.css`, CREATES it empty when missing (`writeFileSync(path, "")`), and unconditionally sends `addTheme ("legcord-quick-css", <utf-8 content>)` — even when empty — so enabled-vs-disabled is style-present vs style-absent. `injectThemesMain` calls it on `did-finish-load` before the theme loop exactly when `getConfig("quickCss")` is truthy (`settings.quickCss`, default `true`, safe-mode `false`); the Themes page toggles via `enableQuickCss`/`disableQuickCss` IPC (re-read + watcher restart, or watcher close + `removeTheme`) plus `setConfig("quickCss", e)`. The live path (`fs.watch` + 300 ms debounce → `removeTheme` + `addTheme`) is deferred natively. `native/tests/electron-compat.test.mjs` asserts this lifecycle and enablement contract are byte-identical and that no Quick CSS pull exists on either side.

### Native chain

On every trusted `PageLoadFinished`, BEFORE the theme block, the host runs `quick_css::load_quick_css`: authoritative `load_settings` → `quick_css` flag (corrupt settings fail closed to disabled, matching Electron's falsy fallback) → missing file is created empty (Electron parity) → 1 MiB bound → UTF-8 → content (possibly empty) or a typed state. When content is present the host evaluates one `managed_css_script("legcord-quick-css", css)`; when disabled/unreadable after a previous injection it evaluates the idempotent `managed_css_remove_script` so no stale style survives a reset. Style identity preserves Electron's `legcord-quick-css` verbatim (it collides with no native style). No renderer request, response, capability, or path crosses the boundary; a DEV-only `LEGCORD_NATIVE_QUICKCSS_SMOKE=1` observation shares the managed-styles script and additionally reports `quickcss_present`, `quickcss_bytes`, and the computed `--legcord-native-order-probe` value through the evaluate callback (never the bridge).

Deliberate Electron differences: 1 MiB bound and explicit UTF-8/invalid states (Electron is unbounded and would throw); corrupt settings are a named `settings_unavailable` state instead of an unlogged falsy read. Everything else is parity: fixed host path, create-when-missing, empty push, verbatim identity, Quick CSS-before-themes emission, disabled-means-absent.

## Live theme and Quick CSS watching (Phase D)

Bounded native filesystem watching replaces both Electron watchers (`initQuickCss`'s per-window `fs.watch` on the single file; `startThemeWatcher`'s recursive list-cache-only watch) with exactly TWO application-owned `notify` watchers that live from startup to shutdown: the profile directory (non-recursive, filtered to the single fixed `quickCss.css` name) and the host-controlled themes root (recursive). No watcher per theme, no watcher per document (reload never creates one), no renderer watching, no `MutationObserver`, no modification-time polling. `native/src/css_watch.rs` owns the lifecycle; `main.rs` only forwards signals, debounces, re-reads, and injects.

### Electron chain (unchanged)

Quick CSS: `initQuickCss` sends `addTheme` unconditionally, then `fs.watch(quickCssPath)` reacts only to `"change"` events with a 300 ms debounce before `removeTheme` + re-read + `addTheme`; `disableQuickCss` and window `"closed"` close the watcher. Delete/rename events are ignored, so a deleted file leaves a stale style. A live update also appends the re-added style at the end of `<head>`, after themes. Themes: `startThemeWatcher` (`fs.watch(themesFolder, recursive)`, 500 ms debounce, `manifest.json`/rename filter) refreshes only the manifest list cache — per-theme CSS edits have NO live path and need an enable toggle or a reload. `native/tests/electron-compat.test.mjs` pins both contracts and asserts the native side exposes no renderer watch surface (`fs.watch`/`watchFile`/`MutationObserver` absent from the facade).

### Native chain

A watcher signal only bumps a per-resource epoch and arms exactly one 300 ms timer per burst (`ResourceDebouncer`: first event spawns, later events only bump). The timer fires once with the latest epoch; the event-loop handler discards it unless both the epoch and the document generation still match (a superseded burst or a mid-debounce reload dies before any filesystem or renderer work). A fresh fire re-reads authoritative state — `quick_css::load_quick_css` (settings flag + fixed file, missing file recreated empty, error states never presented as CSS) or `themes::load_enabled_themes` — diffs against the last-pushed snapshot (`quick_css_transition` / `diff_theme_snapshots`), and issues minimal managed-CSS work: content updates evaluate `managed_css_script` (in place, never re-appended), removals evaluate `managed_css_remove_script`, and ONLY structural changes (Quick CSS recreated, theme added/removed) finish with `managed_css_order_script` over `[legcord-quick-css + sorted theme ids]`. Identical rescans issue zero renderer work. Every fire logs `[QuickCss][Live]` / `[Themes][Live]` (transition, bytes, read/injection latency) plus cumulative `[LiveCss][Metrics]` (`watchers`, `raw_fs_events`, `debounced_reloads`, `css_updates`, `css_removes`, `stale_discarded`, `order_repairs`), so rapid-edit bursts report raw events → coalesced updates. Watcher creation failure only disables live watching (startup injection still runs); the host never crashes on watcher, read, UTF-8, oversize, manifest, or shutdown paths.

Deliberate Electron differences (correctness, not regressions): one persistent watcher set instead of per-window create/close (reconfiguration replaces the handle; document reload never touches it); the themes root is watched for CSS/manifest/dir changes rather than manifest names only (diffing keeps renderer work minimal); rename/delete events are actionable (delete → empty-file recreation like startup; malicious manifest values remove the theme instead of escaping); and content updates never move nodes while structural inserts are repaired back into cascade position (Electron's live re-add drifts to the end of `<head>`).

### Native chain (plugin storage, unchanged)

Renderer `await window.legcord.pluginStorage.writeFile(id, rel, text)` → `requireCapability("plugin_storage.write")` (client id-shape/byte checks) → `plugin_storage_write` request (parser checks presence + byte caps: id ≤ 64, path ≤ 256, data ≤ 1 MiB; unknown members rejected by `deny_unknown_fields`) → Rust `plugin_storage::write_plugin_file` (strict id validation, component-wise path validation rejecting traversal/absolute/drive/UNC/device/quirks, `extendedPluginAbilities` gate from the authoritative settings load, namespace-only directory creation, canonical parent containment resolving symlinks/junctions, atomic sibling-temp + rename persist) → validated `plugin_storage_write { payload_bytes }` → resolved byte count. Reads mirror this through `read_plugin_file` (missing file → typed `NOT_FOUND` like Electron; oversized stored files rejected before buffering; canonical file containment; UTF-8 text only) → validated `plugin_storage_read { data, payload_bytes }` → resolved file text. The facade resolves unwrapped values (string/bytes, matching the `getLang`/mutation precedent) and uses the shared pending map (correlation, 5 s timeout, `pagehide` disposal).

Deliberate Electron differences (hardening, not regressions): ids are validated instead of sanitized (Electron maps `"a/b"` to `"ab"`; native rejects `INVALID_PLUGIN_ID`); paths face the full Windows quirks policy (Electron accepts several); both directions are capped at 1 MiB (Electron is unbounded); writes are atomic (Electron writes in place); a corrupt settings file surfaces `SETTINGS_UNAVAILABLE` (Electron falls back to a near-empty cache and reports disabled). No `%`-decoding is performed anywhere, so encoded traversal is an inert literal filename.

## Explicit capability ownership

The document-start compatibility script owns facade creation, local response correlation, `pagehide` promise cleanup, and renderer diagnostics. The static Vencord adapter owns one request per document and the private execution marker; Rust owns the versioned protocol, exact origin gate, request-ID uniqueness, fixed source retrieval/cache path, page-world execution, CSS insertion, response dispatch, and document generation. The native profile remains exactly `E:\Legcord.webview-dev-user-data`. The renderer supplies no path, URL, channel name, host method, executable source, or filesystem operation beyond the validated plugin-storage logical id and relative path, which never leave the host-built namespace; bundle discovery takes no renderer parameters at all, and folder opening takes only a closed location enum or a validated theme id whose final path the host builds and contains.

### Deferred and not tested

Updater; tray; native menus; downloads; permissions; screen capture; WebRTC/native media; `webRequest`; protocol handlers (`legcord://`); arRPC; popup windows; general mod loader; custom bundle execution; runtime plugins; theme install/edit (folder reveal/open now implemented natively); backup UI wiring (save + restore capabilities done, BackupSection unwired); existing broad `window.legcord` APIs beyond the migrated capabilities; all direct Electron preload globals except the documented mod replacements. Each mod's own settings UI, update/ref-based cache policy, and cross-mod interactions are not implemented, and Shelter's Legcord plugin surface is not implemented. This narrow mod path does not establish full Legcord compatibility.

### Validation status

The Rust substrate, protocol, and JavaScript factory/adapters have focused tests for descriptor registration and per-mod shape, artifact identity validation (banner and sentinel), the no-CSS contract, awaited readiness (trap before the bundle, bounded self-clearing poll after), descriptor structural validation, cache miss/hit, per-mod one-start/one-execution guards, document reset, mod independence, host metrics isolation, origin and capability gates, malformed input, adapter initialization, duplicate bootstrap prevention, cross-mod identity rejection, renderer exception locations/stacks, failure propagation, synthetic third-consumer execution, and storage preservation across `delete window.localStorage`. One Rust test walks two deliberately opposite synthetic shapes (stylesheet + banner + synchronous global + settings key, versus no stylesheet + no banner + awaited global + no settings key) through the identical registration/retrieval/validation/bootstrap/execution/CSS/duplicate/reset/failure lifecycle — that is the substrate-generality proof. `cargo fmt -- --check`, `cargo test` (168 tests), `cargo build`, `cargo clippy --all-targets -- -D warnings`, targeted Biome, `node --test native/tests/compatibility.test.mjs` (117 tests), and `node --test native/tests/electron-compat.test.mjs` (41 tests) pass.

Live validation used only `E:\Legcord.webview-dev-user-data`, one launch plus one reload per mod:

- **Shelter** — retrieved `sha256-ba858e9cfb83503f`, 235,850 B, from the fixed `raw.githubusercontent.com` source (disk-cache hit on the second document); executed in the page world; `window.shelter` appeared asynchronously and the awaited verifier observed it with a `plugins` member (`pageGlobalPresent=true`, `pageGlobalMembers="object"`). `marker=shelter-bootstrap-executed`, `execution_count=1` per document, `css_injection_ms=none` (no stylesheet, no CSS stage), `settings_state=not_applicable` (IndexedDB, not `localStorage`), `localStorage_getItem_at_execution=available`. Two documents, two requests, two markers, zero duplicates, `hasAppMount=true`, no login redirect, zero renderer errors. Retrieval 7.7 ms then 0.000 ms (in-memory); execution call 7.2/6.9 ms; request latency 786/324 ms (dominated by Shelter's own wait for Discord's `#app-mount`).
- **Equicord** — retrieved `1489c0e2a4435d854d7343a5cfb0abb894772318`, 2,516,168 B JS + 217,092 B CSS, from the fixed GitHub release source; executed in the page world and published the `Vencord` global synchronously; the real stylesheet was injected (`stylePresent=true`, `styleBytes=217092`, `css_injection_ms=260/274`). `marker=equicord-bootstrap-executed`, `execution_count=1` per document, `settings_state=valid_json` (`EquicordSettings` present and parseable), `localStorage_getItem_at_execution=available`. Two documents, two requests, two markers, zero duplicates, `hasAppMount=true`, no login redirect. Retrieval 2.5 ms then 0.000 ms; execution call 82.9/62.3 ms; request latency 743/386 ms.

The DEV profile's `settings.json` was backed up before the Equicord run (`mods` flipped to `equicord`) and restored byte-for-byte afterwards (53 keys, `mods` back to its original value). The production profile and installed Electron were never touched.

The authenticated interaction probe verified:
1. Navigation to Discord `/app` with authenticated DEV profile.
2. Selection and opening of an existing DM channel (`channel_navigation_requested`).
3. Message composer detection and focus (`composer_focused`).
4. Typing a character without sending (`input_observed`).
5. Erasing the character and restoring composer state (`character_erased`).
6. Full page reload and return to interactive Discord state without login redirects.

### Error attribution

| Error | With Vencord | Without Vencord | Reproducible? | Root cause | Action |
| --- | --- | --- | --- | --- | --- |
| `TypeError: Cannot read properties of undefined (reading 'getItem'/'setItem')` | Yes (intermittent on fast reload) | No (Vencord settings store only) | Yes, on fast cached reloads when Discord executed before the Vencord bundle | Discord deletes `window.localStorage` upon SPA initialization. When Vencord executed after Discord's delete, `window.localStorage` was undefined. | Resolved: (1) Added in-memory bundle caching in Rust to eliminate async dispatch latency (< 1 ms); (2) Added document-start preservation of `Window.prototype.localStorage` so Discord's `delete window.localStorage` does not break `localStorage`. Verified zero errors across 3 consecutive reloads. |
| `TypeError: Failed to fetch` at `My (badges.vencord.dev)` | Yes | No | Yes, on Vencord startup | Vencord's badges API (`My`) attempts to fetch `https://badges.vencord.dev/badges.json`. Discord serves a vanilla CSP header whose `connect-src` does not permit `badges.vencord.dev`. In Electron, Legcord strips CSP via `session.webRequest.onHeadersReceived` (`setupNoCSP()`). Native WebView2 does not yet implement network request/CSP interception (deferred Phase F). Vencord catches this gracefully; Discord and Vencord remain fully operational without badges. | Documented and attributed to deferred Phase F (request interception / network compatibility). |
| `Uncaught Error: Sentry successfully disabled` | Yes | No | Yes, on Discord Sentry initialization | Intentional Vencord design: Vencord defines a setter on `window.DiscordSentry` that throws an error to neutralize Discord's telemetry. | Documented; expected Vencord design behavior, not a defect. |

Mod-consumer errors observed while validating Shelter and Equicord:

| Error | Shelter run | Equicord run | Native bridge? | Classification | Action |
| --- | --- | --- | --- | --- | --- |
| `Uncaught Error: Sentry successfully disabled` | absent | present, once | no | **Known Vencord-family design behavior**, not native: Equicord is a Vencord fork and defines the same `window.DiscordSentry` setter that deliberately throws to neutralize Discord telemetry | Documented, not suppressed. Identical to the pre-existing Vencord attribution above |
| `Native bridge request timed out` (Equicord, first run) | n/a | yes, once | yes | **Genuine native defect, since fixed**: the Equicord adapter declared a `mod_css_result` event the protocol did not accept, so the host never received the CSS result, never ran the marker, and the request timed out | Fixed by routing Equicord's CSS result through the shared mod-agnostic `mod_css_result` event that echoes the host-supplied `mod_id`; the adapter now declares no CSS event at all. Re-run: marker `equicord-bootstrap-executed`, `execution_count=1` per document, real stylesheet injected |
| any `bundle_retrieval` / `script_execution` / `css_injection` / `adapter_initialization` / `duplicate_bootstrap` / `unknown_mod` host log | none | none | — | no failure | — |
| `TypeError` on `getItem`/`setItem` | none | none | no | shared storage preservation is holding for all three mods | unchanged |

No error was hidden: renderer `uncaught`, `unhandled_rejection`, and `console.error` capture stays in place, and every host-side failure stage still prints through the descriptor's own log tag.

## Settings plugin artifact (3d-2 prerequisite A)

The exact Legcord settings Shelter plugin artifact (`ts-out/plugins/settings/plugin.js`, 252,541 bytes, SHA-256 `dcbfc8d9d75535209c40bc9a5817f4eb351ca66f947288ffc94681887f320491`) is now embedded at compile time by `native/build.rs` and exposed read-only through `native/src/settings_plugin.rs`. No execution is wired: the artifact is available to the host, not to the renderer.

### Provenance (traced, not assumed)

- Authored in `src/shelter/settings/` (`index.ts` entrypoint, `plugin.json` name `"Legcord Settings"`, lune per-directory convention).
- Compiled by the existing JS pipeline: `pnpm build:plugins` ? `lune ci --repoSubDir src/shelter --to ts-out/plugins` (`package.json:11`, chained by `package.json:12` `build`, ordered before packaging in `.github/workflows/package.yml`, packaged via `electron-builder.ts` `files: [..., "ts-out", ...]`).
- Electron serves it as `legcord://plugins/settings/*`, which resolves to `<bundle-dir>/plugins/settings/*` (`src/protocol.ts:22-31` maps the scheme onto `import.meta.dirname`, i.e. `ts-out/` at runtime). The Shelter preload pins it as `"legcord-settings"` (`src/discord/preload/mods/shelter.mts:8`).
- `ts-out/` is gitignored (`.gitignore:4`); nothing is committed and no binary blob was added. Rebuild determinism could NOT be re-verified in this environment (the `node_modules/@uwu/lune` entry points are broken symlinks under the bun-shimmed `pnpm`); the recorded identity is from the existing pipeline output and CI rebuilds it via `pnpm build`. NOTE (shim milestone): the artifact was rebuilt here by invoking the lune bundle directly (`node node_modules/@uwu/lune/dist/clibundle.cjs ci --repoSubDir src/shelter --to ts-out/plugins`, pnpm itself being broken) because the previous output predated all native WebView2 branches (`isNativeWebView`, `refreshNativeThemes`, `setNativeThemeEnabled`, `importNativeThemes`, folder helpers). New identity 252,541 bytes / SHA-256 `dcbfc8d9d75535209c40bc9a5817f4eb351ca66f947288ffc94681887f320491`; the five boot markers still validate and the bundle now carries the native theme/folder/version branches the runtime validation depends on.

### Build-pipeline decision

Candidate A: the existing JS build produces `ts-out` before cargo. `build.rs` deliberately does NOT invoke `lune`/`pnpm` itself (candidate B): spawning a package manager from a build script would break offline, cross, and CI-cached builds. Moving the plugin source into a native asset (candidate D) would fork it away from `src/shelter/settings/` and drift. A developer cloning the repository runs `pnpm build:plugins` (already required before packaging) and then `cargo build`; a missing artifact fails the build with an actionable message naming the exact command to run.

### Validation and boundary

- `build.rs` rejects a missing file (actionable panic), empty output, output over 2 MiB, non-UTF-8 output, output missing any of the five synchronous boot members (`getConfig`, `getThemes`, `translations`, `setLang`, `restart` � the exact surface `src/shelter/settings/index.ts:90-99` + `settings.ts:14-20` reads during `onLoad()`, confirmed present in the compiled bundle: `getConfig` x2, `getThemes`/`translations`/`setLang` x1 each, `window.legcord.restart` x3), and a `plugin.json` whose `name` is not `"Legcord Settings"`.
- Identity is a host-computed SHA-256 over the exact embedded bytes (the upstream artifact carries no build banner to pin). Any pipeline byte change yields a different identity and the same bytes always yield the same one.
- The renderer cannot choose a plugin path, source file, URL, or plugin id through this module: `settings_plugin::artifact()` takes no input and resolves only the pinned artifact. Startup logs one identity line (`[SettingsPlugin] artifact name=... bytes=... sha256=...`).
- The compiled bundle was verified to reference exactly the five traced boot members above, matching the source trace file-for-file. Full settings-plugin execution remains intentionally unwired (3d-2 prerequisite B, the synchronous boot snapshot, is still pending).

## Settings boot snapshot (3d-2 prerequisite B)

The native shell now provides the exact synchronous boot data the real
settings plugin reads during onLoad(), as one host-created frozen surface:
window.legcord.settingsBoot. Full settings-plugin execution stays
intentionally unwired - this is the prerequisite, not the integration.

### Traced boot contract (verified in source AND the compiled bundle)

src/shelter/settings/index.ts:90-99 runs synchronously, no await:

| Member | Source site | Electron mechanism | Return shape | Boot role |
| --- | --- | --- | --- | --- |
| getConfig | settings.ts:15, index.ts:62 (showExperimentalPluginMenu) | bridge.ts:58 sendSync(getEntireConfig) to ipc.ts:337 cached Settings | Readonly<Settings> (settings.d.ts:31-105) | whole config + experimental-menu flag |
| getThemes | settings.ts:19 | bridge.ts:121 sendSync(getThemes) to ipc.ts:212 cached list | Readonly<ThemeManifest[]> (themeManifest.d.ts:1-18) | whole theme inventory |
| translations | index.ts:94 | bridge.ts:83 preload-time sendSync(getTranslations) (zero IPC at onLoad) | runtime Record<string,string> (i18nStrings.d.ts:1; the as-string casts in bridge.ts:83 are inaccurate) | whole catalog into store.i18n |
| setLang | index.ts:96 with storesFlat.LocaleStore.locale | bridge.ts:64 fire-and-forget send(setLang, lang) to ipc.ts:224 to lang.ts:55-66 writes storage/lang.json, clears caches | void | INVOKED at every boot (not mere existence) |
| restart | index.ts:83, SettingsPage.tsx:59, AddDetectableModal.tsx:81 - modal/button callbacks only | bridge.ts:116 fire-and-forget send(restart) to ipc.ts:284 to main.ts:124-133 app.relaunch() + app.exit() | void | existence only at boot; never invoked during init |

The compiled ts-out/plugins/settings/plugin.js (252,541 bytes after the rebuild below) references exactly this surface (getConfig x2, getThemes/translations/setLang x1 each, window.legcord.restart x3).

### Snapshot shape (exact, frozen, host-created)

window.legcord.settingsBoot carries exactly seven members: settingsSource (file | defaults | unavailable), settings (the Settings object, or null only when unavailable), themes (Array of raw manifest-plus-id documents, the Electron getThemes() shape), lang (resolved selection), i18n (merged catalog record), setLang(locale) and restart() as narrow fire-and-forget senders.

Sourced from the actual owners, never duplicated or invented: settings.rs load_settings (missing file maps to compiled-in defaults; corrupt maps to unavailable with null config, never fabricated defaults), themes.rs raw_theme_manifests (raw manifest-plus-id passthrough sorted by id, capped at 256, invalid manifests skipped; NO loose-file auto-import - a boot read must have no write side effects), localization.rs load_raw_lang (selected catalog over en-US fallback fill, the exact loadLanguage() rules). setLang persists via localization.rs apply_lang (4-space lang.json layout identical to Electron, atomic temp-plus-rename; no cache exists natively to invalidate). Caps: snapshot at most 1 MiB, catalog at most 8192 keys / 512 KiB, locale [A-Za-z0-9_-] at most 32 bytes.

### Security and lifecycle boundaries

- Host-created per launch, spliced into the document-start script BEFORE the mod adapters (initialization_script fails fast on malformed JSON or an unspliced placeholder; ordering is unit-tested). Every document executes the same script and receives its own frozen object identity - per-document isolation without cross-document leakage.
- No renderer-supplied values, paths, URLs, filenames, plugin ids, or generic invocation: the two functions validate input (TypeError before dispatch) and post closed one-way events (settings_boot_set_lang with one bounded locale, settings_boot_restart with an exact empty payload; unknown members rejected by deny_unknown_fields at parse plus length/charset validation).
- No generic synchronous IPC was added: the existing async capabilities are untouched, and settingsBoot is a separate surface (a future loader shim will map it onto the sync member names at plugin-eval time - that mapping is the remaining 3d-2 step, not this milestone).
- restart currently reloads the document; Electron relaunch (app.relaunch() + app.exit()) is deliberately NOT claimed (no native relauncher exists yet).
- Values reflect launch-time state; mid-run host writes (a setLang persist, a settings mutation, a theme install) are honored by the async capabilities immediately but appear in the boot snapshot on the next launch - consistent with the Electron restart-required model. This boundary is documented, not hidden.
- DEV-only settings_boot_probe event (LEGCORD_NATIVE_SETTINGS_BOOT_SMOKE=1) reports source/bytes/counts/frozen/keys-exact per document - counts and lengths only, never values, manifests, translations, paths, or URLs. The probe proves presence and shape in the live document; ordering is proven by construction plus the unit test.

## Deferred Phase F gap: remote-`@import` theme stylesheets (documented, no change)

Native theme install, enable/disable, and managed style-node injection are functioning (runtime-proven: enabling a theme creates the `legcord-native-theme-<id>` node with the stored bytes; disabling removes it). What does NOT work natively is any theme whose CSS payload is a remote `@import`:

- The concrete case is ClearVision: `ClearVision_v6.theme.css` (9,605 bytes) is almost entirely `@import url(https://clearvision.github.io/ClearVision-v6/main.css)`. Import parses and installs, the toggle flips the manifest, the style node is injected — but the browser must then fetch the remote stylesheet under the Discord document CSP.
- Discord serves `style-src 'self' 'unsafe-inline' https://cdn.discordapp.com ...` (verified against the live header): `clearvision.github.io` is not allowlisted, so the fetch is refused and the theme has no visual effect.
- Production Electron is unaffected by design: the default `csp: "none"` setting runs `setupNoCSP()` (`src/discord/extensions/csp.ts:39-54`), which strips the CSP header on the main frame.
- The native shell only patches `connect-src` (badges fix); `style-src` is untouched, and no CSP change was made in the settings-shim milestone.
- For CSS-application testing, use a disposable self-contained theme (local rules only) so CSP cannot contaminate the result. Remote-stylesheet loading stays deferred to Phase F network compatibility.

## Closed settings compatibility shim (3d-2)

The native shell now executes the real Legcord settings plugin. A closed host-controlled shim maps the frozen boot snapshot onto the exact synchronous names the plugin reads, and the host registers the embedded artifact through the proven add/start loader lifecycle once Shelter itself is up.

### Why a shim (no generic sync IPC)

The plugin reads five synchronous members during onLoad() (verified in source and in the compiled bundle: getConfig x2, getThemes/translations/setLang x1 each, restart x3 in later callbacks only, all direct window.legcord reads, never destructured). The native bridge is Promise-only by architecture. The shim backfills exactly those five names from the launch-frozen snapshot - plus two static platform strings (below) - and nothing else.

### Shim members (exact)

- settings.getConfig(): dual contract. Synchronous callers receive a per-document mutable working copy of the frozen snapshot (same identity on every call of the document, mirroring the Electron main-process cache object); the live settings_get_config request still fires eagerly, so awaited callers (power plugin, smoke) keep exact async behavior incl. timeouts, disposal, and response-schema validation. A non-enumerable then property carries the live branch; a settled noop branch prevents unhandled rejections from never-awaited sync calls.
- themes.getThemes(): per-document mutable projection of the snapshot inventory onto exactly id/name/author/description/version/enabled (missing optional text becomes empty string; malformed entries skipped). No paths, stylesheets, remote references, or other manifest properties cross.
- translations: the frozen snapshot catalog object as-is (reads only; the plugin never writes it).
- settings.setLang(locale): the existing narrow settingsBoot sender (validated, fire-and-forget settings_boot_set_lang event, returns undefined - the plugin discards the return).
- restart(): the existing narrow settingsBoot sender (empty settings_boot_restart event; the host reloads the document - Electron relaunch is deliberately not claimed).
- platform / osRelease: static launch-time strings with Electron parity (process.platform / os.release()). REQUIRED, not optional: SettingsPage render calls isMinWindowsVersion() (osRelease.split, throws on undefined) and gates sections on platform comparisons. platform reuses the boot snapshot constant (win32); osRelease is queried once per launch via RtlGetVersion (app.rs, immune to compatibility-version lies) and validated as a dotted numeric triple. No capability, no request, no renderer input. version is deliberately NOT shimmed (AboutPopup reads it optional-chained with a fallback; HeroUpdater uses the async capability natively).

### Mutability without weakening the snapshot

The frozen snapshot is never unfrozen. Deep per-document clones give the plugin its mutable working copies (setConfig writes store.settings[key], keybind/detectable pushes - all of which throw on frozen objects in strict mode). Writes to a working copy never reach the frozen source (tested). No plugin code compares object identity (verified by source grep); the per-document shared clone additionally preserves Electron cache identity.

### Host loader (closed add/start)

After Shelter's own bootstrap marker proves window.shelter.plugins exists, the host evaluates one fixed script per document: remove-first (idempotent), addLocalPlugin with the host-fixed id legcord-settings + embedded artifact bytes (JSON-encoded splice) + fixed name-only manifest, then startPlugin. Plain status strings (settings-plugin-started / settings-plugin-failed) come back for host observability. Guarded once per document generation (reloads start fresh, replays log a loud duplicate and are refused). No renderer input at any step: id, source, and manifest are host literals.

### Ordering

document-start init script (facade + settingsBoot + shim clones, all before adapters) -> Shelter bundle evaluation (host ModBundleReady) -> shelter async init (#app-mount) -> bootstrap verdict -> settings add/start -> onLoad reads the shim. Proven by construction plus the splice-ordering unit test; the runtime smoke confirms presence/shape/frozen state per document.

### What the shim does NOT do

No sendSync, generic invoke, eval RPC, arbitrary source/plugin/filesystem/URL access, renderer-controlled ids, or snapshot construction input. The async capabilities are untouched. Theme operations reached by the executing UI are the already-implemented native capabilities (list/setEnabled/importFiles/folders), never duplicated. Deferred editor/URL/uninstall paths stay hidden via the existing isNativeThemesMode gates. legcord:// asset images (Trash/Edit/Folder/UpgradeArrow icons, screenshots) now resolve through the native legcord protocol (P0.2); before it they rendered broken.

### Runtime acceptance (DEV profile only, 11 items)

1. Discord loads (health hasAppMount, no login redirect). 2. Shelter loads (bootstrap marker per document). 3. Real artifact loads (start ok=true, 252,541 bytes, once per generation). 4. Plugin does not unload (Legcord section found on both documents). 5. Settings UI appears (modal via plain click). 6. ThemesPage exists (Installed Themes header). 7. Themes list renders (both cards). 8. ClearVision appears (card found, persisted disabled baseline shown). 9. No getThemes failure (sync shim + native list path; zero such errors anywhere). 10. No new unhandled rejections (stderr holds only previously attributed diagnostics). 11. No duplicate execution (start count tracks generations 1:1, no duplicate logs, execution_count=1).
Beyond the eleven: enable/disable drive real managed-node add/remove (host css_updates/css_removes), two themes simultaneously (2 nodes), import click keeps the page alive (file selection itself is inherently manual - host modal), folder action proven twice by host Metrics lines, reload persistence verified (2 nodes without opening UI), DEV profile restored (0 nodes, both manifests disabled). CSS proof uses a self-contained fixture theme, never ClearVision visuals (Phase F gap above). setLang proven live (locale persisted per document).

## legcord:// protocol (P0.2)

Native serves all six Electron namespaces with the same bundle-relative roots: plugins/html/js/css under the bundle dir, assets under `<bundle>/assets/app/`, local under `<userData>/userAssets/`. The `legcord://plugins/settings/*` files resolve from the same `ts-out/plugins/settings/` tree the embedded settings artifact is built from (filesystem serve, no embedded dump).

### Registration (exact mechanism)

wry 0.57 creates the WebView2 environment internally with no scheme hook, so the host creates the environment itself with `webview2-com 0.39` (`CoreWebView2EnvironmentOptions` + `CoreWebView2CustomSchemeRegistration`, verified against the installed sources, not guessed) and hands it to wry via `WebViewBuilderExtWindows::with_environment` (which adopts a caller-provided environment verbatim). Every other option mirrors wry's own `create_environment` (same additional-args string, system language, default scroll style, same user-data directory), so the only delta is the registration:

- scheme `legcord`, `HasAuthorityComponent=true` (the `legcord://namespace/path` syntax), `TreatAsSecure=true`
- `AllowedOrigins=["https://discord.com"]` only — the one origin proven to load legcord:// subresources; origin restriction (the AllowedOrigins equivalent) is additionally enforced by navigation policy (HTTPS Discord hosts + `about:blank` boot document only, popups denied), the `legcord://*`-only filter, and CSP
- registration happens at environment creation, before navigation. A generic Rust-side scheme registry was not invented.

No `TreatAsSecure`/bypass parity beyond this is claimed. `supportFetchAPI`/`corsEnabled`/`stream` have no registration switch: fetch/XHR reach the same handler, and no `Access-Control-Allow-Origin` is sent (cors-mode fetch of a cross-origin legcord:// URL is rejected renderer-side while the handler still serves 200; observed, not worked around).

### Request handling

One `WebResourceRequested` handler with one filter per resource context actually served (`legcord://*` × DOCUMENT/STYLESHEET/SCRIPT/IMAGE/MEDIA/FETCH/XML_HTTP_REQUEST), via `ICoreWebView2_22::AddWebResourceRequestedFilterWithRequestSourceKinds` when the runtime supports it (same overload wry itself uses) and the two-argument overload otherwise. The existing Discord HTTPS filter, the Vencord badges interception, and every other filter are untouched. Success is 200 + extension MIME + bytes; traversal is 400 + minimal body; missing is 404. Rejections and successes log namespace + status + bytes + MIME only — never URIs or filesystem paths.

### Path security (stronger than Electron by design)

One authoritative resolver (`native/src/legcord_protocol/resolver.rs`): fixed six-namespace set, URI parsed as URI (raw text is checked BEFORE `url::Url` normalization, which would otherwise hide dot segments), single percent-decode with malformed-encoding and double-encoding rejection, Windows absolute/drive-relative/UNC/extended-length rejection, both separators normalized, canonical base + canonical target with containment, symlink/junction escape refused, directories refused, empty path refused, query/fragment rejected (no current consumer uses them — verified by source grep over `src/`, and the DEV probe asserts no `?`/`#`), no renderer-provided roots, no path leakage in errors or logs. Electron's weak `filePath.includes("..")` check is deliberately NOT reproduced.

### MIME (extension-only, no sniffing)

html/htm, js/mjs, css, json/map, txt/md, wasm, png, svg, jpg/jpeg, webp, ico, gif, bmp, avif, woff/woff2, ttf/otf/eot, webm/mp4, mp3/wav/ogg/m4a, zip; unknown → `application/octet-stream`.

### CSP (narrow extension, never removed)

Electron's `bypassCSP=true` has no WebView2 equivalent — registered custom schemes adhere to CSP. The existing Discord main-document interception now also appends `legcord:` to exactly the directives traced consumers need (`default-src` when present as fallback, `script-src`, `style-src`, `img-src`, `media-src`, `connect-src`; `font-src`/`object-src` untouched — no consumer). All existing sources preserved, badges patch preserved. Concrete modification, not "bypass parity".

### Two-phase boot (filter-registration race)

wry starts the initial navigation inside `build()`, before any `WebResourceRequested` filter can exist, so a single-phase boot would leave the first Discord document on Discord's original CSP until a reload. The host boots `about:blank` (generation 1, allowed as the empty boot document only), registers both interception filters, then navigates to Discord (generation 2+). Side effect, observed honestly: the two-phase boot also fixed the pre-existing first-load CSP race for the Vencord badges patch (badges `Failed to fetch` no longer appears; connect-src carries both badges and legcord:).

### Tests

- 25 resolver tests (valid/nested, `..`/`..\`, mixed/encoded/double-encoded traversal, drive/UNC/extended-length/drive-relative, prefix confusion, symlink escape with privilege skip, namespace crossing, empty path, directory, missing, malformed encoding, repeated separators, Unicode, query/fragment, invalid namespace, all-namespaces)
- 4 MIME tests, 6 handler tests (roots mapping, 200+MIME, 400/404 mapping, no path leakage), 6 CSP tests (only-present directives extended, absent stay absent, badges preserved, no duplication, unlisted untouched, no substring false positives), 3 scheme tests (fixed name/origin, COM-level option values, options carry exactly one registration), 1 probe guard test (fixed bundle URLs only, no eval/Function/innerHTML, no query/fragment)
- 2 electron-compat tests (namespace/root parity pin vs `src/protocol.ts`, hardening markers; known-broken `disableAutogain.js` caller left untouched — resolves 404, never fabricated)
- Full suite: 397 Rust + 217 compatibility + 84 electron-compat, clippy `-D warnings` clean, fmt clean

### Runtime evidence (DEV only, `LEGCORD_NATIVE_LEGCORD_PROTOCOL_SMOKE=1`)

Cold start: scheme registered → `about:blank` gen 1 → Discord gen 2 with `[CSP] document policy extended badges=true legcord_sources=6` before its navigation → real WebView2 `status=200` for `assets/css/discord.css` (790 B, text/css), `assets/desktop.png` (29697 B, image/png), `plugins/settings/plugin.js` ×2 (256903 B, javascript) → probe `css=loaded img=loaded csp=none` (zero CSP violations) with `nocors=resolved` (cors-mode fetch stays rejected: no ACAO sent, by design). Reload run: same on generation 2, no duplicate handlers, HTTPS Discord networking intact, health `hasAppMount=true`, settings plugin starts per generation. Earlier transient runs additionally proved top-level `legcord://html/splash.html` navigation serves 200 (7850 B, text/html) with its `Loading.webm` subresource (160381 B, video/webm). Splash/setup/editor WINDOWS are not exercised by the native shell (deferred multi-window) — marked unverified as windows, validated as protocol resources. No traversal reached outside authorized roots; production Legcord/profile untouched.

### Known limitations

- `legcord://js/*` has no files in the current bundle (`ts-out/js/` absent): resolves 400 like any outside-root request, not a special case.
- `disableAutogain.js` does not exist in the repo: 404, caller untouched.
- cors-mode `fetch()` of cross-origin legcord:// URLs is rejected renderer-side (no ACAO sent); subresource loads (link/img/media/top-level) are unaffected.
- `local/*` (userAssets) serves only what the profile holds; no writer was added in this milestone.
- Query/fragment forms are rejected even though Electron would fold them into the path — hardening wins, documented.

## Native permission policy (P0.3)

Electron contract (`src/main.ts:368-411`, the sole `setPermissionRequestHandler` on `session.defaultSession`): allow exactly `fullscreen`, `notifications`, `clipboard-sanitized-write`, and one undifferentiated `media`; deny everything else with a log. No origin/URL is inspected. Display capture never reaches that handler — it is a separate `setDisplayMediaRequestHandler` surface (`src/discord/screenshare.ts`) and stays out of this milestone.

### WebView2 mapping (installed `webview2-com-sys 0.39.1`, verified)

Only MICROPHONE (1), CAMERA (2), and NOTIFICATIONS (4) are allowlisted. There is NO WebView2 permission kind for fullscreen, clipboard-write, or screen/display capture (PERSISTENT_STORAGE is not exposed by the installed bindings either) — those cannot arrive here, and any unknown or future kind integer denies via the catch-all. Every other installed kind (geolocation, sensors, clipboard-read, multi-downloads, file-read-write, autoplay, local-fonts, MIDI, window-management, unknown) denies.

### Origin policy

Exact `https://discord.com` only (bridge-gate parity, deliberately stricter than the navigation gate's `*.discord.com`). Both `args.Uri()` and the live WebView source must validate (https, exact host case-insensitive, no explicit port even `:443`, no userinfo, no opaque origin, no `about:blank`, no subdomains); any parse failure or disagreement denies.

### Decision mechanics

One `PermissionRequested` handler on the main WebView (never wry's `with_permission_handler` — it drops the origin this policy is built on). Per event: read URI/kind/`IsUserInitiated` (diagnostics only), decide from the positive allowlist, set `SavesInProfile=false` explicitly (WebView2 persists grants by default; Electron answered per-request), then set ALLOW or DENY explicitly — State is never left DEFAULT. No deferral (synchronous policy), no `Handled` (single handler, wry precedent of `SetState`-only), no `SetPermissionState`/profile APIs, no filesystem/fetch/paths/renderer input. Logs carry kind + trusted/untrusted + gesture + decision only.

### Tests

9 Rust tests (3 allow, 10-kind deny + fabricated/future integers, 14-origin deny matrix, exact-form acceptance, enum name coverage, shipped-surface structural test) + 1 probe guard test (fixed DOM APIs only, tracks stopped, no display media/eval/bridge). Full suite: 406 Rust + 217 compatibility + 84 electron-compat, clippy `-D warnings` clean, fmt clean.

### Runtime evidence (DEV only)

`LEGCORD_NATIVE_PERMISSIONS_SMOKE=1`: notifications ALLOW → renderer `granted`, no browser prompt; microphone ALLOW → live stream obtained and stopped (`live-stopped`); geolocation DENY → renderer `PERMISSION_DENIED` (code 1); camera UNVERIFIED — no camera device in this environment, `getUserMedia({video:true})` rejects with `NotFoundError` before any PermissionRequested fires (policy arm unit-tested, code path shared with microphone). Reload re-issues identical decisions on the new document (no persisted grants, no duplicate handlers); same DEV profile across all runs reaches the handler every time. P0.2 intact in the same runs (CSP extended with badges + 6 `legcord:`, legcord 200s, subresources loaded). Renderer errors: only pre-existing diagnostics. Production untouched.

### Known limitations

- Camera grant unvalidated live (no device); fullscreen/clipboard-write produce no WebView2 permission event (verified by absence — nothing to allow, nothing claimed).
- No persistence by design: repeated visits re-prompt through policy (Electron parity).
- A mic/camera grant is not voice/video parity; screensharing/venmic/loopback/pickers untouched.

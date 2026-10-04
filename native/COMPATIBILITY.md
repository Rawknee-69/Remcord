# Native WebView2 compatibility boundary

This document records the existing Electron renderer/preload contract and the intentionally narrow native bridge. The Electron sources remain the behavioral specification. Native WebView2 now has a Vencord-only execution path; nothing here claims that the Rust shell is Legcord-compatible.

“Boot required” means necessary to open Discord in a bare WebView, not necessary to preserve every existing Legcord feature. Injection and the renderer bridge are also tested in this prototype even though bare Discord can load without them.

| Electron capability | Current Legcord call sites / contract | WebView2 equivalent | Migration difficulty | Boot required? | Can defer? |
| --- | --- | --- | --- | --- | --- |
| `BrowserWindow` | [`src/discord/window.ts`](../src/discord/window.ts) `createWindow`; [`src/discord/ipc.ts`](../src/discord/ipc.ts) diagnostics windows; setup, splash and CSS editor windows | Native HWND plus `CoreWebView2Controller`; Tao owns the prototype window | High: bounds, titlebar, visibility, multi-window and lifecycle policies | Yes: one window | Multi-window, custom chrome and diagnostics can defer |
| `webContents` | `window.ts` owns navigation, reload, title/focus/maximize; `ipc.ts` sends native updates | `CoreWebView2` events and controller | High: event coverage differs and needs one owner | Yes: navigation/view | Most parity behaviors can defer |
| `executeJavaScript` | `window.ts` focus/maximize state; `common/dom.ts`; `common/keybindActions.ts`; `rpcProcess.ts`; preload mod files | `CoreWebView2.ExecuteScriptAsync` | Medium: async errors and document/frame targeting | Not for bare Discord | Yes; bootstrap and prototype probes are present |
| Preload / early script | `window.ts` configures `discord/preload.mjs`; `discord/preload/preload.mts` imports `mods/vencord.mts` | `AddScriptToExecuteOnDocumentCreatedAsync` | High: registration must precede navigation; main-frame vs iframe scope must be explicit | No for bare Discord; yes for Vencord startup | Vencord adapter is main-frame/document-start; other preload modules defer |
| `contextBridge` | [`src/discord/preload/bridge.ts`](../src/discord/preload/bridge.ts) exposes `window.legcord` | No direct equivalent; a document-start facade over `WebMessageReceived` | High: current facade is broad and synchronous APIs must be redesigned | No for bare Discord | Broad Legcord bridge can defer; this prototype exposes the snapshot, ping, and one typed Vencord operation |
| `webFrame` | `window.ts` frame-created YouTube injection; `preload/plugins.mts`; `preload/mods/{custom,equicord,shelter,vencord}.mts` | Frame-aware WebView2 scripting / `CoreWebView2Frame` | High: frame lifetime and cross-origin behavior differ | No | Yes, except where an enabled mod specifically needs frame injection |
| `insertCSS` | `preload/mods/vencord.mts` injects `browser.css` after bundle execution; `preload/themes.ts` and [`src/common/themes.ts`](../src/common/themes.ts) manage unrelated themes | Native host runs a fixed style-element insertion script with stable identity, in the same document and after Vencord JS | Medium: CSP, style order, removal/reapplication must be qualified | No | Only Vencord CSS is implemented; full themes and other mod CSS defer |
| `ipcMain` | [`src/discord/ipc.ts`](../src/discord/ipc.ts) registers `getVencordBundle` plus bundle/config/theme/plugin/RPC/backup handlers; `screenshare.ts`, Venmic and other main modules register IPC | `WebMessageReceived` with a versioned, explicitly dispatched Rust command enum | Very high: hundreds of exposed calls and sync IPC need contract-by-contract migration | No for bare Discord; yes for existing mod feature parity | Native supports `ping` and the fixed `mod_vencord_get_bundle` operation plus typed lifecycle/error events; other handlers defer |
| `ipcRenderer` | Vencord preload uses only `invoke("getVencordBundle")`; `bridge.ts`, `preload/plugins.mts`, and other mod preloads use separate APIs | `window.ipc.postMessage` / `WebMessageReceived` with one exact Vencord request contract | Very high: subscriptions, events, invokes, and synchronous calls need async typed equivalents | No for bare Discord | Only Vencord bundle startup is bridged; all other IPC surfaces defer |
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
| Renderer request (Vencord capability) | `{"version":1,"request_id":"...","command":{"type":"mod_vencord_get_bundle","payload":{}}}` |
| Renderer request (settings capability) | `{"version":1,"request_id":"...","command":{"type":"settings_get_config","payload":{}}}` |
| Renderer request (settings mutation capability) | `{"version":1,"request_id":"...","command":{"type":"settings_set_block_power_saving","payload":{"value":true}}}` |
| Renderer request (localization capability) | `{"version":1,"request_id":"...","command":{"type":"localization_get_lang","payload":{"key":"invite-goBackToApp"}}}` |
| Ping response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"ping","payload":{"nonce":"...","message":"pong"}},"error":null}` |
| Vencord response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"vencord_bundle","payload":{"bundle_version":"...","bundle_bytes":1,"css_bytes":1,"request_latency_ms":1.0,"retrieval_ms":1.0,"execution_ms":1.0,"css_injection_ms":1.0,"marker":"vencord-bootstrap-executed","document_execution_count":1}},"error":null}` |
| Settings response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"settings_config","payload":{"settings":{...},"source":"file","payload_bytes":2048}},"error":null}` |
| Settings mutation response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"settings_mutation","payload":{"block_power_saving_in_voice_chat":true,"source":"file","payload_bytes":2048}},"error":null}` |
| Localization response | `{"version":1,"request_id":"...","ok":true,"payload":{"type":"localization_string","payload":{"value":"Go back to the Discord app","lang":"en-US"}},"error":null}` |
| Renderer event | `{"version":1,"event":{"type":"round_trip_ack","payload":{"nonce":"..."}}}` |

The commands are `ping`, `mod_vencord_get_bundle`, `settings_get_config`, `settings_set_block_power_saving`, and `localization_get_lang`; events include `round_trip_ack`, `css_probe`, `renderer_error`, `bootstrap_metrics`, the typed Vencord CSS/bootstrap/adapter/response acknowledgements, and the DEV-only `settings_probe`/`get_lang_probe`/`settings_mutation_probe` diagnostics (emitted only by the `LEGCORD_NATIVE_SETTINGS_SMOKE=1` / `LEGCORD_NATIVE_GETLANG_SMOKE=1` / `LEGCORD_NATIVE_SETTINGS_MUTATION_SMOKE=1` probes, never by shipped adapters). Request IDs are limited to 128 ASCII alphanumeric, `-`, or `_` characters, may not be reused within a document, and are capped at 4096 tracked IDs per document. The request/response contract is asynchronous. The renderer correlates responses by ID, ignores unknown IDs and wrong-version responses, and rejects outstanding promises on timeout or `pagehide`. Native request state and Vencord's one-start/one-execution guard reset on a new document. Unknown versions, commands, events, malformed payloads, duplicate IDs, oversized messages, and untrusted origins are rejected and logged; no unknown request is dispatched.

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

The document-start Vencord adapter in `native/web/mods/vencord/adapter.js` is a thin consumer of the shared factory in `native/web/mod-substrate/mod-bootstrap.js`. It declares only Vencord's identity and typed wire vocabulary (capability name, expected marker, result/ack/error event names, state property names) and auto-requests one explicit `native.getVencordBundle()` call per Discord document. It can only send `mod_vencord_get_bundle` with an empty object payload. Rust resolves the `VENCORD_MOD` descriptor from `KNOWN_MODS` and runs the generic substrate pipeline: schema, exact stable Discord origin, request ID, per-document duplicate guard, and fixed bundle release sources. The substrate downloads only the descriptor's fixed JS/CSS release assets, allows only that descriptor's HTTPS redirect hosts, enforces byte limits, and caches only the fixed file names under `E:\Legcord.webview-dev-user-data` plus an in-memory `ModCache`. Renderer input cannot select a URL, path, filename, channel, command, or executable source.

To avoid transferring a multi-megabyte script back through the renderer and to preserve page-world execution despite Discord's script CSP, the capability returns a typed metadata result only after Rust retrieves and invokes that fixed source via Wry/WebView2 `ExecuteScriptAsync` in the main document. The existing Vencord source is run as a direct classic script, not wrapped in an IIFE or `eval`; no generic script-execution method is exposed. After execution, a descriptor-parameterized verifier checks for the `globalThis.Vencord` object. Only then does the adapter's non-enumerable `window.__legcordNativeVencord` marker become `executed`.

CSS uses a descriptor-parameterized host-generated script to create/update one `<style id="legcord-native-vencord-css">` after bundle execution. It appends to the document's current head/root, reports a typed result, and replaces rather than duplicates that style if reused. Each full reload receives a fresh adapter, pending-request map, marker, style element, and native document generation; stale worker completions are discarded. The substrate permits only one request and one successful marker per mod per document.

Failure logs distinguish `bundle_retrieval`, `adapter_initialization`, `script_execution`, `css_injection`, and renderer `uncaught`/`unhandled_rejection`/`console.error` reports. The host remains alive on these failures. The native response contains byte counts, bundle header version, request/retrieval/execution/CSS timings, and the marker; it deliberately does not return source text to JavaScript.

## Reusable mod substrate (Phase B)

`native/src/mod_substrate.rs` owns every behavior identical for all client mods; it names no mod. The typed adapter contract is `ModDescriptor`: stable `id`, boot `capability`, fixed JS/CSS URLs and cache file names, allowed redirect hosts, byte limits, version header prefix, style element id, page-world global name, success marker, settings key, CSS-result event name, bootstrap DOM event name, storage-context global name, and log tag. `main.rs` registers `VENCORD_MOD` in `KNOWN_MODS`; lookup is by exact id only.

The substrate additionally owns `ModCache` (in-memory validated bundles keyed by mod id), `DocumentModLifecycle` (document generation plus per-mod one-start/one-execution guards cleared on navigation), `ModHostMetrics` (per-mod request/bootstrap/duplicate counters), and the fixed script builders for CSS insertion, storage-context snapshot, and execution verification. A startup check fails fast if any registered descriptor's capability is missing from the boot snapshot.

`native/web/mod-substrate/` mirrors the split: `storage-preservation.js` keeps `Window.prototype.localStorage` reachable after Discord deletes its own property (shared by every mod, covered by a dedicated regression test); `mod-bootstrap.js` exposes `window.__legcordNativeModSubstrate.createModAdapter(config)`, which owns origin gating, once-per-document loading, request correlation, bootstrap verdict handling, typed result reporting, and adapter error reporting. Mod adapters supply only identity plus hooks (`requestBundle`, `validateResponse`, `resultExtra`, `resultPayloadExtra`, `responseExtra`).

`native/web/mods/testmod/adapter.js` is a synthetic second consumer proving the factory is not Vencord-specific. It is test-only: it is never included in the document-start script, never touches Discord UI, and never submits account state. Shelter and Equicord remain unimplemented; adding either means a new descriptor plus a thin adapter, with no substrate changes.

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

## Localization capability (Phase C-2)

The second mod-facing `window.legcord` capability migrates the already-async `getLang(key)` contract to the typed bridge. Exactly one caller is migrated: the invite back-button preload's `invite-goBackToApp` label lookup (`src/discord/preload/inviteBackButton.mts`). No language mutation, no new format, no duplicated database.

### Electron chain (unchanged)

`bridge.ts` exposes `getLang: async (toGet) => await ipcRenderer.invoke("getLang", toGet)`; `ipc.ts` handles `getLang` with `getLang(toGet)` from `src/common/lang.ts`: in-memory cached catalog for the `<userData>/storage/lang.json` selection (default `en-US`, two-letter codes normalized to `xx-XX`), selected catalog with `en-US` fallback merge, and `data[key] ?? ""` — a missing key returns `""`, never rejects. The invite caller itself uses the raw invoke plus `|| "Go back to the Discord app"`; history patching and the JANK-02 interval removal are untouched.

### Native chain

Renderer `window.legcord.getLang(key)` → `requireCapability("localization.get_lang")` → key-only `localization_get_lang` request (empty/oversized/non-string keys rejected client-side and host-side) → Rust `localization::lookup` over the compile-time-embedded `assets/lang/*.json` catalogs (via `native/build.rs`) with the identical selection/fallback/empty-string rules, selection re-read per request from `<DEV profile>/storage/lang.json` → validated `localization_string { value, lang }` → resolved string. The invite caller branches on `boot.runtime === "webview2"`; Electron keeps the raw invoke byte-for-byte.

### Required dependency categories

- **Needed to execute Vencord:** one standalone Vencord browser JS bundle, the matching browser CSS bundle, the page-world execution mechanism, and fixed CSS insertion after the JS. The native bridge transports only a typed request/result, not bundle data.
- **Optional Legcord features:** mod enable/disable settings, existing `userData` location, cache/ref update policy, broad `window.legcord` API, the runtime-plugin loader, other mods, themes, RPC, and all other `ipcRenderer` channels.
- **Electron-only replacements in this slice:** `app.getPath` plus exact local bundle reads are replaced by the isolated DEV cache and fixed URL retrieval; `ipcRenderer.invoke` by the one typed operation; `webFrame.executeJavaScript` by fixed-source Wry main-world execution; `webFrame.insertCSS` by the one identified style element.
- **Deferred:** ambient `require("electron")`, `window.require`, arbitrary `native.invoke`, arbitrary `eval`/host source execution, `contextBridge` parity, `process` parity, arbitrary file reads, config migration, generic plugin scripts, other mods, and Vencord's full settings/update UX.

Limits: a bridge message is at most 8 KiB; a ping nonce is non-empty and at most 128 bytes; CSS probe text is at most 128 bytes; renderer error kind is at most 64 bytes and its message at most 2 KiB; measured bootstrap time must be finite and in `[0, 60000]` ms. There are no arbitrary channels, host method names, path arguments, eval commands, or filesystem operations.

### Implemented `window.legcord`

The exact facade is `window.legcord = Object.freeze({ boot, native, settings, getLang })`, installed in the document-start script only when `location.origin === "https://discord.com"`.

| Property | Kind | Behavior |
| --- | --- | --- |
| `window.legcord.boot.runtime` | immutable boot snapshot | `"webview2"` |
| `window.legcord.boot.platform` | immutable boot snapshot | `"win32"`; the native prototype is Windows-only |
| `window.legcord.boot.protocol_version` | immutable boot snapshot | `1` |
| `window.legcord.boot.capabilities` | immutable boot snapshot | `["bridge.ping", "mod.vencord.get_bundle", "settings.get_config", "settings.set_block_power_saving_in_voice_chat", "localization.get_lang"]` |
| `window.legcord.native.ping(nonce)` | async native request | Returns `{ nonce, message: "pong" }`; rejects malformed input, transport errors, native errors, timeout, or document disposal |
| `window.legcord.native.getVencordBundle()` | explicit Vencord-only async request | Requests the fixed native operation; resolves typed size/version/timing/marker metadata only after the host executes the bundle and applies CSS |
| `window.legcord.native.supports(name)` | renderer-local | Boolean membership test against the frozen boot capability list |
| `window.legcord.native.requireCapability(name)` | renderer-local | Returns `true` when present; otherwise throws with `code === "UNSUPPORTED_CAPABILITY"` |
| `window.legcord.native.reportCss(value)` | typed one-way diagnostic event | Sends the native prototype's CSS injection probe result; not a general CSS API |
| `window.legcord.settings.getConfig()` | explicit settings-only async request (Phase C) | Requires the `settings.get_config` capability (`UNSUPPORTED_CAPABILITY` otherwise). Sends the parameter-free `settings_get_config` command and resolves the validated `Settings` snapshot object (Electron-compatible keys). Rejects malformed snapshots, `SETTINGS_UNAVAILABLE` host failures, timeout (default 5 s), or document disposal. No synchronous facade exists |
| `window.legcord.settings.setBlockPowerSavingInVoiceChat(value)` | explicit allowlisted settings-mutation async request (Phase C-3) | Requires the `settings.set_block_power_saving_in_voice_chat` capability (`UNSUPPORTED_CAPABILITY` otherwise). Sends exactly one boolean (`settings_set_block_power_saving`, non-boolean client-rejected) and resolves the host-confirmed applied value from the validated `settings_mutation` response (`source` must be `"file"`). Rejects malformed responses, `SETTINGS_UNAVAILABLE` host failures (missing/unreadable/corrupt file, write failure), timeout (default 5 s), or document disposal. No generic `setConfig` exists on the native facade |
| `window.legcord.getLang(key)` | explicit localization-only async request (Phase C-2) | Requires the `localization.get_lang` capability (`UNSUPPORTED_CAPABILITY` otherwise). Sends exactly one bounded key (`localization_get_lang`, non-empty, at most 128 UTF-8 bytes; client- and host-validated) and resolves the localized string — `""` for a missing key, exactly like Electron. Rejects malformed responses, host failures, timeout (default 5 s), or document disposal. No synchronous facade exists |

No other existing Electron-facing `window.legcord` methods are implemented by this facade. Boot snapshot values are native runtime metadata only; they do not impersonate Legcord config, Legcord version, OS release, translations, or Electron version.

### Classification of the existing renderer contract

The table lists the exposed `LegcordWindow` groups from `src/@types/legcordWindow.d.ts` and the behavior actually wired by `src/discord/preload/bridge.ts`. Existing API calls must not be redirected to an untyped generic bridge.

| API / caller | Sync or async; arguments / return | Side effects and boot-critical status | Migration decision |
| --- | --- | --- | --- |
| `window` — preload titlebar and Shelter titlebar | `show/hide/minimize/maximize/unmaximize/quit(): void`; `maximized()/isNormal(): boolean`; no arguments | Native window lifecycle; custom titlebar is not needed to open Discord | Deferred native commands; dynamic state is asynchronous or event-cached, never sync IPC |
| `settings.getConfig()` — Shelter settings/power/titlebar | Synchronous; no arguments; full `Settings` object | Reads mutable app settings; not needed for bare Discord, but current UI assumes synchronous state | Populate an initial documented config snapshot only if native settings support is added; migrate updates to async |
| `settings.setConfig(key,value)` — Shelter settings | Synchronous; key/value; handler returns `undefined` after persisting | Mutates config; not boot-critical | Phase C-3 ports exactly one mutation (`blockPowerSavingInVoiceChat` via the bool-only `settings_set_block_power_saving` command and `settings.set_block_power_saving_in_voice_chat` capability, atomic host-controlled persist, host-confirmed value back). All other keys keep the sync Electron implementation; no generic native writer exists |
| `settings.addKeybind(keybind)`, `toggleKeybind(id)`, `removeKeybind(id)` | Fire-and-forget; typed keybind or ID; `void` | Mutate settings and refresh global keybinds | Deferred |
| `settings.openStorageFolder/openThemesFolder/openCustomIconDialog/copyDebugInfo/copyGPUInfo/openWebRTCInternals/openGPUInfo` | Fire-and-forget; no args; `void` | Opens filesystem locations/dialogs/internal pages or writes clipboard | Deferred; host-owned dialogs/paths only |
| `settings.setLang(lang)` | Fire-and-forget; language string; `void` (type declaration says `() => void`, implementation sends without returning) | Changes app language | Deferred async request |
| `settings.dumpFlags()` | Synchronous; no args; applied flags object | Logs and returns runtime flags; not boot-critical | Async diagnostics request or native-local equivalent |
| `touchbar.setVoiceTouchbar(state)`, `setVoiceState(mute,deafen)`, `importGuilds(guilds)` — Shelter touchbar | Fire-and-forget; boolean(s) / string array; `void` | Mutates Touch Bar or voice state / imports guilds | Deferred; Touch Bar is platform-specific |
| `power.setPowerSaving(state)`, `isPowerSavingEnabled()` — Shelter power | Fire-and-forget boolean / synchronous boolean read | Mutates/reads native power-saving behavior | Deferred; mutable/dynamic read becomes async or cached event |
| `electron`, `version`, `platform`, `osRelease`, `translations` — preload and Shelter | Synchronous properties; respectively Electron version, Legcord version, OS platform/release, raw translation JSON | Snapshot-like values; platform/version/translation are read at preload boot | Only true native runtime metadata is in this prototype snapshot. Electron and Legcord version, OS release, and translations are not substituted with invented values |
| `getLang(key)` — invite-back preload and UI | Async; translation key string; `Promise<string>` | Reads one localized string; not boot-critical | Phase C-2 ports exactly one key path (`invite-goBackToApp` via `localization.get_lang`, empty-string missing-key parity, embedded catalogs). No language mutation; other callers deferred |
| `screenshare.getSources(callback)`, `start(source,name,audio)`, `venmicStart(include)`, `venmicSystemStart(exclude)`, `venmicList()`, `venmicStop()` — Shelter screenshare | Event subscription callback / fire-and-forget / async promises; source IDs, names, booleans, venmic node arrays; returns callback event, void, or `Promise` values | Capture source selection, native media and audio routing | Deferred; no source IDs, paths, or media authority exposed in this milestone |
| `themes.install(url)`, `uninstall(id)`, `edit(id)`, `getThemes()`, `refresh()`, `openImportPicker()`, `set(id,state)`, `folder(id)`, `openQuickCss()`, `importQuickCss(css)`, `enableQuickCss()`, `disableQuickCss()` | Mixed; install is declared void but implementation returns an async invoke; theme list/refresh are sync arrays; others fire-and-forget | Reads/updates theme files, opens dialogs, edits CSS; not bare-boot-critical | Snapshot only for a proven initial immutable listing; mutations/import are async and host-owned; full theme handling deferred |
| `rpc.refreshProcessList()`, `getProcessList()`, `addDetectable(game)`, `removeDetectable(id)`, `getDetectables()`, `getBlacklist()`, `blacklistGame(name,id)`, `unblacklistGame(id)` — Shelter registered games/RPC | Mixed fire-and-forget, synchronous process/list/cache reads, synchronous blacklist mutation | Reads process list, edits detectable/blacklist state, emits `rpcBlacklistUpdated`; not boot-critical | Process/detectable reads become async or locally cached; blacklist state uses typed events and async mutations |
| `backup.save(data)`, `restore()` — settings | Async; serialized backup string / no args; promise result | Opens save/open dialogs and reads/writes archive contents | Deferred native user-mediated file picker; renderer must not supply a filesystem path |
| `plugins.list()`, `setEnabled(id,enabled)`, `reload(id)`, `openFolder()` — plugins page | Async list/toggle/reload; fire-and-forget folder open | Enumerates and changes runtime plugins | Deferred; no plugin loader is part of the prototype |
| `fs.writeFile(pluginId,relativePath,data)`, `readFile(pluginId,relativePath)` — plugin storage | Async; plugin ID, relative path, text; structured success/error result | Reads/writes scoped plugin storage, gated by user setting and path containment | Deferred; never pass renderer-controlled paths to Rust |
| `window.legcordRPC` — `src/discord/rpcProcess.ts` / Shelter RPC | Renderer-side API; receives activity and notifies consumers | Page-world RPC data delivery; not a `LegcordWindow` member | Deferred arRPC integration |
| `window.legcord.channel` — `src/splash/redirect.html` | Synchronous string property | Selects redirect channel on splash page | Not part of the Discord document contract; splash is deferred |

### Direct Electron dependencies in Discord preload

These are separate from `window.legcord`. Only the exact Vencord integration operations in `preload/mods/vencord.mts` have a native replacement; other current preload modules remain unimplemented and are not loaded.

| Caller | Electron dependency and contract | Migration |
| --- | --- | --- |
| `preload/bridge.ts` | `require("electron")`, `contextBridge.exposeInMainWorld`, `ipcRenderer.send/sendSync/invoke/on/removeListener`; defines the broad Electron facade and RPC blacklist cache. Also reads `process.versions.electron` | Only the narrow native boot facade exists; broad `contextBridge`, `process`, and IPC behaviors remain deferred |
| `preload/plugins.mts` | `ipcRenderer.invoke` for preload/renderer runtime plugin entries and source; `webFrame.executeJavaScript`; dynamic `new Function` plugin evaluation | Entire loader deferred; script sources must not be introduced as generic host RPC |
| `preload/mods/vencord.mts` | `require("electron")`; `ipcRenderer.invoke("getVencordBundle")`; `webFrame.executeJavaScript(bundle.js)` in page world, then `webFrame.insertCSS(bundle.css)`. No `contextBridge`, `process`, or direct filesystem API in this module | Replaced by the fixed `mod_vencord_get_bundle` command and native one-purpose bundle/CSS injection; nothing is polyfilled |
| `preload/mods/{shelter,equicord,custom}.mts` | `ipcRenderer.invoke` for local bundle reads; `webFrame.executeJavaScript/insertCSS`; Shelter additionally uses `process.platform` | Deferred; no corresponding native adapter |
| `preload/themes.ts` | `ipcRenderer.on("addTheme"/"removeTheme")`; inserts/removes DOM styles | Deferred typed theme events and style lifecycle |
| `preload/titlebar.mts` | `ipcRenderer.sendSync` for OS/config/window state and `send` for window actions | Deferred titlebar; config boot values would need a native snapshot and dynamic state async |
| `preload/newTitlebar.mts` | `ipcRenderer.sendSync` for windowStyle/transparency/OS; `process.platform` | Deferred; no native chrome settings are asserted |
| `preload/patches.mts` | `ipcRenderer.sendSync` for display version and two settings; `process.platform`; DOM injection and timing | Requires a deliberate preload settings/version migration; not loaded |
| `preload/inviteBackButton.mts` | Async `ipcRenderer.invoke("getLang")`, DOM/page-world history patches | Deferred; translation lookup may later be a typed async request |
| all preloads above | `webFrame`, `contextBridge`, Node `require`, `process`, `ipcRenderer` are Electron-only globals, not WebView globals | Explicit adapters or TS migrations per feature; Vencord is the single implemented adapter, with no ambient Electron emulation |
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
| mod preloads: `getShelterBundle`, `getVencordBundle`, `getEquicordBundle`, `getCustomBundle`; plugins runtime entries/scripts | No args or plugin target/ID; promises yield bundle text/flags or runtime entries/source | Vencord has the one typed native replacement described above; Shelter, Equicord, custom bundles, and runtime plugin entries remain deferred |
| `themes.ts`: `addTheme(name,css)`, `removeTheme(name)`; bridge RPC blacklist update | Event payloads are name/CSS or entry arrays; listener returns unsubscribe for blacklist only | Updates renderer styles/cache; future bridge must use narrow typed subscriptions with lifecycle cleanup |
| `inviteBackButton.mts`: `getLang(key)` | Translation key; promise resolves string or undefined | Async localized text read; deferred typed locale request |

## Explicit capability ownership

The document-start compatibility script owns facade creation, local response correlation, `pagehide` promise cleanup, and renderer diagnostics. The static Vencord adapter owns one request per document and the private execution marker; Rust owns the versioned protocol, exact origin gate, request-ID uniqueness, fixed source retrieval/cache path, page-world execution, CSS insertion, response dispatch, and document generation. The native profile remains exactly `E:\Legcord.webview-dev-user-data`. The renderer supplies no path, URL, channel name, host method, executable source, or filesystem operation.

### Deferred and not tested

Filesystem and plugin storage; updater; tray; native menus; downloads; permissions; screen capture; WebRTC/native media; `webRequest`; protocol handlers; arRPC; popup windows; general mod loader; Shelter, Equicord, and custom bundles; runtime plugins; existing broad `window.legcord` APIs; all direct Electron preload globals except the documented Vencord replacements. Vencord's settings, enablement/config migration, ref-based update policy, and interactions with other mods are not implemented. This narrow Vencord path does not establish full Legcord compatibility.

### Validation status

The Rust substrate, protocol, and JavaScript factory/adapter have focused tests for descriptor registration, bundle identity validation, cache miss/hit, per-mod one-start/one-execution guards, document reset, mod independence, host metrics isolation, origin and capability gates, malformed input, adapter initialization, duplicate bootstrap prevention, renderer exception locations/stacks, failure propagation, synthetic second-mod execution, and storage preservation across `delete window.localStorage`. `cargo fmt -- --check`, `cargo test` (25 tests), `cargo build`, `cargo clippy --all-targets -- -D warnings`, targeted Biome, and the JavaScript tests (19 tests) pass.

The live smoke validation used only `E:\Legcord.webview-dev-user-data`. It retrieved the 698,272-byte Vencord JS and 41,335-byte CSS bundle, cached them in memory in Rust, executed once per document, acknowledged all markers, and completed multi-generation full reloads without host crashes or duplicate bootstraps. In-memory bundle retrieval takes 0.000 ms; execution calls take 17–27 ms; CSS insertion takes 24–75 ms; document loads complete in 60–100 ms on cached reload; and marker request latencies range 49–115 ms.

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

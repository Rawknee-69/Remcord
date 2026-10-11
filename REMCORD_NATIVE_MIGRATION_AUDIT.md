# LEGcord Native Migration & Performance Audit

**Repository assessed:** `E:\Legcord` at `b750f1e` (`fix: add .gitattributes...`), 3 October 2026.  
**Method:** static source and checked-in asset audit; no production runtime, heap snapshot, trace, or package build was available. Statements tagged **Confirmed** follow directly from source. **Likely** means the control flow is clear but the size/rate is supplied by Discord or a dependency. **Possible** requires measurement.

## 1. Executive Summary

Legcord is an Electron 44 shell around `https://discord.com/app`, with a privileged preload/injection layer, native media/window integrations, and a Shelter-based settings UI. It does **not** contain Discord's React application, gateway processing, media pipeline, or most plugin code in this repository. Consequently, this audit cannot attribute a measured amount of RAM/CPU to Discord itself, and it must not label Discord's renderer a leak from static source alone.

The Electron shell is a real fixed cost: Electron's bundled Chromium, V8/Node main process, preload environment, Crashpad, and Chromium child processes are packaged with every installation. Moving the shell to Rust plus the installed Windows WebView2 runtime should remove the Electron/Node application process and packaged Chromium distribution cost. It will **not** make Discord a native app: WebView2 still runs browser, renderer, GPU, network/media, and crash-handling processes as required by the web application.

The largest code-supported idle problem is separate from Electron: main-window construction sets `backgroundThrottling` to `getConfig("sleepInBackground")` in `src/discord/window.ts:499`. The default is `false` in `src/common/config.ts:70`, while minimize-to-tray defaults to `true` at line 34. Electron documents that `false` keeps animations, timers, painting, and Page Visibility active when a window is hidden. Thus a normally tray-hidden Discord renderer is deliberately left foreground-like. This is **Confirmed P1** behavior, though its CPU/RAM magnitude must be measured. [Electron BrowserWindow documentation](https://www.electronjs.org/docs/latest/api/browser-window) confirms the semantics.

The migration should therefore be an incremental replacement of the shell, not a Rust rewrite of Discord or the mod ecosystem:

```text
Rust host / lifecycle / storage / native services
              |  narrow, origin-checked message bridge
              v
system web view (WebView2 first)
              |
Discord web app + retained JS bootstrap + Shelter/Vencord/Equicord/themes
```

The recommended Windows implementation is a Rust host using Tauri 2 for window/tray/updater/capability plumbing where it fits, plus a small Windows-specific WebView2 adapter for APIs Tauri/Wry does not expose uniformly (document-created script registration, WebResourceRequested interception, display-media integration, and popup setup). Do not expose Tauri's broad invoke/plugin APIs to `discord.com`; use an explicit bridge whose commands are allowlisted by origin and frame.

## 2. Repository Coverage

### Files inspected

Tracked repository inventory: 293 files. The audit examined all tracked application source, build metadata, lockfile importer entries, tests, documentation/examples, Docker/packaging files, local HTML/CSS/JS, and source-relevant asset sizes.

| Class | Count | Total bytes | Coverage |
|---|---:|---:|---|
| TypeScript (`.ts`) | 70 | 266,501 | Read/traced; includes native shell, common utilities, and Shelter source. |
| TSX (`.tsx`) | 31 | 196,237 | Read/traced; only settings/modal UI and screen-share picker. |
| Preload modules (`.mts`) | 13 | 31,472 | Read/traced individually. |
| Checked-in JS | 8 | 29,372 | Read/traced; includes YouTube ad injector and plugin examples. |
| CSS | 31 | 82,797 | Inventory/read for injection/loading behavior; no scriptable runtime work. |
| HTML | 4 | 10,432 | Read/traced. |
| JSON/i18n | 42 | 235,965 | Schema/content role and load path inspected; translations treated as data. |
| Lockfiles | 2 | 414,936 | `pnpm-lock.yaml` importer and `bun.lock` header inspected. |
| Images/media/icons | 51 | ~3.1 MB | Size/load behavior inventoried; binary pixels not semantically inspected. |

### Directories inspected

`src/`, `assets/`, `assets/app/`, `assets/lang/`, `build/`, `scripts/`, `docs/`, `tests/`, `docker/`, and `meta/` were traversed. `package.json`, `pnpm-lock.yaml`, `bun.lock`, `electron-builder.ts`, `rolldown.config.ts`, `tsconfig.json`, `biome.jsonc`, workspace metadata, and all Docker/metadata files were inspected.

### Generated/binary files

`build/*.ico`, `build/icon.icns`, `build/appx/*.png`, `assets/**/*.png`, `assets/**/*.ico`, `assets/app/mockup.jpg`, `assets/app/Loading.webm`, and `assets/app/Update.webp` are images/media. They add disk/package size but create no startup execution path other than the splash WebM and UI icons. The three documentation screenshots alone total 1.59 MB and should be excluded from packaged application payload if the builder currently copies the broad `assets` tree.

`node_modules` exists locally (47,453 files, 653,252,916 bytes) but is third-party generated/install state, not checked-in source. It was not recursively source-audited. `dist`, `ts-out`, and `out` are absent. The module tree is corrupt for the installed Bun shim, so the supplied test command could not execute.

### Uninspected items and reasons

* Discord's downloaded HTML/JS/CSS, gateway/WebSocket implementation, React reconciliation, WebRTC internals, browser cache, Vencord/Equicord/Shelter downloaded bundles, user-installed extensions, themes, and runtime plugins are outside the repository or user-data directory. Their actual runtime behavior needs profiling.
* Binary image/video pixels and third-party `node_modules` source were not code-reviewed; package intent and direct call sites were audited instead.
* `pnpm test` could not start: Bun reported that its executable remap in `node_modules` is corrupt and recommended `bun install --force`. No dependency repair was performed because it would mutate local install state outside this audit.

## 3. Current Architecture

```text
Electron executable (Electron 44.2.0, Node >=26)
  -> src/main.ts main process
     -> config / CLI / Chromium switches / proxy / Crashpad
     -> download mod bundles + load filesystem main plugins
     -> Electron default session, CSP/protocol hooks
     -> optional splash/setup BrowserWindow
     -> main Discord BrowserWindow
        -> isolated Electron preload (`discord/preload.mjs`)
           -> contextBridge `window.legcord`
           -> early page scripts, Vencord/Equicord/Shelter/custom injection
           -> runtime preload/page plugins
        -> remote Discord app
        -> Shelter settings and built-in Legcord plugins
  -> optional Node worker thread running arRPC process scanning
```

There is one production browser surface for Discord. Setup, splash, Monaco Quick CSS editor, Chrome internals pages, Discord popouts, and `window.open` windows add more `BrowserWindow`/renderer instances on demand. User-installed Chromium extensions are loaded after startup.

The app stores config, themes, plugins, bundle downloads, and plugin storage under Electron `userData`; portable mode redirects it to `<exe-dir>/legcord-data`.

## 4. Process Architecture

Static source cannot prove an exact OS process count because Chromium/WebView process allocation varies with OS, site isolation, GPU driver, extensions, OOPIFs, and active media. The following is the defensible map.

| Scenario | Processes/threads directly created by Legcord | Chromium/Electron processes expected or possible | Why / migration result |
|---|---|---|---|
| Cold normal start | Electron main; splash renderer; Discord renderer; Node **worker thread** for arRPC if `inviteWebsocket` | GPU, network/service utility, Crashpad, audio/video/media, extension/OOPIF processes as Chromium decides | Rust removes Electron main and Node worker only if arRPC becomes native; WebView2 still needs its browser-renderer-GPU/media process model. |
| Setup first run | Electron main; setup renderer; setup tray on non-macOS | GPU/Crashpad possible; Discord window is not created until restart | Native setup can be a local webview or Rust UI. |
| Idle after login | Main + Discord renderer retained even when tray-hidden; arRPC worker starts by default | GPU/network/Crashpad remain implementation-dependent | Current default disables background throttling. Native shell must suspend/throttle deliberately outside voice/media. |
| Active chat | Same | Network and GPU active; Discord may create workers/OOPIFs | Renderer workload largely stays. |
| Voice/video/screen share | Same plus media activity; screen source picker transports thumbnails | Audio/video utility/GPU/encoder paths likely | Hardware encode/capture behavior must be qualification-tested per backend. |
| Multiple windows/popouts | `mainWindows[]` can hold more than one; stream popout and allowed `about:blank` windows | Additional renderer(s), sometimes extra GPU/media memory | Rust host needs a single owner registry; system WebView does not eliminate popout renderers. |
| CSS editor | `cssWindow` extra renderer plus Monaco assets | Additional renderer/GPU resources | Lazy/on-demand; replace with a lightweight local editor or retain web editor. |
| `chrome://gpu` / WebRTC diagnostics | Up to two cached diagnostic BrowserWindows | More renderer resources | Debug-only; native diagnostic equivalents are not parity-critical. |
| Minimize-to-tray | No window destruction; Discord renderer remains | Same backing processes can remain active | Primary idle CPU/RAM opportunity; hiding is not suspension. |

`src/discord/rpcProcess.ts` uses `node:worker_threads`, not Electron `utilityProcess`; it adds work and JS heap inside the application process rather than proving an extra OS process. `arrpc` may internally scan at the configured interval, but its dependency source was not part of this repository audit.

## 5. Startup Execution Trace

| Stage | File / symbol | Context | Blocking and resource work |
|---|---|---|---|
| Module load | `src/main.ts` | Electron main | Imports CSP and protocol side effects, checks data/config folders, registers Discord URI handler. |
| Pre-ready configuration | `args`, single-instance lock, switches in `main.ts` | Main | Reads config synchronously; applies GPU/media/proxy command switches; starts Crashpad with upload disabled. Linux starts DBus asynchronously. |
| Mod/plugin preparation | `fetchMods()` and `initializePluginSystem()` at `main.ts:225` | Main | **Blocking:** `Promise.all` awaits Shelter's GitHub ref check and scans/imports enabled main filesystem plugins. Vencord/Equicord iteration uses `forEach(async ...)`, so those downloads are not awaited. |
| Ready | `app.whenReady()` | Main | Awaits `applySessionProxy()` and initializes app. Linux adds a fixed 1.5 s delay before `init()`. |
| Setup branch | `createSetupWindow()` | Setup renderer | Creates a 840x720 local BrowserWindow and a non-macOS tray. The returned Promise has no resolve path; normal continuation is a restart. |
| Normal branch | `createSplashWindow()` then `createWindow()` | Separate splash/main renderers | Splash is intentionally unawaited; it fetches `latest.json` and holds at least 3 seconds. Discord window is constructed concurrently. |
| Window setup | `doAfterDefiningTheWindow()` | Main | Registers IPC once, watchers, tray, permissions, screen capture, web-request hooks, title/badge handlers, app handlers, global shortcuts, optional RPC worker, and the 1 s bounds poll. |
| Remote navigation | `BrowserWindow.loadURL` | Renderer/network | Loads stable/PTB/Canary Discord URL. Network/cache/login costs are owned by remote Discord/Chromium. |
| Preload evaluation | `src/discord/preload/preload.mts` | Isolated preload + page world injection | Exposes bridge; begins mod and runtime-plugin asynchronous injection; applies WebRTC/camera/notification/title/theme patches. Bundle injections are launched independently rather than awaited as one ordered bootstrap. |
| DOM complete | `injectThemesMain()` `did-finish-load` | Main -> renderer IPC | Sends Quick CSS and enabled theme CSS. Splash's `splashEnd` closes splash and shows/hides main window. |

### Startup execution diagram

```text
main.ts
  -> config + flags + proxy + crash reporter
  -> await [GitHub mod-ref fetch, filesystem-plugin initialization]
  -> app ready -> await session.setProxy + closeAllConnections
  -> [unawaited splash renderer] + Discord BrowserWindow
       -> register native features / worker / watchers / timer
       -> navigate Discord
       -> preload bridge and concurrent JS/CSS injection
       -> Discord application boot
```

Startup cost is therefore not solely Electron. It includes synchronous JSON/filesystem work, an awaited remote GitHub operation, shell creation, the remote Discord navigation/bundles, and client-mod bundle parsing/execution.

## 6. Renderer / Discord Loading Architecture

`src/discord/window.ts:createWindow()` creates a sandboxed, context-isolated, Node-disabled BrowserWindow and loads `discord.com/app`, `canary.discord.com/app`, or `ptb.discord.com/app`. It intentionally changes the user agent and supports an experimental mobile UA.

The preload entry imports bridge, downloaded mod loaders, runtime plugin loader, patches, title bars, themes, and invite-button code. Importing each `mods/*.mts` starts an async `ipcRenderer.invoke` / `webFrame.executeJavaScript` chain; there is no all-mod readiness barrier. Shelter is always retrieved with `enabled: true`; Vencord/Equicord/custom depend on settings. `webFrame.insertCSS` adds Vencord/Equicord/custom CSS. Main-process themes are delivered later by IPC as `<style>` nodes.

This ordering is compatibility-sensitive. Preserve it as an explicit bootstrap manifest with phases: **document-created patches**, **page-world bridge**, **mod bootstrap**, **runtime plugins**, **DOM-ready UI/theme hooks**. Do not replace it with ad hoc `eval` calls scattered across Rust.

## 7. Preload / IPC Architecture

`src/discord/preload/bridge.ts` exposes a large `window.legcord` object via `contextBridge`. It includes window controls, settings, native dialogs/folders, debug clipboard, screen-share source selection, power blocker, themes, arRPC data, backup, runtime plugin administration, and conditionally-gated plugin file storage.

`src/discord/ipc.ts:registerIpc()` is registered once, guarded by `ipcRegistered`. It captures the first `passedWindow`; a later primary window can therefore send commands that manipulate the original captured window. Every sync IPC call blocks the renderer/main boundary, including full config/theme/translation data. None of the handlers validates `event.senderFrame.url`, window identity, or origin.

Migration rule: replace channels with typed async messages, attach each message to its WebView identity, verify top-level origin is an approved Discord origin, reject subframe/unknown origins, validate payloads, and return data only from capability-specific commands. Do not offer a generic `invoke`, host object, or arbitrary Rust-function bridge.

## 8. Plugin / Vencord / Shelter Architecture

| Layer | Location | Execution model | Portability |
|---|---|---|---|
| Downloaded Vencord/Equicord/Shelter/custom bundles | `src/discord/extensions/modloader.ts`, `preload/mods/*` | Fetches JS/CSS into user data; injects as page scripts/styles | Renderer-only technique is portable to WebView2; trust/update policy must be redesigned. |
| Built-in Shelter plugins | `src/shelter/**` -> Lune output | Shelter module system, Discord Flux/module internals, `window.legcord` bridge | Keep JavaScript/TS mostly unchanged; replace bridge calls and verify WebView engine behavior. |
| Runtime filesystem plugins | `src/discord/plugins/manager.ts`, `preload/plugins.mts` | `main`, preload, and page scripts dynamically imported/evaluated | Renderer portions portable; Electron-main plugin API must be replaced and permission-scoped. |
| Chromium extensions | `src/discord/extensions/plugin.ts` | Loads user-data extension directories into Electron default session | Electron/Chromium specific; no guaranteed Wry/Tauri/WebView2 parity. Redesign/retire or provide a separate vetted extension model. |
| BetterDiscord themes / Quick CSS | `src/common/themes.ts`, renderer IPC | Local files, watcher, injected `<style>` | Portable, with host file watcher and one CSS lifecycle manager. |

### Confirmed lifecycle concerns

* `src/shelter/screenshare/components/ScreensharePicker.tsx:35` stores `const original = navigator.mediaDevices.getDisplayMedia` once, then `patchNavigator()` replaces the method. `onLoad` calls it at `src/shelter/screenshare/index.tsx:327`, and every share confirmation calls it again at the picker line 133. The latter wraps the already-wrapped function. **Confirmed:** repeated shares accumulate wrappers until the document is replaced; each subsequent capture can repeat constraints/device enumeration/audio manipulation. **P1 during screenshare.**
* `src/shelter/power/index.ts` subscribes to `TRACK` in `onLoad` but has no `onUnload`; `src/shelter/touchbar/index.ts` has the same missing `TRACK` unsubscribe. Reloading these Shelter plugins duplicates callbacks. **Confirmed P2.**
* `src/shelter/touchbar/index.ts:64` only appends to global `guildItems`; it never clears existing buttons. Reloading/importing duplicates native Touch Bar items and retains icon objects. **Confirmed P2, macOS-only.**
* `src/discord/preload/plugins.mts` tracks preload cleanups but renderer plugins have no activation cleanup contract. Page navigation normally discards their realm, so a leak across a normal navigation is not proven; live reload is not supported and needs an explicit teardown design.

## 9. Network Architecture

| Existing hook | Files | Why it exists | Cost / migration disposition |
|---|---|---|---|
| Discord URL load | `discord/window.ts` | Main application | Required; renderer workload remains in WebView2. |
| Bundle ref/download checks | `extensions/modloader.ts` | Update Shelter/Vencord/Equicord/custom JS/CSS | GitHub ref fetch is on the startup critical path for Shelter. Move to background job with cached signed/hashed manifests and an explicit stale-cache policy. |
| Proxy | `common/proxy.ts` | Sync Node and Chromium proxy config | Retain via Rust HTTP/proxy configuration and WebView-specific profile proxy capability; platform disparity requires adapter. |
| Telemetry/local WS block | `sanitization.ts`, `window.ts` | Cancels Discord science, Sentry/NEL, unexpected localhost WS | Static URL predicate is cheap per request; preserve as a declarative rule set, but test WebSocket coverage under each backend. |
| YouTube Referer rewrite | `window.ts` | UMG video playback workaround | WebView2 can alter/reply to filtered resources; WebKit backends may need JS or local proxy. Verify still needed before porting. |
| CSP rewrite/removal | `extensions/csp.ts` | Strict policy or intentionally removes main-frame CSP | Critical security behavior. Avoid `none` as default in a new shell; replace with an allowlisted policy and isolated mod/bootstrap scheme. |
| YouTube ad injector | `assets/app/js/adguard.js` | Hides/skips ads in embeds/proxy frames | Long-lived MutationObserver scans on every mutation in matching frames. Keep only if user-enabled and scoped; measure playback impact. |
| Theme/update/About fetches | `themes.ts`, `HeroUpdater.tsx`, `AboutPopup.tsx`, splash | User selected themes/manual checks/startup version UI | Background/cache update endpoints; add timeouts/error states and do not block renderer readiness. |

## 10. Native Integrations

The Electron shell provides: custom windows/titlebars/materials, tray/menu/badges, global shortcuts, Linux DBus portal shortcuts, deep links, dialogs/folders/clipboard, Crashpad, proxy, display capture source picker, video/audio flags, Linux Venmic/PipeWire native addon, power-save blocker, Touch Bar, and auto-update components.

The migration boundary should be native services, not Electron API-shaped calls. For example, `NativeWindowService`, `NotificationBadgeService`, `CaptureService`, `ShortcutService`, `GameActivityService`, `StorageService`, and `ExternalUrlService` should own platform decisions and expose only requested operation data.

## 11. Memory Usage Audit

### Confirmed bounded/expected allocations

* Config, language, window state, blacklist, mod metadata, and theme lists are cached objects/maps. They are bounded by configuration/theme file count in a process lifetime.
* The theme CSS cache intentionally retains all enabled theme source strings. This is linear in enabled CSS; not a leak unless the user installs unusually large themes.
* Screen-share source serialization holds base64 thumbnails/icons only for the picker request. A 320x180 decoded thumbnail is about 230 KB RGBA before JS/data-URL overhead; many windows can transiently consume several MB. It is not an idle allocation.
* Electron/Chromium renderer, GPU cache, image/media surfaces, V8 heaps, Discord JS state, browser cache, and WebRTC buffers are expected major categories but cannot be apportioned statically.

### Confirmed unbounded structures

| ID | File / symbol | Classification | Evidence and consequence |
|---|---|---|---|
| MEM-01 | `src/shelter/rpc/asset.ts:3-6` `cache`, `assetCache`, `appCache` | Confirmed unbounded cache; impact likely low-to-medium | Keys are unique asset URL/application ID and are never evicted. Rich-presence activity over a long session can retain app metadata and asset arrays indefinitely. Add a size/TTL/LRU budget and clear on activity reset. |
| MEM-02 | `src/discord/touchbar.ts:13`, `refreshGuilds()` | Confirmed unbounded per reload/import | `guildItems.push` never clears and creates native image-backed buttons. Limit to current guild set and dispose/rebuild once. |
| MEM-03 | `ScreensharePicker.patchNavigator` | Confirmed wrapper stacking | Each selection wraps `getDisplayMedia`; closures retain the preceding wrapper. Restore/replace once instead of nesting. |

### Not proven as leaks

No repository `Map`, `Set`, array, observer, worker, or BrowserWindow establishes a general renderer memory leak by itself. Discord's module cache, React tree, downloaded mod bundles, video frames, source maps, and user plugin behavior require DevTools heap snapshots and allocation timelines.

## 12. CPU Usage Audit

| ID | Files / symbol | State while idle | Confidence | Recommendation |
|---|---|---|---|---|
| CPU-01 | `discord/window.ts:440` bounds poll | Runs every 1 s for every main BrowserWindow | Confirmed | Remove it; move/resize events already debounce persistence at 500 ms. Keep a one-shot restore/close save fallback only. |
| CPU-02 | `preload/inviteBackButton.mts:71` | DOM `sync()` every second for the entire page lifetime | Confirmed | Use history hooks plus a navigation event/MutationObserver with teardown; do not poll. Small individual cost. |
| CPU-03 | `assets/app/js/adguard.js` | On every DOM mutation in matching YouTube frame | Confirmed | Scope to player/route changes or user-enable it; retain observer handle and disconnect on navigation. |
| CPU-04 | `rpc.ts` via arRPC | Default config passes `processScanning: true`, 5,000 ms | Likely | arRPC source controls exact scan; stop worker outside feature need and reactivate only when rich presence is enabled. Profile process enumeration. |
| CPU-05 | `window.ts` with default `backgroundThrottling:false` | Discord's own timers/animations can keep running in tray | Confirmed behavior; impact unmeasured | Make hidden non-voice state throttle/suspend. This is the highest-priority idle experiment. |
| CPU-06 | `extensions/modloader.ts` | Startup GitHub ref request and local bundle download | Confirmed | Do not wait for remote update check before first navigation; fetch after ready with ETag/hash cache. |
| CPU-07 | `shelter/screenshare/index.tsx` | Only while a stream changes/starts | Confirmed | Three controlled reapply timers are bounded. Wrapper stacking is the real problem; do not remove legitimate codec/quality setup blindly. |

There is no repository-wide polling loop besides the 1 s window poll, 1 s invite poll, short-lived screen-share timers, and external arRPC behavior. CPU at 5/15/30 idle minutes cannot be determined without tracing Discord itself, hidden state, installed mods, and arRPC.

## 13. GPU / Rendering Audit

Hardware acceleration is enabled by default and should remain so. `main.ts` only calls `app.disableHardwareAcceleration()` when the setting is false. `common/flags.ts` contains GPU/video presets and platform-specific H.264/VAAPI/VideoToolbox flags; its high-performance/latency/VoIP modes intentionally disable Chromium backgrounding, increasing battery/idle cost in exchange for media behavior.

The screen-share plugin permits 480p through 2160p, 5 through 60 FPS, and a 25 Mbps ceiling. This is an explicit quality feature, not evidence of a GPU defect. It can increase capture, encoding, compositing, memory, network, and battery use dramatically at 4K/60. Validate encoder implementation through existing WebRTC diagnostics and `getStats`, not by disabling GPU.

Transparent/mica/acrylic window options can add composition work; their actual impact is driver/OS dependent. The current dynamic tray favicon decodes/resizes only on favicon updates, not per frame.

## 14. Background / Idle Activity Audit

| Subsystem | Runs while idle? | Why | Frequency | CPU risk | RAM risk | Can be disabled? |
|---|---|---|---:|---|---|---|
| Discord renderer | Yes, including tray-hidden by default | `backgroundThrottling:false` default | Discord-defined | High, unmeasured | High, expected renderer heap | Yes outside voice/media through policy. |
| arRPC worker | Yes when `inviteWebsocket` default true | `startRPC` after window setup | Default scan interval 5 s | Medium | Low-medium | Yes setting/service lifecycle. |
| Bounds poll | Yes | Fallback persistence | 1 s/window | Low | Negligible | Yes; remove. |
| Invite back-button poll | Yes | SPA fallback | 1 s/page | Low | Negligible | Yes; redesign. |
| Theme directory watch | Yes | Refresh theme list | Event driven | Low | Cache linear in themes | Yes/stop on shutdown. |
| Quick CSS watch | Yes when Quick CSS enabled | Live editor sync | Event driven + 300 ms debounce | Low | One CSS string | Yes. |
| Crashpad/Chromium services | Usually | Browser framework | N/A | Low | Fixed shell cost | Not all safely. |
| Tray | Yes | Native lifecycle | Event driven | Negligible | Negligible | User setting. |
| DBus portal | Linux | Global command registration | Event driven | Low | Low | Feature/platform setting. |
| Vencord/Equicord/Shelter/user plugins | Depends | Arbitrary injected code | Unknown | Unknown | Unknown | Per mod/plugin. |
| Media capture/voice/video | Only active call/stream | WebRTC | Media-clock driven | High | High | End call/stream only. |

## 15. Window / WebView Lifecycle Audit

`mainWindows` retains main BrowserWindow references. In tray mode close is prevented and the window is hidden, so its renderer and web contents are intentionally retained. Diagnostic windows use a map and delete their entry on `closed`; CSS editor nulls its singleton on close. Splash has no automatic memory leak: `splashEnd` closes it, but a failed/never-fired splash flow can leave it until process exit.

The multi-window path is weak: app-level event listeners (`activate`, `open-url`, `before-quit`, `second-instance`) are added from `doAfterDefiningTheWindow` for each created main window. The initial `registerIpc` capture also keeps the first main window as the control target. This is **Confirmed P2 correctness/lifetime duplication** when multi-instance/multiple windows are used. Native target design should have one application event owner and a `WindowRegistry` keyed by window/webview ID.

## 16. Event Listener / Timer / Observer Audit

See Appendix C for full resource lifetime inventory. Key result: all three one-shot document-root MutationObservers in `patches.mts` disconnect after injection; the Quick CSS watcher closes on window close; the theme watcher is singleton but is never stopped during normal app shutdown; the YouTube observer intentionally persists for its frame; and two Shelter subscriptions lack unload cleanup.

## 17. Injection / Patching Audit

| Mechanism | Files | Classification |
|---|---|---|
| WebRTC SDP wrapper, `getUserMedia` wrapper, notification click hook | `preload/patches.mts` | Renderer-only, WebView2-compatible page script. Timing must be document-created. |
| Vencord / Equicord / Shelter / custom JS/CSS | `preload/mods/*` | Renderer-only and portable in principle; depends on Chromium/Discord internals and downloaded code. |
| Runtime renderer plugins | `preload/plugins.mts` | Page-world `new Function`/script execution; portable only with explicit user trust and cleanup. |
| BetterDiscord themes/Quick CSS | `common/themes.ts`, `preload/themes.ts` | Portable CSS injection. |
| Titlebar / Invite button | preloads plus Shelter plugins | Portable DOM/window bridge logic. |
| YouTube ad patch | `window.ts`, `assets/app/js/adguard.js` | Chromium/DOM-specific but broadly WebView2 compatible; not core Discord parity. |
| Electron Chrome extensions | `extensions/plugin.ts` | Electron-specific; needs redesign. |

The common principle is to keep the JavaScript modification plane. Replacing it with Rust hooks would destroy compatibility and force rewrites of third-party renderer mods.

## 18. Media / WebRTC Audit

Discord owns normal voice, camera, video, WebRTC, device enumeration, and most codecs. Legcord modifies:

* Electron display-media handler and custom source picker (`discord/screenshare.ts`), including 320x180 thumbnails and optional loopback audio.
* Screen quality/codec/constraints via Shelter (`shelter/screenshare/**`), including H.264 profile handling and RTP limits.
* Camera switching/getUserMedia behavior and SDP mutation (`preload/patches.mts`).
* Linux PipeWire/Pulse capture through optional native `@vencord/venmic` (`discord/venmic.ts`).
* macOS native system audio loopback/VideoToolbox Chromium feature flags and Touch Bar.

Windows WebView2 is the only first target with close Chromium/WebRTC behavior. It still needs explicit qualification of microphone/camera permission, display capture picker, codec availability, GPU driver behavior, H.264 profile choice, and Discord stream popouts. Linux WebKitGTK and macOS WKWebView should be treated as compatibility projects, not automatic ports; screen capture/WebRTC behavior is the highest-risk parity area.

## 19. Dependency Audit

| Dependency | Current use | Runtime/build | Electron-dependent | Native/WebView alternative | Remove? |
|---|---|---|---|---|---|
| `electron` 44.2.0 | Entire shell/Chromium | Runtime package | Yes | Tauri/Wry + WebView2/WKWebView/WebKitGTK | Target removal. |
| `electron-builder`, `app-builder-lib` | Packaging/update targets | Build | Yes | `tauri-cli`, platform packaging | Replace. |
| `electron-updater` | macOS/Linux only in `updater.ts` | Runtime | Yes | Tauri updater or Rust updater | Replace; Windows path currently absent. |
| `electron-context-menu` | Image copy/search context menu | Runtime | Yes | Native webview context API or injected menu + narrow command | Replace/simplify. |
| `electron-is-dev` | Development version checks | Runtime | Yes | Rust cfg/build metadata | Replace. |
| `arrpc` | Process scanner/RPC server | Runtime | Node-oriented | Rust process scanner/RPC service or quarantined sidecar | Redesign, cannot simply retain in Rust host. |
| `@jellybrick/dbus-next` | Linux portal shortcuts | Runtime | Node | `zbus`/portal Rust bindings | Replace on Linux. |
| `@vencord/venmic` | Linux audio loopback node addon | Optional runtime | Node native addon | Rust PipeWire/Pulse implementation or audited sidecar | Preserve behavior, redesign binding. |
| `adm-zip` | Backup restore/save | Runtime | No | Rust `zip` | Replace. |
| `spitroast` | Runtime patcher | JS preload/main plugin API | No | Keep small JS patcher | Retain JS layer. |
| Solid/Lune/Shelter defs/UI/motion/lucide | Setup + built-in Shelter plugin builds | Build/browser output | No | Keep JS UI bundles | Retain. |
| Monaco | Local Quick CSS editor | Browser asset | No | Retain lazily or use lightweight editor | Optional optimization. |
| `ws`, `ms`, `tslib` | No direct repository import found | Declared runtime | Unclear/transitive | Remove from direct manifest only after clean build/package test | Needs verification. |

The lockfile contains both pnpm and untracked Bun lock state; use pnpm as declared package manager. Do not infer runtime memory from lockfile disk size.

## 20. Electron-Specific Overhead

**Removed/reduced by Rust + system WebView:** Electron main executable/framework, embedded Node main/preload runtime, Electron IPC, bundled Electron Chromium distribution, Electron-specific Crashpad/config overhead, Electron builder/update stack, and Node worker/plugin-main execution if ported or retired.

**Still present in a webview architecture:** a browser renderer for Discord, JavaScript engine heap, DOM/layout/compositor/GPU, web/network/cache/cookies, WebRTC/audio/video codecs, media capture, service/renderer processes selected by WebView2, Discord's remote bundles, and injected JS/theme/plugin work.

Windows WebView2 supports script execution, early document scripts, host/web messaging, permissions, downloads, DevTools protocol, resource interception, and suspension APIs; it is not a single-process lightweight HTML widget. [WebView2 API overview](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/overview-features-apis) and [scripting reference](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/winrt/microsoft_web_webview2_core/corewebview2) describe these capabilities.

## 21. Electron APIs Used

| API family | Files / purpose | Criticality | Rust/WebView replacement |
|---|---|---|---|
| `app`, command line, single instance, lifecycle | `main.ts`, `window.ts`, setup/tray | Critical | Rust app lifecycle + single-instance/deep-link crate. |
| `BrowserWindow`, `webContents` | window, splash, setup, CSS editor, diagnostics | Critical | Native window/WebView registry. |
| `webContents.executeJavaScript`, `webFrame.executeJavaScript`, `insertCSS` | preloads, keybinds, injection | Critical | document-created script manifest + evaluate JS + DOM CSS manager. |
| `contextBridge`, `ipcMain`, `ipcRenderer` | bridge, IPC, all preloads | Critical | custom typed postMessage bridge. |
| `session`, `webRequest`, permission handler | CSP, proxy, filters, media | Critical | WebView2 adapter; partial/nonuniform other backends. |
| `protocol`, `net` | `legcord://` local content | Critical | custom scheme/virtual-host adapter. |
| `desktopCapturer`, display-media handler | screen share | Critical | platform capture adapter; high-risk parity. |
| `Tray`, `Menu`, badges/TouchBar | tray/menu/touchbar | High | Tauri tray/native menu; platform modules. |
| `shell`, `dialog`, `clipboard`, `nativeImage` | folders, URL, backup, context image copy | High | narrow native commands/dialog/clipboard/image decode. |
| `globalShortcut`, DBus | keybinds | Medium | platform shortcut service/portal API. |
| `powerSaveBlocker` | voice power setting | Medium | platform power-inhibition crate/API. |
| `crashReporter`, `getGPUFeatureStatus`, Chrome diagnostics | debugging/recovery | Medium | Rust crash reporting + WebView2 diagnostics/CDP. |
| `session.loadExtension` | user Chromium extensions | High compatibility risk | No assumed replacement. |

## 22. Electron -> Rust/WebView Mapping

| Current feature | Electron API | WebView2 equivalent | Wry/Tauri support | Custom implementation needed? | Difficulty |
|---|---|---|---|---|---|
| Main window / titlebar | BrowserWindow | Native HWND + controller | Good | Window style mapping | Medium |
| Early preload | preload/context isolation | `AddScriptToExecuteOnDocumentCreatedAsync` | Generic init script insufficient for all remote cases | Bootstrap ordering/origin guards | High |
| Run page JS / CSS | webFrame/webContents | ExecuteScript + injected `<style>` | `eval` exists, backend-specific behavior | Lifecycle IDs/removal | Medium |
| IPC bridge | contextBridge/ipc | `postMessage`/WebMessageReceived | Tauri commands/capabilities | Yes: typed Discord bridge | High |
| Cookies/cache/profile | session | WebView2 profile/user-data folder | Backend profile config varies | Migration/login strategy | Medium |
| Permissions | session handler | PermissionRequested | Tauri backend-dependent | Origin/type policy | Medium |
| Request block/replace | webRequest | WebResourceRequested filters/response | Not common Wry API | Windows adapter; proxy/JS elsewhere | High |
| Header/CSP mutations | webRequest | Intercept/replace response where feasible | No portable abstraction | Local proxy or redesigned CSP on Apple/Linux | High |
| `legcord://` assets | protocol.handle | virtual host mapping / resource response | Custom protocol | Yes for path validation | Medium |
| Popouts / `window.open` | setWindowOpenHandler | NewWindowRequested | Window events only | popup registry and policy | Medium |
| Tray/menu/notifications | Tray/Menu/Notification | native APIs | Tauri plugin support | Badge behavior | Low-medium |
| File dialogs/folders | dialog/shell | native Rust dialog/shell | Tauri plugins | Per-command allowlist | Low |
| Clipboard/image copy | clipboard/nativeImage/copyImageAt | clipboard/DOM fallback | plugins/platform APIs | Native image copy | Medium |
| Screen share | desktopCapturer/media handler | WebView2 capture APIs + browser media | Not generic | Full CaptureService | High |
| Global shortcuts | globalShortcut/DBus | OS APIs | Tauri plugin/backend-specific | Linux portal service | Medium-high |
| Power inhibition | powerSaveBlocker | Win32/macOS/Linux power APIs | Plugin/crate | yes | Medium |
| Chrome extensions | loadExtension | No assumed equivalent | No | New plugin model | Very high |
| Auto update | electron-updater | Native updater | Tauri updater | release/key migration | Medium |
| DevTools/GPU diagnostics | Chrome URLs | CDP / WebView2 devtools | limited generic | diagnostics panel | Medium |

WebView2 document-created scripts run after the global object but before document parsing and page scripts, and persist across top-level/child-frame navigations until explicitly removed. That meets the timing required by the current WebRTC patches, provided registration finishes before navigation. [Microsoft's API reference](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2.addscripttoexecuteondocumentcreatedasync?view=webview2-dotnet-1.0.4129.50).

## 23. Windows WebView2 Feasibility

**Feasible, with a custom adapter.** Windows is the correct first target because Discord compatibility is closest to Chromium and WebView2 offers early script injection, `ExecuteScript`, web/native messaging, permission events, new-window handling, resource filters/replacement, cookies/profile APIs, and DevTools protocol access.

`WebResourceRequested` needs URL/resource filters and can block the request while the host handles it. It can provide a response, making the existing telemetry block and local asset protocol practical. Frame interception needs the newer source-kind filter behavior; test Discord embeds explicitly. [WebView2 request management](https://learn.microsoft.com/en-us/microsoft-edge/webview2/how-to/webresourcerequested) and [frame interception notes](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/frames) are the implementation references.

Do not claim runtime-wide process reuse as a guaranteed RAM win. The Evergreen WebView2 runtime removes Legcord's bundled Chromium distribution, but each app/profile/instance can still cause Edge runtime processes. Measure the same profile/scenario against Electron.

Use one explicitly located WebView2 user-data folder per Legcord profile. Start with persistent storage; document that Electron cookies/local-storage cannot safely be copied wholesale into WebView2's different profile format, so initial login migration may be required.

## 24. Linux WebKitGTK Feasibility

**Partial; do not promise feature parity in phase one.** WebKitGTK offers user-content scripts/styles, isolated script worlds, JavaScript evaluation, permissions, navigation signals, download/UI hooks, and user-content filters. [WebKitWebView reference](https://webkitgtk.org/reference/webkit2gtk/stable/class.WebView.html) and [UserContentManager reference](https://webkitgtk.org/reference/webkit2gtk/stable/class.UserContentManager.html) support those assertions.

However, Discord/Vencord/Equicord assume Chromium behavior in places; full HTTP header/response rewriting, Chrome-extension compatibility, codec behavior, Wayland capture/PipeWire, GPU driver support, and screen-share quality patches require a dedicated compatibility matrix. WebKitGTK permissions are host decisions via `permission-request`, including user-media related request types. [Permission request reference](https://webkitgtk.org/reference/webkit2gtk/2.42.0/iface.PermissionRequest.html). Treat Linux as a separate backend after the Windows bridge and portable JS contract are stable.

## 25. macOS WKWebView Feasibility

**Partial; native shell/window/tray/notifications are feasible, while media and interception are high risk.** WKWebView supports configuration, navigation delegation, custom scheme handlers, user content, and UI delegate-controlled popups. Its UI delegate also has media capture permission methods. [WKWebView](https://developer.apple.com/documentation/webkit/wkwebview), [WKNavigationDelegate](https://developer.apple.com/documentation/webkit/wknavigationdelegate), and [WKUIDelegate](https://developer.apple.com/documentation/webkit/wkuidelegate) document those primitives.

WKWebView does not provide an Electron-style, universal HTTP response/header interceptor. Use content rules for static blocking, native/custom schemes for local content, and consider renderer-side logic or a local proxy only where justified. Screen sharing/system-audio/WebRTC codec behavior must be a formal acceptance gate; do not port Electron Chromium flags literally.

## 26. Rust/Tauri/Wry Target Architecture

```text
                         +-----------------------------+
                         | Rust Native Core            |
                         | lifecycle / config / crash  |
                         | storage / updater / security|
                         +---+------+------+------+-----+
                             |      |      |      |
  native integrations -------+      |      |      +------- platform services
  tray, windows, capture, shortcuts  |      |               (DBus, PipeWire, Touch Bar)
                                    bridge  |
                                            v
                         +-----------------------------+
                         | ShellBackend                |
                         | WebView2 / WKWebView / GTK  |
                         | navigation, scripts, CSS,   |
                         | messages, permissions, net  |
                         +-------------+---------------+
                                       |
                          approved Discord top frames
                                       |
         +-----------------------------+---------------------------+
         | Discord web app + JS modification layer                 |
         | Shelter / Vencord / Equicord / themes / runtime plugins |
         +----------------------------------------------------------+
```

Tauri 2 is useful for native lifecycle, tray, updater, dialog, notification, and capability governance. Its capability model grants permissions per webview/window, which is useful for local setup/editor surfaces. [Tauri capabilities](https://v2.tauri.app/security/capabilities/). The remote Discord webview must not receive broad Tauri core/plugin permissions. Where raw WebView2 APIs are needed, add a narrow Windows backend rather than weakening all webviews.

## 27. Compatibility Layer Design

```rust
/// No arbitrary eval/native command endpoint is exposed to Discord.
trait ShellBackend {
    fn create_discord_view(&self, profile: &ProfileId, opts: DiscordViewOptions) -> Result<ViewId>;
    fn navigate(&self, view: ViewId, url: ApprovedDiscordUrl) -> Result<()>;
    fn install_document_script(&self, view: ViewId, script: DocumentScript) -> Result<ScriptId>;
    fn remove_document_script(&self, view: ViewId, script: ScriptId) -> Result<()>;
    fn evaluate_page_script(&self, view: ViewId, script: TrustedPageScript) -> Result<()>;
    fn set_user_style(&self, view: ViewId, id: StyleId, css: &str) -> Result<()>;
    fn remove_user_style(&self, view: ViewId, id: StyleId) -> Result<()>;
    fn set_navigation_policy(&self, policy: NavigationPolicy) -> Result<()>;
    fn set_permission_policy(&self, policy: PermissionPolicy) -> Result<()>;
    fn set_request_rules(&self, rules: &[RequestRule]) -> Result<()>;
    fn post_message(&self, view: ViewId, message: HostEvent) -> Result<()>;
}

enum BridgeCommand {
    GetPublicConfig, SetValidatedConfig, Window(WindowAction),
    Themes(ThemeAction), Capture(CaptureAction), GameActivity(GameAction),
    OpenExternal(ApprovedExternalUrl), FileDialog(FileDialogKind),
    PluginStorage(ScopedPluginStorageAction), PowerInhibit(bool),
}
```

Bridge inbound validation must require: known view ID, top-level `https://discord.com`/Canary/PTB origin, approved frame relationship, schema-valid command, capability allowed for that surface, and user consent where required. `PluginStorage` must derive plugin identity from a registered plugin manifest/session—not trust a caller-supplied ID alone.

Adapters:

* **Electron adapter (temporary):** implement the trait around existing APIs while extracting business logic from `ipc.ts`.
* **Windows:** WebView2 profile, document-created bootstrap scripts, web messages, resource filters, `NewWindowRequested`, `PermissionRequested`, Win32 tray/capture/power services.
* **Linux:** WebKitGTK content manager, message handler, policy/permission callbacks, portal/PipeWire/DBus services; omit unsupported request rewrites until validated.
* **macOS:** WKUserContentController/message handler/navigation/UI delegates, AppKit status item/notifications/power, ScreenCaptureKit/AVFoundation decisions.

## 28. Security Architecture

### Confirmed security findings

| ID | Severity | Files | Evidence | Recommendation |
|---|---|---|---|---|
| SEC-01 | P0 | `preload/bridge.ts`, `ipc.ts` | Remote renderer receives powerful `window.legcord` APIs; main handlers do not verify sender origin/frame. A Discord XSS, malicious injected bundle, or inherited preload in a popup could reach config, dialogs/folders, restart/quit, clipboard/debug data, and optionally plugin storage. | Replace with origin/frame-checked, typed, least-capability message bridge. |
| SEC-02 | P0 (trust boundary) | `plugins/manager.ts:176-181` | Enabled filesystem **main** plugins receive `app`, `BrowserWindow`, `ipcMain`, `dialog`, `shell`, and patcher; module code is imported dynamically. | Declare as full-trust native extensions, require explicit install/permission consent, separate from renderer plugins, and avoid supplying raw host handles. |
| SEC-03 | P1 | `extensions/modloader.ts`, `common/consts.ts` | Startup downloads executable mod JavaScript from mutable GitHub release/raw URLs and executes it; ref check/download does not verify artifact hash/signature. | Signed/versioned manifest, SHA-256 verified content, cached rollback, explicit update UI. |
| SEC-04 | P1 | `extensions/csp.ts`, default `csp:"none"` | Default policy removes Discord main-frame CSP to facilitate injections. | New architecture must keep CSP/isolated worlds and allow only required bridge/bootstrap paths. |
| SEC-05 | P2 | `ipc.ts` theme install and local plugin file handling | Path traversal protections exist for plugin storage/backup, but theme IDs passed to edit/folder routes are not validated consistently; remote themes are arbitrary CSS. | Centralize path-capability validation and theme trust disclosure. |

Keep `contextIsolation`, renderer sandboxing, node integration off, navigation denial/external URL policy, and unknown-permission deny behavior. Current main Discord window is configured securely in those respects, but the bridge is the decisive escalation boundary.

## 29. Performance Optimization Opportunities

1. Enable true hidden-state throttling/suspension outside calls, then measure.
2. Delete periodic bounds polling and invite DOM polling.
3. Fix `getDisplayMedia` patch idempotency before optimizing any media flags.
4. Make arRPC worker demand-driven and stop it on feature disable/window destruction.
5. Put mod update checks off the startup critical path and verify artifacts.
6. Make windows, global listeners, request rules, and IPC registries singleton services.
7. Add LRU/TTL budgets to RPC asset/application caches and clear Touch Bar guild state.
8. Consolidate styles/scripts into an injection registry keyed by document + source, with removals.
9. Ship docs/screenshots only in source/release metadata, not runtime assets, and lazy-load Monaco.
10. Use WebView2 `TrySuspend`/resume policy only after confirming voice/video/capture exemption behavior; do not suspend active calls.

## 30. Confirmed Memory Problems

No high-volume memory leak is proven from the repository. Confirmed resource-retention problems are MEM-01/02/03 above. The tray-hidden renderer retention is intentional lifecycle behavior, not a leak, but it is likely the dominant native-side reason memory remains allocated while “closed.”

## 31. Confirmed CPU Problems

CPU-01, CPU-02, CPU-03, and the default non-throttled hidden renderer behavior are confirmed. Only the last can plausibly be a major general idle cost; its magnitude must be profiled. arRPC’s scan work is likely but owned by an external dependency.

## 32. Suspected Problems Requiring Profiling

* Discord renderer React/DOM/Gateway activity and service workers.
* Vencord/Equicord/Shelter downloaded bundle CPU/heap and any user-installed plugins/themes/extensions.
* GPU composition and video encode path by device/driver/quality setting.
* Chromium cache/media buffers and disk cache disabled by user settings.
* Process count/memory difference between Electron and equivalent WebView2 profile.
* Whether dynamic tray favicon updates, title updates, and notification patches contribute materially in large active guilds.
* arRPC scan implementation, process enumeration cost, and websocket/IPC volume.

## 33. Dead / Legacy / Redundant Code

| Item | Evidence | Disposition |
|---|---|---|
| `src/discord/preload/optimizer.ts` | No import from preload entry or another source; not a rolldown input | **Safe to remove after clean build confirms no external inclusion.** |
| `src/splash/redirect.html` | No source path loads it; it expects `window.legcord.channel`, which bridge does not expose | **Needs verification; likely dead.** |
| `inviteWindow` export in `discord/window.ts` | No repository reference found | **Safe to remove after TypeScript/build verification.** |
| `customJsBundle` / `customCssBundle` config keys | Present in types/defaults, no consumer found | **Needs migration/compatibility decision.** |
| `getLangSync` IPC handler | No sender found | **Safe after IPC integration test.** |
| `disableAutogain` feature | `patches.mts:262` requests `legcord://assets/js/disableAutogain.js`; no such checked-in asset exists | **Confirmed broken P2 feature; fix or remove setting before migration.** |
| Splash `installState` UI | `splash.html` reads fields absent from exposed `internal` bridge | **Likely obsolete/dead branch.** |
| Direct `ms`, `ws`, `tslib` dependencies | No direct source import | **Needs clean package/build test before manifest removal.** |

## 34. Migration Plan

### Phase 0 — establish measurements and fix source-obvious defects

Instrument current Electron release; fix idempotent capture wrapper, plugin unsubscriptions, bounds/invite polls, missing asset/setting, and singleton window services. These fixes improve baseline and reduce migration noise.

### Phase 1 — extract shell contracts in TypeScript

Separate `ipc.ts` business operations from Electron plumbing. Introduce `ShellBackend`, `NativeServices`, injection manifest, `WindowRegistry`, and typed bridge schemas while retaining Electron implementation.

### Phase 2 — create Rust core and profile/data layout

Create Tauri/Rust workspace, config schema migration, logging/crash strategy, profile directory layout, secure local assets scheme, tray/window/deep-link service, and origin policy. Do not attempt cookie-file conversion; plan a login migration.

### Phase 3 — Windows WebView2 Discord prototype

Build a single Discord WebView2 profile/window with approved navigation, persistent cookies, early bootstrap scripts, postMessage bridge, JS/CSS theme injection, and page lifecycle registry. Benchmark it before claiming a memory improvement.

### Phase 4 — preserve modification ecosystem

Port Shelter built-in bridge calls, Vencord/Equicord/custom injection, Quick CSS, themes, and runtime renderer plugins. Implement strict plugin tiers: renderer-only untrusted-by-default, scoped storage, and separately consented native extensions.

### Phase 5 — window/tray/notifications/native UX

Port titlebar modes, transparency, badges, tray lifecycle, menu/shortcuts, dialogs, clipboard, deep links, popouts, setup, CSS editor, diagnostics, and backup.

### Phase 6 — network/security

Port proxy/profile policy, telemetry filters, external URL routing, custom scheme, permissions, CSP strategy, and only the request rewrites that still demonstrate value.

### Phase 7 — media/RPC parity

Implement/test Windows screen source picker, mic/camera permissions, voice/video/screen share, hardware codec policy, rich presence/process scanning, and power inhibition. Gate release on calls/streams—not mere page load.

### Phase 8 — Linux/macOS feasibility branches

Use portable bridge/JS modules but separate backend acceptance tests. Retain Electron fallback on unsupported media/interception combinations until parity is demonstrated.

### Phase 9 — optimize measured bottlenecks

Apply suspension/hidden policy, cache budgets, loader deferral, package trimming, and media tuning based on traces.

### Phase 10 — feature parity and staged rollout

Run matrix tests, migration/rollback tests, crash telemetry, feature flags, and side-by-side Electron fallback before defaulting users to native shell.

## 35. Feature Parity Matrix

| Existing capability | Current implementation | Electron dependency | WebView2/Rust path | Portable JS? | Status |
|---|---|---:|---|---:|---|
| Discord stable/PTB/Canary | `window.ts` URL load | Low | WebView navigation/profile | Yes | Feasible |
| Login/cookies/cache | Electron session | High | WebView2 profile | No | Feasible; migration risk |
| Vencord/Equicord/Shelter | preload JS | Medium | early/page scripts | Yes | Feasible Windows |
| Custom JS/CSS bundle | user-data + webFrame | Medium | verified script/style registry | Yes | Feasible |
| BetterDiscord themes/Quick CSS | watcher + IPC styles | Medium | Rust watch + DOM styles | Yes | Feasible |
| Runtime renderer/preload plugins | dynamic source execution | High | JS plugin host | Mostly | Feasible with redesign |
| Main-process plugins | raw Electron API | Critical | capability-scoped native extension host | No | Redesign |
| Chromium extensions | `loadExtension` | Critical | no assumed equivalent | No | Not parity target initially |
| Tray/minimize/start-in-tray | Electron Tray/window | High | native tray/window | No | Feasible |
| Custom titlebars/materials | BrowserWindow options/CSS | High | native windows + CSS | Partial | Feasible |
| Badges/favicons | nativeImage/dock/overlay | High | platform badges/icons | No | Feasible |
| Native menus/zoom/DevTools | Menu/webContents | High | native menu/CDP | Partial | Feasible |
| Deep links | protocol/app | High | single-instance/deep link | No | Feasible |
| External link policy | shell/window open handler | High | navigation/popup policy | No | Feasible |
| Proxy | command/session proxy | High | backend profile + Rust requests | No | Feasible Windows; test others |
| Telemetry filtering | webRequest | High | WebView2 rules | Partial | Feasible Windows |
| CSP/header rewrites | webRequest | High | WebView2 or redesign/proxy | Partial | High risk non-Windows |
| Native image copy | context menu/net/nativeImage | High | native clipboard/image bridge | No | Feasible |
| CSS editor | BrowserWindow + Monaco | Medium | local WebView/editor | Yes | Feasible/lazy |
| Backup/restore | Node ZIP/dialog | High | Rust zip/dialog | No | Feasible |
| Global keybinds | Electron/DBus | High | OS/portal service | No | Feasible; Linux risk |
| arRPC games | Node worker/arrpc | High | Rust service or sidecar | Page dispatch only | Redesign |
| Voice/camera/video | Chromium/Electron | High | WebView2/WebRTC | Patch JS | Qualification required |
| Screen picker/audio loopback | desktopCapturer/Venmic | Critical | capture service | Partial | High risk |
| Power inhibition | powerSaveBlocker | High | OS power API | No | Feasible |
| Touch Bar | Electron TouchBar | High | AppKit Touch Bar | No | macOS-specific |
| Updates | electron-updater/splash | High | native updater | No | Feasible |
| Crash/relaunch/GPU info | Electron/Chrome pages | High | Rust crash + CDP | No | Partial |

## 36. Benchmark Plan

Use a fresh profile and the same Discord account/server/mod set for Electron and prototype. Record build SHA, WebView2 runtime version, OS build, GPU driver, power state, network RTT, settings, and enabled plugins/themes.

| Scenario | Duration | Measure |
|---|---:|---|
| Cold startup / warm startup | 10 runs each | process creation, first paint, Discord interactive, disk I/O, network waterfall. |
| Idle visible / minimized / tray | 5, 15, 30 min | per-process RSS/private bytes, committed GPU memory, CPU %, wakeups, timers, network, Page Visibility. |
| Normal active chat / large server | 10 min | renderer CPU, long tasks, DOM nodes, heap/allocation snapshots, GPU. |
| Voice / video / screen 1080p30 and 4K60 | 10 min each | encoder implementation, frame drops, CPU/GPU, latency, media buffers, battery/thermals. |
| Mod-heavy | 10 min | one baseline, Shelter, Vencord, Equicord, user runtime plugins/themes separately. |
| Multiple windows/popouts/editor | 5 min | renderer count, memory ownership, cleanup after close. |
| Tray resume / reconnect / crash-restart | 10 cycles | process/window leaks, recovery, permission/cookie continuity. |

Windows tools: Task Manager and Process Explorer for process-private bytes/command lines; Windows Performance Recorder/Analyzer for CPU, disk, DWM/GPU, scheduling; WebView2 DevTools Protocol/DevTools Performance and Memory panels; ETW GPU/Media providers; `chrome://gpu`/WebRTC internals on current Electron only; and application structured lifecycle logs. Heap snapshots must compare retained DOM/listeners before/after repeated screen shares, plugin reloads, and tray cycles.

## 37. Performance Targets

These are targets, not achieved values. “Expected” identifies the intended direction only; it is not a benchmark claim.

| Metric | Current | Target | Expected from native shell | Measured |
|---|---|---|---|---|
| Shell cold start | Unknown | Define p50/p95 after baseline | Less packaged runtime initialization | No |
| Discord time-to-interactive | Unknown | No regression under equal network | May improve modestly; Discord fetch dominates | No |
| Idle RAM | Unknown | Measure total and per-process; reduce shell-exclusive allocation | Electron/Node reduction, renderer remains | No |
| Active RAM | Unknown | No regression excluding proven shell reduction | Depends on renderer/mods/media | No |
| Idle CPU, visible/tray | Unknown | Near-zero host CPU; renderer throttled outside call | Major opportunity from lifecycle policy | No |
| Voice/video CPU/GPU | Unknown | No regression; HW encode active where supported | Requires backend qualification | No |
| Background work | Unknown | No 1 s polls; no scanner when disabled | Direct source fixes | No |
| Process topology | Unknown | No Electron main/Node worker in native build; document remaining WebView processes | Reduced shell only | No |
| Disk footprint | Unknown | Package no bundled Electron Chromium; profile controlled separately | Material installer reduction | No |
| Battery | Unknown | Lower idle energy; equal/better call quality | Must measure on laptop | No |

## 38. Risk Register

| Risk | Severity | Mitigation |
|---|---|---|
| Discord changes break internal patches | P0 | Keep JS bootstrap modular, feature flags, compatibility probes, Electron fallback. |
| Remote Discord gains native bridge access | P0 | Origin/frame/capability validation, no generic invoke, minimal commands. |
| WebView2 media/screen share differs from Electron | P0 | Acceptance tests before migration; retain Electron path/fallback. |
| Linux/macOS lack request/media parity | P0 | Windows-first, backend feature gates, do not market unsupported parity. |
| Downloaded mod supply chain | P1 | Hash/sign versioned manifests and consent/rollback. |
| Login/profile migration breaks sessions | P1 | First-run profile strategy, clear user messaging, rollback preserves Electron profile. |
| Plugin ecosystem expects Electron raw APIs | P1 | Tiered capability model and compatibility shim; do not expose Rust internals. |
| Claimed RAM win is actually Discord workload | P1 | Side-by-side measured process/heap attribution. |
| Tauri abstraction hides needed WebView2 controls | P1 | Backend trait and raw Windows adapter, avoid unsupported generic promises. |
| Native updater/release signing migration | P2 | Start after shell prototype with staging/rollback. |

## 39. Recommended Order of Implementation

1. Establish benchmark harness and current-Electron traces.
2. Correct background policy, polling, screen-share wrapper, plugin cleanup, and broken asset baseline defects.
3. Extract typed shell contracts and one application-level window/event owner.
4. Build secure Rust config/profile/window/tray core.
5. Build one Windows WebView2 Discord window with persistent profile and approved navigation.
6. Implement document-created script registry and narrow postMessage bridge.
7. Port themes, Shelter settings bridge, Vencord/Equicord/custom bundles, and renderer plugins.
8. Port popup/media/capture/RPC services and certify calls/streams.
9. Port request rules/proxy/CSP policy after proving individual rules necessary.
10. Add macOS/Linux backends only behind parity tests and a maintained Electron fallback.

## 40. Final Technical Assessment

The repository is a viable candidate for a Windows-first Rust/WebView2 shell, but not for a “quick Electron removal.” The architecture is already mostly split between a thin native shell and a JS modification layer, which is favorable. The hard work is recreating secure early injection, capture/media, request policy, profile lifecycle, plugin trust boundaries, and popup behavior—not drawing the initial Discord page.

The source provides clear baseline fixes and one major lifecycle policy error: tray-hidden operation currently leaves Discord unthrottled by default. Correcting that and measuring the result should precede any claim that Electron itself is the principal cause of high CPU/RAM. Electron replacement can reduce fixed shell/package overhead; only measurement can show how much of total observed usage survives as Discord renderer/media/mod workload.

## What I would change first

| Change | Reason | Files | Expected impact | Risk / difficulty |
|---|---|---|---|---|
| Make hidden renderer throttling policy-aware | Default tray state currently keeps renderer foreground-like | `common/config.ts`, `discord/window.ts` | Largest likely idle CPU/battery reduction | Medium: exempt active voice/video/capture; medium difficulty |
| Delete bounds poll | Events already persist bounds | `discord/window.ts:440` | Removes 60 main-process wakeups/hour/window | Low / low |
| Replace invite poll | History hooks already exist | `preload/inviteBackButton.mts` | Removes persistent page timer | Low / low |
| Make `patchNavigator` idempotent | Repeated shares stack wrappers | `screenshare/components/ScreensharePicker.tsx` | Fixes progressive capture overhead/correctness | Medium / medium |
| Unsubscribe Shelter plugins and clear guild state | Reload duplicates callbacks/buttons | `shelter/power`, `shelter/touchbar` | Stops plugin-reload leaks | Low / low |
| Move mod update check post-ready + verify hashes | GitHub request blocks startup and executes mutable code | `extensions/modloader.ts`, `common/consts.ts` | Faster/reliable startup; security | Medium / medium |
| Create a one-owner WindowRegistry | Prevent duplicated app listeners/stale first-window IPC target | `window.ts`, `ipc.ts`, tray | Correctness and cleanup | Medium / medium |
| Bound RPC caches and stop worker on disable | Infinite app/asset retention and needless scanning | `shelter/rpc/asset.ts`, `rpcProcess.ts` | Long-session memory/idle CPU | Medium / medium |
| Remove/fix dead broken features | Missing autogain JS and stale paths mislead users | `patches.mts`, assets/settings | Reliability, smaller scope | Low / low |
| Build Windows prototype behind feature flag | Measure real shell reduction before a rewrite | new Rust workspace | Evidence-based migration | High / high |

## What should NOT be rewritten

* The renderer-side Shelter settings/pages, custom theme model, CSS injection semantics, and most Vencord/Equicord/Shelter bootstrap logic should remain JavaScript/TypeScript.
* Discord should continue to render from its official web app.
* Screen-share codec/quality behavior should be retained as JS policy after the host capture surface is replaced; do not recode Discord media logic in Rust.
* Solid/Lune build output for native-looking Discord settings can be retained.

## What must be redesigned

* Electron main/preload/IPC and raw plugin-main Electron API.
* Browser window/profile/request/interception implementation and local protocol.
* Capture, global shortcuts, arRPC process scanner, updater, power inhibition, tray/menu, and platform media integrations.
* Origin-aware security model and signed/moderated mod update pipeline.
* Chrome extension support: no parity promise until a separate architecture exists.

## Biggest unknowns

* Actual Electron vs Discord vs mod memory/process attribution.
* Discord's idle timers/network/GPU behavior and production module layout.
* User plugins/extensions/themes and their lifecycle correctness.
* WebView2/WebKitGTK/WKWebView codec, capture, permission, popup, and request-rewrite compatibility on supported platforms.
* WebView2 process reuse and total RAM effect under representative profiles.

## Next profiling experiments

1. Electron idle visible/minimized/tray 5/15/30-minute traces with `sleepInBackground` both false and true, no mods first.
2. A/B remove the two 1-second polls and arRPC worker, then quantify CPU wakeups.
3. Heap/allocation snapshots before/after 20 screen-share cycles and 20 Shelter plugin reloads.
4. Process Explorer/WPR per-process Electron tracing through cold start, active chat, voice, video, and 1080p/4K share.
5. Minimal WebView2 prototype with the same Discord account/profile scenario and no bridge, then add injection bridge/mods one stage at a time.

---

## Appendix A — Complete File Inventory

The table groups files only where identical static role makes individual rows non-informative; every repository-relative tracked source/config/data path is named.

| Subsystem | Paths (type) | Context / purpose | Startup / continuous / resource relevance |
|---|---|---|---|
| Root metadata | `package.json`, `pnpm-lock.yaml`, `bun.lock`, `pnpm-workspace.yaml`, `tsconfig.json`, `biome.jsonc`, `lune.config.js`, `rolldown.config.ts`, `electron-builder.ts`, `license.txt` | dependencies, bundling, package targets, lint/license | Build/startup metadata; Electron/package migration-critical. |
| Main bootstrap | `src/main.ts`, `src/updater.ts`, `src/protocol.ts`, `src/power.ts`, `src/dbus.ts`, `src/rpc.ts` (TS) | application lifecycle, updater, local protocol, power, DBus, worker entry | `main.ts` startup-critical; worker/scanner/background services. |
| Shared paths/types | `src/shared/consts/paths.ts`; `src/@types/{consts,cssModules,i18nStrings,keybind,legcordWindow,ModBundle,settings,themeManifest,windowState}.d.ts` | type-only API/settings contracts | No runtime except shared paths; bridge/migration contract source. |
| Common storage/utility | `src/common/{backup,blacklistGame,commandDefinitions,config,consts,detectables,dom,flags,forceQuit,handleCommands,keybindActions,lang,proxy,sanitization,sleep,themes,version,windowBounds,windowState,windowVisibility}.ts` | config/state, backup, themes, proxy, sanitization, window state/actions | Config/flags/proxy startup; theme watchers and caches continuous; migration-critical. |
| Discord shell | `src/discord/{window,ipc,tray,screenshare,screenshareSources,rpcProcess,venmic,globalKeybinds,menu,touchbar}.ts` | window, IPC, tray, capture, RPC, audio, menus | Main-process native integrations; primary performance/lifecycle scope. |
| Extensions/plugins | `src/discord/extensions/{csp,modloader,plugin}.ts`; `src/discord/plugins/manager.ts` | CSP/header changes, bundle fetch, Chrome extensions, filesystem plugin host | CSP at ready; bundle/plugin init startup; extension/plugin code can continue. |
| Discord preload | `src/discord/preload/{preload.mts,bridge.ts,inviteBackButton.mts,newTitlebar.mts,optimizer.ts,patches.mts,plugins.mts,themes.ts,titlebar.mts}`; `src/discord/preload/mods/{custom,equicord,shelter,vencord}.mts` | bridge, early patches, title UI, CSS/JS mods/plugins | Runs every Discord document; DOM/timer/observer/IPC relevance; migration-critical. |
| Setup/splash/editor | `src/setup/{main.ts,preload.mts,setup.tsx,setup.html,setup.css,tray.ts}`; `src/splash/{main.ts,preload.mts,splash.html,redirect.html}`; `src/cssEditor/{main.ts,preload.mts,editor.html}` | first-run UI, splash, local Monaco editor | On-demand extra renderers; splash network/timer; editor lazy. |
| Shelter core plugins | `src/shelter/{power/index.ts,rpc/index.ts,rpc/asset.ts,titlebar/index.ts,touchbar/index.ts,screenshare/index.tsx,screenshare/components/ScreensharePicker.tsx,screenshare/components/SourceCard.tsx}` and each sibling `plugin.json` | Discord Flux integrations, rich presence, title bar, Touch Bar, media picker | Injected renderer code; subscriptions/timers/media/caches. |
| Shelter settings root/pages | `src/shelter/settings/{index.ts,settings.ts,plugin.json}`; `pages/{SettingsPage,ThemesPage,KeybindsPage,PluginsPage,RegisteredGamesPage}.tsx`; matching `*.module.css` | settings registration and Solid pages | Mount-on-settings UI; no steady work except page effects/timers. |
| Shelter settings components | `components/{AboutPopup,AddDetectableModal,BackupSection,DetectableCard,Dropdown,DropdownItem,EmptyState,HeroUpdater,KeybindCard,KeybindMaker,PluginCard,SegmentedControl,SettingsPageHeader,SettingsPanel,SettingsSearch,SupportBanner,TextBoxItem,ThemesCard}.tsx`; matching module CSS; `components/icons/{InfoIcon,QuickActionIcons,SettingsNavIcons,SettingsPanelIcons}.tsx` | modal/forms/cards/icons | UI-only except listeners/timers/fetches documented above. |
| Static injected JS/CSS | `assets/app/js/{adguard,patchVencordQuickCSS}.js`; `assets/app/css/{baseTitlebar,darwinTitlebar,discord,editor,inviteBack,linuxTitlebar,titlebar,winTitlebar}.css` | page patches/styles | `adguard.js` observer is runtime-relevant; other CSS passive. |
| App/images | `assets/app/{Loading.webm,mockup.jpg,Update.webp,custom.png,desktop.png,equicord.png,Folder.png,Download.png,Edit.png,native.png,overlay.png,shelter.svg,Trash.png,UpgradeArrow.png,vencord.png,Window.png,WindowUnsupported.png,Wordmark.png}` | splash/setup/settings art | Runtime assets; `Loading.webm` decodes in splash; others on demand. |
| Root images | `assets/{ac_black_plug.png,ac_black_plug_hollow.png,ac_icon.png,ac_icon_transparent.png,ac_plug_classic.png,ac_plug_colored.png,ac_white_plug.png,ac_white_plug_hollow.png,badge-1.ico,badge-2.ico,badge-3.ico,badge-4.ico,badge-5.ico,badge-6.ico,badge-7.ico,badge-8.ico,badge-9.ico,badge-10.ico,badge-11.ico,clsc-dsc-tray.png,deafen.png,deafen-off.png,desktop.png,disconnect.png,dsc-tray.png,mute.png,mute-off.png,Square150x150Logo.png,Square44x44Logo.png,StoreLogo.png,Wide310x150Logo.png}` | tray, badges, Touch Bar, package art | Native image decode on relevant UI events. |
| i18n data | `assets/lang/{ar,cs,da,de,en-US,es,es_419,fa,fi,fil,fr,hu,id,it,ja,ko,lt,nb_NO,nl,pl,pt_BR,pt_PT,ro,ru,sk,sv,ta,th,tr,uk,zh_Hans,zh_Hant}.json` | translations | One selected language loaded/cached; no continuous work. |
| Docs/examples | `README.md`, `CONTRIBUTING.md`, `docs/plugin-system.md`, `docs/examples/hello-plugin/{main,preload,renderer}.js`, `docs/examples/hello-plugin/manifest.json`, `docs/examples/clipboard-fallback-plugin/{README.md,custom-bundle.js,renderer.js,manifest.json}` | documentation and plugin examples | Not packaged runtime unless manually copied. |
| Tests | `tests/register-ts.mjs`, `tests/screenshare-previews.test.ts` | source resolver and source-thumbnail tests | Build/test-only; test execution blocked by corrupt local Bun shim. |
| Build scripts | `scripts/{copyVenmic,spdxLicenceDebianFormat}.ts`, `scripts/utils/updateMeta.ts`, `scripts/build/sandboxFix.mjs` | native addon copy, Debian metadata, release metadata, AppImage wrapper | Build/release only. |
| Packaging/containers | `build/{.DS_Store,entitlements.mac.plist,icon.icns,icon.ico,icon.png,appx/Square150x150Logo.png,appx/Square44x44Logo.png,appx/StoreLogo.png,appx/Wide310x150Logo.png}`; `meta/app.legcord.Legcord.metainfo.xml`; `docker-compose.yml`; `docker/{start.sh,Dockerfile.bookworm,Dockerfile.pacman,Dockerfile.trixie,Dockerfile.ubuntu}` | platform icons, entitlement, AppStream, packaging test images | Build/release-only; Electron-specific replacement scope. |

## Appendix B — Electron API Inventory

See Section 21 for the API-to-replacement matrix. Direct API imports are in `main.ts`, `protocol.ts`, `power.ts`, `dbus.ts`, setup/splash/editor, common config/backup/proxy/state, every `src/discord/*.ts` native shell module, and preload bridge/mod files. No `window.require` use was found. Direct `require("electron")` remains in several preloads, while ESM imports are used elsewhere. `BrowserWindow`, `webContents`, `webFrame`, `ipcMain`, `ipcRenderer`, `contextBridge`, `session`, `webRequest`, `protocol`, `Tray`, `nativeImage`, `shell`, `powerSaveBlocker`, and `app` are all in active use.

## Appendix C — Timers / Observers / Listeners / Resource Lifetimes

| Resource | Created | Destroyed | Duplication/leak assessment |
|---|---|---|---|
| Bounds interval | `window.ts:440`, 1 s/window | `closed` | Cleaned, but unnecessary continuous work. |
| Bounds debounce | `window.ts` move/resize | overwritten/executes; not explicitly cleared on close | Low risk; clear on close for rigor. |
| Invite interval | `inviteBackButton.mts:71` | Never until document destruction | One persistent polling timer per Discord document. |
| Root MutationObservers (3) | `patches.mts` when no document root | `disconnect()` after root exists | Correct one-shot cleanup. |
| YouTube observer | `adguard.js` | Never until frame destruction | Intentional but per-mutation DOM scan. |
| Theme watcher | `themes.ts:startThemeWatcher` | `stopThemeWatcher`, never called in normal shutdown | Singleton; should stop during app shutdown. |
| Theme debounce | `themes.ts` | cleared in stop/replaced | Correct singleton behavior. |
| Quick CSS watcher/debounce | `themes.ts:initQuickCss` | window `closed`, disable/replacement | Correct for one tracked watcher; multiwindow semantics need redesign. |
| Screen quality timers | `screenshare/index.ts` | stream delete / `onUnload` | Correct bounded cleanup. |
| `getDisplayMedia` wrapper | picker `patchNavigator` | Never restored | Confirmed stacking on repeated selection. |
| Shelter settings `TRACK` | settings `onLoad` | settings `onUnload` | Correct. |
| Shelter screenshare subscriptions | onLoad | onUnload | Correct. |
| Shelter titlebar subscriptions | onLoad | onUnload | Correct. |
| Shelter power `TRACK` | onLoad | Missing | Confirmed duplicate on reload. |
| Shelter Touch Bar `TRACK` | onLoad | Missing | Confirmed duplicate on reload; other two unsubscribe. |
| Dropdown document click | `onMount` | `onCleanup` | Correct. |
| Keybind recorder keyup/timer | start record | stop/cleanup | Correct. |
| arRPC worker | `startRPC` | terminate on replacement/before quit | Needs guard against `refreshProcessList` after stop; scanner otherwise retained intentionally. |
| Chrome diagnostics windows | IPC open | map delete `closed` | Correct. |
| CSS editor IPC | open editor | `removeAllListeners` on close | Correct singleton but global channel removal can conflict with concurrent editor design. |
| Main window/app listeners | `doAfterDefiningTheWindow` | Never explicit | Duplicate on multiple main windows; centralize. |
| Web-request listeners | `doAfterDefiningTheWindow` default session | Electron uses last listener for event type | Not cumulative listener leak; multiwindow setup is fragile. |
| Runtime plugin main cleanups | manager enable/reload | `disableMain` | Correct if plugin registers cleanup. |
| Runtime preload cleanups | preload loader | prior reload only | Navigation realm destruction cleans normally; no global unload. |

## Appendix D — Native Dependencies

Runtime native dependencies are Electron/Chromium, optional Linux `@vencord/venmic` addon, arRPC process scanning, and Linux DBus. Packaging declares GTK/NSS/ALSA/etc. dependencies because Electron is shipped today. A Rust/WebView2 Windows package moves that burden to the installed Evergreen WebView2 runtime; Linux/macOS remain dependent on system WebKitGTK/WKWebView and native portal/media frameworks.

## Appendix E — Performance Hotspot Index

```text
P0
├── Security: remote renderer -> unrestricted-ish native bridge
│   ├── src/discord/preload/bridge.ts
│   └── src/discord/ipc.ts
└── Plugin trust: filesystem main plugins receive raw Electron objects
    └── src/discord/plugins/manager.ts

P1
├── Idle lifecycle: tray-hidden Discord renderer is not background-throttled by default
│   ├── src/common/config.ts (sleepInBackground = false; minimizeToTray = true)
│   └── src/discord/window.ts (backgroundThrottling setting)
├── Media: repeated getDisplayMedia wrapper stacking
│   └── src/shelter/screenshare/components/ScreensharePicker.tsx
└── Startup/supply chain: awaited mutable remote Shelter update check/executable bundle
    └── src/discord/extensions/modloader.ts

P2
├── Main CPU wakeup: one-second bounds polling
│   └── src/discord/window.ts
├── Page CPU wakeup: one-second invite route polling
│   └── src/discord/preload/inviteBackButton.mts
├── Plugin reload lifecycle: power/touchbar subscriptions and guild buttons retained
│   ├── src/shelter/power/index.ts
│   └── src/shelter/touchbar/index.ts
├── Long-session cache growth: RPC asset/application maps
│   └── src/shelter/rpc/asset.ts
└── Multi-window listener/control ownership
    ├── src/discord/window.ts
    └── src/discord/ipc.ts

P3
└── YouTube injected ad observer scans on every matching-frame DOM mutation
    └── assets/app/js/adguard.js
```

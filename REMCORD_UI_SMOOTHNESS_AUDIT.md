# LEGcord UI Smoothness Audit

**Repository assessed:** `E:\Legcord` on 3 October 2026.  
**Scope:** repository-controlled Electron/native-shell, preload, Shelter, CSS, and injected JavaScript. Discord's downloaded renderer, third-party mod bundles, user themes, and user runtime plugins were not available for source review. Findings marked **Confirmed** are direct source evidence; **Likely** needs a trace to quantify; **Unknown** cannot be attributed without runtime profiling.

## Outcome

Smoothness is a frame-pacing and input-latency problem, not a CPU-average problem. The source contains a small number of continuous renderer/native wakeups, several synchronous IPC paths in settings UI, and one unbounded document-wide observer in YouTube frames. These are credible contributors to periodic hitches, but this repository does not include Discord's React/message-list code, so it cannot establish the dominant source of scroll or channel-switch jank without traces.

The first measured fixes should be the bounds poll, invite poll, stacked screen-share wrapper, and synchronous settings bridge. Electron replacement remains a separate experiment: a Rust/WebView2 host can reduce shell overhead and improve host scheduling, but it does not remove Discord's JS/style/layout/paint pipeline.

## Frame-time model and attribution rule

```text
Input -> event handlers -> Discord/mod state -> JavaScript -> style -> layout
      -> paint -> compositing -> GPU -> display
```

For every long frame, identify the longest stage in an Edge/Chromium Performance trace before choosing a fix. Record P50/P95/P99 frame time, worst frame, dropped frames, long-task duration/count, and input-to-presentation latency. A 60 Hz display has a 16.67 ms frame budget; 120 Hz 8.33 ms; 144 Hz 6.94 ms. At 144 Hz, even short synchronous work that is harmless at 60 Hz is visible.

## Confirmed and likely jank findings

| ID | Classification | Evidence / trigger | User-visible risk | Fix / measurement |
|---|---|---|---|---|
| JANK-01 | Native host / CPU | `src/discord/window.ts:440` polls window bounds every second despite move/resize persistence already being registered. | Periodic main-process wakeup; may contend with window movement or power-sensitive idle work. | Delete after validating move/resize/maximize coverage. Compare ETW wakeups and frame-time outliers. |
| JANK-02 | JS / style | `src/discord/preload/inviteBackButton.mts:71` runs `sync()` once/second in the Discord document. `sync()` looks up/mutates a fixed button even during active UI use. | Periodic renderer task and possible style work; low single-call cost, persistent frequency. | Keep history/popstate hooks; add an explicit Discord-navigation signal only if needed. Trace long tasks before/after. |
| JANK-03 | JS / DOM | `assets/app/js/adguard.js:106-121` observes the complete YouTube-frame document subtree and executes a `querySelectorAll` scan on every mutation. | Likely high in active YouTube embeds: mutation storms can monopolize that frame's main thread. | Restrict observation to the feed container, batch once per task/idle period, and disconnect when no matching surface exists. Measure embed frame long tasks. |
| JANK-04 | JS / media | `ScreensharePicker.tsx` repeatedly replaces `navigator.mediaDevices.getDisplayMedia`; every share confirmation can wrap a prior wrapper. | Work grows per share and can add capture-start latency/correctness failures. | Make the patch idempotent and restore it on unload; measure 20 consecutive share starts. |
| JANK-05 | IPC / input | `src/discord/preload/bridge.ts` uses synchronous IPC for config, themes, process lists, translations, window state and more. Settings code calls `getConfig()`/`getThemes()` synchronously. | A renderer waits for main-process filesystem/serialization work; visible when opening/settings interactions occur. | Publish a small cached bootstrap snapshot once, use async mutations, then push invalidations. Record IPC duration and input delay. |
| JANK-06 | Style / layout | `src/shelter/settings/settings.ts:18-77` traverses ancestors with `getComputedStyle`, `scrollHeight`, and `clientHeight`, then writes `scrollTop` repeatedly (immediate, microtask, two rAFs, 0/50 ms timers). | Likely layout work and scroll correction bursts on every settings mutation; protects UX but may itself hitch settings. | Keep a cached known scroller, coalesce one restore in rAF, and retain later retries only when measurement proves required. |
| JANK-07 | Layout | `Dropdown.tsx:23-29` obtains two bounding rects and changes max height inside a reactive effect. | Small and interaction-local, but can force layout during dropdown opening. | Read both rects first, then write; use `ResizeObserver` only while open if dynamic resizing is required. |
| JANK-08 | Style / paint | `SettingsPanel.module.css:99` transitions `margin-bottom`; `SupportBanner.module.css` uses several `transition: all` declarations. | Layout/paint may occur during settings animations; impact depends on affected subtree. | Transition only known compositor-safe `transform`/`opacity` where visual behavior permits; verify DevTools paint/layout track. |
| JANK-09 | IPC / startup | Theme delivery sends whole CSS strings and removes/recreates style nodes on Quick CSS edits (`themes.ts`, `preload/themes.ts`). | CSS parse/style recalculation burst when themes change; not a per-frame cost in steady state. | Install each named sheet once; replace its text atomically and version resources. Test large themes while scrolling. |
| JANK-10 | JS / lifecycle | `power` and `touchbar` Shelter plugins retain subscriptions across reload; Touch Bar retains guild items. | Duplicate callbacks cause increasingly frequent work after reloads. | Add `onUnload` subscription cleanup and reset guild state. |
| JANK-11 | Native / JS | arRPC worker scans processes and posts JSON; results are reinjected using `executeJavaScript` (`rpcProcess.ts`). | Background work can compete for CPU; visible impact is unknown and depends on interval/results. | Demand-drive scanning, cap caches, avoid page eval for frequent updates, and profile enabled/disabled. |
| JANK-12 | Host-to-renderer | Focus/blur/maximize events execute page JavaScript in `window.ts:396-410`. | Usually rare, but window movement/focus transitions can add bridge work and style changes. | Use a single state message / document-created listener; measure window-drag frame pacing. |

### Layout-thrash assessment

No confirmed repeated read-write-read loop inside a hot scroll, wheel, resize, or animation handler was found in repository-controlled code. The closest likely cases are JANK-06 and JANK-07. The settings helper performs geometry/style reads before its writes, which avoids the worst alternating pattern, but its repeated scheduled `scrollTop` writes can cause later work to observe invalidated layout. Discord and downloaded plugins remain an unmeasured source of forced layout.

### CSS assessment

Repository CSS is relatively small and mostly applies titlebar/settings surfaces. There is no confirmed heavy `backdrop-filter` in the Discord document; the only discovered `backdrop-filter: blur(18px)` is setup UI. The titlebar and invite button use shadows and color/background transitions, which are localized. The main candidates are broad `transition: all` rules in `SupportBanner.module.css` and a `margin-bottom` transition in `SettingsPanel.module.css`; neither should be changed without paint/layout evidence. The continuously spinning settings icon is compositor-friendly only if no other paint-triggering properties are involved; verify in the Layers/Paint profiler.

## Plugin and theme render-cost inventory

| Subsystem | Cost class | Rationale / control |
|---|---|---|
| Titlebar, power, touchbar | Low normally; Medium after reload | Event/subscription driven; existing reload cleanup defects can multiply callbacks. |
| Screen share | Medium while active | Timers, stats, stream handling, and the stacking capture wrapper affect media-start path, not normal scrolling. |
| Settings UI | Low when closed; Medium when open | Synchronous bridge reads and scroll-preservation scheduling occur during settings updates. |
| Theme / Quick CSS | Low steady state; Medium/High on change | CSS parsing and full style recalculation scale with CSS size and selector complexity. User themes are unknown. |
| Invite back control | Low but continuous | One-second DOM work in the main Discord document. |
| YouTube ad injector | High in active matching frame | Global subtree observer plus selector scan; user-enable and scope it. |
| Vencord/Equicord/Shelter downloads and runtime plugins | Unknown | Dynamically fetched/user-provided code requires per-plugin measurement and a resource manifest. |

## Injection-manager target

Use one document-scoped registry rather than independent insertions:

```text
Document -> { id, owner, version, enabled, createdAt, node/sheet, dispose }
  core | legcord | theme:<id> | quick-css | plugin:<id>
```

`install(id, css)` must update an existing resource rather than duplicate it; `remove(id)` must be idempotent; navigation/unload must dispose the entire document registry. Batch changes in one animation frame only for user-visible stylesheet changes, never during text input/scroll. Track CSS bytes, update time, and active resource count in diagnostics.

## UI Smoothness Root-Cause Map

| Pipeline stage | Repository-controlled factors that can interfere |
|---|---|
| Input | Synchronous IPC called by settings actions; document click/keyup handlers; global shortcut/window actions. No repository scroll handler was found. |
| Event handlers | Invite button history hooks plus fallback poll; dropdown outside-click handler; screen-share selection; plugin/Flux subscriptions (including duplicate power/touchbar subscriptions after reload). |
| Discord state / modifications | Shelter settings mutations, downloaded mod bootstrap, runtime plugin scripts, theme enable/disable, Discord-internal state outside this repository. |
| JavaScript | Invite interval; YouTube mutation callback/query scan; capture wrapper accumulation; screen-share timers; arRPC serialization; `executeJavaScript` injections; startup mod/plugin loading. |
| Style | Theme/Quick CSS insertion/replacement; body attribute changes for focus/maximize; invite button visibility; broad user-theme selectors unknown. |
| Layout | Settings scroller discovery/restoration; dropdown rect reads; layout behavior of settings `margin-bottom` transition; Discord and user CSS unknown. |
| Paint | Shadows, filters, background/color transitions on localized titlebar/settings elements; CSS/theme changes can repaint broad Discord surfaces. |
| Composite | Transform/opacity animations are generally preferred; fixed titlebar/invite surfaces and layers must be verified under GPU traces. |
| GPU | Hardware acceleration is intentionally retained. Video/screen share, Discord rendering, WebView/Electron compositor, driver/device conditions are unmeasured. |
| Display | Refresh rate and VRR scheduling are external to source; certify separately at 60/120/144 Hz+. |

## Top 20 Frame-Jank Risks

| Rank | Risk | Severity | Evidence | Frequency | Expected impact | Difficulty |
|---:|---|---|---|---|---|---|
| 1 | Discord/downloaded mod main-thread work | P0 | Unknown external code | Interaction-dependent | Potentially dominant | High |
| 2 | Global YouTube MutationObserver scan | P1 | Confirmed | Every matching-frame mutation | High in embeds | Medium |
| 3 | Synchronous IPC in settings bridge | P1 | Confirmed | Settings/UI actions | Medium | Medium |
| 4 | Screen-share wrapper stacking | P1 | Confirmed | Per repeated share | Medium/growing | Low |
| 5 | Non-throttled hidden renderer | P1 | Confirmed prior audit | Hidden idle | Indirect foreground contention | Medium |
| 6 | Theme/user CSS style recalculation | P1 | Likely | Theme changes / continuously for bad CSS | High possible | Medium |
| 7 | arRPC scans/results | P2 | Likely | Config/dependency defined | Low/medium | Medium |
| 8 | Invite one-second renderer poll | P2 | Confirmed | Every second | Low periodic hitch | Low |
| 9 | Bounds one-second host poll | P2 | Confirmed | Every second/window | Low periodic wakeup | Low |
| 10 | Duplicate power/touchbar subscriptions | P2 | Confirmed | After reload | Medium/growing | Low |
| 11 | Settings scroll restore burst | P2 | Confirmed | Settings mutation | Medium localized | Medium |
| 12 | Quick CSS remove/reinsert | P2 | Confirmed | File change | Medium localized | Medium |
| 13 | `transition: all` settings CSS | P2 | Confirmed | Hover/state changes | Low/medium | Low |
| 14 | Layout transition on panel margin | P2 | Confirmed | Panel state changes | Low/medium | Low |
| 15 | Renderer page eval on host events | P2 | Confirmed | Focus/maximize/RPC | Low/medium | Medium |
| 16 | Runtime filesystem plugins | P2 | Unknown | Plugin-dependent | High possible | High |
| 17 | CSP/mod bootstrap concurrent injection | P2 | Confirmed | Navigation/startup | Startup/channel-ready hitch | High |
| 18 | Backup JSON serialization | P3 | Confirmed | Explicit backup only | No ordinary UI impact | Low |
| 19 | Dropdown geometry measurement | P3 | Confirmed | Dropdown opening | Low localized | Low |
| 20 | Setup blur/shadow effects | P3 | Confirmed | First run only | Low | Low |

## Top 10 Smoothness Improvements

| Change | Files | Before -> after | Expected effect | Measurement / risk |
|---|---|---|---|---|
| Remove bounds poll | `src/discord/window.ts` | 1 Hz polling -> move/resize/maximize events | Fewer host wakeups | WPR/ETW; low risk |
| Remove invite fallback interval | `src/discord/preload/inviteBackButton.mts` | 1 Hz DOM sync -> history/navigation event | Removes periodic document work | DevTools long tasks; low risk |
| Scope/batch YouTube observer | `assets/app/js/adguard.js` | Whole-document scan per mutation -> narrow, scheduled scan | Better embed responsiveness | Embedded-video trace; medium compatibility risk |
| Idempotent capture patch | `ScreensharePicker.tsx` | N wrappers -> one wrapper + cleanup | Stable capture-start latency | 20-share test; medium media risk |
| Async cached settings contract | `bridge.ts`, `ipc.ts`, settings | Sync round trips -> bootstrap snapshot + async writes | Lower input delay in settings | IPC timings/INP; medium redesign risk |
| Coalesce scroll preservation | `settings.ts` | Immediate + four deferred restores -> one measured/coalesced restore | Fewer settings layout bursts | Settings trace; medium UX risk |
| Registry-based style lifecycle | `themes.ts`, `preload/themes.ts` | remove/re-add / loose IDs -> versioned upsert/dispose | No duplicates; controlled recalc | CSS update trace; medium scope |
| Fix reload cleanups | `shelter/power`, `shelter/touchbar` | retained subscriptions -> unload disposal | Prevents cumulative work | repeated reload trace; low risk |
| Demand-driven bounded RPC | `rpcProcess.ts`, RPC Shelter | periodic scans/unbounded maps -> active-view scan + cap/TTL | Less background contention/memory | A/B with RPC; medium feature risk |
| Add performance policy state machine | host + bridge | one behavior -> visible/hidden/media-aware modes | Stronger idle performance without call regressions | voice/video/capture qualification; high risk |

## Performance modes and hidden-state policy

Performance: normal visible renderer and GPU acceleration. Balanced: background-throttle a hidden, non-media window and reduce nonessential plugin/RPC work. Battery: stronger inactivity reductions and auxiliary-WebView suspension only after proving safe resume behavior. All modes must exempt active voice, camera, screen capture, and active media; never disable GPU acceleration as a generic performance action.

## Measurement protocol and gates

Use Edge/Chromium DevTools Performance, Rendering/FPS, Memory, and event timing plus WPR/WPA or ETW and Process Explorer. Capture a clean profile first (no mods/themes), then add modifications one layer at a time. Record hardware, display refresh/VRR, GPU driver, Windows build, WebView/Electron version, build SHA, account scenario, plugins, and themes.

### 60/120/144 Hz Acceptance Checklist

- [ ] Test 60, 120, and 144 Hz (or highest available), with hardware acceleration enabled.
- [ ] Capture idle, fast message scrolling, typing, channel/server switches, search, settings, large guild/DM, GIF/image-heavy channel, popouts, voice, video, and screen share.
- [ ] Record average FPS, P50/P95/P99/worst frame time, dropped frames, long tasks, input delay, CPU/GPU/RAM, process count, and idle wakeups.
- [ ] Verify no periodic frame-time spike corresponds to a 1 Hz poll, RPC scan, plugin timer, or theme watcher.
- [ ] Verify event handlers and host messages do not create synchronous input-path waits.
- [ ] Verify theme/Quick CSS updates leave one resource per ID and do not duplicate styles after navigation.
- [ ] Run 20 screen-share start/stop cycles and 20 plugin reloads without growing callbacks/wrappers, memory, or capture-start latency.
- [ ] Compare Electron and minimal WebView2 using the same Discord profile and scenario; add bridge/mods one layer at a time.
- [ ] Validate hidden/tray throttling in no-media state and prove that active calls/capture are unaffected.
- [ ] Accept a change only when before/after traces show no regression in frame pacing or input latency.

## Implementation order

1. Build repeatable trace capture and a metrics ledger; establish Electron clean/modded baselines.
2. Land low-risk event-driven/lifecycle fixes: JANK-01, JANK-02, JANK-04, JANK-10.
3. Profile and scope JANK-03; do not alter ad behavior blindly.
4. Replace synchronous settings IPC and introduce the style-resource registry.
5. Implement media-aware performance modes and measure them.
6. Prototype WebView2, then compare frame pacing before claiming a native-shell improvement.

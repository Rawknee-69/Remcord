# Native WebView2 shell (production Windows app)

This Rust application is the production Windows app (built + signed in CI). The Electron application/codebase is stale legacy code that is no longer built, signed, or released.

## Requirements

- Windows
- Rust toolchain and the Microsoft C++ Build Tools
- Microsoft Edge WebView2 Runtime (Evergreen)
- An `E:` drive for the dedicated profile path

## Launch

From the repository root, run:

```powershell
.\native\run-dev.ps1
```

Or launch Cargo directly:

```powershell
cargo run --manifest-path .\native\Cargo.toml
```

Both commands build and launch only `native/`. The prototype always uses the isolated persistent profile `E:\Remcord.webview-dev-user-data`. It never selects the legacy Electron profile or launches any Electron binary (the Electron app is no longer built or released).

Close the native window to drop the WebView and its profile lock cleanly. Discord navigation is limited to HTTPS `discord.com` hosts in this prototype; popup windows are logged and blocked.

## Prototype behavior

- Creates one Tao native window containing a Wry WebView2 view.
- Navigates to `https://discord.com/app`.
- Registers the main-frame initialization script before the first navigation.
- Injects a CSS custom-property probe after page load and reports its computed value.
- Registers the compatibility facade, the shared mod substrate factory, and one adapter per active registered mod at document start on the exact `https://discord.com` origin. The facade exposes `native.ping` and the registry-scoped `native.getModBundle(modId)` operation (plus the `native.getVencordBundle()` wrapper); Rust accepts only those typed requests and their documented diagnostic/lifecycle events. Bundle ids are validated against the compile-time known-mod registry, so no URL, path, filename, or executable source can be selected from the renderer. It does not expose generic host invocation, filesystem access, or renderer-supplied JavaScript.
- Retrieves only the descriptor-registered fixed release assets for the three download-published mods — Vencord and Equicord from their fixed `browser.js`/`browser.css` GitHub release URLs, Shelter from the fixed `raw.githubusercontent.com` `shelter.js` — caching only the fixed file names inside `E:\Remcord.webview-dev-user-data` plus an in-memory validated bundle. The fourth registered mod is user-owned Custom: its `custom.js`/`custom.css` are read from the host-built profile-root paths on every request (never downloaded, never cached), with an optional CSS half. Rust executes the fixed JS bundle in the page world, then appends/replaces one identified style element **for the mods that declare a stylesheet** (Vencord and Equicord; Shelter ships none, so no empty CSS stage is manufactured; Custom's stylesheet is optional per document). A private document-local marker is set only after the descriptor's page-world global is present — except Custom, which publishes no global: its marker proves the host's own storage-context probe object defined immediately before the user JS, i.e. evaluation without delivery failure, matching Electron's execute-and-log semantics. A mod may declare that its global is published asynchronously (Shelter, which waits for Discord's `#app-mount`): the substrate then installs a one-shot trap before the bundle and keeps a coarse bounded poll as a fallback, dispatching exactly one verdict.
- Artifact identity is proven per descriptor and never invented: Vencord and Equicord must carry their `// Vencord …` / `// Equicord …` build banner, while Shelter — whose published artifact has no banner at all — must contain its own upstream initialization sentinels and is reported as a host-computed `sha256-…` content digest. Custom is user-owned local content with no upstream publisher, so any UTF-8 bytes within its size bounds are accepted and versioned by the same digest shape.
- Which adapters are injected is host-controlled: each descriptor's own traced Electron enablement policy (`settings.mods` for Vencord/Equicord/Custom, unconditional for Shelter) plus an optional DEV diagnostic allowlist. The renderer supplies nothing to this decision.
- Resets pending requests, marker state, style state, and native duplicate guards on a full navigation/reload. Stale bundle results from an older document are discarded.
- Logs navigation, popup decisions, renderer console errors, uncaught errors with source locations, unhandled-rejection stacks, page path/readiness/storage API health, and bridge/injection status to the launch console. Bridge messages are accepted only from the stable `https://discord.com` origin.
- Records bootstrap script bytes, document-start execution time, request/response latency, and accepted bridge message/request/event/response counts. Allocation counts are not measured.

This is not the compatibility layer. No Electron APIs, existing renderer mods, production build scripts, or production profile are wired to this prototype. Vencord, Shelter, Equicord, and Custom are real client-mod paths, each served as a thin typed consumer of the one reusable substrate in `native/src/mod_substrate.rs` and `native/web/mod-substrate/`; user-authored runtime plugins run through the `plugins.*` capabilities and `native/web/plugins/loader.js`. Shelter's Remcord plugin protocol (`remcord://` canonical, legacy `remcord://` rewritten by the host), and broader Remcord features remain deferred. `native/web/mods/testmod/adapter.js` is a test-only synthetic consumer and is never injected into Discord.

Registering a new client mod is one `ModDescriptor` in `KNOWN_MODS` plus one adapter file under `native/web/mods/<id>/`. The substrate itself stays mod-agnostic: no mod name appears in `mod_substrate.rs`.

`native/COMPATIBILITY.md` records the inspected Electron renderer/preload contract, every repository `sendSync` category, snapshot-versus-async migration decisions, and the explicitly deferred compatibility surface. The existing `src/` implementation has not been changed.

## Checks

```powershell
cargo fmt --manifest-path .\native\Cargo.toml -- --check
cargo test --manifest-path .\native\Cargo.toml
cargo build --manifest-path .\native\Cargo.toml
cargo clippy --manifest-path .\native\Cargo.toml --all-targets -- -D warnings
node --test .\native\tests\compatibility.test.mjs
```

Run the targeted JavaScript style check from the repository root with `.\node_modules\.bin\biome.exe check native\web\compatibility.js native\web\mod-substrate\mod-bootstrap.js native\web\mod-substrate\storage-preservation.js native\web\mods\vencord\adapter.js native\web\mods\shelter\adapter.js native\web\mods\equicord\adapter.js native\web\mods\testmod\adapter.js native\tests\compatibility.test.mjs native\tests\electron-compat.test.mjs`.

The last check links the Windows desktop executable and therefore requires the Microsoft C++ Build Tools in addition to the Rust MSVC target.

For an opt-in mod reload smoke test, set `REMCORD_NATIVE_SMOKE_RELOAD=1` (or `2` for multiple reloads) before launch. The first registered mod to reach a successful execution marker and renderer acknowledgement triggers `window.location.reload()` until the count is exhausted; normal launches do not force a reload. Each reload produces one fresh adapter/bootstrap in the new document.

For a DEV-only mod selection, set `REMCORD_NATIVE_DIAGNOSTIC_MODS=shelter` (comma-separated) before launch. This is a host-controlled allowlist that can only REMOVE mods from the set the settings policy already selected — it can never add an unregistered mod or bypass a `settings.mods` gate. It is how a "native without <mod>" control run is produced without editing the DEV profile. Startup logs one `[Mod:Registry] mod='<id>' adapter_enabled=<bool> enabling=<policy>` line per registered mod, which shows exactly which adapters are active and why.

For a DEV-only mod runtime observation, set `REMCORD_NATIVE_MOD_SMOKE=1`. The host evaluates a read-only observation built from the active descriptors themselves (it can only read those mods' own page-world globals, managed style ids, and adapter state properties) on page load, and again immediately after each mod's execution marker. The `[Mods][Smoke][after-marker]` line is the reliable one for a mod whose global is published asynchronously, because at page load that global may not exist yet. The observation requests nothing, installs nothing, and executes nothing.

For an unmodified Discord/WebView comparison, set `REMCORD_NATIVE_DIAGNOSTIC_BARE_PAGE=1`. This injects only a document-start storage observation and the read-only page-health query; it omits the compatibility facade, every mod adapter, CSS probe, and bridge probe. The page health log includes the `localStorage` property descriptors and `Storage` constructor type to help identify page-side replacement versus WebView API availability.

For a one-time interaction probe, set `REMCORD_NATIVE_DIAGNOSTIC_INTERACTION=1`. If no composer is open, it navigates to the first visible existing channel link, then focuses an empty message composer, types a character, erases it, and reports `channel_navigation_requested`, `composer_focused`, `input_observed`, and `character_erased`. It refuses to type when the composer is non-empty, captures no message content, and never submits a message.

For a settings capability smoke probe, set `REMCORD_NATIVE_SETTINGS_SMOKE=1`. After each trusted document load the host evaluates a DEV-only script that calls `window.remcord.settings.getConfig()` exactly like the migrated Shelter power caller (reads `blockPowerSavingInVoiceChat`), then reports the typed `settings_probe` event (`ok`, payload bytes, value). The host log shows the matching `[Settings][Metrics]` line (authoritative `source: file|defaults`, payload bytes, request latency) per document, including after reloads.

For a settings mutation smoke probe, set `REMCORD_NATIVE_SETTINGS_MUTATION_SMOKE=1`. On the first trusted document the host evaluates a DEV-only script that reads `blockPowerSavingInVoiceChat` via `getConfig`, flips it via the allowlisted `setBlockPowerSavingInVoiceChat`, re-reads via `getConfig`, and reports the typed `settings_mutation_probe` event (`stage: "mutated"`). On success the host reloads the document; on the next document it evaluates a restore script (host-formatted with the pre-mutation value) that reads the post-reload value, writes the original back, re-reads, and reports `stage: "restored"`. The host log shows the `[Settings][Mutation]` request line, both `[Settings][MutationSmoke]` stage lines, and whether the mutation survived the reload. The DEV settings file ends with its original value.

For a localization capability smoke probe, set `REMCORD_NATIVE_GETLANG_SMOKE=1`. After each trusted document load the host evaluates a DEV-only script that calls `window.remcord.getLang("invite-goBackToApp")` plus a missing key — exactly the invite back-button lookup and its empty-string case — then reports the typed `get_lang_probe` event (`ok`, value, `missing_is_empty`). The host log shows the matching `[Localization][Metrics]` line (resolved `lang`, fallback flag, key/payload bytes, request latency) per document, including after reloads.

For an app version capability smoke probe, set `REMCORD_NATIVE_APP_VERSION_SMOKE=1`. After each trusted document load the host evaluates a DEV-only script that calls `window.remcord.native.getVersion()` — exactly the update-checker local-version read — then reports the typed `app_version_probe` event (`ok`, version). The host log shows the matching `[App][Metrics]` line (version, payload bytes, request latency) per document, including after reloads.

For a plugin storage capability smoke probe, set `REMCORD_NATIVE_PLUGIN_STORAGE_SMOKE=1` (requires `extendedPluginAbilities: true` in the DEV `storage/settings.json`; restore the file afterwards). After each trusted document load the host evaluates a DEV-only script that calls `window.remcord.pluginStorage.writeFile`/`readFile` under the temporary logical plugin id `remcordNativeProbe` — exactly the public third-party contract — verifies exact bytes, then attempts traversal (`../escape.txt`), absolute (`/absolute/path.txt`), and drive (`C:\secret.txt`) reads and requires all three to be rejected. It reports the typed `plugin_storage_probe` event (`ok`, `stage: "write_read"`, payload bytes, `round_trip`, `attacks_blocked: 3`). The host log shows the matching `[PluginStorage][Metrics]` lines (op, payload bytes, request latency) and `[PluginStorage][INVALID_PATH]` rejections per document, including after reloads. The fixture lives only under `<DEV profile>/plugin-storage/remcordNativeProbe/`; delete that directory through the host after the run — never through the renderer.

For a native theme injection smoke observation, set `REMCORD_NATIVE_THEMES_SMOKE=1`. On every trusted document load the host discovers `<DEV profile>/themes/<id>/`, reads each `manifest.json`, honors `enabled`, resolves the manifest-provided CSS filename under a strict bare-`*.css` allowlist with canonical root containment, and pushes each enabled CSS through the shared managed-CSS lifecycle in sorted-id order — with no renderer request, capability, or path involved. The host log shows one `[Themes][Metrics]` line per document (`discovered`, `enabled`, `injected`, `skipped`, `css_bytes`, `read_ms`) plus `[Themes][Skip] id reason` lines (`disabled`, `missing_css`, `invalid_filename`, …), and a `[Themes][Smoke]` observation of the managed `<style>` nodes (ids, byte sizes, head order, Vencord coexistence). Seed temporary fixtures only under `<DEV profile>/themes/` and delete that directory after the run; production theme files are never touched.

For a native Quick CSS smoke observation, set `REMCORD_NATIVE_QUICKCSS_SMOKE=1`. On every trusted document load the host reads the authoritative settings flag and, when enabled, pushes its own fixed `<DEV profile>/quickCss.css` through the shared managed-CSS lifecycle BEFORE themes (a missing file is created empty, exactly like Electron) — with no renderer request, capability, or path involved. The host log shows one `[QuickCss][Metrics]` line per document (`enabled`, `state`, `injected`, `css_bytes`, `read_ms`), and a `[QuickCss][Smoke]` observation of the managed styles (`quickcss_present`, `quickcss_bytes`, full head order, Vencord coexistence, and the computed `--remcord-native-order-probe` cascade value). Seed a temporary `quickCss.css` (and, for ordering evidence, temporary theme fixtures setting the same probe property) only under the DEV profile; flip the DEV `quickCss` flag only from a backed-up `storage/settings.json` and restore it afterwards; delete all fixtures after the run. Production Quick CSS is never touched.

Live watching needs no extra flag: from startup to shutdown the host owns exactly two `notify` watchers (profile directory filtered to `quickCss.css`; themes root recursive), debounces bursts at 300 ms, re-reads authoritative state, and applies minimal managed-CSS updates (content in place, structural changes finish with an order repair). Every debounced fire logs `[QuickCss][Live]` / `[Themes][Live]` (transition, bytes, read/injection latency) plus cumulative `[LiveCss][Metrics]` (`watchers`, `raw_fs_events`, `debounced_reloads`, `css_updates`, `css_removes`, `stale_discarded`, `order_repairs`), and live theme skips log `[Themes][Skip] id reason` exactly like startup. Edit the DEV fixtures while the binary runs to exercise it; document reload never creates watchers (one `[LiveCss][Watch] started` line per process).

For a Shelter plugin-loader probe, set `REMCORD_NATIVE_LOADER_PROBE=1`. After each trusted document load — and again right after Shelter's own bootstrap marker, since Shelter publishes its global asynchronously — the host evaluates a DEV-only script that verifies the upstream `shelter.plugins` loader surface (`addLocalPlugin`, `startPlugin`, `stopPlugin`, `removePlugin`) exists and that the URL-based `ensureLoaderPlugin` is NOT exposed, then round-trips one synthetic plugin with host-fixed id and source (`remcord-loader-probe`, `({onLoad(){}})`) through add/start/stop/remove and verifies no residue remains. It reports the typed `shelter_loader_probe` event (`ok`, `stage: "presence"|"synthetic"|"cleanup"`, `synthetic: "verified"|"skipped"`). The page-load run may report presence-false on timing; the post-marker run is authoritative. No settings code executes, nothing is fetched, no renderer input is accepted, and the payload carries no source, paths, or URLs. A leftover probe id from an earlier run is removed first, so reloads never duplicate it.

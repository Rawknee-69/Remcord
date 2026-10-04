# Native WebView2 shell prototype (DEV only)

This isolated Rust application is a prototype. It does not replace, build, package, or modify the production Electron application.

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

Both commands build and launch only `native/`. The prototype always uses the isolated persistent profile `E:\Legcord.webview-dev-user-data`. It never selects the production Electron profile or launches the installed Legcord binary.

Close the native window to drop the WebView and its profile lock cleanly. Discord navigation is limited to HTTPS `discord.com` hosts in this prototype; popup windows are logged and blocked.

## Prototype behavior

- Creates one Tao native window containing a Wry WebView2 view.
- Navigates to `https://discord.com/app`.
- Registers the main-frame initialization script before the first navigation.
- Injects a CSS custom-property probe after page load and reports its computed value.
- Registers the compatibility facade, the shared mod substrate factory, and an explicit Vencord adapter at document start on the exact `https://discord.com` origin. The facade exposes `native.ping` and the fixed `native.getVencordBundle` operation; Rust accepts only those typed requests and their documented diagnostic/lifecycle events. It does not expose generic host invocation, filesystem access, or renderer-supplied JavaScript.
- Retrieves only the descriptor-registered fixed `browser.js` and `browser.css` release assets (Vencord is the sole registered mod), caching only the fixed file names inside `E:\Legcord.webview-dev-user-data` plus an in-memory validated bundle. Rust executes the fixed JS bundle in the page world, then appends/replaces one identified style element. A private document-local marker is set only after the descriptor's page-world global is present.
- Resets pending requests, marker state, style state, and native duplicate guards on a full navigation/reload. Stale bundle results from an older document are discarded.
- Logs navigation, popup decisions, renderer console errors, uncaught errors with source locations, unhandled-rejection stacks, page path/readiness/storage API health, and bridge/injection status to the launch console. Bridge messages are accepted only from the stable `https://discord.com` origin.
- Records bootstrap script bytes, document-start execution time, request/response latency, and accepted bridge message/request/event/response counts. Allocation counts are not measured.

This is not the compatibility layer. No Electron APIs, existing renderer mods, production build scripts, or production profile are wired to this prototype. Vencord is the only real client-mod path, now served as the first consumer of the reusable substrate in `native/src/mod_substrate.rs` and `native/web/mod-substrate/`; Shelter, Equicord, custom bundles, runtime plugins, and broader Legcord features remain deferred. `native/web/mods/testmod/adapter.js` is a test-only synthetic second consumer and is never injected into Discord.

`native/COMPATIBILITY.md` records the inspected Electron renderer/preload contract, every repository `sendSync` category, snapshot-versus-async migration decisions, and the explicitly deferred compatibility surface. The existing `src/` implementation has not been changed.

## Checks

```powershell
cargo fmt --manifest-path .\native\Cargo.toml -- --check
cargo test --manifest-path .\native\Cargo.toml
cargo build --manifest-path .\native\Cargo.toml
cargo clippy --manifest-path .\native\Cargo.toml --all-targets -- -D warnings
node --test .\native\tests\compatibility.test.mjs
```

Run the targeted JavaScript style check from the repository root with `.\node_modules\.bin\biome.exe check native\web\compatibility.js native\web\mod-substrate\mod-bootstrap.js native\web\mod-substrate\storage-preservation.js native\web\mods\vencord\adapter.js native\web\mods\testmod\adapter.js native\tests\compatibility.test.mjs`.

The last check links the Windows desktop executable and therefore requires the Microsoft C++ Build Tools in addition to the Rust MSVC target.

For an opt-in Vencord reload smoke test, set `LEGCORD_NATIVE_SMOKE_RELOAD=1` (or `2` for multiple reloads) before launch. Each successful Vencord execution marker and renderer acknowledgement triggers `window.location.reload()` until the count is exhausted; normal launches do not force a reload. Each reload produces one fresh adapter/bootstrap in the new document.

For a controlled DEV-only baseline, set `LEGCORD_NATIVE_DIAGNOSTIC_NO_VENCORD=1` before launch. This omits the automatic Vencord adapter while retaining the same WebView profile, Discord navigation, and health/error diagnostics; unset it to restore normal behavior. The page health log includes the `localStorage` property descriptors and `Storage` constructor type to help identify page-side replacement versus WebView API availability.

For an unmodified Discord/WebView comparison, set `LEGCORD_NATIVE_DIAGNOSTIC_BARE_PAGE=1`. This injects only a document-start storage observation and the read-only page-health query; it omits the compatibility facade, Vencord adapter, CSS probe, and bridge probe.

For a one-time interaction probe, set `LEGCORD_NATIVE_DIAGNOSTIC_INTERACTION=1`. If no composer is open, it navigates to the first visible existing channel link, then focuses an empty message composer, types a character, erases it, and reports `channel_navigation_requested`, `composer_focused`, `input_observed`, and `character_erased`. It refuses to type when the composer is non-empty, captures no message content, and never submits a message.

For a settings capability smoke probe, set `LEGCORD_NATIVE_SETTINGS_SMOKE=1`. After each trusted document load the host evaluates a DEV-only script that calls `window.legcord.settings.getConfig()` exactly like the migrated Shelter power caller (reads `blockPowerSavingInVoiceChat`), then reports the typed `settings_probe` event (`ok`, payload bytes, value). The host log shows the matching `[Settings][Metrics]` line (authoritative `source: file|defaults`, payload bytes, request latency) per document, including after reloads.

For a settings mutation smoke probe, set `LEGCORD_NATIVE_SETTINGS_MUTATION_SMOKE=1`. On the first trusted document the host evaluates a DEV-only script that reads `blockPowerSavingInVoiceChat` via `getConfig`, flips it via the allowlisted `setBlockPowerSavingInVoiceChat`, re-reads via `getConfig`, and reports the typed `settings_mutation_probe` event (`stage: "mutated"`). On success the host reloads the document; on the next document it evaluates a restore script (host-formatted with the pre-mutation value) that reads the post-reload value, writes the original back, re-reads, and reports `stage: "restored"`. The host log shows the `[Settings][Mutation]` request line, both `[Settings][MutationSmoke]` stage lines, and whether the mutation survived the reload. The DEV settings file ends with its original value.

For a localization capability smoke probe, set `LEGCORD_NATIVE_GETLANG_SMOKE=1`. After each trusted document load the host evaluates a DEV-only script that calls `window.legcord.getLang("invite-goBackToApp")` plus a missing key — exactly the invite back-button lookup and its empty-string case — then reports the typed `get_lang_probe` event (`ok`, value, `missing_is_empty`). The host log shows the matching `[Localization][Metrics]` line (resolved `lang`, fallback flag, key/payload bytes, request latency) per document, including after reloads.

For an app version capability smoke probe, set `LEGCORD_NATIVE_APP_VERSION_SMOKE=1`. After each trusted document load the host evaluates a DEV-only script that calls `window.legcord.native.getVersion()` — exactly the update-checker local-version read — then reports the typed `app_version_probe` event (`ok`, version). The host log shows the matching `[App][Metrics]` line (version, payload bytes, request latency) per document, including after reloads.

For a plugin storage capability smoke probe, set `LEGCORD_NATIVE_PLUGIN_STORAGE_SMOKE=1` (requires `extendedPluginAbilities: true` in the DEV `storage/settings.json`; restore the file afterwards). After each trusted document load the host evaluates a DEV-only script that calls `window.legcord.pluginStorage.writeFile`/`readFile` under the temporary logical plugin id `legcordNativeProbe` — exactly the public third-party contract — verifies exact bytes, then attempts traversal (`../escape.txt`), absolute (`/absolute/path.txt`), and drive (`C:\secret.txt`) reads and requires all three to be rejected. It reports the typed `plugin_storage_probe` event (`ok`, `stage: "write_read"`, payload bytes, `round_trip`, `attacks_blocked: 3`). The host log shows the matching `[PluginStorage][Metrics]` lines (op, payload bytes, request latency) and `[PluginStorage][INVALID_PATH]` rejections per document, including after reloads. The fixture lives only under `<DEV profile>/plugin-storage/legcordNativeProbe/`; delete that directory through the host after the run — never through the renderer.

For a native theme injection smoke observation, set `LEGCORD_NATIVE_THEMES_SMOKE=1`. On every trusted document load the host discovers `<DEV profile>/themes/<id>/`, reads each `manifest.json`, honors `enabled`, resolves the manifest-provided CSS filename under a strict bare-`*.css` allowlist with canonical root containment, and pushes each enabled CSS through the shared managed-CSS lifecycle in sorted-id order — with no renderer request, capability, or path involved. The host log shows one `[Themes][Metrics]` line per document (`discovered`, `enabled`, `injected`, `skipped`, `css_bytes`, `read_ms`) plus `[Themes][Skip] id reason` lines (`disabled`, `missing_css`, `invalid_filename`, …), and a `[Themes][Smoke]` observation of the managed `<style>` nodes (ids, byte sizes, head order, Vencord coexistence). Seed temporary fixtures only under `<DEV profile>/themes/` and delete that directory after the run; production theme files are never touched.

For a native Quick CSS smoke observation, set `LEGCORD_NATIVE_QUICKCSS_SMOKE=1`. On every trusted document load the host reads the authoritative settings flag and, when enabled, pushes its own fixed `<DEV profile>/quickCss.css` through the shared managed-CSS lifecycle BEFORE themes (a missing file is created empty, exactly like Electron) — with no renderer request, capability, or path involved. The host log shows one `[QuickCss][Metrics]` line per document (`enabled`, `state`, `injected`, `css_bytes`, `read_ms`), and a `[QuickCss][Smoke]` observation of the managed styles (`quickcss_present`, `quickcss_bytes`, full head order, Vencord coexistence, and the computed `--legcord-native-order-probe` cascade value). Seed a temporary `quickCss.css` (and, for ordering evidence, temporary theme fixtures setting the same probe property) only under the DEV profile; flip the DEV `quickCss` flag only from a backed-up `storage/settings.json` and restore it afterwards; delete all fixtures after the run. Production Quick CSS is never touched.

Live watching needs no extra flag: from startup to shutdown the host owns exactly two `notify` watchers (profile directory filtered to `quickCss.css`; themes root recursive), debounces bursts at 300 ms, re-reads authoritative state, and applies minimal managed-CSS updates (content in place, structural changes finish with an order repair). Every debounced fire logs `[QuickCss][Live]` / `[Themes][Live]` (transition, bytes, read/injection latency) plus cumulative `[LiveCss][Metrics]` (`watchers`, `raw_fs_events`, `debounced_reloads`, `css_updates`, `css_removes`, `stale_discarded`, `order_repairs`), and live theme skips log `[Themes][Skip] id reason` exactly like startup. Edit the DEV fixtures while the binary runs to exercise it; document reload never creates watchers (one `[LiveCss][Watch] started` line per process).

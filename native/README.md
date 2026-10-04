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
- Registers a document-start script that exposes only the initial `window.legcord` compatibility facade on the exact `https://discord.com` origin. The facade contains an immutable native boot snapshot and `native.ping`, `native.supports`, `native.requireCapability`, and the CSS probe diagnostic. Rust accepts only the versioned `ping` command and `round_trip_ack`, `css_probe`, `renderer_error`, and `bootstrap_metrics` events; it does not expose generic host invocation.
- Logs navigation, popup decisions, renderer console errors, uncaught errors, unhandled rejections, and bridge/injection status to the launch console. Bridge messages are accepted only from the stable `https://discord.com` origin.
- Records bootstrap script bytes, document-start execution time, request/response latency, and accepted bridge message/request/event/response counts. Allocation counts are not measured.

This is not the compatibility layer. No Electron APIs, existing renderer mods, production build scripts, or production profile are wired to this prototype.

`native/COMPATIBILITY.md` records the inspected Electron renderer/preload contract, every repository `sendSync` category, snapshot-versus-async migration decisions, and the explicitly deferred compatibility surface. The existing `src/` implementation has not been changed.

## Checks

```powershell
cargo fmt --manifest-path .\native\Cargo.toml -- --check
cargo test --manifest-path .\native\Cargo.toml
cargo build --manifest-path .\native\Cargo.toml
cargo clippy --manifest-path .\native\Cargo.toml --all-targets -- -D warnings
node --test .\native\tests\compatibility.test.mjs
```

Run the targeted JavaScript style check from the repository root with `.\node_modules\.bin\biome.exe check native\web\compatibility.js native\tests\compatibility.test.mjs`.

The last check links the Windows desktop executable and therefore requires the Microsoft C++ Build Tools in addition to the Rust MSVC target.

For an opt-in one-time reload smoke test, set `LEGCORD_NATIVE_SMOKE_RELOAD=1` before launch. The first successful facade request/response triggers exactly one `window.location.reload()`; normal launches do not force a reload. A new document gets a new facade and request-correlation state.

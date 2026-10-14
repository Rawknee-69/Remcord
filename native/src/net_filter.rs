//! Network request filtering (F21): telemetry / localhost-WebSocket blocking,
//! YouTube embed Referer rewrite, and proxy support.
//!
//! Electron parity (`src/discord/window.ts:219-233`,
//! `src/common/sanitization.ts`, `src/common/proxy.ts`, `src/main.ts`):
//! - `onBeforeRequest` cancels telemetry/noise URLs
//!   ([`is_telemetry_blocked_url`]) and stray localhost WebSocket probes
//!   ([`is_blocked_localhost_websocket`]); native enforces the same
//!   decisions in a `WebResourceRequested` handler ([`register`]), where a
//!   block is served as an empty `403` response (WebView2 has no
//!   cancel verb — the page observes a failed request either way).
//! - `onBeforeSendHeaders` rewrites `Referer` to `https://google.com` for
//!   `https://www.youtube.com/embed/*` (the UMG video-playback fix);
//!   native sets the identical header on the identical URL shape.
//! - Proxy (`proxyMode` / `proxyRules` / `proxyBypassRules` /
//!   `proxyPacScript` plus the `HTTPS_PROXY` / `NO_PROXY` environment)
//!   maps to WebView2 additional browser arguments at environment creation
//!   (the only WebView2 proxy surface; Chromium accepts the same
//!   `--proxy-server` / `--proxy-bypass-list` / `--proxy-pac-url` /
//!   `--proxy-auto-detect` / `--no-proxy-server` switches Electron sets).
//!   Values are read from the host's own settings file at startup — never
//!   from the renderer (those keys stay out of the renderer's
//!   `settings.set` allowlist, so a page cannot reroute traffic).
//!
//! Deliberate differences from Electron:
//! - NEL reports (`nel.cloudflare.com`) issued by the browser network
//!   service itself may bypass `WebResourceRequested`; the decision layer
//!   still classifies them as blocked, and every page-issued request to
//!   those hosts IS intercepted.
//! - Proxy changes apply on next launch (environment creation is
//!   process-lifetime), matching Electron's startup-registered handlers.
//!
//! Privacy: log lines carry fixed vocabulary plus booleans only — never
//! URLs, hosts, ports, or proxy values.

/// Referer sent for YouTube embeds (Electron `window.ts` parity).
pub const YOUTUBE_REWRITE_REFERER: &str = "https://google.com";

/// Localhost WebSocket ports that stay reachable (Electron
/// `DEFAULT_ALLOWED_LOCALHOST_WS_PORTS` parity: arRPC-adjacent local RPC).
pub const ALLOWED_LOCALHOST_WS_PORTS: &[u16] = &[1211, 1112, 6888];

/// Telemetry/noise hosts cancelled in `onBeforeRequest` parity
/// (`sentry.io`, `nel.cloudflare.com`, exact-or-subdomain).
const TELEMETRY_HOSTS: &[&str] = &["sentry.io", "nel.cloudflare.com"];

/// Maximum length accepted for one proxy value. Proxy rules are short
/// (`host:port` lists); anything longer is malformed and dropped.
const MAX_PROXY_VALUE_BYTES: usize = 4096;

/// True when `host` is exactly `domain` or a subdomain of it
/// (case-insensitive; Electron `hostnameMatches` parity).
pub fn host_matches(host: &str, domain: &str) -> bool {
    let host = host.to_lowercase();
    let domain = domain.to_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// True when `path` is a Discord science endpoint (`/api/vN/science…`,
/// Electron `/^\/api\/v\d+\/science(?:\/|$)/` parity on the URL path).
fn is_science_path(path: &str) -> bool {
    let rest = match path.strip_prefix("/api/v") {
        Some(rest) => rest,
        None => return false,
    };
    let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 {
        return false;
    }
    match rest[digits..].strip_prefix("/science") {
        Some(tail) => tail.is_empty() || tail.starts_with('/'),
        None => false,
    }
}

/// Telemetry / noise URLs cancelled in `webRequest` parity (Electron
/// `isTelemetryBlockedUrl`): https-only; Discord science endpoints plus
/// the telemetry hosts. Anything unparseable is allowed (fail-open would
/// be wrong here — an unparseable URL cannot match, so it passes).
pub fn is_telemetry_blocked_url(request_url: &str) -> bool {
    let url = match url::Url::parse(request_url) {
        Ok(url) => url,
        Err(_) => return false,
    };
    if url.scheme() != "https" {
        return false;
    }
    if is_science_path(url.path()) {
        return true;
    }
    let host = url.host_str().unwrap_or_default();
    TELEMETRY_HOSTS
        .iter()
        .any(|domain| host_matches(host, domain))
}

/// Stray localhost WebSocket probes (Electron `isBlockedLocalhostWebSocket`
/// parity): `ws:`/`wss:` to exactly `127.0.0.1` or `localhost` whose port
/// is outside `allowed_ports`. Unparseable URLs and non-WebSocket schemes
/// are never blocked. An explicit port of `0` is blocked (not allowlisted).
pub fn is_blocked_localhost_websocket(request_url: &str, allowed_ports: &[u16]) -> bool {
    let url = match url::Url::parse(request_url) {
        Ok(url) => url,
        Err(_) => return false,
    };
    if url.scheme() != "ws" && url.scheme() != "wss" {
        return false;
    }
    // Exact-host parity with Electron (`127.0.0.1` / `localhost` only —
    // no subdomains, no `::1`).
    let host = url.host_str().unwrap_or_default();
    if host != "127.0.0.1" && host != "localhost" {
        return false;
    }
    let port = url.port_or_known_default().unwrap_or(0);
    !allowed_ports.contains(&port)
}

/// Referer rewrite target for YouTube embeds (Electron
/// `https://www.youtube.com/embed/*` URL-filter parity): `Some` with the
/// fixed Google referer for exact-host `www.youtube.com` https URLs whose
/// path starts with `/embed/`, else `None`.
pub fn youtube_embed_rewrite_referer(request_url: &str) -> Option<&'static str> {
    let url = url::Url::parse(request_url).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    if url.host_str().unwrap_or_default() != "www.youtube.com" {
        return None;
    }
    if !url.path().starts_with("/embed/") {
        return None;
    }
    Some(YOUTUBE_REWRITE_REFERER)
}

/// What the request filter does with one request URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterAction {
    /// Pass through untouched.
    Allow,
    /// Cancel (served as an empty `403`).
    Block(BlockKind),
    /// Rewrite `Referer` to [`YOUTUBE_REWRITE_REFERER`].
    RewriteReferer,
}

/// Why a request was blocked (fixed vocabulary for logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// Discord science endpoint.
    Science,
    /// Telemetry host (`sentry.io`, `nel.cloudflare.com`).
    TelemetryHost,
    /// Stray localhost WebSocket probe.
    LocalhostWs,
}

impl BlockKind {
    fn as_str(self) -> &'static str {
        match self {
            BlockKind::Science => "science",
            BlockKind::TelemetryHost => "telemetry-host",
            BlockKind::LocalhostWs => "localhost-ws",
        }
    }
}

/// Classify one request URL: blocks win over the Referer rewrite (a
/// cancelled request never reaches the header stage, as in Electron).
pub fn classify_request(request_url: &str) -> FilterAction {
    if is_telemetry_blocked_url(request_url) {
        // Distinguish science from host telemetry for the log line only.
        let kind = match url::Url::parse(request_url) {
            Ok(url) if is_science_path(url.path()) => BlockKind::Science,
            _ => BlockKind::TelemetryHost,
        };
        return FilterAction::Block(kind);
    }
    if is_blocked_localhost_websocket(request_url, ALLOWED_LOCALHOST_WS_PORTS) {
        return FilterAction::Block(BlockKind::LocalhostWs);
    }
    if youtube_embed_rewrite_referer(request_url).is_some() {
        return FilterAction::RewriteReferer;
    }
    FilterAction::Allow
}

/// Fixed-vocabulary `[NetFilter]` log line for one filter decision.
/// Carries the action plus booleans only — never URLs or hosts.
pub fn format_decision_log(action: FilterAction) -> String {
    match action {
        FilterAction::Allow => "[NetFilter] allow".to_owned(),
        FilterAction::Block(kind) => format!("[NetFilter] block kind={}", kind.as_str()),
        FilterAction::RewriteReferer => "[NetFilter] rewrite kind=youtube-referer".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Proxy
// ---------------------------------------------------------------------------

/// Chromium/Electron proxy modes (exact `ProxyMode` vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyMode {
    System,
    Direct,
    FixedServers,
    PacScript,
    AutoDetect,
}

impl ProxyMode {
    /// Exact-string parse; unknown values are `None` (caller fails closed
    /// to [`ProxyMode::System`]).
    pub fn parse(text: &str) -> Option<ProxyMode> {
        match text {
            "system" => Some(ProxyMode::System),
            "direct" => Some(ProxyMode::Direct),
            "fixed_servers" => Some(ProxyMode::FixedServers),
            "pac_script" => Some(ProxyMode::PacScript),
            "auto_detect" => Some(ProxyMode::AutoDetect),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            ProxyMode::System => "system",
            ProxyMode::Direct => "direct",
            ProxyMode::FixedServers => "fixed_servers",
            ProxyMode::PacScript => "pac_script",
            ProxyMode::AutoDetect => "auto_detect",
        }
    }
}

/// Proxy inputs decoupled from the settings snapshot so the pure resolver
/// stays unit-testable without a 60-field fixture.
#[derive(Debug, Clone, Copy)]
pub struct ProxyInputs<'a> {
    pub mode: &'a str,
    pub rules: &'a str,
    pub bypass: &'a str,
    pub pac: &'a str,
}

impl<'a> ProxyInputs<'a> {
    /// View over the native settings snapshot (host-owned values; the
    /// renderer cannot mutate these keys).
    pub fn from_snapshot(snapshot: &'a crate::settings::SettingsSnapshot) -> Self {
        Self {
            mode: snapshot.proxy_mode.as_str(),
            rules: snapshot.proxy_rules.as_str(),
            bypass: snapshot.proxy_bypass_rules.as_str(),
            pac: snapshot.proxy_pac_script.as_str(),
        }
    }
}

/// Process proxy environment (already read; empty means absent).
#[derive(Debug, Clone, Default)]
pub struct ProxyEnv {
    pub https_proxy: String,
    pub no_proxy: String,
}

impl ProxyEnv {
    /// Read `HTTPS_PROXY`/`https_proxy` and `NO_PROXY`/`no_proxy`
    /// (Electron `envProxy`/`envNoProxy` parity).
    pub fn read() -> Self {
        Self {
            https_proxy: std::env::var("HTTPS_PROXY")
                .or_else(|_| std::env::var("https_proxy"))
                .unwrap_or_default(),
            no_proxy: std::env::var("NO_PROXY")
                .or_else(|_| std::env::var("no_proxy"))
                .unwrap_or_default(),
        }
    }
}

/// Convert a Chromium-style bypass list to `NO_PROXY` form (Electron
/// `toNoProxy` parity): split on `,`/`;`, drop empties, `<local>` expands
/// to the loopback set, a leading `*.` becomes `.`.
pub fn to_no_proxy(bypass: &str) -> String {
    bypass
        .split([',', ';'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            if part == "<local>" {
                "localhost,127.0.0.1,::1".to_owned()
            } else if let Some(rest) = part.strip_prefix("*.") {
                format!(".{rest}")
            } else {
                part.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Pick a single proxy URL from Chromium `proxyRules` (Electron
/// `primaryProxyFromRules` parity): prefer an explicit
/// `https=`/`http=`/`socks=`/`socks5=`/`socks4=` mapping, else the first
/// token; `direct://` means no proxy; a bare `host:port` gains `http://`.
pub fn primary_proxy_from_rules(rules: &str) -> Option<String> {
    let trimmed = rules.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    for scheme in ["https", "http", "socks", "socks5", "socks4"] {
        let prefix = format!("{scheme}=");
        if let Some(mapped) = parts
            .iter()
            .find(|part| part.to_lowercase().starts_with(&prefix))
        {
            let value = mapped[prefix.len()..].split(',').next()?.trim();
            if value.is_empty() || value == "direct://" {
                continue;
            }
            if value.contains("://") {
                return Some(value.to_owned());
            }
            return Some(format!("http://{value}"));
        }
    }
    let first = parts.first()?.split(',').next()?.trim();
    if first.is_empty() || first == "direct://" {
        return None;
    }
    if let Some((_, value)) = first.split_once('=') {
        let value = value.trim();
        if value.is_empty() || value == "direct://" {
            return None;
        }
        if value.contains("://") {
            return Some(value.to_owned());
        }
        return Some(format!("http://{value}"));
    }
    if first.contains("://") {
        return Some(first.to_owned());
    }
    Some(format!("http://{first}"))
}

/// A proxy value is safe to splice into the space-joined additional-args
/// string only when it is short and carries no ASCII whitespace, controls,
/// or double quotes (a space would silently split one switch into two;
/// Electron passes these as single argv elements, WebView2 takes one
/// joined string — so malformed values fail closed here instead).
fn is_arg_safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PROXY_VALUE_BYTES
        && !value
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || b == b'"')
}

/// Non-fatal proxy resolution notes (fixed vocabulary; values never logged).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyNote {
    /// `proxyMode` was unknown; resolved as `system`.
    UnknownMode,
    /// A set value was malformed for arg splicing and ignored.
    UnsafeValueDropped,
}

/// Resolve the additional-browser-args fragment for one proxy
/// configuration (leading space included when non-empty, so the caller
/// concatenates directly after the base args). Pure: fully determined by
/// `inputs` + `env`.
pub fn resolve_proxy_args(inputs: ProxyInputs<'_>, env: &ProxyEnv) -> (String, Vec<ProxyNote>) {
    let mut notes = Vec::new();
    let mode = match ProxyMode::parse(inputs.mode) {
        Some(mode) => mode,
        None => {
            notes.push(ProxyNote::UnknownMode);
            ProxyMode::System
        }
    };
    let bypass = inputs.bypass.trim();
    let bypass_arg = if bypass.is_empty() {
        None
    } else if !is_arg_safe(bypass) {
        notes.push(ProxyNote::UnsafeValueDropped);
        None
    } else {
        Some(format!(" --proxy-bypass-list={bypass}"))
    };
    let with_bypass = |base: String| base + bypass_arg.as_deref().unwrap_or_default();

    match mode {
        ProxyMode::Direct => (" --no-proxy-server".to_owned(), notes),
        ProxyMode::AutoDetect => (with_bypass(" --proxy-auto-detect".to_owned()), notes),
        ProxyMode::PacScript => {
            let pac = inputs.pac.trim();
            if !is_arg_safe(pac) {
                if !pac.is_empty() {
                    notes.push(ProxyNote::UnsafeValueDropped);
                }
                return (String::new(), notes);
            }
            (with_bypass(format!(" --proxy-pac-url={pac}")), notes)
        }
        ProxyMode::FixedServers => {
            let rules = inputs.rules.trim();
            if !is_arg_safe(rules) {
                if !rules.is_empty() {
                    notes.push(ProxyNote::UnsafeValueDropped);
                }
                return (String::new(), notes);
            }
            (with_bypass(format!(" --proxy-server={rules}")), notes)
        }
        ProxyMode::System => {
            let from_env = env.https_proxy.trim();
            if from_env.is_empty() || !is_arg_safe(from_env) {
                if !from_env.is_empty() {
                    notes.push(ProxyNote::UnsafeValueDropped);
                }
                return (String::new(), notes);
            }
            // Electron parity: explicit bypass wins, else convert NO_PROXY
            // (defaulting to `<local>`) to the bypass list.
            let bypass_list = if let Some(arg) = bypass_arg {
                // Strip the leading switch; reuse the validated value.
                arg.trim_start_matches(" --proxy-bypass-list=").to_owned()
            } else {
                let source = if env.no_proxy.trim().is_empty() {
                    "<local>"
                } else {
                    env.no_proxy.trim()
                };
                to_no_proxy(source)
            };
            if !is_arg_safe(&bypass_list) {
                notes.push(ProxyNote::UnsafeValueDropped);
                return (format!(" --proxy-server={from_env}"), notes);
            }
            (
                format!(" --proxy-server={from_env} --proxy-bypass-list={bypass_list}"),
                notes,
            )
        }
    }
}

/// Startup proxy resolution: the host's own settings file plus the process
/// environment. Unreadable/missing settings fail closed to `system` (the
/// Electron default); renderer input never reaches this path.
///
/// Besides the browser-args fragment, this performs Electron's
/// `configureNodeProxyEnv` equivalent: `fixed_servers` exports the primary
/// proxy URL into the process `HTTP(S)_PROXY` environment (plus `NO_PROXY`
/// when a bypass list is set) so env-honoring fetches route identically,
/// and `direct` clears proxy env so nothing routes through a proxy.
/// `system` / `pac_script` / `auto_detect` leave the inherited environment
/// untouched. Runs once at environment creation.
pub struct ProxyStartup {
    /// Args fragment (leading space when non-empty) for environment creation.
    pub args: String,
    /// Whether any proxy switch was produced.
    pub configured: bool,
}

pub fn proxy_startup_args(profile_dir: &std::path::Path) -> ProxyStartup {
    let snapshot = match crate::settings::load_settings(profile_dir) {
        Ok(loaded) => loaded.snapshot,
        Err(error) => {
            eprintln!("[NetFilter][Proxy] settings unreadable; using system proxy ({error})");
            crate::settings::SettingsSnapshot::default()
        }
    };
    let env = ProxyEnv::read();
    let (args, notes) = resolve_proxy_args(ProxyInputs::from_snapshot(&snapshot), &env);
    for note in &notes {
        eprintln!(
            "[NetFilter][Proxy] note={}",
            match note {
                ProxyNote::UnknownMode => "unknown-mode",
                ProxyNote::UnsafeValueDropped => "unsafe-value-dropped",
            }
        );
    }
    let mode = ProxyMode::parse(snapshot.proxy_mode.as_str()).unwrap_or(ProxyMode::System);
    sync_process_proxy_env(mode, &snapshot, &notes);
    let configured = !args.is_empty();
    eprintln!(
        "[NetFilter][Proxy] mode={} configured={configured}",
        mode.as_str()
    );
    ProxyStartup { args, configured }
}

/// Electron `configureNodeProxyEnv` equivalent (see [`proxy_startup_args`]).
/// Skipped entirely when resolution produced an unsafe-value note (never
/// export a value that failed arg validation).
fn sync_process_proxy_env(
    mode: ProxyMode,
    snapshot: &crate::settings::SettingsSnapshot,
    notes: &[ProxyNote],
) {
    if notes.contains(&ProxyNote::UnsafeValueDropped) {
        return;
    }
    match mode {
        ProxyMode::Direct => {
            for key in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
                std::env::remove_var(key);
            }
            std::env::set_var("NO_PROXY", "*");
            std::env::set_var("no_proxy", "*");
        }
        ProxyMode::FixedServers => {
            if let Some(proxy_url) = primary_proxy_from_rules(snapshot.proxy_rules.as_str()) {
                for key in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
                    std::env::set_var(key, &proxy_url);
                }
                let bypass = snapshot.proxy_bypass_rules.trim();
                if !bypass.is_empty() {
                    let no_proxy = to_no_proxy(bypass);
                    std::env::set_var("NO_PROXY", &no_proxy);
                    std::env::set_var("no_proxy", &no_proxy);
                }
            }
        }
        ProxyMode::System | ProxyMode::PacScript | ProxyMode::AutoDetect => {}
    }
}

// ---------------------------------------------------------------------------
// WebView2 registration
// ---------------------------------------------------------------------------

/// URL filters for the request handler: telemetry travels over XHR/fetch,
/// stray probes over WebSocket handshakes, and the Referer rewrite targets
/// YouTube embed documents. `*` patterns are exact WebView2 filter syntax.
#[cfg(target_os = "windows")]
const REQUEST_FILTERS: &[(
    webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT,
    &str,
)] = &[
    (
        webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST,
        "*://*/*",
    ),
    (
        webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH,
        "*://*/*",
    ),
    (
        webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT_WEBSOCKET,
        "*://*/*",
    ),
    (
        webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
        "https://www.youtube.com/embed/*",
    ),
];

/// Registers the F21 request filter for one WebView: telemetry and stray
/// localhost-WebSocket requests are answered with an empty `403`, YouTube
/// embed documents get the fixed Google `Referer`, everything else passes
/// through untouched. Runs after `build()` and before the first Discord
/// navigation (called from `attach_main_content_filters`), so the first
/// document is filtered like every later one. Failures are returned (the
/// caller logs loudly); filtering is never boot-required.
#[cfg(target_os = "windows")]
pub fn register(
    core_webview2: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_2;
    use webview2_com::WebResourceRequestedEventHandler;
    use windows::{
        core::{Interface, HSTRING, PWSTR},
        Win32::System::Com::IStream,
    };

    let webview2 = core_webview2
        .cast::<ICoreWebView2_2>()
        .map_err(|_| "ICoreWebView2_2 unavailable".to_owned())?;
    for (context, pattern) in REQUEST_FILTERS {
        unsafe {
            webview2
                .AddWebResourceRequestedFilter(&HSTRING::from(*pattern), *context)
                .map_err(|_| format!("AddWebResourceRequestedFilter refused {pattern}"))?;
        }
    }
    let env = unsafe {
        webview2
            .Environment()
            .map_err(|_| "WebView2 environment unavailable".to_owned())?
    };
    let handler = WebResourceRequestedEventHandler::create(Box::new(
        move |_sender: Option<
            webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
        >,
              args: Option<
            webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2WebResourceRequestedEventArgs,
        >| {
            let Some(args) = args else {
                return Ok(());
            };
            let request = match unsafe { args.Request() } {
                Ok(request) => request,
                Err(_) => return Ok(()),
            };
            let uri = {
                let mut uri = PWSTR::null();
                if unsafe { request.Uri(&mut uri) }.is_err() {
                    return Ok(());
                }
                unsafe { uri.to_string().unwrap_or_default() }
            };
            match classify_request(&uri) {
                FilterAction::Allow => Ok(()),
                FilterAction::RewriteReferer => {
                    if let Ok(headers) = unsafe { request.Headers() } {
                        let _ = unsafe {
                            headers.SetHeader(
                                &HSTRING::from("Referer"),
                                &HSTRING::from(YOUTUBE_REWRITE_REFERER),
                            )
                        };
                    }
                    println!("{}", format_decision_log(FilterAction::RewriteReferer));
                    Ok(())
                }
                FilterAction::Block(kind) => {
                    let action = FilterAction::Block(kind);
                    let empty: Option<IStream> = None;
                    let response = unsafe {
                        env.CreateWebResourceResponse(
                            empty.as_ref(),
                            403,
                            &HSTRING::from("Blocked"),
                            &HSTRING::from("Content-Length: 0\r\n"),
                        )
                    };
                    match response {
                        Ok(web_response) => {
                            let _ = unsafe { args.SetResponse(&web_response) };
                        }
                        Err(_) => return Ok(()),
                    }
                    println!("{}", format_decision_log(action));
                    Ok(())
                }
            }
        },
    ));
    let mut token = 0i64;
    unsafe {
        core_webview2
            .add_WebResourceRequested(&handler, &mut token)
            .map_err(|_| "add_WebResourceRequested failed".to_owned())?;
    }
    println!("[NetFilter] request filter registered (telemetry block, localhost-ws block, youtube-referer)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs<'a>(mode: &'a str, rules: &'a str, bypass: &'a str, pac: &'a str) -> ProxyInputs<'a> {
        ProxyInputs {
            mode,
            rules,
            bypass,
            pac,
        }
    }

    fn env(https_proxy: &str, no_proxy: &str) -> ProxyEnv {
        ProxyEnv {
            https_proxy: https_proxy.to_owned(),
            no_proxy: no_proxy.to_owned(),
        }
    }

    #[test]
    fn science_endpoints_block_on_any_version() {
        for url in [
            "https://discord.com/api/v9/science",
            "https://discord.com/api/v9/science/",
            "https://discord.com/api/v11/science/batch",
            "https://canary.discord.com/api/v6/science?a=1",
        ] {
            assert!(is_telemetry_blocked_url(url), "{url} must block");
            assert_eq!(
                classify_request(url),
                FilterAction::Block(BlockKind::Science)
            );
        }
    }

    #[test]
    fn science_lookalikes_pass() {
        for url in [
            "https://discord.com/api/v9/sciences", // trailing suffix, no boundary
            "https://discord.com/api/science",     // missing version
            "https://discord.com/api/v/science",   // missing digits
            "https://discord.com/api/v9/xscience",
            "https://discord.com/app",
        ] {
            assert!(!is_telemetry_blocked_url(url), "{url} must pass");
        }
    }

    #[test]
    fn telemetry_hosts_block_exact_and_subdomain_only() {
        for url in [
            "https://sentry.io/api/1",
            "https://o123.sentry.io/ping",
            "https://SENTRY.IO/upper",
            "https://nel.cloudflare.com/report",
            "https://sub.nel.cloudflare.com/x",
        ] {
            assert!(is_telemetry_blocked_url(url), "{url} must block");
        }
        for url in [
            "https://discord.com/api/v9/channels",
            "https://cdn.discordapp.com/avatars/1.png",
            "https://sentry.io.evil.com/x", // suffix trick, not a subdomain
            "https://evilsentry.io/x",
            "https://nel.cloudflare.com.evil.com/x",
            "http://sentry.io/plain", // Electron parity: https-only
            "wss://sentry.io/socket",
        ] {
            assert!(!is_telemetry_blocked_url(url), "{url} must pass");
        }
    }

    #[test]
    fn userinfo_and_port_tricks_do_not_confuse_host_match() {
        // `url` parsing isolates the real host: userinfo and ports cannot
        // smuggle a telemetry host past (or into) the blocklist.
        assert!(!is_telemetry_blocked_url("https://sentry.io@discord.com/x"));
        assert!(is_telemetry_blocked_url("https://discord.com@sentry.io/x"));
        assert!(is_telemetry_blocked_url("https://sentry.io:8443/x"));
    }

    #[test]
    fn localhost_websockets_block_except_allowlisted_ports() {
        for url in [
            "ws://127.0.0.1:6463/?v=1",
            "ws://localhost:3000/socket",
            "wss://127.0.0.1:9999/x",
            "ws://localhost:0/x",
            "ws://127.0.0.1/x",  // default ws port 80
            "wss://localhost/x", // default wss port 443
        ] {
            assert!(
                is_blocked_localhost_websocket(url, ALLOWED_LOCALHOST_WS_PORTS),
                "{url} must block"
            );
            assert_eq!(
                classify_request(url),
                FilterAction::Block(BlockKind::LocalhostWs)
            );
        }
        for url in [
            "ws://127.0.0.1:1211/v1/rpc",
            "ws://localhost:1112/x",
            "wss://127.0.0.1:6888/stream",
            "wss://gateway.discord.gg/?v=10", // not localhost at all
            "https://127.0.0.1:6463/x",       // not a websocket scheme
            "ws://sub.localhost:9999/x",      // exact-host parity: subdomains pass
            "ws://[::1]:9999/x",              // exact-host parity: IPv6 loopback passes
        ] {
            assert!(
                !is_blocked_localhost_websocket(url, ALLOWED_LOCALHOST_WS_PORTS),
                "{url} must pass"
            );
        }
    }

    #[test]
    fn youtube_embed_rewrite_matches_electron_filter_exactly() {
        assert_eq!(
            youtube_embed_rewrite_referer("https://www.youtube.com/embed/abc123"),
            Some("https://google.com")
        );
        assert_eq!(
            classify_request("https://www.youtube.com/embed/abc123?rel=0"),
            FilterAction::RewriteReferer
        );
        for url in [
            "https://www.youtube.com/watch?v=abc123",
            "https://www.youtube.com/embed", // no trailing slash: filter needs /embed/*
            "https://youtube.com/embed/abc123", // bare host, not www
            "https://m.youtube.com/embed/abc123",
            "http://www.youtube.com/embed/abc123", // https-only
            "https://www.youtube.com.evil.com/embed/abc123",
            "https://discord.com/app",
        ] {
            assert_eq!(
                youtube_embed_rewrite_referer(url),
                None,
                "{url} must not rewrite"
            );
        }
    }

    #[test]
    fn blocks_win_over_rewrite_and_garbage_passes() {
        // A URL cannot be both; classification order is pinned anyway.
        assert_eq!(
            classify_request("https://discord.com/api/v9/science"),
            FilterAction::Block(BlockKind::Science)
        );
        assert_eq!(classify_request("not a url"), FilterAction::Allow);
        assert_eq!(classify_request(""), FilterAction::Allow);
        assert_eq!(
            classify_request("https://cdn.discordapp.com/x.png"),
            FilterAction::Allow
        );
    }

    #[test]
    fn decision_logs_are_fixed_vocabulary_and_data_free() {
        let lines = [
            format_decision_log(FilterAction::Allow),
            format_decision_log(FilterAction::Block(BlockKind::Science)),
            format_decision_log(FilterAction::Block(BlockKind::TelemetryHost)),
            format_decision_log(FilterAction::Block(BlockKind::LocalhostWs)),
            format_decision_log(FilterAction::RewriteReferer),
        ];
        assert_eq!(lines[0], "[NetFilter] allow");
        assert_eq!(lines[1], "[NetFilter] block kind=science");
        assert_eq!(lines[2], "[NetFilter] block kind=telemetry-host");
        assert_eq!(lines[3], "[NetFilter] block kind=localhost-ws");
        assert_eq!(lines[4], "[NetFilter] rewrite kind=youtube-referer");
        for line in &lines {
            // Kind labels (`telemetry-host`, `localhost-ws`,
            // `youtube-referer`) are fixed vocabulary, not data: what must
            // never appear is a URL or a concrete host/port/proxy value.
            for forbidden in [
                "://",
                "sentry.io",
                "cloudflare.com",
                "discord.com",
                "youtube.com",
                "google.com",
                "127.0.0.1",
                "localhost:",
                "proxy=",
                "referer=",
            ] {
                assert!(
                    !line.to_lowercase().contains(forbidden),
                    "log line must not contain {forbidden:?}: {line}"
                );
            }
        }
    }

    #[test]
    fn proxy_modes_resolve_to_electron_equivalent_switches() {
        let empty_env = env("", "");
        let (args, notes) = resolve_proxy_args(inputs("direct", "", "", ""), &empty_env);
        assert_eq!(args, " --no-proxy-server");
        assert!(notes.is_empty());

        let (args, _) = resolve_proxy_args(
            inputs("auto_detect", "", "<local>,*.intranet.example", ""),
            &empty_env,
        );
        assert_eq!(
            args,
            " --proxy-auto-detect --proxy-bypass-list=<local>,*.intranet.example"
        );

        let (args, _) = resolve_proxy_args(
            inputs("pac_script", "", "", "https://proxy.example/pac.js"),
            &empty_env,
        );
        assert_eq!(args, " --proxy-pac-url=https://proxy.example/pac.js");

        let (args, _) = resolve_proxy_args(
            inputs("fixed_servers", "http=127.0.0.1:8080", "<local>", ""),
            &empty_env,
        );
        assert_eq!(
            args,
            " --proxy-server=http=127.0.0.1:8080 --proxy-bypass-list=<local>"
        );

        // system without env: no switches at all.
        let (args, notes) = resolve_proxy_args(inputs("system", "", "", ""), &empty_env);
        assert_eq!(args, "");
        assert!(notes.is_empty());
    }

    #[test]
    fn system_mode_honors_proxy_env_with_bypass_fallback() {
        // Env proxy + explicit bypass wins verbatim.
        let (args, _) = resolve_proxy_args(
            inputs("system", "", "*.corp.example", ""),
            &env("http://proxy:3128", "example.com"),
        );
        assert_eq!(
            args,
            " --proxy-server=http://proxy:3128 --proxy-bypass-list=*.corp.example"
        );
        // No explicit bypass: NO_PROXY converts (`<local>` default expands).
        let (args, _) =
            resolve_proxy_args(inputs("system", "", "", ""), &env("http://proxy:3128", ""));
        assert_eq!(
            args,
            " --proxy-server=http://proxy:3128 --proxy-bypass-list=localhost,127.0.0.1,::1"
        );
        let (args, _) = resolve_proxy_args(
            inputs("system", "", "", ""),
            &env("http://proxy:3128", "a.example, *.b.example"),
        );
        assert_eq!(
            args,
            " --proxy-server=http://proxy:3128 --proxy-bypass-list=a.example,.b.example"
        );
    }

    #[test]
    fn unknown_modes_and_empty_values_fail_closed() {
        let (args, notes) = resolve_proxy_args(inputs("warp_drive", "", "", ""), &env("", ""));
        assert_eq!(args, "");
        assert!(notes.contains(&ProxyNote::UnknownMode));

        // Empty fixed_servers rules: no switch (Electron appends nothing).
        let (args, _) = resolve_proxy_args(inputs("fixed_servers", "  ", "", ""), &env("", ""));
        assert_eq!(args, "");

        // Empty pac: no switch.
        let (args, _) = resolve_proxy_args(inputs("pac_script", "", "", " "), &env("", ""));
        assert_eq!(args, "");
    }

    #[test]
    fn unsafe_proxy_values_are_dropped_never_spliced() {
        // A space would split one switch into two argv elements: drop it.
        let (args, notes) = resolve_proxy_args(
            inputs("fixed_servers", "http=127.0.0.1:8080 evil", "", ""),
            &env("", ""),
        );
        assert_eq!(args, "");
        assert!(notes.contains(&ProxyNote::UnsafeValueDropped));

        let (args, notes) = resolve_proxy_args(
            inputs("system", "", "", ""),
            &env("http://proxy:3128\" --evil", ""),
        );
        assert_eq!(args, "");
        assert!(notes.contains(&ProxyNote::UnsafeValueDropped));
    }

    #[test]
    fn primary_proxy_from_rules_matches_electron_preference() {
        assert_eq!(
            primary_proxy_from_rules("http=127.0.0.1:8080"),
            Some("http://127.0.0.1:8080".to_owned())
        );
        assert_eq!(
            primary_proxy_from_rules("http=a:1;https=https://b:2"),
            Some("https://b:2".to_owned())
        );
        assert_eq!(
            primary_proxy_from_rules("socks5://h:1080"),
            Some("socks5://h:1080".to_owned())
        );
        assert_eq!(primary_proxy_from_rules("direct://"), None);
        assert_eq!(primary_proxy_from_rules("  "), None);
        assert_eq!(
            primary_proxy_from_rules("proxy.example:3128"),
            Some("http://proxy.example:3128".to_owned())
        );
    }

    #[test]
    fn to_no_proxy_converts_chromium_lists() {
        assert_eq!(
            to_no_proxy("<local>,*.intranet.example, host.example "),
            "localhost,127.0.0.1,::1,.intranet.example,host.example"
        );
        assert_eq!(to_no_proxy(""), "");
    }

    #[test]
    fn process_env_sync_mirrors_electron_node_env() {
        // Tests share one process environment: save and restore every key.
        let keys = [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "http_proxy",
            "https_proxy",
            "NO_PROXY",
            "no_proxy",
        ];
        let saved: Vec<Option<String>> = keys.iter().map(|key| std::env::var(key).ok()).collect();

        let snapshot_with =
            |mode: &str, rules: &str, bypass: &str| crate::settings::SettingsSnapshot {
                proxy_mode: mode.to_owned(),
                proxy_rules: rules.to_owned(),
                proxy_bypass_rules: bypass.to_owned(),
                ..Default::default()
            };

        // fixed_servers exports the primary proxy URL plus converted bypass.
        let snapshot = snapshot_with("fixed_servers", "http=127.0.0.1:8080", "<local>");
        sync_process_proxy_env(ProxyMode::FixedServers, &snapshot, &[]);
        assert_eq!(
            std::env::var("HTTPS_PROXY").ok(),
            Some("http://127.0.0.1:8080".to_owned())
        );
        assert_eq!(
            std::env::var("no_proxy").ok(),
            Some("localhost,127.0.0.1,::1".to_owned())
        );

        // direct clears proxy env and wildcards NO_PROXY.
        let snapshot = snapshot_with("direct", "", "");
        sync_process_proxy_env(ProxyMode::Direct, &snapshot, &[]);
        assert!(std::env::var("HTTPS_PROXY").is_err());
        assert_eq!(std::env::var("NO_PROXY").ok(), Some("*".to_owned()));

        // system leaves the inherited environment untouched.
        std::env::set_var("HTTPS_PROXY", "http://sentinel:1");
        let snapshot = snapshot_with("system", "", "");
        sync_process_proxy_env(ProxyMode::System, &snapshot, &[]);
        assert_eq!(
            std::env::var("HTTPS_PROXY").ok(),
            Some("http://sentinel:1".to_owned())
        );

        // An unsafe-value note suppresses the sync entirely.
        std::env::remove_var("HTTPS_PROXY");
        let snapshot = snapshot_with("fixed_servers", "has a space", "");
        sync_process_proxy_env(
            ProxyMode::FixedServers,
            &snapshot,
            &[ProxyNote::UnsafeValueDropped],
        );
        assert!(std::env::var("HTTPS_PROXY").is_err());

        for (key, value) in keys.iter().zip(saved) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    #[test]
    fn shipped_module_never_navigates_executes_or_mutates_settings() {
        let source = include_str!("net_filter.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // Decision + filter registration + the Electron-parity proxy env
        // sync only: no navigation, no script execution, no settings
        // mutation, no fetch, no process spawning. (`set_var`/`remove_var`
        // appear solely in `sync_process_proxy_env`, confined to the four
        // `*_PROXY` keys plus `NO_PROXY` — pinned below.)
        for forbidden in [
            "Navigate(",
            "Reload(",
            "ExecuteScript",
            "evaluate_script",
            "postMessage",
            "PostWebMessage",
            "apply_settings_set",
            "persist_settings",
            "std::process::Command",
            "ureq",
            "fetch(",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
        // The wiring surface is real, not documented-only, and the env sync
        // touches exactly the proxy keys (no PATH, no loader variables).
        assert!(shipped.contains("AddWebResourceRequestedFilter"));
        assert!(shipped.contains("SetHeader"));
        assert!(shipped.contains("classify_request"));
        assert!(shipped.contains("sync_process_proxy_env"));
        for key in ["PATH", "LD_PRELOAD", "DYLD", "NODE_"] {
            assert!(!shipped.contains(key), "env sync must never touch {key}");
        }
    }
}

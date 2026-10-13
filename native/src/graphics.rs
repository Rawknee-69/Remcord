//! CDP-backed graphics diagnostics (`graphics.get_info` capability).
//!
//! The pre-existing `diagnostics.get_info` readout reports only host-owned
//! facts (app version, WebView2 Runtime version, profile kind, modes, mod
//! ids): WebView2 exposes no GPU inventory through wry, so there was nothing
//! measured to copy. This module closes that gap with the supported WebView2
//! API — `ICoreWebView2::CallDevToolsProtocolMethod` — probing exactly two
//! fixed Chromium DevTools Protocol methods with fixed empty parameters:
//!
//! - `Browser.getVersion` (product, revision, user agent, JS/protocol versions)
//! - `System.getInfo` (GPU devices, feature status, model name, plus the
//!   active Chromium command line, from which ONLY a fixed token list of
//!   GPU-relevant flags and the two feature-list values are extracted)
//!
//! Browser-level protocol commands are NOT assumed to work on a page target:
//! every probe outcome is classified explicitly as `measured`, `unsupported`
//! (the runtime answered with JSON-RPC `-32601` method-not-found), or
//! `unavailable` (the call failed, timed out at the renderer, or the answer
//! did not parse). Unsupported and unavailable fields are reported as such —
//! GPU names are never invented and adapter availability is never equated
//! with active utilization.
//!
//! Privacy: the raw Chromium command line carries the profile user-data
//! directory, so it NEVER crosses the bridge. Only presence of a fixed
//! GPU-relevant token list plus the path-free `--enable-features=` /
//! `--disable-features=` values are reported. Bounded throughout: the raw
//! CDP JSON is capped, devices and feature entries are capped in count and
//! string length, and error text is truncated.
//!
//! Threading: the two probes are issued synchronously on the Tao event-loop
//! thread (the only thread that may touch the `ICoreWebView2`); each
//! one-shot completion handler forwards ONLY bounded strings plus the
//! request id through `NativeEvent::GraphicsCdpPart`. The event loop
//! assembles the report when both parts arrive and only then answers the
//! bridge call, so the UI thread never blocks waiting for the runtime.

use serde::Serialize;

/// `System.getInfo` CDP method name (fixed; the renderer cannot select it).
pub const CDP_SYSTEM_INFO_METHOD: &str = "System.getInfo";
/// `Browser.getVersion` CDP method name (fixed; the renderer cannot select it).
pub const CDP_BROWSER_VERSION_METHOD: &str = "Browser.getVersion";
/// Fixed CDP parameters for both probes: no target, session, or selector.
pub const CDP_EMPTY_PARAMS: &str = "{}";

/// Outcome when a probe returned usable data.
pub const STATUS_MEASURED: &str = "measured";
/// Outcome when the runtime answered JSON-RPC `-32601` (method not found):
/// the command is not supported on this page target by this runtime.
pub const STATUS_UNSUPPORTED: &str = "unsupported";
/// Outcome when the call failed or the answer did not parse.
pub const STATUS_UNAVAILABLE: &str = "unavailable";

/// JSON-RPC "method not found" code: the principled unsupported signal.
pub const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;

/// Ceiling for one raw CDP answer in characters (real answers are a few KiB).
pub const MAX_CDP_JSON_CHARS: usize = 131_072;
/// Maximum GPU devices kept from `System.getInfo` (real runtimes report few).
pub const MAX_GPU_DEVICES: usize = 16;
/// Maximum characters kept per GPU/device/model string.
pub const MAX_GPU_STRING_CHARS: usize = 512;
/// Maximum feature-status entries kept.
pub const MAX_FEATURE_ENTRIES: usize = 64;
/// Maximum characters kept per feature name/status string.
pub const MAX_FEATURE_STRING_CHARS: usize = 128;
/// Maximum characters kept per probe error string.
pub const MAX_CDP_ERROR_CHARS: usize = 512;
/// Maximum characters kept per extracted feature-list value.
pub const MAX_FEATURE_LIST_CHARS: usize = 1024;
/// Maximum concurrent in-flight graphics probes (fail-fast past this).
pub const MAX_PENDING_GRAPHICS: usize = 8;

/// Environment label: the host pre-registers the WebView2 environment with
/// wry-parity options plus the `remcord` scheme; everything else is the
/// WebView2 default (hardware acceleration enabled by default).
pub const ENVIRONMENT_LABEL: &str = "remcord-pre-registered (wry parity + remcord scheme)";

/// Fixed GPU-relevant command-line tokens whose presence is reported.
/// Presence-only (never values, never paths): a token is listed exactly when
/// it occurs in the runtime-reported Chromium command line.
pub const KNOWN_GPU_TOKENS: &[&str] = &[
    "--disable-gpu",
    "--disable-software-rasterizer",
    "--enable-unsafe-swiftshader",
    "--disable-accelerated-video-decode",
    "--use-gl",
    "--use-angle",
    "--in-process-gpu",
    "--single-process",
    "--no-sandbox",
    "--disable-gpu-sandbox",
    "--enable-features",
    "--disable-features",
];

/// Which of the two fixed probes a completion record belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdpMethod {
    SystemInfo,
    BrowserVersion,
}

impl CdpMethod {
    pub fn name(self) -> &'static str {
        match self {
            CdpMethod::SystemInfo => CDP_SYSTEM_INFO_METHOD,
            CdpMethod::BrowserVersion => CDP_BROWSER_VERSION_METHOD,
        }
    }

    pub fn params(self) -> &'static str {
        CDP_EMPTY_PARAMS
    }
}

/// One GPU device from `System.getInfo`, with every string bounded and
/// `None` exactly when the runtime did not report the field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GpuDevice {
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub vendor_string: Option<String>,
    pub device_string: Option<String>,
    pub driver_version: Option<String>,
}

/// One `featureStatus` entry (`{name, status}` keeps validation strict and
/// ordering stable; both strings are bounded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphicsFeature {
    pub name: String,
    pub status: String,
}

/// The `Browser.getVersion` section: measured fields plus an explicit
/// status, or the failure reason when not measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrowserVersionSection {
    pub status: &'static str,
    pub method: &'static str,
    pub product: Option<String>,
    pub revision: Option<String>,
    pub user_agent: Option<String>,
    pub js_version: Option<String>,
    pub protocol_version: Option<String>,
    pub error: Option<String>,
}

/// The `System.getInfo` section: measured GPU inventory plus an explicit
/// status, or the failure reason when not measured. `command_line_flags`
/// lists which fixed GPU-relevant tokens the runtime-reported command line
/// contains (absent token = not present, never "disabled by Remcord").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SystemInfoSection {
    pub status: &'static str,
    pub method: &'static str,
    pub model_name: Option<String>,
    pub devices: Vec<GpuDevice>,
    pub feature_status: Vec<GraphicsFeature>,
    pub command_line_flags: Vec<String>,
    pub disable_features: Option<String>,
    pub enable_features: Option<String>,
    pub error: Option<String>,
}

/// Bounded graphics report served by the `graphics.get_info` capability.
/// Fixed keys only — no paths, tokens, cookies, or file contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphicsInfoResponse {
    pub webview2_runtime: String,
    pub additional_browser_arguments: String,
    pub environment: &'static str,
    pub browser: BrowserVersionSection,
    pub system: SystemInfoSection,
}

/// Truncate to a character boundary (never splits UTF-8): CDP answers and
/// driver strings are untrusted lengths, so every kept string passes here.
pub fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    value.chars().take(max_chars).collect()
}

/// Classified probe failure: whether the method is unsupported by this
/// runtime/target or merely unavailable, plus the bounded reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionError {
    pub status: &'static str,
    pub error: String,
}

fn unavailable(reason: impl Into<String>) -> SectionError {
    SectionError {
        status: STATUS_UNAVAILABLE,
        error: truncate_chars(&reason.into(), MAX_CDP_ERROR_CHARS),
    }
}

fn unsupported(reason: impl Into<String>) -> SectionError {
    SectionError {
        status: STATUS_UNSUPPORTED,
        error: truncate_chars(&reason.into(), MAX_CDP_ERROR_CHARS),
    }
}

/// Read one optional string member, bounded. Non-string JSON types report
/// as absent (`None`), never stringified garbage.
fn opt_string(map: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(|value| value.as_str())
        .map(|text| truncate_chars(text, MAX_GPU_STRING_CHARS))
}

/// Read one optional id member: CDP reports ids as numbers or strings.
/// Numbers stringify exactly; anything else reports as absent.
fn opt_id(map: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(serde_json::Value::Number(number)) => Some(number.to_string()),
        Some(serde_json::Value::String(text)) => Some(truncate_chars(text, MAX_GPU_STRING_CHARS)),
        _ => None,
    }
}

/// Descend one `{ "result": { ... } }` envelope when present. WebView2
/// returns the CDP result object directly, but if a runtime ever wraps it,
/// the wrapped shape still parses instead of failing. Only the two fixed
/// probe methods flow through here, whose genuine results never carry a
/// top-level `result` member, so the descent cannot misread real data.
fn descend_envelope(value: &serde_json::Value) -> &serde_json::Value {
    match value {
        serde_json::Value::Object(map) => match map.get("result") {
            Some(inner @ serde_json::Value::Object(_)) => inner,
            _ => value,
        },
        _ => value,
    }
}

/// Classify a top-level `"error"` member: JSON-RPC `-32601` is the standard
/// method-not-found signal (`unsupported`); any other error shape is
/// `unavailable` with its bounded message.
fn classify_protocol_error(map: &serde_json::Map<String, serde_json::Value>) -> SectionError {
    let message = map
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(|message| message.as_str())
        .unwrap_or("the runtime reported an error");
    let code = map
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(|code| code.as_i64());
    if code == Some(JSON_RPC_METHOD_NOT_FOUND) {
        unsupported(format!(
            "the runtime does not support this method ({message})"
        ))
    } else {
        unavailable(format!("the runtime reported an error ({message})"))
    }
}

/// Parsed `Browser.getVersion` fields; every field is `None` exactly when
/// the runtime omitted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserFields {
    pub product: Option<String>,
    pub revision: Option<String>,
    pub user_agent: Option<String>,
    pub js_version: Option<String>,
    pub protocol_version: Option<String>,
}

/// Parse one `Browser.getVersion` answer (enveloped or bare). A present
/// `"error"` member classifies before any field is read.
pub fn parse_browser_version(json: &str) -> Result<BrowserFields, SectionError> {
    let root: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| unavailable(format!("answer did not parse ({error})")))?;
    let result = descend_envelope(&root);
    let Some(map) = result.as_object() else {
        return Err(unavailable("answer was not a JSON object"));
    };
    if map.contains_key("error") {
        return Err(classify_protocol_error(map));
    }
    Ok(BrowserFields {
        product: opt_string(map, "product"),
        revision: opt_string(map, "revision"),
        user_agent: opt_string(map, "userAgent"),
        js_version: opt_string(map, "jsVersion"),
        protocol_version: opt_string(map, "protocolVersion"),
    })
}

/// Parsed `System.getInfo` fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFields {
    pub model_name: Option<String>,
    pub devices: Vec<GpuDevice>,
    pub features: Vec<GraphicsFeature>,
    pub flags_present: Vec<String>,
    pub disable_features: Option<String>,
    pub enable_features: Option<String>,
}

fn parse_device(value: &serde_json::Value) -> Option<GpuDevice> {
    let map = value.as_object()?;
    Some(GpuDevice {
        vendor_id: opt_id(map, "vendorId"),
        device_id: opt_id(map, "deviceId"),
        vendor_string: opt_string(map, "vendorString"),
        device_string: opt_string(map, "deviceString"),
        driver_version: opt_string(map, "driverVersion"),
    })
}

/// Split a Chromium command line into tokens, respecting single and double
/// quotes. Substring matching on the raw line would confuse
/// `--disable-gpu` with `--disable-gpu-sandbox`, so every flag check below
/// works on exact tokens instead.
fn split_command_line(command_line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut in_token = false;
    for char in command_line.chars() {
        if let Some(active) = quote {
            if char == active {
                quote = None;
            } else {
                current.push(char);
            }
            continue;
        }
        if char == '"' || char == '\'' {
            quote = Some(char);
            in_token = true;
            continue;
        }
        if char.is_whitespace() {
            if in_token {
                tokens.push(std::mem::take(&mut current));
                in_token = false;
            }
            continue;
        }
        current.push(char);
        in_token = true;
    }
    if in_token {
        tokens.push(current);
    }
    tokens
}

/// Whether an exact token (or its `flag=value` form) is present.
fn token_present(tokens: &[String], flag: &str) -> bool {
    let prefixed = format!("{flag}=");
    tokens
        .iter()
        .any(|token| token == flag || token.starts_with(&prefixed))
}

/// Extract the value of a `flag=` token. Feature lists never contain paths
/// or spaces, so the scan cannot leak the user-data directory into the
/// report.
fn extract_flag_value_from_tokens(tokens: &[String], flag_with_eq: &str) -> Option<String> {
    let value = tokens
        .iter()
        .find_map(|token| token.strip_prefix(flag_with_eq))?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some(truncate_chars(value, MAX_FEATURE_LIST_CHARS))
}

fn parse_command_line(command_line: &str) -> (Vec<String>, Option<String>, Option<String>) {
    let tokens = split_command_line(command_line);
    let mut flags: Vec<String> = KNOWN_GPU_TOKENS
        .iter()
        .filter(|token| token_present(&tokens, token))
        .map(|token| token.to_string())
        .collect();
    flags.sort();
    flags.dedup();
    let disable_features = extract_flag_value_from_tokens(&tokens, "--disable-features=");
    let enable_features = extract_flag_value_from_tokens(&tokens, "--enable-features=");
    (flags, disable_features, enable_features)
}

/// Parse one `System.getInfo` answer (enveloped or bare). Devices are
/// capped, feature entries are capped, and the command line contributes
/// only the fixed token presence plus the two path-free feature lists.
pub fn parse_system_info(json: &str) -> Result<SystemFields, SectionError> {
    let root: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| unavailable(format!("answer did not parse ({error})")))?;
    let result = descend_envelope(&root);
    let Some(map) = result.as_object() else {
        return Err(unavailable("answer was not a JSON object"));
    };
    if map.contains_key("error") {
        return Err(classify_protocol_error(map));
    }
    let model_name = opt_string(map, "modelName");
    let mut devices = Vec::new();
    if let Some(gpu) = map.get("gpu").and_then(|gpu| gpu.as_object()) {
        if let Some(list) = gpu.get("devices").and_then(|devices| devices.as_array()) {
            for entry in list.iter().take(MAX_GPU_DEVICES) {
                if let Some(device) = parse_device(entry) {
                    devices.push(device);
                }
            }
        }
    }
    let mut features = Vec::new();
    if let Some(status) = map
        .get("gpu")
        .and_then(|gpu| gpu.get("featureStatus"))
        .and_then(|status| status.as_object())
    {
        for (name, value) in status.iter().take(MAX_FEATURE_ENTRIES) {
            let Some(text) = value.as_str() else {
                continue;
            };
            features.push(GraphicsFeature {
                name: truncate_chars(name, MAX_FEATURE_STRING_CHARS),
                status: truncate_chars(text, MAX_FEATURE_STRING_CHARS),
            });
        }
    }
    let (flags_present, disable_features, enable_features) = map
        .get("commandLine")
        .and_then(|line| line.as_str())
        .map(parse_command_line)
        .unwrap_or_default();
    Ok(SystemFields {
        model_name,
        devices,
        features,
        flags_present,
        disable_features,
        enable_features,
    })
}

fn browser_section(outcome: &Result<String, String>) -> BrowserVersionSection {
    match outcome {
        Err(reason) => BrowserVersionSection {
            status: STATUS_UNAVAILABLE,
            method: CDP_BROWSER_VERSION_METHOD,
            product: None,
            revision: None,
            user_agent: None,
            js_version: None,
            protocol_version: None,
            error: Some(truncate_chars(reason, MAX_CDP_ERROR_CHARS)),
        },
        Ok(json) => match parse_browser_version(json) {
            Ok(fields) => BrowserVersionSection {
                status: STATUS_MEASURED,
                method: CDP_BROWSER_VERSION_METHOD,
                product: fields.product,
                revision: fields.revision,
                user_agent: fields.user_agent,
                js_version: fields.js_version,
                protocol_version: fields.protocol_version,
                error: None,
            },
            Err(failure) => BrowserVersionSection {
                status: failure.status,
                method: CDP_BROWSER_VERSION_METHOD,
                product: None,
                revision: None,
                user_agent: None,
                js_version: None,
                protocol_version: None,
                error: Some(failure.error),
            },
        },
    }
}

fn system_section(outcome: &Result<String, String>) -> SystemInfoSection {
    match outcome {
        Err(reason) => SystemInfoSection {
            status: STATUS_UNAVAILABLE,
            method: CDP_SYSTEM_INFO_METHOD,
            model_name: None,
            devices: Vec::new(),
            feature_status: Vec::new(),
            command_line_flags: Vec::new(),
            disable_features: None,
            enable_features: None,
            error: Some(truncate_chars(reason, MAX_CDP_ERROR_CHARS)),
        },
        Ok(json) => match parse_system_info(json) {
            Ok(fields) => SystemInfoSection {
                status: STATUS_MEASURED,
                method: CDP_SYSTEM_INFO_METHOD,
                model_name: fields.model_name,
                devices: fields.devices,
                feature_status: fields.features,
                command_line_flags: fields.flags_present,
                disable_features: fields.disable_features,
                enable_features: fields.enable_features,
                error: None,
            },
            Err(failure) => SystemInfoSection {
                status: failure.status,
                method: CDP_SYSTEM_INFO_METHOD,
                model_name: None,
                devices: Vec::new(),
                feature_status: Vec::new(),
                command_line_flags: Vec::new(),
                disable_features: None,
                enable_features: None,
                error: Some(failure.error),
            },
        },
    }
}

/// Assemble the bridge report from the two probe outcomes. `runtime` falls
/// back to `"unknown"` (never guessed) like the diagnostics readout.
pub fn assemble_report(
    runtime: Option<&str>,
    browser_args: &str,
    browser_outcome: &Result<String, String>,
    system_outcome: &Result<String, String>,
) -> GraphicsInfoResponse {
    GraphicsInfoResponse {
        webview2_runtime: runtime
            .filter(|version| !version.trim().is_empty())
            .unwrap_or("unknown")
            .to_owned(),
        additional_browser_arguments: browser_args.to_owned(),
        environment: ENVIRONMENT_LABEL,
        browser: browser_section(browser_outcome),
        system: system_section(system_outcome),
    }
}

/// One in-flight graphics probe: the host facts plus the two CDP outcomes
/// as they arrive. The bridge answer goes out only when both are present.
#[derive(Debug)]
pub struct PendingGraphics {
    runtime_version: Option<String>,
    browser_args: String,
    received_at: std::time::Instant,
    system: Option<Result<String, String>>,
    browser: Option<Result<String, String>>,
}

impl PendingGraphics {
    pub fn new(
        runtime_version: Option<String>,
        browser_args: &str,
        system: Option<Result<String, String>>,
        browser: Option<Result<String, String>>,
    ) -> Self {
        Self {
            runtime_version,
            browser_args: browser_args.to_owned(),
            received_at: std::time::Instant::now(),
            system,
            browser,
        }
    }

    /// Record one probe completion. Late duplicates (a part arriving twice)
    /// keep the first answer, never overwrite.
    pub fn record(&mut self, method: CdpMethod, outcome: Result<String, String>) {
        match method {
            CdpMethod::SystemInfo => {
                if self.system.is_none() {
                    self.system = Some(outcome);
                }
            }
            CdpMethod::BrowserVersion => {
                if self.browser.is_none() {
                    self.browser = Some(outcome);
                }
            }
        }
    }

    pub fn is_complete(&self) -> bool {
        self.system.is_some() && self.browser.is_some()
    }

    /// Measured/unsupported/unavailable status of the system part for
    /// metrics logs (`pending` while the part has not arrived).
    pub fn system_status(&self) -> &'static str {
        match &self.system {
            None => "pending",
            Some(outcome) => system_section(outcome).status,
        }
    }

    /// Measured/unsupported/unavailable status of the browser part for
    /// metrics logs (`pending` while the part has not arrived).
    pub fn browser_status(&self) -> &'static str {
        match &self.browser {
            None => "pending",
            Some(outcome) => browser_section(outcome).status,
        }
    }

    pub fn elapsed_ms(&self) -> f64 {
        self.received_at.elapsed().as_secs_f64() * 1000.0
    }

    /// Build the report. Missing parts (should not happen once complete,
    /// but fail-safe) read as unavailable, never blocking the answer.
    pub fn build_response(&self) -> GraphicsInfoResponse {
        const MISSING: Result<String, String> = Err(String::new());
        assemble_report(
            self.runtime_version.as_deref(),
            &self.browser_args,
            self.browser.as_ref().unwrap_or(&MISSING),
            self.system.as_ref().unwrap_or(&MISSING),
        )
    }
}

/// Issue both fixed CDP probes on the event-loop thread and forward each
/// completion as `NativeEvent::GraphicsCdpPart`. Returns, per probe, whether
/// the call was issued (`false` = synchronous failure; the caller records
/// the part as failed immediately). The handlers own only bounded strings,
/// the request id, and an event-loop proxy — never COM objects.
#[cfg(target_os = "windows")]
pub fn issue_cdp_calls(
    core: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
    proxy: &tao::event_loop::EventLoopProxy<crate::NativeEvent>,
    request_id: &str,
) -> (bool, bool) {
    let system = issue_one_call(core, proxy, request_id, CdpMethod::SystemInfo);
    let browser = issue_one_call(core, proxy, request_id, CdpMethod::BrowserVersion);
    (system, browser)
}

/// Issue one fixed probe. A synchronous `Err` from the runtime means the
/// probe never started; the async completion (success or runtime error)
/// always forwards exactly one event.
#[cfg(target_os = "windows")]
fn issue_one_call(
    core: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
    proxy: &tao::event_loop::EventLoopProxy<crate::NativeEvent>,
    request_id: &str,
    method: CdpMethod,
) -> bool {
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::{HSTRING, PCWSTR};

    let proxy = proxy.clone();
    let request_id = request_id.to_owned();
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
        move |result: windows::core::Result<()>, json: String| {
            let outcome = match result {
                Ok(()) => Ok(truncate_chars(&json, MAX_CDP_JSON_CHARS)),
                Err(error) => Err(truncate_chars(&error.to_string(), MAX_CDP_ERROR_CHARS)),
            };
            let _ = proxy.send_event(crate::NativeEvent::GraphicsCdpPart {
                request_id,
                method,
                outcome,
            });
            Ok(())
        },
    ));
    let method_name = HSTRING::from(method.name());
    let params = HSTRING::from(method.params());
    unsafe {
        core.CallDevToolsProtocolMethod(
            PCWSTR::from_raw(method_name.as_ptr()),
            PCWSTR::from_raw(params.as_ptr()),
            &handler,
        )
        .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser_answer() -> String {
        serde_json::json!({
            "protocolVersion": "1.3",
            "product": "Chrome/155.0.4283.45",
            "revision": "@abcdef",
            "userAgent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
            "jsVersion": "15.5.1"
        })
        .to_string()
    }

    fn system_answer(command_line: &str) -> String {
        serde_json::json!({
            "gpu": {
                "devices": [
                    {
                        "vendorId": 4318,
                        "deviceId": 1234,
                        "vendorString": "Google Inc. (Intel)",
                        "deviceString": "ANGLE (Intel, Intel(R) Iris(R) Xe Graphics Direct3D11)",
                        "driverVersion": "31.0.101.4502"
                    },
                    {
                        "vendorString": "Google Inc. (NVIDIA)",
                        "deviceString": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3050 Laptop GPU Direct3D11)"
                    }
                ],
                "featureStatus": {
                    "2d_canvas": "hardware_accelerated",
                    "gpu_compositing": "hardware_accelerated",
                    "webgl": "hardware_accelerated",
                    "video_decode": "hardware_accelerated"
                }
            },
            "modelName": "Laptop",
            "commandLine": command_line
        })
        .to_string()
    }

    #[test]
    fn truncates_on_char_boundaries() {
        assert_eq!(truncate_chars("abc", 8), "abc");
        assert_eq!(truncate_chars("abcdef", 4), "abcd");
        // Multi-byte content truncates by character, never by byte.
        assert_eq!(truncate_chars("héllo wörld", 5), "héllo");
    }

    #[test]
    fn flag_matching_is_token_exact_never_substring() {
        // `--disable-gpu-sandbox` must NOT imply `--disable-gpu`, and a
        // `--use-angle=swiftshader` token must match `--use-angle`.
        let (flags, _, _) = parse_command_line("exe --disable-gpu-sandbox --use-angle=swiftshader");
        assert!(flags.contains(&"--disable-gpu-sandbox".to_owned()));
        assert!(!flags.contains(&"--disable-gpu".to_owned()));
        assert!(flags.contains(&"--use-angle".to_owned()));
        // Quoted values with spaces tokenize as one token.
        let tokens =
            split_command_line("exe --user-data-dir=\"C:\\Some Dir\\EBWebView\" --disable-gpu");
        assert_eq!(tokens.len(), 3);
        let (flags, _, _) = parse_command_line("exe --user-data-dir=\"C:\\Some Dir\\EBWebView\"");
        assert!(flags.is_empty());
    }

    #[test]
    fn browser_probe_parses_the_measured_shape() {
        let fields = parse_browser_version(&browser_answer()).expect("must parse");
        assert_eq!(fields.product.as_deref(), Some("Chrome/155.0.4283.45"));
        assert_eq!(fields.js_version.as_deref(), Some("15.5.1"));
        assert_eq!(fields.protocol_version.as_deref(), Some("1.3"));
    }

    #[test]
    fn browser_probe_accepts_a_result_envelope() {
        let wrapped = serde_json::json!({ "result": serde_json::from_str::<serde_json::Value>(&browser_answer()).unwrap() }).to_string();
        let fields = parse_browser_version(&wrapped).expect("envelope must parse");
        assert_eq!(fields.product.as_deref(), Some("Chrome/155.0.4283.45"));
    }

    #[test]
    fn browser_method_not_found_is_unsupported() {
        let answer =
            serde_json::json!({ "error": { "code": -32601, "message": "Method not found" } })
                .to_string();
        let failure = parse_browser_version(&answer).expect_err("must classify");
        assert_eq!(failure.status, STATUS_UNSUPPORTED);
    }

    #[test]
    fn browser_runtime_error_is_unavailable_not_unsupported() {
        let answer = serde_json::json!({ "error": { "code": -32000, "message": "Broken pipe" } })
            .to_string();
        let failure = parse_browser_version(&answer).expect_err("must classify");
        assert_eq!(failure.status, STATUS_UNAVAILABLE);
    }

    #[test]
    fn browser_garbage_is_unavailable() {
        let failure = parse_browser_version("not json").expect_err("must fail");
        assert_eq!(failure.status, STATUS_UNAVAILABLE);
        let failure = parse_browser_version("[1,2]").expect_err("must fail");
        assert_eq!(failure.status, STATUS_UNAVAILABLE);
    }

    #[test]
    fn system_probe_parses_devices_features_and_flags() {
        let command_line = "msedgewebview2.exe --disable-features=msWebOOUI,msPdfOOUI --enable-features=msEdgeFluentOverlayScrollbar --disable-gpu-sandbox";
        let fields = parse_system_info(&system_answer(command_line)).expect("must parse");
        assert_eq!(fields.devices.len(), 2);
        // Numeric ids stringify exactly; strings stay verbatim and bounded.
        assert_eq!(fields.devices[0].vendor_id.as_deref(), Some("4318"));
        assert_eq!(fields.devices[0].device_id.as_deref(), Some("1234"));
        assert_eq!(
            fields.devices[0].driver_version.as_deref(),
            Some("31.0.101.4502")
        );
        // Absent fields are None, never invented.
        assert_eq!(fields.devices[1].vendor_id, None);
        assert_eq!(fields.devices[1].driver_version, None);
        assert_eq!(fields.features.len(), 4);
        assert!(fields
            .features
            .iter()
            .any(|entry| entry.name == "webgl" && entry.status == "hardware_accelerated"));
        assert!(fields
            .flags_present
            .contains(&"--disable-gpu-sandbox".to_owned()));
        assert!(fields
            .flags_present
            .contains(&"--disable-features".to_owned()));
        assert!(!fields.flags_present.contains(&"--disable-gpu".to_owned()));
        assert_eq!(
            fields.disable_features.as_deref(),
            Some("msWebOOUI,msPdfOOUI")
        );
        assert_eq!(
            fields.enable_features.as_deref(),
            Some("msEdgeFluentOverlayScrollbar")
        );
    }

    #[test]
    fn system_probe_caps_devices_and_feature_entries() {
        let mut devices = Vec::new();
        for index in 0..(MAX_GPU_DEVICES + 5) {
            devices.push(serde_json::json!({ "deviceString": format!("gpu-{index}") }));
        }
        let mut features = serde_json::Map::new();
        for index in 0..(MAX_FEATURE_ENTRIES + 5) {
            features.insert(format!("feature-{index}"), serde_json::json!("ok"));
        }
        let answer = serde_json::json!({
            "gpu": { "devices": devices, "featureStatus": features },
            "commandLine": "exe --disable-gpu"
        })
        .to_string();
        let fields = parse_system_info(&answer).expect("must parse");
        assert_eq!(fields.devices.len(), MAX_GPU_DEVICES);
        assert_eq!(fields.features.len(), MAX_FEATURE_ENTRIES);
    }

    #[test]
    fn system_report_never_carries_paths() {
        let command_line = "C:\\Program Files (x86)\\Microsoft\\EdgeWebView\\Application\\155.0.4283.45\\msedgewebview2.exe --user-data-dir=\"C:\\Users\\Test\\AppData\\Local\\Remcord\\EBWebView\" --disable-features=msWebOOUI";
        let report = assemble_report(
            Some("155.0.4283.45"),
            "--disable-features=msWebOOUI",
            &Ok(browser_answer()),
            &Ok(system_answer(command_line)),
        );
        let text = serde_json::to_string(&report).expect("must serialize");
        for forbidden in [
            "C:\\",
            "EBWebView",
            "AppData",
            "Local\\",
            "Users\\Test",
            "commandLine",
            "user-data-dir",
        ] {
            assert!(
                !text.contains(forbidden),
                "report must not carry {forbidden:?}: {text}"
            );
        }
        assert_eq!(report.system.status, STATUS_MEASURED);
        assert_eq!(report.browser.status, STATUS_MEASURED);
    }

    #[test]
    fn failed_probes_report_explicitly_without_invented_hardware() {
        let report = assemble_report(
            Some("155.0.4283.45"),
            "--disable-features=msWebOOUI",
            &Err("the probe could not start".to_owned()),
            &Ok(
                serde_json::json!({ "error": { "code": -32601, "message": "Not found" } })
                    .to_string(),
            ),
        );
        assert_eq!(report.browser.status, STATUS_UNAVAILABLE);
        assert_eq!(report.system.status, STATUS_UNSUPPORTED);
        assert!(report.system.devices.is_empty());
        assert!(report.system.feature_status.is_empty());
        assert!(report.browser.product.is_none());
        let text = serde_json::to_string(&report).expect("must serialize");
        assert!(text.contains(STATUS_UNAVAILABLE));
        assert!(text.contains(STATUS_UNSUPPORTED));
    }

    #[test]
    fn absent_runtime_is_unknown_never_guessed() {
        let report = assemble_report(
            None,
            "--disable-features=x",
            &Err("x".to_owned()),
            &Err("y".to_owned()),
        );
        assert_eq!(report.webview2_runtime, "unknown");
        let blank = assemble_report(
            Some("   "),
            "--disable-features=x",
            &Err("x".to_owned()),
            &Err("y".to_owned()),
        );
        assert_eq!(blank.webview2_runtime, "unknown");
    }

    #[test]
    fn pending_assembly_completes_only_when_both_parts_arrive() {
        let mut pending = PendingGraphics::new(Some("155.0.0".to_owned()), "args", None, None);
        assert!(!pending.is_complete());
        pending.record(CdpMethod::SystemInfo, Ok(system_answer("exe")));
        assert!(!pending.is_complete());
        pending.record(CdpMethod::BrowserVersion, Ok(browser_answer()));
        assert!(pending.is_complete());
        // Late duplicates keep the first answer.
        pending.record(CdpMethod::SystemInfo, Err("late".to_owned()));
        let report = pending.build_response();
        assert_eq!(report.system.status, STATUS_MEASURED);
        assert_eq!(report.browser.status, STATUS_MEASURED);
    }

    #[test]
    fn incomplete_pending_still_builds_a_fail_safe_report() {
        let pending = PendingGraphics::new(None, "args", None, None);
        let report = pending.build_response();
        assert_eq!(report.browser.status, STATUS_UNAVAILABLE);
        assert_eq!(report.system.status, STATUS_UNAVAILABLE);
        assert_eq!(report.webview2_runtime, "unknown");
    }

    #[test]
    fn error_text_is_bounded() {
        let long = "e".repeat(MAX_CDP_ERROR_CHARS + 100);
        let report = assemble_report(Some("1.0"), "args", &Err(long), &Err("short".to_owned()));
        let browser_error = report.browser.error.expect("must carry the reason");
        assert!(browser_error.chars().count() <= MAX_CDP_ERROR_CHARS);
    }

    #[test]
    fn response_shape_has_fixed_keys() {
        let value = serde_json::to_value(assemble_report(
            Some("155.0.4283.45"),
            "--disable-features=msWebOOUI",
            &Ok(browser_answer()),
            &Ok(system_answer("exe --disable-gpu")),
        ))
        .expect("must serialize");
        for key in [
            "webview2_runtime",
            "additional_browser_arguments",
            "environment",
            "browser",
            "system",
        ] {
            assert!(value.get(key).is_some(), "missing top-level {key}");
        }
        assert_eq!(value["browser"]["method"], "Browser.getVersion");
        assert_eq!(value["system"]["method"], "System.getInfo");
        assert_eq!(value["system"]["devices"].as_array().unwrap().len(), 2);
    }
}

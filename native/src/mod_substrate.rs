//! Reusable native client-mod substrate.
//!
//! This module owns every behavior that is identical for all client mods:
//! fixed-source bundle retrieval, validation, disk and in-memory caching,
//! per-document bootstrap guards, page-world execution orchestration, CSS
//! lifecycle scripts, typed failure stages, and per-mod metrics.
//!
//! The module deliberately contains **no** mod-specific data. Each mod is
//! described by a [`ModDescriptor`] supplied by the caller (see the Vencord
//! descriptor in `main.rs`). Adding Shelter or Equicord means registering a
//! new descriptor and a thin renderer adapter; this file must not change.
//!
//! The substrate never executes renderer-supplied source. It only executes
//! the fixed bundle bytes it retrieved itself through the host allowlist.

use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

/// Typed contract describing one client mod to the shared substrate.
///
/// Every field is host-controlled. The renderer can request the capability
/// named here but cannot alter any of these values.
#[derive(Debug, Clone, Copy)]
pub struct ModDescriptor {
    /// Stable mod identifier, e.g. `"vencord"`. Keys per-document state.
    pub id: &'static str,
    /// Capability name exposed in the boot snapshot, e.g. `"mod.vencord.get_bundle"`.
    pub capability: &'static str,
    /// Fixed HTTPS URL of the JavaScript bundle release asset.
    pub js_url: &'static str,
    /// Fixed HTTPS URL of the CSS bundle release asset.
    pub css_url: &'static str,
    /// Fixed cache file name for the JavaScript bundle inside the DEV profile.
    pub js_file: &'static str,
    /// Fixed cache file name for the CSS bundle inside the DEV profile.
    pub css_file: &'static str,
    /// Exact HTTPS hosts allowed for bundle download and redirects.
    pub allowed_hosts: &'static [&'static str],
    /// Maximum accepted JavaScript bundle size in bytes.
    pub max_js_bytes: usize,
    /// Maximum accepted CSS bundle size in bytes.
    pub max_css_bytes: usize,
    /// Required first-line header prefix proving bundle identity, e.g. `"// Vencord "`.
    pub version_header_prefix: &'static str,
    /// Deterministic `<style>` element id managed by the substrate.
    pub style_element_id: &'static str,
    /// Page-world global object proving successful execution, e.g. `"Vencord"`.
    pub page_global: &'static str,
    /// Success marker reported after the page-world global is observed.
    pub success_marker: &'static str,
    /// Storage key inspected for diagnostics, e.g. `"VencordSettings"`.
    pub settings_key: &'static str,
    /// Renderer event type carrying the CSS injection result.
    pub css_result_event: &'static str,
    /// Document-local DOM event carrying the page-world bootstrap verdict.
    pub bootstrap_dom_event: &'static str,
    /// Page-world global holding the storage context snapshot.
    pub storage_context_global: &'static str,
    /// Log tag used for host-side diagnostics, e.g. `"Mod:Vencord"`.
    pub log_tag: &'static str,
}

/// A validated bundle owned by the substrate cache.
#[derive(Debug, Clone)]
pub struct ModBundle {
    pub js: String,
    pub css: String,
    pub version: String,
    pub js_bytes: usize,
    pub css_bytes: usize,
}

/// In-memory bundle cache keyed by mod id.
///
/// Disk remains the durable cache inside the isolated DEV profile; this map
/// only avoids re-reading validated bundles across documents of one run.
#[derive(Debug, Default)]
pub struct ModCache {
    entries: HashMap<&'static str, Arc<ModBundle>>,
}

impl ModCache {
    pub fn get(&self, mod_id: &'static str) -> Option<Arc<ModBundle>> {
        self.entries.get(mod_id).cloned()
    }

    pub fn insert(&mut self, mod_id: &'static str, bundle: ModBundle) {
        self.entries.insert(mod_id, Arc::new(bundle));
    }
}

/// Per-document bootstrap state for one mod.
#[derive(Debug, Default)]
struct SingleModState {
    started: bool,
    execution_count: u8,
}

/// Document generation plus per-mod bootstrap guards.
///
/// A new document generation clears every mod's guards so each mod bootstraps
/// exactly once per document and stale results are discarded.
#[derive(Debug, Default)]
pub struct DocumentModLifecycle {
    generation: u64,
    mods: HashMap<&'static str, SingleModState>,
}

impl DocumentModLifecycle {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn begin_document(&mut self) {
        self.generation += 1;
        self.mods.clear();
    }

    /// Returns `true` for the first bootstrap request of this mod in this
    /// document; `false` marks a duplicate that must be rejected.
    pub fn begin_mod(&mut self, mod_id: &'static str) -> bool {
        let state = self.mods.entry(mod_id).or_default();
        if state.started {
            return false;
        }
        state.started = true;
        true
    }

    /// Returns `true` for the first successful execution of this mod in this
    /// document; `false` marks a duplicate execution.
    pub fn record_mod_execution(&mut self, mod_id: &'static str) -> bool {
        let state = self.mods.entry(mod_id).or_default();
        state.execution_count += 1;
        state.execution_count == 1
    }

    pub fn mod_execution_count(&self, mod_id: &'static str) -> u8 {
        self.mods
            .get(mod_id)
            .map(|state| state.execution_count)
            .unwrap_or(0)
    }
}

/// Per-mod runtime counters reported at shutdown.
#[derive(Debug, Default)]
pub struct ModRuntimeMetrics {
    pub capability_requests: u64,
    pub successful_bootstraps: u64,
    pub duplicate_bootstraps: u64,
}

/// Host-side metrics keyed by mod id.
#[derive(Debug, Default)]
pub struct ModHostMetrics {
    mods: HashMap<&'static str, ModRuntimeMetrics>,
}

impl ModHostMetrics {
    pub fn record_request(&mut self, mod_id: &'static str) {
        self.mods.entry(mod_id).or_default().capability_requests += 1;
    }

    pub fn record_bootstrap(&mut self, mod_id: &'static str) {
        self.mods.entry(mod_id).or_default().successful_bootstraps += 1;
    }

    pub fn record_duplicate(&mut self, mod_id: &'static str) {
        self.mods.entry(mod_id).or_default().duplicate_bootstraps += 1;
    }

    pub fn snapshot(&self, mod_id: &'static str) -> Option<&ModRuntimeMetrics> {
        self.mods.get(mod_id)
    }
}

/// Typed failure stages shared by every mod pipeline.
pub mod stage {
    pub const BUNDLE_RETRIEVAL: &str = "bundle_retrieval";
    pub const ADAPTER_INITIALIZATION: &str = "adapter_initialization";
    pub const SCRIPT_EXECUTION: &str = "script_execution";
    pub const CSS_INJECTION: &str = "css_injection";
    pub const DUPLICATE_BOOTSTRAP: &str = "duplicate_bootstrap";
}

pub fn is_allowed_mod_download_url(descriptor: &ModDescriptor, url: &str) -> bool {
    url.parse::<wry::http::Uri>().is_ok_and(|uri| {
        uri.scheme_str() == Some("https")
            && uri.authority().is_some_and(|authority| {
                descriptor
                    .allowed_hosts
                    .iter()
                    .any(|host| authority.host().eq_ignore_ascii_case(host))
            })
    })
}

fn download_mod_asset(
    agent: &ureq::Agent,
    descriptor: &ModDescriptor,
    url: &str,
    max_bytes: usize,
    kind: &str,
) -> Result<Vec<u8>, String> {
    let mut current_url = url.to_owned();
    for _ in 0..=3 {
        if !is_allowed_mod_download_url(descriptor, &current_url) {
            return Err(format!(
                "[{}] {kind} redirect used an unapproved HTTPS host",
                descriptor.id
            ));
        }

        let response = match agent.get(&current_url).call() {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(error) => {
                return Err(format!(
                    "[{}] {kind} request failed: {error}",
                    descriptor.id
                ));
            }
        };
        let status = response.status();
        if (200..300).contains(&status) {
            let mut bytes = Vec::new();
            response
                .into_reader()
                .take(max_bytes as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| format!("[{}] could not read {kind}: {error}", descriptor.id))?;
            if bytes.is_empty() || bytes.len() > max_bytes {
                return Err(format!(
                    "[{}] {kind} was empty or exceeded its size limit",
                    descriptor.id
                ));
            }
            return Ok(bytes);
        }

        if (300..400).contains(&status) {
            let location = response.header("Location").ok_or_else(|| {
                format!("[{}] {kind} redirect had no Location header", descriptor.id)
            })?;
            current_url = location.to_owned();
            continue;
        }
        return Err(format!(
            "[{}] {kind} request returned HTTP {status}",
            descriptor.id
        ));
    }
    Err(format!(
        "[{}] {kind} exceeded the redirect limit",
        descriptor.id
    ))
}

fn cached_or_downloaded_asset(
    agent: &ureq::Agent,
    descriptor: &ModDescriptor,
    profile: &Path,
    file_name: &str,
    url: &str,
    max_bytes: usize,
    kind: &str,
) -> Result<Vec<u8>, String> {
    let path = profile.join(file_name);
    if path.exists() {
        let bytes = fs::read(&path)
            .map_err(|error| format!("[{}] could not read {file_name}: {error}", descriptor.id))?;
        if bytes.is_empty() || bytes.len() > max_bytes {
            return Err(format!(
                "[{}] {file_name} cache was empty or exceeded its size limit",
                descriptor.id
            ));
        }
        return Ok(bytes);
    }

    let bytes = download_mod_asset(agent, descriptor, url, max_bytes, kind)?;
    let temp_path = profile.join(format!("{file_name}.tmp"));
    fs::write(&temp_path, &bytes).map_err(|error| {
        format!(
            "[{}] could not cache {file_name} in the DEV profile: {error}",
            descriptor.id
        )
    })?;
    fs::rename(&temp_path, &path).map_err(|error| {
        format!(
            "[{}] could not finalize {file_name} in the DEV profile: {error}",
            descriptor.id
        )
    })?;
    Ok(bytes)
}

/// Retrieves (disk cache first, fixed-source download second) and validates
/// the bundle described by `descriptor`.
pub fn load_mod_bundle(
    descriptor: &ModDescriptor,
    profile: std::path::PathBuf,
) -> Result<ModBundle, String> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(30))
        .build();
    let js = cached_or_downloaded_asset(
        &agent,
        descriptor,
        &profile,
        descriptor.js_file,
        descriptor.js_url,
        descriptor.max_js_bytes,
        "JS bundle",
    )?;
    let css = cached_or_downloaded_asset(
        &agent,
        descriptor,
        &profile,
        descriptor.css_file,
        descriptor.css_url,
        descriptor.max_css_bytes,
        "CSS bundle",
    )?;
    validate_mod_bundle(descriptor, js, css)
}

/// Validates raw bundle bytes against the descriptor contract.
pub fn validate_mod_bundle(
    descriptor: &ModDescriptor,
    js: Vec<u8>,
    css: Vec<u8>,
) -> Result<ModBundle, String> {
    if js.is_empty() || js.len() > descriptor.max_js_bytes {
        return Err(format!(
            "[{}] JS bundle was empty or exceeded its size limit",
            descriptor.id
        ));
    }
    if css.is_empty() || css.len() > descriptor.max_css_bytes {
        return Err(format!(
            "[{}] CSS bundle was empty or exceeded its size limit",
            descriptor.id
        ));
    }
    let js = String::from_utf8(js)
        .map_err(|error| format!("[{}] JS bundle is not UTF-8: {error}", descriptor.id))?;
    let css = String::from_utf8(css)
        .map_err(|error| format!("[{}] CSS bundle is not UTF-8: {error}", descriptor.id))?;
    let version = js
        .lines()
        .next()
        .and_then(|line| line.strip_prefix(descriptor.version_header_prefix))
        .filter(|version| !version.is_empty() && version.len() <= 64)
        .ok_or_else(|| {
            format!(
                "[{}] bundle did not contain the expected build header",
                descriptor.id
            )
        })?
        .to_owned();

    Ok(ModBundle {
        js_bytes: js.len(),
        css_bytes: css.len(),
        js,
        css,
        version,
    })
}

/// Builds the fixed CSS insertion script for one bundle.
///
/// The script creates or reuses exactly one `<style>` element with the
/// descriptor's stable id and reports a typed result; it never duplicates
/// the element and never evaluates renderer input.
pub fn mod_css_script(
    descriptor: &ModDescriptor,
    request_id: &str,
    css: &str,
) -> Result<String, serde_json::Error> {
    let request_id = serde_json::to_string(request_id)?;
    let css = serde_json::to_string(css)?;
    let style_id = serde_json::to_string(descriptor.style_element_id)?;
    let result_event = descriptor.css_result_event;
    Ok(format!(
        r#"(() => {{
  const requestId = {request_id};
  const report = (ok, error = null) => window.ipc.postMessage(JSON.stringify({{
    version: 1,
    event: {{ type: "{result_event}", payload: {{ request_id: requestId, ok, error }} }}
  }}));
  const inject = () => {{
  try {{
    const root = document.head;
    if (!root) return false;
    let style = document.getElementById({style_id});
    if (!style) {{
      style = document.createElement("style");
      style.id = {style_id};
    }}
    style.textContent = {css};
    root.appendChild(style);
    if (!style.sheet) throw new Error("Mod stylesheet was rejected by the document");
    report(true);
    return true;
  }} catch (error) {{
    report(false, String(error?.message || error).slice(0, 2048));
    return true;
  }}
  }};
  if (!inject()) document.addEventListener("DOMContentLoaded", inject, {{ once: true }});
}})();"#
    ))
}

/// Builds the storage-context snapshot executed immediately before the mod
/// bundle. It records only whether `localStorage` access works and whether
/// the mod's settings key holds valid JSON.
pub fn mod_execution_context_script(descriptor: &ModDescriptor) -> String {
    let settings_key = serde_json::to_string(descriptor.settings_key)
        .expect("a static settings key must serialize to JSON");
    let context_global = descriptor.storage_context_global;
    format!(
        r#"(() => {{
  let localStorageApi = "blocked";
  let settingsState = "storage_unavailable";
  try {{
    const storage = window.localStorage;
    localStorageApi = typeof storage?.getItem === "function" ? "available" : "missing";
    if (localStorageApi === "available") {{
      try {{
        const settings = storage.getItem({settings_key});
        if (settings === null) {{
          settingsState = "missing";
        }} else {{
          JSON.parse(settings);
          settingsState = "valid_json";
        }}
      }} catch (error) {{
        settingsState = error instanceof SyntaxError ? "invalid_json" : "read_error";
      }}
    }}
  }} catch {{}}
  Object.defineProperty(globalThis, "{context_global}", {{
    configurable: false,
    enumerable: false,
    value: Object.freeze({{ localStorageApi, settingsState }})
  }});
}})();
"#
    )
}

/// Builds the post-execution verifier. It checks the descriptor's page-world
/// global and dispatches the bootstrap verdict on the document-local DOM
/// event; it never evaluates renderer input.
pub fn mod_marker_script(
    descriptor: &ModDescriptor,
    request_id: &str,
    execution_ms: f64,
) -> Result<String, serde_json::Error> {
    let request_id = serde_json::to_string(request_id)?;
    let marker = serde_json::to_string(descriptor.success_marker)?;
    let dom_event = descriptor.bootstrap_dom_event;
    let context_global = descriptor.storage_context_global;
    let page_global = descriptor.page_global;
    Ok(format!(
        r#"(() => {{
  const requestId = {request_id};
  const marker = {marker};
  const storageContext = globalThis.{context_global} || {{}};
  const localStorageApi = storageContext.localStorageApi || "blocked";
  const settingsState = storageContext.settingsState || "storage_unavailable";
  let result;
  try {{
    if (!globalThis.{page_global} || typeof globalThis.{page_global} !== "object") {{
      throw new Error("The mod global was not initialized by the bundle");
    }}
    result = {{
      request_id: requestId,
      ok: true,
      marker,
      execution_ms: {execution_ms},
      local_storage_api: localStorageApi,
      settings_state: settingsState,
      error: null
    }};
  }} catch (error) {{
    result = {{
      request_id: requestId,
      ok: false,
      marker: "",
      execution_ms: {execution_ms},
      local_storage_api: localStorageApi,
      settings_state: settingsState,
      error: String(error?.message || error).slice(0, 2048)
    }};
  }}
  window.dispatchEvent(new CustomEvent("{dom_event}", {{ detail: result }}));
}})();"#
    ))
}

/// Builds a managed-CSS insertion script for CSS-only consumers (themes).
///
/// Unlike [`mod_css_script`], this entry point is not tied to a
/// [`ModDescriptor`] and carries no request correlation or typed result
/// event: theme injection is a host push (the renderer never requests theme
/// CSS), so there is no capability round trip to correlate. The script
/// creates or reuses exactly one `<style>` element with the host-supplied id,
/// sets its text, and appends it to `<head>` (falling back to
/// `DOMContentLoaded` when the head is not yet present). Both arguments are
/// host-controlled and JSON-escaped; the script never evaluates renderer
/// input and exposes no eval surface.
pub fn managed_css_script(style_id: &str, css: &str) -> Result<String, serde_json::Error> {
    if style_id.is_empty() || style_id.len() > 256 {
        return Err(serde::de::Error::custom(
            "managed style id must be a non-empty string of at most 256 bytes",
        ));
    }
    let style_id = serde_json::to_string(style_id)?;
    let css = serde_json::to_string(css)?;
    Ok(format!(
        r#"(() => {{
  const inject = () => {{
  try {{
    const root = document.head;
    if (!root) return false;
    let style = document.getElementById({style_id});
    if (!style) {{
      style = document.createElement("style");
      style.id = {style_id};
    }}
    style.textContent = {css};
    root.appendChild(style);
    return true;
  }} catch (error) {{
    console.error("[Legcord native][themes] managed CSS injection failed", error);
    return true;
  }}
  }};
  if (!inject()) document.addEventListener("DOMContentLoaded", inject, {{ once: true }});
}})();"#
    ))
}

/// Builds the removal script for one managed `<style>` element. A missing
/// element is not an error: removal is idempotent so disable/reset paths can
/// run unconditionally.
pub fn managed_css_remove_script(style_id: &str) -> Result<String, serde_json::Error> {
    if style_id.is_empty() || style_id.len() > 256 {
        return Err(serde::de::Error::custom(
            "managed style id must be a non-empty string of at most 256 bytes",
        ));
    }
    let style_id = serde_json::to_string(style_id)?;
    Ok(format!(
        r#"(() => {{
  try {{
    const style = document.getElementById({style_id});
    if (style) style.remove();
  }} catch (error) {{
    console.error("[Legcord native][themes] managed CSS removal failed", error);
  }}
}})();"#
    ))
}

/// Timing and identity state for one in-flight mod bootstrap.
pub struct PendingModBootstrap {
    pub mod_id: &'static str,
    pub generation: u64,
    pub received_at: Instant,
    pub retrieval_ms: f64,
    pub bundle_version: String,
    pub bundle_bytes: usize,
    pub css_bytes: usize,
    pub execution_ms: f64,
    pub css_started_at: Instant,
    pub css_injection_ms: f64,
}

#[cfg(test)]
mod tests {
    use super::{
        is_allowed_mod_download_url, managed_css_remove_script, managed_css_script, mod_css_script,
        mod_execution_context_script, mod_marker_script, validate_mod_bundle, DocumentModLifecycle,
        ModCache, ModDescriptor, ModHostMetrics,
    };

    /// Synthetic adapter contract proving the substrate is not Vencord-specific.
    pub fn test_descriptor() -> ModDescriptor {
        ModDescriptor {
            id: "testmod",
            capability: "mod.test.get_bundle",
            js_url: "https://github.com/example/TestMod/releases/download/devbuild/browser.js",
            css_url: "https://github.com/example/TestMod/releases/download/devbuild/browser.css",
            js_file: "testmod.js",
            css_file: "testmod.css",
            allowed_hosts: &["github.com", "release-assets.githubusercontent.com"],
            max_js_bytes: 1024 * 1024,
            max_css_bytes: 256 * 1024,
            version_header_prefix: "// TestMod ",
            style_element_id: "legcord-native-testmod-css",
            page_global: "TestMod",
            success_marker: "testmod-bootstrap-executed",
            settings_key: "TestModSettings",
            css_result_event: "testmod_css_result",
            bootstrap_dom_event: "legcord-native:testmod-bootstrap",
            storage_context_global: "__legcordNativeTestModStorageAtExecution",
            log_tag: "Mod:TestMod",
        }
    }

    #[test]
    fn synthetic_adapter_contract_validates_bundle_identity() {
        let descriptor = test_descriptor();
        assert_eq!(descriptor.capability, "mod.test.get_bundle");
        let bundle = validate_mod_bundle(
            &descriptor,
            b"// TestMod 1.0.0\nvar TestMod = {};\n".to_vec(),
            b"/* test CSS */".to_vec(),
        )
        .unwrap();
        assert_eq!(bundle.version, "1.0.0");
        assert_eq!(bundle.js_bytes, bundle.js.len());
        assert_eq!(bundle.css_bytes, bundle.css.len());
        assert!(validate_mod_bundle(
            &descriptor,
            b"// Vencord b52ed36\nvar Vencord = {};\n".to_vec(),
            b"/*css*/".to_vec(),
        )
        .is_err());
    }

    #[test]
    fn bundle_cache_records_a_miss_then_a_hit() {
        let descriptor = test_descriptor();
        let mut cache = ModCache::default();
        assert!(cache.get(descriptor.id).is_none());

        let bundle = validate_mod_bundle(
            &descriptor,
            b"// TestMod 1.0.0\nvar TestMod = {};\n".to_vec(),
            b"/*css*/".to_vec(),
        )
        .unwrap();
        cache.insert(descriptor.id, bundle);
        assert!(cache.get(descriptor.id).is_some());
        let hit = cache.get(descriptor.id).unwrap();
        assert_eq!(hit.version, "1.0.0");
        assert_eq!(hit.js_bytes, hit.js.len());
    }

    #[test]
    fn document_lifecycle_allows_one_bootstrap_per_mod_and_resets_on_navigation() {
        let descriptor = test_descriptor();
        let mut lifecycle = DocumentModLifecycle::default();
        lifecycle.begin_document();
        assert_eq!(lifecycle.generation(), 1);
        assert!(lifecycle.begin_mod(descriptor.id));
        assert!(!lifecycle.begin_mod(descriptor.id));
        assert!(lifecycle.record_mod_execution(descriptor.id));
        assert!(!lifecycle.record_mod_execution(descriptor.id));
        assert_eq!(lifecycle.mod_execution_count(descriptor.id), 2);

        lifecycle.begin_document();
        assert_eq!(lifecycle.generation(), 2);
        assert!(lifecycle.begin_mod(descriptor.id));
        assert_eq!(lifecycle.mod_execution_count(descriptor.id), 0);
    }

    #[test]
    fn document_lifecycle_tracks_mods_independently() {
        let mut lifecycle = DocumentModLifecycle::default();
        lifecycle.begin_document();
        assert!(lifecycle.begin_mod("vencord"));
        assert!(lifecycle.begin_mod("testmod"));
        assert!(!lifecycle.begin_mod("vencord"));
        assert!(lifecycle.record_mod_execution("testmod"));
        assert_eq!(lifecycle.mod_execution_count("vencord"), 0);
        assert_eq!(lifecycle.mod_execution_count("testmod"), 1);
    }

    #[test]
    fn download_allowlist_rejects_unapproved_hosts() {
        let descriptor = test_descriptor();
        assert!(is_allowed_mod_download_url(
            &descriptor,
            "https://github.com/example/TestMod/releases/download/devbuild/browser.js"
        ));
        assert!(is_allowed_mod_download_url(
            &descriptor,
            "https://release-assets.githubusercontent.com/asset/browser.js"
        ));
        assert!(!is_allowed_mod_download_url(
            &descriptor,
            "http://github.com/example/browser.js"
        ));
        assert!(!is_allowed_mod_download_url(
            &descriptor,
            "https://github.com.attacker.invalid/browser.js"
        ));
        assert!(!is_allowed_mod_download_url(
            &descriptor,
            "https://badges.example.invalid/badges.json"
        ));
    }

    #[test]
    fn generated_scripts_use_only_descriptor_values_and_no_eval_surface() {
        let descriptor = test_descriptor();
        let css = mod_css_script(&descriptor, "request-1", r#"body::after{content:"ok"}"#).unwrap();
        assert!(css.contains("legcord-native-testmod-css"));
        assert!(css.contains("testmod_css_result"));
        assert!(css.contains(r#"body::after{content:\"ok\"}"#));
        assert!(css.contains("getElementById"));
        // One stable element reused, never duplicated: exactly one creation site.
        assert_eq!(css.matches("createElement(\"style\")").count(), 1);

        let context = mod_execution_context_script(&descriptor);
        assert!(context.contains("TestModSettings"));
        assert!(context.contains("__legcordNativeTestModStorageAtExecution"));

        let marker = mod_marker_script(&descriptor, "request-1", 1.5).unwrap();
        assert!(marker.contains("globalThis.TestMod"));
        assert!(marker.contains("testmod-bootstrap-executed"));
        assert!(marker.contains("legcord-native:testmod-bootstrap"));
        assert!(!marker.contains("eval("));
        assert!(!marker.contains("new Function"));
        assert!(!css.contains("eval("));
        assert!(!css.contains("new Function"));
    }

    #[test]
    fn managed_css_scripts_use_a_stable_identity_without_an_eval_surface() {
        let inject =
            managed_css_script("legcord-native-theme-demo", r#"body::after{content:"ok"}"#)
                .unwrap();
        assert!(inject.contains("legcord-native-theme-demo"));
        assert!(inject.contains(r#"body::after{content:\"ok\"}"#));
        assert!(inject.contains("getElementById"));
        // One stable element reused, never duplicated: exactly one creation
        // site plus the reuse lookup.
        assert_eq!(inject.matches("createElement(\"style\")").count(), 1);
        assert_eq!(inject.matches("getElementById").count(), 1);
        assert!(inject.contains("DOMContentLoaded"));
        assert!(!inject.contains("eval("));
        assert!(!inject.contains("new Function"));
        // No request correlation: a host push has no renderer request id.
        assert!(!inject.contains("requestId"));

        let remove = managed_css_remove_script("legcord-native-theme-demo").unwrap();
        assert!(remove.contains("legcord-native-theme-demo"));
        assert!(remove.contains(".remove()"));
        assert!(!remove.contains("createElement"));
        assert!(!remove.contains("eval("));

        assert!(managed_css_script("", "body{}").is_err());
        assert!(managed_css_remove_script("",).is_err());
        assert!(managed_css_script(&"x".repeat(257), "body{}").is_err());
    }

    #[test]
    fn host_metrics_count_per_mod_without_cross_talk() {
        let mut metrics = ModHostMetrics::default();
        metrics.record_request("vencord");
        metrics.record_request("vencord");
        metrics.record_bootstrap("vencord");
        metrics.record_duplicate("testmod");
        let vencord = metrics.snapshot("vencord").unwrap();
        assert_eq!(vencord.capability_requests, 2);
        assert_eq!(vencord.successful_bootstraps, 1);
        assert_eq!(vencord.duplicate_bootstraps, 0);
        let testmod = metrics.snapshot("testmod").unwrap();
        assert_eq!(testmod.duplicate_bootstraps, 1);
        assert!(metrics.snapshot("shelter").is_none());
    }
}

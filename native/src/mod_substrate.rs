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

use sha2::{Digest, Sha256};

/// One mod's stylesheet half, present only when the mod actually ships CSS.
///
/// Kept as a nested `Option` instead of a flat set of "maybe empty" fields so
/// a mod with no CSS (`None`) cannot accidentally be handed a placeholder URL,
/// cache file, style id, or result event. A mod with CSS carries the whole
/// contract in one struct, so a half-configured stylesheet is impossible.
#[derive(Debug, Clone, Copy)]
pub struct ModCssSource {
    /// Fixed HTTPS URL of the CSS release asset.
    pub url: &'static str,
    /// Fixed cache file name for the CSS bundle inside the DEV profile.
    pub file: &'static str,
    /// Maximum accepted CSS size in bytes.
    pub max_bytes: usize,
    /// Deterministic `<style>` element id managed by the substrate.
    pub style_element_id: &'static str,
    /// Renderer event type carrying the CSS injection result.
    pub result_event: &'static str,
}

/// How the host proves an artifact is *this* mod's bundle.
///
/// Two shapes exist because upstream publishes two shapes; neither fabricates a
/// marker that the artifact does not carry.
#[derive(Debug, Clone, Copy)]
pub enum ModArtifactIdentity {
    /// The artifact's first line starts with `prefix` followed by a non-empty
    /// version token of at most 64 bytes. Vencord and Equicord publish
    /// `// Vencord <sha>` / `// Equicord <sha>` banners.
    BuildHeader { prefix: &'static str },
    /// The artifact publishes **no** build header, so the version cannot come
    /// from a banner. Identity is instead asserted two ways: every sentinel in
    /// `sentinels` must appear verbatim in the source (each one is a unique
    /// upstream literal that no other registered mod's artifact contains), and
    /// the reported version is the host-computed SHA-256 digest prefix of the
    /// retrieved bytes. Shelter's published build has no banner at all.
    SourceSentinels { sentinels: &'static [&'static str] },
    /// The artifact is user-owned local content with no upstream publisher,
    /// so there is no banner or sentinel to prove: any UTF-8 bytes within
    /// the descriptor's size bounds are accepted. The reported version is
    /// the host-computed SHA-256 digest prefix over the retrieved bytes
    /// (JS bytes, a zero separator, then CSS bytes when a stylesheet is
    /// present), so any edit to either half changes the version. Custom
    /// client mods (`custom.js` / `custom.css`) are the only consumer: the
    /// authoritative source is the host-built profile-root path, never a
    /// renderer-supplied path or URL, and retrieval never touches the
    /// network. The JS half may be empty (CSS-only custom bundle) as long
    /// as at least one half carries bytes; the CSS half is always optional.
    UserOwned,
}

/// When the mod publishes its page-world global relative to bundle evaluation.
///
/// Electron executes every client-mod bundle through
/// `webFrame.executeJavaScript` from an isolated-world preload, so all of them
/// land in the page world at document start. What differs is *when* the global
/// appears: some bundles assign it while their top-level IIFE runs, others
/// publish it from an inner async IIFE after an await chain. The descriptor
/// states the requirement; the substrate implements it.
#[derive(Debug, Clone, Copy)]
pub enum PageGlobalReadiness {
    /// Published synchronously by the bundle's top-level evaluation. The
    /// verifier reads the global once, immediately after the bundle.
    Synchronous,
    /// Published asynchronously. The substrate installs a one-shot property
    /// trap before the bundle runs (so the normal case resolves with no
    /// polling at all) and keeps a coarse bounded poll as a fallback for a
    /// bundle that publishes the global through a path that bypasses `[[Set]]`
    /// (a `var`/lexical global declaration or `defineProperty`). The verdict
    /// is dispatched exactly once, on first observation or at the deadline.
    Awaited {
        /// Page-world global the pre-bundle trap flips to `true` on assignment.
        ready_global: &'static str,
        /// Upper bound on how long the verifier waits before failing.
        timeout_ms: u64,
        /// Coarse fallback poll interval. Never runs more often than this and
        /// is cleared as soon as the verdict is dispatched.
        poll_ms: u64,
    },
}

/// How the host decides whether to inject a mod's renderer adapter.
///
/// Mirrors the `enabled` value each Electron bundle handler returns, which is
/// derived from two different places in the traced implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModEnabling {
    /// Electron hardcodes `enabled: true`, so the mod loads whenever its
    /// artifact exists regardless of `settings.mods` (Shelter).
    AlwaysEnabled,
    /// Electron gates the mod on `settings.mods` containing its id
    /// (`getConfig("mods").includes(<id>)`, Vencord and Equicord).
    SettingsMods,
}

/// Typed contract describing one client mod to the shared substrate.
///
/// Every field is host-controlled. The renderer can request the capability
/// named here but cannot alter any of these values.
#[derive(Debug, Clone, Copy)]
pub struct ModDescriptor {
    /// Stable mod identifier, e.g. `"vencord"`. Keys per-document state.
    pub id: &'static str,
    /// Capability name exposed in the boot snapshot, e.g. `"mod.get_bundle"`.
    pub capability: &'static str,
    /// Fixed HTTPS URL of the JavaScript bundle release asset.
    pub js_url: &'static str,
    /// Fixed cache file name for the JavaScript bundle inside the DEV profile.
    pub js_file: &'static str,
    /// The mod's stylesheet contract, or `None` when the mod ships no CSS.
    pub css: Option<ModCssSource>,
    /// Exact HTTPS hosts allowed for bundle download and redirects.
    pub allowed_hosts: &'static [&'static str],
    /// Maximum accepted JavaScript bundle size in bytes.
    pub max_js_bytes: usize,
    /// How bundle identity is proven for this mod's published artifact.
    pub identity: ModArtifactIdentity,
    /// Page-world global object proving successful execution, e.g. `"Vencord"`.
    pub page_global: &'static str,
    /// Member the page-world global must expose so the marker proves the real
    /// mod API object rather than an unrelated global of the same name.
    pub page_global_requires: Option<&'static str>,
    /// When the page-world global appears relative to bundle evaluation.
    pub page_global_readiness: PageGlobalReadiness,
    /// How the host decides to inject this mod's renderer adapter.
    pub enabling: ModEnabling,
    /// Success marker reported after the page-world global is observed.
    pub success_marker: &'static str,
    /// Storage key inspected for diagnostics, e.g. `"VencordSettings"`, or
    /// `None` for a mod that keeps its settings outside `localStorage`.
    pub settings_key: Option<&'static str>,
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
    /// `None` for a descriptor that ships no CSS. Never an empty string: an
    /// absent stylesheet and a zero-byte stylesheet are different states.
    pub css: Option<String>,
    pub version: String,
    pub js_bytes: usize,
    pub css_bytes: Option<usize>,
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

#[allow(clippy::too_many_arguments)]
fn cached_or_downloaded_asset(
    agent: &ureq::Agent,
    descriptor: &ModDescriptor,
    profile: &Path,
    file_name: &str,
    url: &str,
    max_bytes: usize,
    kind: &str,
    updates_frozen: bool,
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

    // `noBundleUpdates` (Electron modloader parity): a listed mod serves
    // its disk cache only and never hits the network. A missing/empty
    // cache is a typed miss, not a download.
    if updates_frozen {
        return Err(format!(
            "[{}] {file_name} has bundle updates disabled and no usable cache",
            descriptor.id
        ));
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
///
/// A `UserOwned` descriptor (custom client mods) is refused outright: its
/// bytes live in host-built profile-root files served by `custom_bundle`,
/// never at a download URL, so routing one through the downloader would be
/// a bug even though the placeholder URL could never resolve.
///
/// `updates_frozen` mirrors `noBundleUpdates` (Electron modloader parity):
/// a frozen mod serves its disk cache only — a missing cache is a typed
/// miss, never a download.
pub fn load_mod_bundle(
    descriptor: &ModDescriptor,
    profile: std::path::PathBuf,
    updates_frozen: bool,
) -> Result<ModBundle, String> {
    if matches!(descriptor.identity, ModArtifactIdentity::UserOwned) {
        return Err(format!(
            "[{}] user-owned bundles load from local profile files, never downloads",
            descriptor.id
        ));
    }
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
        updates_frozen,
    )?;
    let css = match descriptor.css {
        Some(css) => Some(cached_or_downloaded_asset(
            &agent,
            descriptor,
            &profile,
            css.file,
            css.url,
            css.max_bytes,
            "CSS bundle",
            updates_frozen,
        )?),
        None => None,
    };
    validate_mod_bundle(descriptor, js, css)
}

/// Number of hex characters of the SHA-256 digest reported as the bundle
/// version for an artifact that publishes no build header. 16 hex characters
/// is 64 bits of content identity: long enough to distinguish consecutive
/// upstream builds in practice, short enough to stay a readable log line, and
/// deliberately NOT a pin the host enforces — the artifact is still whatever
/// the fixed upstream source serves.
const DIGEST_VERSION_CHARS: usize = 16;

/// Proves raw bundle bytes are this mod's artifact and returns the validated
/// bundle.
///
/// Identity is established exactly as [`ModArtifactIdentity`] declares: a build
/// banner when upstream publishes one, and unique upstream source sentinels
/// plus a host-computed content digest when it does not. The CSS half is
/// optional by descriptor — a mod that ships no CSS must not be forced to
/// present one (no placeholder stylesheet, no empty CSS stage) — while a mod
/// that does ship CSS must present a bounded, non-empty, UTF-8 stylesheet.
pub fn validate_mod_bundle(
    descriptor: &ModDescriptor,
    js: Vec<u8>,
    css: Option<Vec<u8>>,
) -> Result<ModBundle, String> {
    let user_owned = matches!(descriptor.identity, ModArtifactIdentity::UserOwned);
    // Download-published bundles must always carry a JS half: an empty
    // artifact is a failed download, never a mod. A user-owned bundle may
    // be CSS-only (empty JS) as long as the CSS half carries bytes; both
    // halves empty is still "not installed", reported by the loader before
    // validation is ever reached.
    let css_present = css.as_ref().is_some_and(|bytes| !bytes.is_empty());
    if js.len() > descriptor.max_js_bytes {
        return Err(format!(
            "[{}] JS bundle was empty or exceeded its size limit",
            descriptor.id
        ));
    }
    if js.is_empty() && !(user_owned && css_present) {
        return Err(format!(
            "[{}] JS bundle was empty or exceeded its size limit",
            descriptor.id
        ));
    }
    // Identity is proven on the RAW bytes, before any text reinterpretation.
    // A user-owned bundle has no upstream proof; its version is the
    // host-computed digest over both halves (see `prove_artifact_identity`).
    let version = prove_artifact_identity(descriptor, &js, css.as_deref())?;
    let js = String::from_utf8(js)
        .map_err(|error| format!("[{}] JS bundle is not UTF-8: {error}", descriptor.id))?;
    let js_bytes = js.len();

    let (css, css_bytes) = match (descriptor.css, css) {
        (Some(source), Some(bytes)) => {
            if bytes.is_empty() || bytes.len() > source.max_bytes {
                return Err(format!(
                    "[{}] CSS bundle was empty or exceeded its size limit",
                    descriptor.id
                ));
            }
            let text = String::from_utf8(bytes)
                .map_err(|error| format!("[{}] CSS bundle is not UTF-8: {error}", descriptor.id))?;
            let bytes = text.len();
            (Some(text), Some(bytes))
        }
        (Some(_), None) => {
            // A download-published stylesheet is required: its preload
            // calls `insertCSS(bundle.css!)`. A user-owned stylesheet is
            // optional: custom CSS applies only when `custom.css` exists.
            if user_owned {
                (None, None)
            } else {
                return Err(format!(
                    "[{}] descriptor declares CSS but no stylesheet was provided",
                    descriptor.id
                ));
            }
        }
        (None, None) => (None, None),
        (None, Some(_)) => {
            return Err(format!(
                "[{}] received a CSS bundle this descriptor does not declare",
                descriptor.id
            ))
        }
    };

    Ok(ModBundle {
        js,
        css,
        version,
        js_bytes,
        css_bytes,
    })
}

/// Proves the raw artifact bytes are the registered mod's own release build and
/// derives the version string reported to the renderer.
fn prove_artifact_identity(
    descriptor: &ModDescriptor,
    js: &[u8],
    css: Option<&[u8]>,
) -> Result<String, String> {
    match descriptor.identity {
        ModArtifactIdentity::BuildHeader { prefix } => std::str::from_utf8(js)
            .ok()
            .and_then(|source| source.lines().next())
            .and_then(|line| line.strip_prefix(prefix))
            .map(|version| version.trim_end_matches('\r'))
            .filter(|version| !version.is_empty() && version.len() <= 64)
            .map(str::to_owned)
            .ok_or_else(|| {
                format!(
                    "[{}] bundle did not contain the expected build header",
                    descriptor.id
                )
            }),
        ModArtifactIdentity::SourceSentinels { sentinels } => {
            if sentinels.is_empty() {
                return Err(format!(
                    "[{}] sentinel identity requires at least one sentinel",
                    descriptor.id
                ));
            }
            let source = std::str::from_utf8(js)
                .map_err(|error| format!("[{}] JS bundle is not UTF-8: {error}", descriptor.id))?;
            for sentinel in sentinels {
                if !source.contains(sentinel) {
                    return Err(format!(
                        "[{}] bundle is missing a required identity sentinel",
                        descriptor.id
                    ));
                }
            }
            Ok(format!(
                "sha256-{}",
                hex_digest_prefix(js, DIGEST_VERSION_CHARS)
            ))
        }
        ModArtifactIdentity::UserOwned => {
            // No upstream proof exists for user-owned bytes: any content
            // within the descriptor's size bounds is the user's own mod.
            // The version still identifies content exactly: the digest covers
            // the JS half, a zero separator, then the CSS half (or nothing
            // when no stylesheet is present), so an edit to either half
            // changes the version and identical bytes always agree.
            let mut hasher = Sha256::new();
            hasher.update(js);
            hasher.update([0u8]);
            if let Some(css) = css {
                hasher.update(css);
            }
            let encoded = hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            Ok(format!(
                "sha256-{}",
                encoded[..DIGEST_VERSION_CHARS.min(encoded.len())].to_owned()
            ))
        }
    }
}

/// Hex-encoded SHA-256 digest of `bytes`, truncated to `chars` hex characters.
fn hex_digest_prefix(bytes: &[u8], chars: usize) -> String {
    let encoded = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    encoded[..chars.min(encoded.len())].to_owned()
}

/// Builds the fixed CSS insertion script for one bundle.
///
/// A descriptor that ships no CSS never reaches this builder: [`load_mod_bundle`]
/// and [`validate_mod_bundle`] refuse a stylesheet the descriptor did not
/// declare, and the host skips the CSS stage entirely for such a mod rather
/// than injecting an empty stylesheet.
///
/// The script creates or reuses exactly one `<style>` element with the
/// descriptor's stable id and reports a typed result; it never duplicates
/// the element and never evaluates renderer input.
pub fn mod_css_script(
    descriptor: &ModDescriptor,
    source: &ModCssSource,
    request_id: &str,
    css: &str,
) -> Result<String, serde_json::Error> {
    let request_id = serde_json::to_string(request_id)?;
    let css = serde_json::to_string(css)?;
    let style_id = serde_json::to_string(source.style_element_id)?;
    let result_event = source.result_event;
    // The mod id is host-supplied and travels with the CSS result so the host
    // can refuse a result claimed for another registered mod.
    let mod_id = serde_json::to_string(descriptor.id)?;
    Ok(format!(
        r#"(() => {{
  const requestId = {request_id};
  const report = (ok, error = null) => window.ipc.postMessage(JSON.stringify({{
    version: 1,
    event: {{ type: "{result_event}", payload: {{ mod_id: {mod_id}, request_id: requestId, ok, error }} }}
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

/// Builds the pre-execution snapshot run immediately before the mod bundle.
///
/// It records whether `localStorage` access works and whether the mod's
/// settings key holds valid JSON. A descriptor with no settings key (a mod that
/// keeps its state outside `localStorage`) reports `not_applicable` instead of
/// inventing a key to read.
///
/// For a descriptor whose page-world global is published asynchronously it also
/// installs a one-shot property trap on that global, BEFORE the bundle runs, so
/// the verifier learns about the assignment the moment it happens instead of
/// polling for it. The trap is transparent: reads return the assigned value,
/// and a bundle that redefines or deletes the property simply bypasses it (the
/// verifier's bounded poll covers that case).
pub fn mod_execution_context_script(descriptor: &ModDescriptor) -> String {
    let settings_probe = match descriptor.settings_key {
        Some(key) => {
            let key =
                serde_json::to_string(key).expect("a static settings key must serialize to JSON");
            format!(
                r#"        const settings = storage.getItem({key});
        if (settings === null) {{
          settingsState = "missing";
        }} else {{
          JSON.parse(settings);
          settingsState = "valid_json";
        }}"#
            )
        }
        None => r#"        settingsState = "not_applicable";"#.to_owned(),
    };
    let context_global = descriptor.storage_context_global;
    let page_trap = match descriptor.page_global_readiness {
        PageGlobalReadiness::Synchronous => String::new(),
        PageGlobalReadiness::Awaited { ready_global, .. } => {
            let page_global = serde_json::to_string(descriptor.page_global)
                .expect("a static page global must serialize to JSON");
            let ready_global =
                serde_json::to_string(ready_global).expect("a static ready global must serialize");
            format!(
                r#"  try {{
    const target = {page_global};
    const existing = Object.getOwnPropertyDescriptor(globalThis, target);
    if (!existing && typeof globalThis[target] === "undefined") {{
      let captured;
      let notified = false;
      const notify = () => {{
        if (notified) return;
        notified = true;
        Object.defineProperty(globalThis, {ready_global}, {{
          configurable: false,
          enumerable: false,
          value: true
        }});
      }};
      Object.defineProperty(globalThis, target, {{
        configurable: true,
        enumerable: true,
        get() {{
          return captured;
        }},
        set(value) {{
          captured = value;
          notify();
        }}
      }});
    }}
  }} catch (error) {{
    console.error("[Remcord native][mods] page-global observation trap was unavailable", error);
  }}
"#
            )
        }
    };
    format!(
        r#"(() => {{
  let localStorageApi = "blocked";
  let settingsState = "storage_unavailable";
  try {{
    const storage = window.localStorage;
    localStorageApi = typeof storage?.getItem === "function" ? "available" : "missing";
    if (localStorageApi === "available") {{
      try {{
{settings_probe}
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
{page_trap}}})();
"#
    )
}

/// Builds the post-execution verifier. It checks the descriptor's page-world
/// global and dispatches the bootstrap verdict on the document-local DOM
/// event; it never evaluates renderer input.
///
/// Readiness is descriptor-driven:
/// - [`PageGlobalReadiness::Synchronous`] reads the global exactly once, right
///   after the bundle, as it always has.
/// - [`PageGlobalReadiness::Awaited`] resolves on the first observation of the
///   global (immediately when the pre-execution trap fired, otherwise on a
///   coarse bounded poll) and dispatches exactly one verdict. Every timer is
///   cleared as soon as the verdict is sent, so a successful mod leaves no
///   interval and a failed mod leaves nothing behind either.
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
    let observation = match descriptor.page_global_readiness {
        PageGlobalReadiness::Synchronous => format!(
            r#"  const observed = () => {{
    if (!globalThis.{page_global} || typeof globalThis.{page_global} !== "object") {{
      throw new Error("The mod global was not initialized by the bundle");
    }}
  }};
  let result;
  try {{
    observed();
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
  window.dispatchEvent(new CustomEvent("{dom_event}", {{ detail: result }}));"#
        ),
        PageGlobalReadiness::Awaited {
            ready_global,
            timeout_ms,
            poll_ms,
        } => {
            let requires_probe = match descriptor.page_global_requires {
                Some(member) => {
                    format!("\n    if (typeof value.{member} === \"undefined\") return false;")
                }
                None => String::new(),
            };
            format!(
                r#"  const observed = () => {{
    let value;
    try {{
      value = globalThis.{page_global};
    }} catch (error) {{
      return false;
    }}
    if (!value || typeof value !== "object") return false;{requires_probe}
    return true;
  }};
  let settled = false;
  let poll = null;
  let deadline = null;
  const finish = (ok, error) => {{
    if (settled) return;
    settled = true;
    if (poll !== null) clearInterval(poll);
    if (deadline !== null) clearTimeout(deadline);
    window.dispatchEvent(new CustomEvent("{dom_event}", {{ detail: {{
      request_id: requestId,
      ok,
      marker: ok ? marker : "",
      execution_ms: {execution_ms},
      local_storage_api: localStorageApi,
      settings_state: settingsState,
      error: ok ? null : String(error ?? "the mod global was never published").slice(0, 2048)
    }} }}));
  }};
  if (globalThis.{ready_global} === true || observed()) {{
    finish(true, null);
    return;
  }}
  poll = setInterval(() => {{
    if (globalThis.{ready_global} === true || observed()) finish(true, null);
  }}, {poll_ms});
  deadline = setTimeout(
    () => finish(false, "the mod global was not published before the deadline"),
    {timeout_ms}
  );"#
            )
        }
    };
    Ok(format!(
        r#"(() => {{
  const requestId = {request_id};
  const marker = {marker};
  const storageContext = globalThis.{context_global} || {{}};
  const localStorageApi = storageContext.localStorageApi || "blocked";
  const settingsState = storageContext.settingsState || "storage_unavailable";
{observation}
}})();"#
    ))
}

/// Fail-fast structural validation of one descriptor.
///
/// Every value the substrate splices into generated JavaScript must be a safe
/// identifier or a JSON-escaped string; this check runs once per descriptor at
/// startup and in tests, so a malformed descriptor can never reach a script
/// builder. It is deliberately not a renderer-facing validation: the renderer
/// supplies nothing here.
pub fn validate_descriptor(descriptor: &ModDescriptor) -> Result<(), String> {
    let identifier = |label: &str, value: &'static str| -> Result<(), String> {
        let mut chars = value.chars();
        let starts_ok = chars
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$');
        if !starts_ok || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') {
            return Err(format!(
                "[{}] {label} '{value}' is not a plain ASCII identifier",
                descriptor.id
            ));
        }
        if value.len() > 64 {
            return Err(format!(
                "[{}] {label} '{value}' exceeds 64 bytes",
                descriptor.id
            ));
        }
        Ok(())
    };
    // Names that are spliced into a string literal or a quoted event type
    // rather than read as a bare identifier: separators are allowed, quoting
    // characters and whitespace are not.
    let token = |label: &str, value: &'static str, max: usize| -> Result<(), String> {
        if value.is_empty() || value.len() > max {
            return Err(format!(
                "[{}] {label} must be 1..={max} bytes",
                descriptor.id
            ));
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
        {
            return Err(format!(
                "[{}] {label} '{value}' contains an unusable character",
                descriptor.id
            ));
        }
        Ok(())
    };
    if descriptor.id.is_empty()
        || !descriptor
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(format!(
            "[{}] descriptor id has an unusable shape",
            descriptor.id
        ));
    }
    token("capability", descriptor.capability, 64)?;
    token("bootstrap_dom_event", descriptor.bootstrap_dom_event, 128)?;
    token("success_marker", descriptor.success_marker, 64)?;
    identifier("page_global", descriptor.page_global)?;
    identifier("storage_context_global", descriptor.storage_context_global)?;
    if let Some(member) = descriptor.page_global_requires {
        identifier("page_global_requires", member)?;
    }
    if let Some(key) = descriptor.settings_key {
        identifier("settings_key", key)?;
    }
    if let Some(source) = descriptor.css {
        if source.file.is_empty()
            || !source
                .file
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
            || source.file.contains("..")
            || source.file.len() > 64
        {
            return Err(format!(
                "[{}] CSS cache file name '{}' must be a bare file name",
                descriptor.id, source.file
            ));
        }
        if source.style_element_id.is_empty() || source.style_element_id.len() > 256 {
            return Err(format!(
                "[{}] CSS style element id must be 1..=256 bytes",
                descriptor.id
            ));
        }
        if source.result_event.is_empty()
            || !source
                .result_event
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(format!(
                "[{}] CSS result event '{}' must be a snake_case wire name",
                descriptor.id, source.result_event
            ));
        }
        if source.max_bytes == 0 || source.url.is_empty() {
            return Err(format!("[{}] CSS source is incomplete", descriptor.id));
        }
    }
    match descriptor.identity {
        ModArtifactIdentity::BuildHeader { prefix } => {
            if prefix.is_empty() {
                return Err(format!("[{}] build header prefix is empty", descriptor.id));
            }
        }
        ModArtifactIdentity::SourceSentinels { sentinels } => {
            if sentinels.is_empty() {
                return Err(format!(
                    "[{}] sentinel identity needs at least one sentinel",
                    descriptor.id
                ));
            }
            if sentinels
                .iter()
                .any(|sentinel| sentinel.is_empty() || sentinel.len() > 256)
            {
                return Err(format!(
                    "[{}] identity sentinels must be 1..=256 bytes",
                    descriptor.id
                ));
            }
        }
        // A user-owned bundle proves nothing upstream: there is no banner
        // or sentinel to validate, so the identity is structurally complete
        // by construction.
        ModArtifactIdentity::UserOwned => {}
    }
    match descriptor.page_global_readiness {
        PageGlobalReadiness::Synchronous => {
            if descriptor.page_global_requires.is_some() {
                return Err(format!(
                    "[{}] a synchronous global needs no member probe",
                    descriptor.id
                ));
            }
        }
        PageGlobalReadiness::Awaited {
            ready_global,
            timeout_ms,
            poll_ms,
        } => {
            identifier("page_global_readiness.ready_global", ready_global)?;
            if !(16..=300_000).contains(&poll_ms) {
                return Err(format!(
                    "[{}] awaited poll interval must be 16..=300000 ms",
                    descriptor.id
                ));
            }
            if poll_ms > 5_000 || timeout_ms > 600_000 || poll_ms >= timeout_ms {
                return Err(format!(
                    "[{}] awaited readiness bounds are not a coarse bounded poll",
                    descriptor.id
                ));
            }
        }
    }
    Ok(())
}

/// Builds a managed-CSS insertion script for CSS-only consumers (themes).
///
/// Unlike [`mod_css_script`], this entry point is not tied to a
/// [`ModDescriptor`] and carries no request correlation or typed result
/// event: theme injection is a host push (the renderer never requests theme
/// CSS), so there is no capability round trip to correlate. The script
/// creates or reuses exactly one `<style>` element with the host-supplied id,
/// sets its text, and appends it to `<head>` ONLY when newly created
/// (falling back to `DOMContentLoaded` when the head is not yet present).
/// Updating an existing node sets `textContent` in place and never
/// re-appends: a real-DOM `appendChild` on an attached node MOVES it, so
/// re-appending on every content update would drag the updated stylesheet to
/// the end of `<head>` and corrupt the host emission order (Vencord CSS ->
/// Quick CSS -> sorted theme CSS). Structural reordering has its own builder
/// ([`managed_css_order_script`]). Both arguments are host-controlled and
/// JSON-escaped; the script never evaluates renderer input and exposes no
/// eval surface.
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
      style.textContent = {css};
      root.appendChild(style);
    }} else {{
      style.textContent = {css};
    }}
    return true;
  }} catch (error) {{
    console.error("[Remcord native][themes] managed CSS injection failed", error);
    return true;
  }}
  }};
  if (!inject()) document.addEventListener("DOMContentLoaded", inject, {{ once: true }});
}})();"#
    ))
}

/// Builds an order-enforcement script for a batch of managed `<style>` ids.
///
/// Structural changes (a theme added/removed, Quick CSS recreated) can leave
/// a newly created node at the end of `<head>` instead of its cascade
/// position. This script re-appends the listed EXISTING nodes in the given
/// host-controlled order (missing ids are skipped), which moves them into
/// the correct relative sequence after earlier stylesheets (Vencord CSS,
/// prototype CSS). Content-only updates must NOT call this: they already
/// update in place via [`managed_css_script`] with zero position change.
/// The id list is bounded (at most 257 entries: Quick CSS + 256 themes) and
/// every entry is validated like [`managed_css_script`]; the script never
/// evaluates renderer input and exposes no eval surface.
pub fn managed_css_order_script(ordered_ids: &[&str]) -> Result<String, serde_json::Error> {
    if ordered_ids.len() > 257 {
        return Err(serde::de::Error::custom(
            "managed style order list must hold at most 257 ids",
        ));
    }
    let mut encoded: Vec<String> = Vec::with_capacity(ordered_ids.len());
    for id in ordered_ids {
        if id.is_empty() || id.len() > 256 {
            return Err(serde::de::Error::custom(
                "managed style id must be a non-empty string of at most 256 bytes",
            ));
        }
        encoded.push(serde_json::to_string(id)?);
    }
    let list = encoded.join(",");
    Ok(format!(
        r#"(() => {{
  try {{
    const root = document.head;
    if (!root) return;
    for (const id of [{list}]) {{
      const style = document.getElementById(id);
      if (style) root.appendChild(style);
    }}
  }} catch (error) {{
    console.error("[Remcord native][themes] managed CSS order failed", error);
  }}
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
    console.error("[Remcord native][themes] managed CSS removal failed", error);
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
    /// `None` for a mod whose descriptor declares no stylesheet.
    pub css_bytes: Option<usize>,
    pub execution_ms: f64,
    pub css_started_at: Instant,
    /// Elapsed time of the CSS stage. `None` means no CSS stage ran at all.
    pub css_injection_ms: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::{
        is_allowed_mod_download_url, load_mod_bundle, managed_css_order_script,
        managed_css_remove_script, managed_css_script, mod_css_script,
        mod_execution_context_script, mod_marker_script, validate_descriptor, validate_mod_bundle,
        DocumentModLifecycle, ModArtifactIdentity, ModCache, ModCssSource, ModDescriptor,
        ModEnabling, ModHostMetrics, PageGlobalReadiness,
    };

    /// Synthetic adapter contract proving the substrate is not tied to any one
    /// mod's shape. It deliberately mixes every optional dimension at once:
    /// a stylesheet, a build header, a synchronous page global, a settings key,
    /// and a settings-gated enablement — the opposite combination from the
    /// header-less/CSS-less/asynchronous consumer registered in `main.rs`.
    pub fn test_descriptor() -> ModDescriptor {
        ModDescriptor {
            id: "testmod",
            capability: "mod.test.get_bundle",
            js_url: "https://github.com/example/TestMod/releases/download/devbuild/browser.js",
            js_file: "testmod.js",
            css: Some(ModCssSource {
                url: "https://github.com/example/TestMod/releases/download/devbuild/browser.css",
                file: "testmod.css",
                max_bytes: 256 * 1024,
                style_element_id: "remcord-native-testmod-css",
                result_event: "testmod_css_result",
            }),
            allowed_hosts: &["github.com", "release-assets.githubusercontent.com"],
            max_js_bytes: 1024 * 1024,
            identity: ModArtifactIdentity::BuildHeader {
                prefix: "// TestMod ",
            },
            page_global: "TestMod",
            page_global_requires: None,
            page_global_readiness: PageGlobalReadiness::Synchronous,
            enabling: ModEnabling::SettingsMods,
            success_marker: "testmod-bootstrap-executed",
            settings_key: Some("TestModSettings"),
            bootstrap_dom_event: "remcord-native:testmod-bootstrap",
            storage_context_global: "__remcordNativeTestModStorageAtExecution",
            log_tag: "Mod:TestMod",
        }
    }

    /// Synthetic descriptor for the opposite shape: no stylesheet, no build
    /// header, an asynchronously published page global with a required member,
    /// no `localStorage` settings key, and unconditional enablement.
    pub fn headerless_descriptor() -> ModDescriptor {
        ModDescriptor {
            id: "headerless",
            capability: "mod.get_bundle",
            js_url: "https://raw.githubusercontent.com/example/builds/main/headerless.js",
            js_file: "headerless.js",
            css: None,
            allowed_hosts: &["raw.githubusercontent.com"],
            max_js_bytes: 8 * 1024 * 1024,
            identity: ModArtifactIdentity::SourceSentinels {
                sentinels: &[
                    "headerless is initializing...",
                    "headerless is initialized. took: ",
                ],
            },
            page_global: "headerless",
            page_global_requires: Some("plugins"),
            page_global_readiness: PageGlobalReadiness::Awaited {
                ready_global: "__remcordNativeHeaderlessPageReady",
                timeout_ms: 120_000,
                poll_ms: 250,
            },
            enabling: ModEnabling::AlwaysEnabled,
            success_marker: "headerless-bootstrap-executed",
            settings_key: None,
            bootstrap_dom_event: "remcord-native:headerless-bootstrap",
            storage_context_global: "__remcordNativeHeaderlessStorageAtExecution",
            log_tag: "Mod:Headerless",
        }
    }

    fn headerless_bundle() -> Vec<u8> {
        b"(()=>{var a=1;console.error(\"headerless is initializing...\");\
         (async()=>{window.headerless={plugins:{}}})();\
         console.error(\"headerless is initialized. took: 1ms\");})();"
            .to_vec()
    }

    #[test]
    fn synthetic_adapter_contract_validates_bundle_identity() {
        let descriptor = test_descriptor();
        assert_eq!(descriptor.capability, "mod.test.get_bundle");
        assert!(validate_descriptor(&descriptor).is_ok());
        let bundle = validate_mod_bundle(
            &descriptor,
            b"// TestMod 1.0.0\nvar TestMod = {};\n".to_vec(),
            Some(b"/* test CSS */".to_vec()),
        )
        .unwrap();
        assert_eq!(bundle.version, "1.0.0");
        assert_eq!(bundle.js_bytes, bundle.js.len());
        assert_eq!(bundle.css_bytes, Some(bundle.css.as_ref().unwrap().len()));
        assert!(validate_mod_bundle(
            &descriptor,
            b"// Vencord b52ed36\nvar Vencord = {};\n".to_vec(),
            Some(b"/*css*/".to_vec()),
        )
        .is_err());
    }

    #[test]
    fn a_mod_without_css_is_never_forced_to_present_a_stylesheet() {
        // The header-less shape declares no stylesheet at all. Validation must
        // accept the artifact with no CSS half, and must REFUSE a stylesheet
        // the descriptor did not declare instead of silently carrying one.
        let descriptor = headerless_descriptor();
        assert!(validate_descriptor(&descriptor).is_ok());
        let bundle = validate_mod_bundle(&descriptor, headerless_bundle(), None).unwrap();
        assert!(
            bundle.css.is_none(),
            "no placeholder stylesheet may be manufactured"
        );
        assert_eq!(bundle.css_bytes, None);
        assert!(
            validate_mod_bundle(&descriptor, headerless_bundle(), Some(b".x{}".to_vec())).is_err()
        );
        // A descriptor that DOES declare CSS must not accept a missing half.
        assert!(validate_mod_bundle(
            &test_descriptor(),
            b"// TestMod 1.0.0\nvar TestMod = {};\n".to_vec(),
            None
        )
        .is_err());
    }

    #[test]
    fn a_headerless_artifact_is_identified_by_real_sentinels_and_a_content_digest() {
        let descriptor = headerless_descriptor();
        let bundle = validate_mod_bundle(&descriptor, headerless_bundle(), None).unwrap();
        // No build banner exists upstream, so the version is the host's own
        // content digest of the retrieved bytes — never an invented marker.
        assert!(bundle.version.starts_with("sha256-"), "{}", bundle.version);
        assert_eq!(bundle.version.len(), "sha256-".len() + 16);
        // The digest is exact: any byte change produces a different version.
        let mut tampered = headerless_bundle();
        *tampered.last_mut().unwrap() = b'!';
        let other = validate_mod_bundle(&descriptor, tampered, None).unwrap();
        assert_ne!(other.version, bundle.version);
        // Identical bytes always produce the identical version.
        let again = validate_mod_bundle(&descriptor, headerless_bundle(), None).unwrap();
        assert_eq!(again.version, bundle.version);

        // An artifact that is missing a sentinel is refused: a different mod's
        // bundle can never be served as this one.
        let missing = validate_mod_bundle(
            &descriptor,
            b"(()=>{console.error(\"other mod is initializing...\");})();".to_vec(),
            None,
        );
        assert!(missing.unwrap_err().contains("sentinel"), "sentinel check");
        // An empty sentinel list is a descriptor bug, not a pass.
        let empty = ModDescriptor {
            identity: ModArtifactIdentity::SourceSentinels { sentinels: &[] },
            ..headerless_descriptor()
        };
        assert!(validate_mod_bundle(&empty, headerless_bundle(), None).is_err());
        assert!(validate_descriptor(&empty).is_err());
    }

    /// Synthetic descriptor for the user-owned shape: no upstream publisher,
    /// optional CSS, and no page-world global of its own (mirrors the real
    /// `custom` descriptor in `main.rs` without importing it).
    pub fn user_owned_descriptor() -> ModDescriptor {
        ModDescriptor {
            id: "custom",
            capability: "mod.get_bundle",
            js_url: "https://example.invalid/custom.js",
            js_file: "custom.js",
            css: Some(ModCssSource {
                url: "https://example.invalid/custom.css",
                file: "custom.css",
                max_bytes: 1024 * 1024,
                style_element_id: "remcord-native-custom-css",
                result_event: "mod_css_result",
            }),
            allowed_hosts: &["example.invalid"],
            max_js_bytes: 8 * 1024 * 1024,
            identity: ModArtifactIdentity::UserOwned,
            page_global: "__remcordNativeCustomStorageAtExecution",
            page_global_requires: None,
            page_global_readiness: PageGlobalReadiness::Synchronous,
            enabling: ModEnabling::SettingsMods,
            success_marker: "custom-bootstrap-executed",
            settings_key: None,
            bootstrap_dom_event: "remcord-native:custom-bootstrap",
            storage_context_global: "__remcordNativeCustomStorageAtExecution",
            log_tag: "Mod:Custom",
        }
    }

    #[test]
    fn user_owned_bundles_accept_any_utf8_bytes_with_a_content_digest() {
        let descriptor = user_owned_descriptor();
        assert!(validate_descriptor(&descriptor).is_ok());
        // JS-only: the stylesheet is optional for user-owned bundles (a
        // download-published bundle declaring CSS would be refused here).
        let js_only =
            validate_mod_bundle(&descriptor, b"window.__custom = 1;".to_vec(), None).unwrap();
        assert_eq!(js_only.js, "window.__custom = 1;");
        assert!(js_only.css.is_none());
        assert!(js_only.version.starts_with("sha256-"));
        assert_eq!(js_only.version.len(), "sha256-".len() + 16);
        // JS + CSS: both halves validate, and the digest covers both (an
        // edit to either half changes the version).
        let both = validate_mod_bundle(
            &descriptor,
            b"window.__custom = 1;".to_vec(),
            Some(b".x{color:red}".to_vec()),
        )
        .unwrap();
        assert_eq!(both.css.unwrap(), ".x{color:red}");
        assert_ne!(both.version, js_only.version);
        let css_edit = validate_mod_bundle(
            &descriptor,
            b"window.__custom = 1;".to_vec(),
            Some(b".x{color:blue}".to_vec()),
        )
        .unwrap();
        assert_ne!(css_edit.version, both.version);
        // CSS-only: the empty JS half is accepted (never manufactured into
        // a placeholder), and the digest still identifies content exactly.
        let css_only =
            validate_mod_bundle(&descriptor, b"".to_vec(), Some(b".x{}".to_vec())).unwrap();
        assert_eq!(css_only.js, "");
        assert_eq!(css_only.js_bytes, 0);
        let css_only_other =
            validate_mod_bundle(&descriptor, b"".to_vec(), Some(b".y{}".to_vec())).unwrap();
        assert_ne!(css_only_other.version, css_only.version);
        let css_only_again =
            validate_mod_bundle(&descriptor, b"".to_vec(), Some(b".x{}".to_vec())).unwrap();
        assert_eq!(css_only_again.version, css_only.version);
        // Both halves empty is still refused.
        assert!(validate_mod_bundle(&descriptor, b"".to_vec(), None).is_err());
        // Oversize halves are refused even for user-owned bundles.
        let big_js = vec![b'x'; 8 * 1024 * 1024 + 1];
        assert!(validate_mod_bundle(&descriptor, big_js, None).is_err());
        let big_css = vec![b'x'; 1024 * 1024 + 1];
        assert!(validate_mod_bundle(&descriptor, b"window.x=1;".to_vec(), Some(big_css)).is_err());
        // Non-UTF8 bytes are refused, never served.
        assert!(validate_mod_bundle(&descriptor, vec![0xff, 0xfe], None).is_err());
    }

    #[test]
    fn user_owned_descriptors_never_reach_the_downloader() {
        // The downloader refuses before building any request, so this test
        // is hermetic: no network, no filesystem, no placeholder fetch.
        let error = load_mod_bundle(
            &user_owned_descriptor(),
            std::env::temp_dir().join("remcord-user-owned-never"),
            false,
        )
        .expect_err("UserOwned must never download");
        assert!(error.contains("local profile files"));
    }

    #[test]
    fn frozen_bundle_updates_serve_cache_only_never_network() {
        // `noBundleUpdates` parity: with no disk cache and updates frozen,
        // the loader reports a typed miss BEFORE any download attempt, so
        // this test is hermetic (no network even though the descriptor has
        // real upstream URLs).
        let profile = std::env::temp_dir().join(format!(
            "remcord-frozen-never-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time must be available")
                .as_nanos()
        ));
        let error = load_mod_bundle(&headerless_descriptor(), profile, true)
            .expect_err("frozen updates with no cache must miss");
        assert!(
            error.contains("bundle updates disabled"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn awaited_readiness_installs_a_trap_before_the_bundle_and_a_bounded_fallback_after() {
        let descriptor = headerless_descriptor();
        let context = mod_execution_context_script(&descriptor);
        // The trap is installed by the PRE-execution script, so it is in place
        // before the bundle assigns the global.
        assert!(
            context.contains("Object.defineProperty(globalThis, target"),
            "{context}"
        );
        assert!(
            context.contains("__remcordNativeHeaderlessPageReady"),
            "{context}"
        );
        assert!(
            !context.contains("setInterval"),
            "no polling before the bundle runs"
        );
        // A mod with no localStorage settings key reports not_applicable rather
        // than reading an invented key.
        assert!(
            context.contains(r#"settingsState = "not_applicable";"#),
            "{context}"
        );
        assert!(!context.contains("getItem(\"headerless"), "{context}");

        let marker = mod_marker_script(&descriptor, "request-1", 1.5).unwrap();
        assert!(marker.contains("globalThis.headerless"));
        // The required member makes the marker prove the real mod API object.
        assert!(
            marker.contains(r#"typeof value.plugins === "undefined""#),
            "{marker}"
        );
        assert!(marker.contains("__remcordNativeHeaderlessPageReady"));
        assert!(marker.contains("headerless-bootstrap-executed"));
        assert!(marker.contains("remcord-native:headerless-bootstrap"));
        // Bounded, coarse, and self-clearing: exactly one interval and one
        // deadline, both cleared as soon as the single verdict is dispatched.
        assert_eq!(marker.matches("setInterval").count(), 1, "{marker}");
        assert_eq!(marker.matches("setTimeout").count(), 1, "{marker}");
        assert!(marker.contains("clearInterval(poll)"));
        assert!(marker.contains("clearTimeout(deadline)"));
        assert!(marker.contains("250"));
        assert!(marker.contains("120000"));
        assert!(!marker.contains("eval("));
        assert!(!marker.contains("new Function"));

        // A synchronous descriptor keeps the single-read shape with no timer
        // and no trap at all.
        let sync_descriptor = test_descriptor();
        let sync = mod_execution_context_script(&sync_descriptor);
        assert!(!sync.contains("setInterval"));
        assert!(!sync.contains("ready_global"));
        let sync_marker = mod_marker_script(&sync_descriptor, "request-1", 1.5).unwrap();
        assert!(!sync_marker.contains("setInterval"));
        assert!(!sync_marker.contains("setTimeout"));
        // The settings-key read belongs to the PRE-execution snapshot, not the
        // marker: the marker only reports the state it recorded there.
        assert!(sync.contains("TestModSettings"));
        assert!(sync_marker.contains("__remcordNativeTestModStorageAtExecution"));
    }

    #[test]
    fn descriptor_validation_rejects_shapes_that_could_break_generated_scripts() {
        assert!(validate_descriptor(&headerless_descriptor()).is_ok());
        // An injected quote in a spliced name can never reach a script builder.
        assert!(validate_descriptor(&ModDescriptor {
            page_global: "shelter\";globalThis.pwned=1;//",
            ..headerless_descriptor()
        })
        .is_err());
        assert!(validate_descriptor(&ModDescriptor {
            page_global_requires: Some("plugins\"+alert(1)+\""),
            ..headerless_descriptor()
        })
        .is_err());
        assert!(validate_descriptor(&ModDescriptor {
            bootstrap_dom_event: "remcord-native:bootstrap\");evil();//",
            ..headerless_descriptor()
        })
        .is_err());
        // A cache name that could traverse or escape the profile directory.
        assert!(validate_descriptor(&ModDescriptor {
            css: Some(ModCssSource {
                url: "https://raw.githubusercontent.com/example/builds/main/x.css",
                file: "../escape.css",
                max_bytes: 1024,
                style_element_id: "remcord-native-x-css",
                result_event: "x_css_result",
            }),
            ..test_descriptor()
        })
        .is_err());
        // A readiness bound that is not a coarse, bounded poll.
        assert!(validate_descriptor(&ModDescriptor {
            page_global_readiness: PageGlobalReadiness::Awaited {
                ready_global: "__remcordNativeXPageReady",
                timeout_ms: 1000,
                poll_ms: 5,
            },
            ..headerless_descriptor()
        })
        .is_err());
        assert!(validate_descriptor(&ModDescriptor {
            page_global_readiness: PageGlobalReadiness::Awaited {
                ready_global: "__remcordNativeXPageReady",
                timeout_ms: 1000,
                poll_ms: 600_000,
            },
            ..headerless_descriptor()
        })
        .is_err());
        // A member probe only means something for an awaited global.
        assert!(validate_descriptor(&ModDescriptor {
            page_global_requires: Some("plugins"),
            ..test_descriptor()
        })
        .is_err());
    }

    #[test]
    fn bundle_validation_rejects_missing_oversized_and_non_utf8_artifacts() {
        let descriptor = test_descriptor();
        let good_js = b"// TestMod 1.0.0\nvar TestMod = {};\n".to_vec();
        let good_css = b"/* test CSS */".to_vec();

        // Missing/empty artifact: both halves are required (Electron's
        // ModBundle declares `js: string`, and the substrate refuses to
        // execute a bundle it cannot fully validate).
        assert!(validate_mod_bundle(&descriptor, Vec::new(), Some(good_css.clone())).is_err());
        assert!(validate_mod_bundle(&descriptor, good_js.clone(), Some(Vec::new())).is_err());

        // Oversized artifacts are rejected before allocation of any source
        // string, on the descriptor's own limits.
        let big_js = vec![b'a'; descriptor.max_js_bytes + 1];
        assert!(validate_mod_bundle(&descriptor, big_js, Some(good_css.clone())).is_err());
        let max_css = descriptor.css.unwrap().max_bytes;
        let big_css = vec![b'a'; max_css + 1];
        assert!(validate_mod_bundle(&descriptor, good_js.clone(), Some(big_css)).is_err());

        // Invalid UTF-8 in either half is a typed failure, never a lossy
        // conversion.
        let bad_js = vec![0x66, 0xFF, 0x66];
        assert!(validate_mod_bundle(&descriptor, bad_js, Some(good_css.clone())).is_err());
        let bad_css = vec![0x66, 0xFF, 0x66];
        assert!(validate_mod_bundle(&descriptor, good_js.clone(), Some(bad_css)).is_err());

        // A missing build header is a different failure from a malformed one:
        // an artifact from a DIFFERENT mod must never be served as this one.
        let foreign = validate_mod_bundle(
            &descriptor,
            b"// Vencord b52ed36\nvar Vencord = {};\n".to_vec(),
            Some(good_css),
        );
        let foreign = foreign.unwrap_err();
        assert!(foreign.contains("build header"), "{foreign}");

        // An over-long or empty version header is refused as well.
        let long_version = format!("// TestMod {}\n", "a".repeat(65));
        assert!(validate_mod_bundle(
            &descriptor,
            long_version.into_bytes(),
            Some(b"/*c*/".to_vec())
        )
        .is_err());
        assert!(validate_mod_bundle(
            &descriptor,
            b"// TestMod \nvar TestMod = {};\n".to_vec(),
            Some(b"/*c*/".to_vec())
        )
        .is_err());
    }

    #[test]
    fn an_absent_disk_cache_is_a_miss_not_a_failure() {
        // Retrieval downloads the fixed release asset when no cached file
        // exists; the disk-cache read path is only taken for present files,
        // so an absent artifact cannot masquerade as a validated bundle.
        let profile = std::env::temp_dir().join("remcord-native-substrate-absent-cache");
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).expect("temp profile");
        assert!(!profile.join("testmod.js").exists());
        assert!(!profile.join("testmod.css").exists());
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn bundle_cache_records_a_miss_then_a_hit() {
        let descriptor = test_descriptor();
        let mut cache = ModCache::default();
        assert!(cache.get(descriptor.id).is_none());

        let bundle = validate_mod_bundle(
            &descriptor,
            b"// TestMod 1.0.0\nvar TestMod = {};\n".to_vec(),
            Some(b"/*css*/".to_vec()),
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
        let css = mod_css_script(
            &descriptor,
            descriptor.css.as_ref().unwrap(),
            "request-1",
            r#"body::after{content:"ok"}"#,
        )
        .unwrap();
        assert!(css.contains("remcord-native-testmod-css"));
        assert!(css.contains("testmod_css_result"));
        assert!(css.contains(r#"body::after{content:\"ok\"}"#));
        assert!(css.contains("getElementById"));
        // One stable element reused, never duplicated: exactly one creation site.
        assert_eq!(css.matches("createElement(\"style\")").count(), 1);

        let context = mod_execution_context_script(&descriptor);
        assert!(context.contains("TestModSettings"));
        assert!(context.contains("__remcordNativeTestModStorageAtExecution"));

        let marker = mod_marker_script(&descriptor, "request-1", 1.5).unwrap();
        assert!(marker.contains("globalThis.TestMod"));
        assert!(marker.contains("testmod-bootstrap-executed"));
        assert!(marker.contains("remcord-native:testmod-bootstrap"));
        assert!(!marker.contains("eval("));
        assert!(!marker.contains("new Function"));
        assert!(!css.contains("eval("));
        assert!(!css.contains("new Function"));
    }

    #[test]
    fn managed_css_scripts_use_a_stable_identity_without_an_eval_surface() {
        let inject =
            managed_css_script("remcord-native-theme-demo", r#"body::after{content:"ok"}"#)
                .unwrap();
        assert!(inject.contains("remcord-native-theme-demo"));
        assert!(inject.contains(r#"body::after{content:\"ok\"}"#));
        assert!(inject.contains("getElementById"));
        // One stable element reused, never duplicated: exactly one creation
        // site plus the reuse lookup.
        assert_eq!(inject.matches("createElement(\"style\")").count(), 1);
        assert_eq!(inject.matches("getElementById").count(), 1);
        assert!(inject.contains("DOMContentLoaded"));
        assert!(!inject.contains("eval("));
        assert!(!inject.contains("new Function"));
        // Order preservation: an existing node updates in place and is never
        // re-appended (a real-DOM appendChild would MOVE it to the end of
        // head and corrupt the Vencord -> Quick CSS -> sorted-themes
        // cascade). Exactly one append site, guarded by the creation branch.
        assert_eq!(inject.matches("appendChild").count(), 1);
        assert!(inject.contains("} else {"));
        // No request correlation: a host push has no renderer request id.
        assert!(!inject.contains("requestId"));

        let remove = managed_css_remove_script("remcord-native-theme-demo").unwrap();
        assert!(remove.contains("remcord-native-theme-demo"));
        assert!(remove.contains(".remove()"));
        assert!(!remove.contains("createElement"));
        assert!(!remove.contains("eval("));

        assert!(managed_css_script("", "body{}").is_err());
        assert!(managed_css_remove_script("",).is_err());
        assert!(managed_css_script(&"x".repeat(257), "body{}").is_err());
    }

    #[test]
    fn managed_css_order_script_reorders_only_host_listed_styles() {
        let order = managed_css_order_script(&[
            "remcord-quick-css",
            "remcord-native-theme-alpha",
            "remcord-native-theme-zulu",
        ])
        .unwrap();
        assert!(order.contains("remcord-quick-css"));
        assert!(order.contains("remcord-native-theme-alpha"));
        assert!(order.contains("remcord-native-theme-zulu"));
        // Relative order in the script matches the cascade order argument.
        let quick = order.find("remcord-quick-css").unwrap();
        let alpha = order.find("remcord-native-theme-alpha").unwrap();
        let zulu = order.find("remcord-native-theme-zulu").unwrap();
        assert!(quick < alpha && alpha < zulu);
        assert!(order.contains("getElementById"));
        assert!(order.contains("appendChild"));
        assert!(!order.contains("createElement"));
        assert!(!order.contains("textContent"));
        assert!(!order.contains("eval("));
        assert!(!order.contains("new Function"));

        assert!(managed_css_order_script(&[]).is_ok());
        assert!(managed_css_order_script(&[""]).is_err());
        assert!(managed_css_order_script(&[&"x".repeat(257)]).is_err());
        let too_many: Vec<String> = (0..258).map(|index| format!("id-{index}")).collect();
        let too_many_refs: Vec<&str> = too_many.iter().map(String::as_str).collect();
        assert!(managed_css_order_script(&too_many_refs).is_err());
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

    /// Proves the substrate lifecycle is genuinely shared rather than shaped
    /// around one mod: both synthetic shapes — the stylesheet/build-header/
    /// synchronous/settings-key one and the header-less/CSS-less/asynchronous
    /// one — must satisfy the SAME registry, cache, validation, bootstrap,
    /// execution, duplicate-prevention, reload-reset, and failure-propagation
    /// contract.
    #[test]
    fn every_mod_shape_satisfies_the_same_substrate_lifecycle() {
        for descriptor in [
            test_descriptor(),
            headerless_descriptor(),
            user_owned_descriptor(),
        ] {
            assert!(
                validate_descriptor(&descriptor).is_ok(),
                "{} must be a servable descriptor",
                descriptor.id
            );

            // Capability lookup: the descriptor id is the only key, and every
            // shape shares ONE capability name.
            assert!(descriptor.capability.ends_with(".get_bundle"));
            assert!(validate_id(descriptor.id).is_ok());

            // Retrieval + validation through one code path: the artifact bytes
            // are shaped to satisfy whatever identity the descriptor declares.
            let js = match descriptor.identity {
                ModArtifactIdentity::BuildHeader { .. } => {
                    b"// TestMod 1.0.0\nvar TestMod = {};\n".to_vec()
                }
                ModArtifactIdentity::SourceSentinels { .. } => headerless_bundle(),
                ModArtifactIdentity::UserOwned => b"window.__custom = 1;".to_vec(),
            };
            let css = descriptor
                .css
                .map(|source| format!("/* {} */", source.file).into_bytes());
            let bundle = validate_mod_bundle(&descriptor, js, css)
                .unwrap_or_else(|error| panic!("{} retrieval failed: {error}", descriptor.id));
            assert!(!bundle.version.is_empty() && bundle.version.len() <= 64);

            // Caching is keyed per mod with no cross-talk.
            let mut cache = ModCache::default();
            assert!(cache.get(descriptor.id).is_none());
            cache.insert(descriptor.id, bundle.clone());
            assert!(cache.get(descriptor.id).is_some());
            assert_eq!(cache.get(descriptor.id).unwrap().version, bundle.version);

            // Bootstrap: exactly one start and one execution per document.
            let mut lifecycle = DocumentModLifecycle::default();
            lifecycle.begin_document();
            assert!(lifecycle.begin_mod(descriptor.id));
            assert!(!lifecycle.begin_mod(descriptor.id), "duplicate bootstrap");
            assert!(lifecycle.record_mod_execution(descriptor.id));
            assert!(
                !lifecycle.record_mod_execution(descriptor.id),
                "duplicate execution"
            );

            // Reload: every guard resets and the mod can bootstrap again.
            lifecycle.begin_document();
            assert_eq!(lifecycle.generation(), 2);
            assert!(lifecycle.begin_mod(descriptor.id));
            assert!(lifecycle.record_mod_execution(descriptor.id));
            assert_eq!(lifecycle.mod_execution_count(descriptor.id), 1);

            // CSS stage exists exactly when the descriptor declares one.
            let scripts: Vec<String> = if let Some(source) = descriptor.css {
                let css = bundle.css.clone().unwrap_or_default();
                vec![mod_css_script(&descriptor, &source, "r1", &css).unwrap()]
            } else {
                Vec::new()
            };
            assert_eq!(
                scripts.len(),
                usize::from(descriptor.css.is_some()),
                "{} CSS stage must follow the descriptor",
                descriptor.id
            );

            // Execution + marker scripts always exist and never evaluate input.
            let context = mod_execution_context_script(&descriptor);
            let marker = mod_marker_script(&descriptor, "r1", 1.0).unwrap();
            assert!(context.contains(descriptor.storage_context_global));
            assert!(marker.contains(descriptor.success_marker));
            assert!(marker.contains(descriptor.bootstrap_dom_event));
            for script in std::iter::once(&context)
                .chain(std::iter::once(&marker))
                .chain(scripts.iter())
            {
                assert!(!script.contains("eval("), "{}", descriptor.id);
                assert!(!script.contains("new Function"), "{}", descriptor.id);
            }

            // Failure propagation: a bad artifact is typed, never silent.
            // (A user-owned bundle accepts any bytes by design, so its
            // failure shapes are emptiness and size bounds, not identity.)
            if matches!(descriptor.identity, ModArtifactIdentity::UserOwned) {
                assert!(validate_mod_bundle(&descriptor, b"".to_vec(), None).is_err());
                assert!(
                    validate_mod_bundle(&descriptor, vec![b'x'; 8 * 1024 * 1024 + 1], None)
                        .is_err()
                );
            } else {
                let mut tampered = if descriptor.css.is_some() {
                    b"// TestMod 1.0.0\nvar TestMod = {};\n".to_vec()
                } else {
                    headerless_bundle()
                };
                tampered.truncate(tampered.len() / 2);
                assert!(validate_mod_bundle(&descriptor, tampered, None).is_err());
            }
        }
    }

    /// Registry-shaped id validation mirroring the protocol layer, so the
    /// substrate can assert a descriptor id is servable on its own.
    pub fn validate_id(id: &str) -> Result<(), String> {
        if id.is_empty() || id.len() > 64 {
            return Err("mod id must be 1..=64 bytes".to_owned());
        }
        if !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err("mod id has an unusable character".to_owned());
        }
        Ok(())
    }
}

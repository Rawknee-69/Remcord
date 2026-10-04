use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::localization::MAX_LANG_KEY_BYTES;
use crate::plugin_storage::{
    MAX_PLUGIN_FILE_BYTES, MAX_PLUGIN_ID_BYTES, MAX_PLUGIN_REL_PATH_BYTES,
};
use crate::settings::SettingsSnapshot;

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_IPC_MESSAGE_BYTES: usize = 8 * 1024;
pub const MAX_REQUEST_ID_BYTES: usize = 128;
pub const MAX_ERROR_BYTES: usize = 2 * 1024;
pub const MAX_TRACKED_REQUEST_IDS: usize = 4096;
/// Maximum logical mod bundle id length in UTF-8 bytes. Registry ids are
/// short lowercase names (`"vencord"`); the bound mirrors the theme id cap.
pub const MAX_MOD_BUNDLE_ID_BYTES: usize = 64;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeRequest {
    pub version: u8,
    pub request_id: String,
    pub command: BridgeCommand,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum BridgeCommand {
    Ping(PingRequest),
    ModGetBundle(ModGetBundleRequest),
    SettingsGetConfig(SettingsGetConfigRequest),
    SettingsSetBlockPowerSaving(SettingsSetBlockPowerSavingRequest),
    LocalizationGetLang(LocalizationGetLangRequest),
    AppGetVersion(AppGetVersionRequest),
    PluginStorageRead(PluginStorageReadRequest),
    PluginStorageWrite(PluginStorageWriteRequest),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PingRequest {
    pub nonce: String,
}

/// Scoped client-mod bundle request: exactly one logical bundle id. There
/// is deliberately NO URL, path, filename, channel, or executable source:
/// the host resolves the id by exact match against its compile-time known
/// mod registry (`KNOWN_MODS`) and rejects everything else. Any extra
/// payload member is rejected by `deny_unknown_fields`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModGetBundleRequest {
    pub mod_id: String,
}

fn is_mod_bundle_id_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
}

/// Shape validation for a logical bundle id: non-empty, bounded, and
/// confined to `[A-Za-z0-9._-]`. This rejects path separators, drive/UNC
/// syntax, URLs, and `javascript:` shapes at parse time; registry
/// membership (exact match) is enforced at dispatch.
pub fn validate_mod_bundle_id(mod_id: &str) -> Result<(), String> {
    if mod_id.is_empty() {
        return Err("mod bundle id must not be empty".to_owned());
    }
    if mod_id.len() > MAX_MOD_BUNDLE_ID_BYTES {
        return Err("mod bundle id exceeds the size limit".to_owned());
    }
    if !mod_id.bytes().all(is_mod_bundle_id_char) {
        return Err("mod bundle id uses characters outside [A-Za-z0-9._-]".to_owned());
    }
    Ok(())
}

/// Empty request for the read-only `settings.get_config` capability. There
/// are no renderer-supplied parameters by design: no config key, no file
/// path, no selector. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsGetConfigRequest {}

/// The single allowlisted `settings.set_block_power_saving_in_voice_chat`
/// mutation parameter: the new boolean value. There is deliberately NO key,
/// field name, path, JSON patch, or object: the mutated setting is fixed at
/// compile time, so arbitrary renderer input can never select another field.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsSetBlockPowerSavingRequest {
    pub value: bool,
}

/// The single allowed `localization.get_lang` parameter: one catalog key.
/// No locale, file, path, URL, or object may be supplied.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalizationGetLangRequest {
    pub key: String,
}

/// Empty request for the read-only `app.get_version` capability. There are no
/// renderer-supplied parameters by design: no channel, key, path, or
/// selector. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppGetVersionRequest {}

/// Scoped plugin-storage read: a logical plugin id plus a logical relative
/// path. There is deliberately NO absolute path, root selector, URL, drive,
/// or file-handle parameter: the host builds the root itself.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginStorageReadRequest {
    pub plugin_id: String,
    pub relative_path: String,
}

/// Scoped plugin-storage write: the same logical addressing as the read plus
/// UTF-8 text data. No binary envelope, no path, no directory-creation flag.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginStorageWriteRequest {
    pub plugin_id: String,
    pub relative_path: String,
    pub data: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeEventMessage {
    pub version: u8,
    pub event: BridgeEvent,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum BridgeEvent {
    RoundTripAck(RoundTripAck),
    CssProbe(CssProbe),
    RendererError(RendererError),
    BootstrapMetrics(BootstrapMetrics),
    VencordCssResult(VencordCssResult),
    VencordBootstrapResult(VencordBootstrapResult),
    VencordAdapterError(VencordAdapterError),
    VencordResponseAck(VencordResponseAck),
    InteractionProbe(InteractionProbe),
    /// DEV-only diagnostic posted by the settings smoke probe
    /// (`LEGCORD_NATIVE_SETTINGS_SMOKE=1`). Never sent by shipped adapters.
    SettingsProbe(SettingsProbe),
    /// DEV-only diagnostic posted by the settings mutation smoke probe
    /// (`LEGCORD_NATIVE_SETTINGS_MUTATION_SMOKE=1`). Never sent by shipped
    /// adapters.
    SettingsMutationProbe(SettingsMutationProbe),
    /// DEV-only diagnostic posted by the localization smoke probe
    /// (`LEGCORD_NATIVE_GETLANG_SMOKE=1`). Never sent by shipped adapters.
    GetLangProbe(GetLangProbe),
    /// DEV-only diagnostic posted by the app-version smoke probe
    /// (`LEGCORD_NATIVE_APP_VERSION_SMOKE=1`). Never sent by shipped code.
    AppVersionProbe(AppVersionProbe),
    /// DEV-only diagnostic posted by the plugin-storage smoke probe
    /// (`LEGCORD_NATIVE_PLUGIN_STORAGE_SMOKE=1`). Never sent by shipped
    /// code.
    PluginStorageProbe(PluginStorageProbe),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundTripAck {
    pub nonce: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CssProbe {
    pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererError {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapMetrics {
    pub execution_ms: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VencordCssResult {
    pub request_id: String,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VencordBootstrapResult {
    pub request_id: String,
    pub ok: bool,
    pub marker: String,
    pub execution_ms: f64,
    pub local_storage_api: String,
    pub settings_state: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VencordAdapterError {
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VencordResponseAck {
    pub request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionProbe {
    pub phase: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsProbe {
    pub ok: bool,
    pub source: String,
    pub payload_bytes: usize,
    pub block_power_saving: bool,
    pub error: Option<String>,
}

/// DEV-only report for one stage of the settings mutation smoke sequence:
/// `stage` is `"mutated"` (write flipped value, re-read, verify) or
/// `"restored"` (write the original value back, re-read, verify). `before`
/// is the value read before the stage's write, `value` the applied value,
/// and `reread_matches` whether a fresh `getConfig` confirmed it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsMutationProbe {
    pub ok: bool,
    pub stage: String,
    pub before: bool,
    pub value: bool,
    pub reread_matches: bool,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetLangProbe {
    pub ok: bool,
    pub lang: String,
    pub value: String,
    pub missing_is_empty: bool,
    pub error: Option<String>,
}

/// DEV-only report for the app-version smoke probe: `version` is the exact
/// string the migrated update-checker caller compares against the remote
/// release.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppVersionProbe {
    pub ok: bool,
    pub version: String,
    pub error: Option<String>,
}

/// DEV-only report for the plugin-storage smoke probe. `stage` is
/// `"write_read"` (write a fixture, read it back, verify bytes, then block
/// three attack paths). `attacks_blocked` counts the traversal/absolute/
/// drive attempts the facade or host rejected. The probe never carries file
/// contents or host paths.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginStorageProbe {
    pub ok: bool,
    pub stage: String,
    pub payload_bytes: usize,
    pub round_trip: bool,
    pub attacks_blocked: u8,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BridgeMessage {
    Request(BridgeRequest),
    Event(BridgeEventMessage),
}

#[derive(Serialize)]
pub struct BridgeResponse {
    pub version: u8,
    pub request_id: String,
    pub ok: bool,
    pub payload: Option<BridgeResponsePayload>,
    pub error: Option<BridgeError>,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum BridgeResponsePayload {
    Ping(PingResponse),
    ModBundle(ModBundleResponse),
    SettingsConfig(Box<SettingsConfigResponse>),
    SettingsMutation(SettingsMutationResponse),
    LocalizationString(LocalizationStringResponse),
    AppVersion(AppVersionResponse),
    PluginStorageRead(PluginStorageReadResponse),
    PluginStorageWrite(PluginStorageWriteResponse),
}

#[derive(Serialize)]
pub struct BridgeError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Serialize)]
pub struct PingResponse {
    pub nonce: String,
    pub message: &'static str,
}

#[derive(Default)]
pub struct RequestTracker {
    seen: HashSet<String>,
    in_flight: HashSet<String>,
}

/// Validated bundle metadata response for the scoped `mod.get_bundle`
/// capability. Carries ONLY what the substrate pipeline needs: the registry
/// id echo (so the adapter can confirm identity), the validated build
/// version, byte sizes, timing measurements, the descriptor's success
/// marker, and the per-document execution count. There is deliberately NO
/// bundle source, URL, path, filename, cache layout, or executable content:
/// the multi-hundred-kilobyte sources travel host-side only (disk cache +
/// page-world execution) and never cross the bridge.
#[derive(Serialize)]
pub struct ModBundleResponse {
    pub mod_id: &'static str,
    pub bundle_version: String,
    pub bundle_bytes: usize,
    pub css_bytes: usize,
    pub request_latency_ms: f64,
    pub retrieval_ms: f64,
    pub execution_ms: f64,
    pub css_injection_ms: f64,
    pub marker: &'static str,
    pub document_execution_count: u8,
}

/// Read-only validated settings snapshot response. `source` is `"file"` when
/// the snapshot came from `<profile>/storage/settings.json` and `"defaults"`
/// when no settings file exists.
#[derive(Serialize)]
pub struct SettingsConfigResponse {
    pub settings: SettingsSnapshot,
    pub source: &'static str,
    pub payload_bytes: usize,
}

/// Validated response for the allowlisted `blockPowerSavingInVoiceChat`
/// mutation. Carries the applied value (so the renderer can update local
/// state instead of holding stale data), the persistence source (always
/// `"file"` after a successful write), and the persisted file size.
/// Electron's synchronous setter returns `undefined`; the async native
/// caller needs the applied value back, hence this minimal shape instead of
/// a full snapshot.
#[derive(Serialize)]
pub struct SettingsMutationResponse {
    pub block_power_saving_in_voice_chat: bool,
    pub source: &'static str,
    pub payload_bytes: usize,
}

/// Single localized string response. `value` is `""` for a missing key
/// (Electron parity); `lang` is the resolved language selection.
#[derive(Serialize)]
pub struct LocalizationStringResponse {
    pub value: String,
    pub lang: String,
}

/// Single application version response. `version` is the exact string
/// Electron serves for `window.legcord.version` (`"0.0.0"` on dev builds,
/// otherwise the package version).
#[derive(Serialize)]
pub struct AppVersionResponse {
    pub version: String,
}

/// Scoped plugin-storage read response: the file text plus its byte length.
/// No path, no metadata, no directory listing.
#[derive(Serialize)]
pub struct PluginStorageReadResponse {
    pub data: String,
    pub payload_bytes: usize,
}

/// Scoped plugin-storage write response: the persisted byte length. The
/// envelope `ok` already signals success; no path is echoed back.
#[derive(Serialize)]
pub struct PluginStorageWriteResponse {
    pub payload_bytes: usize,
}

fn valid_request_id(request_id: &str) -> bool {
    !request_id.is_empty()
        && request_id.len() <= MAX_REQUEST_ID_BYTES
        && request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

impl RequestTracker {
    pub fn begin(&mut self, request_id: &str) -> Result<(), &'static str> {
        if self.seen.contains(request_id) {
            return Err("duplicate request id");
        }
        if self.seen.len() >= MAX_TRACKED_REQUEST_IDS {
            return Err("request id limit reached for this document");
        }

        self.seen.insert(request_id.to_owned());
        self.in_flight.insert(request_id.to_owned());
        Ok(())
    }

    pub fn complete(&mut self, request_id: &str) -> Result<(), &'static str> {
        if self.in_flight.remove(request_id) {
            Ok(())
        } else {
            Err("response has no matching request")
        }
    }

    pub fn clear(&mut self) {
        self.seen.clear();
        self.in_flight.clear();
    }
}

pub fn parse_bridge_message(body: &str) -> Result<BridgeMessage, String> {
    if body.len() > MAX_IPC_MESSAGE_BYTES {
        return Err("message exceeds the size limit".to_owned());
    }

    let message: BridgeMessage = serde_json::from_str(body).map_err(|error| error.to_string())?;
    match &message {
        BridgeMessage::Request(request) => {
            if request.version != PROTOCOL_VERSION {
                return Err("unsupported bridge version".to_owned());
            }
            if !valid_request_id(&request.request_id) {
                return Err("invalid request id".to_owned());
            }
            match &request.command {
                BridgeCommand::Ping(payload) => {
                    if payload.nonce.is_empty() || payload.nonce.len() > 128 {
                        return Err("invalid nonce".to_owned());
                    }
                }
                BridgeCommand::ModGetBundle(payload) => {
                    // Shape check only (presence + byte cap + charset);
                    // registry membership (exact match against KNOWN_MODS)
                    // is enforced at dispatch.
                    if validate_mod_bundle_id(&payload.mod_id).is_err() {
                        return Err("invalid mod bundle id".to_owned());
                    }
                }
                BridgeCommand::SettingsGetConfig(_) => {}
                // The mutation value is a plain bool: serde already rejected
                // non-boolean, missing, or extra payload members via
                // `deny_unknown_fields`, so no further shape check is needed.
                BridgeCommand::SettingsSetBlockPowerSaving(_) => {}
                BridgeCommand::LocalizationGetLang(payload) => {
                    if payload.key.is_empty() || payload.key.len() > MAX_LANG_KEY_BYTES {
                        return Err("invalid localization key".to_owned());
                    }
                }
                // Parameter-free by protocol (`deny_unknown_fields` already
                // rejected any renderer-supplied member), so no shape check
                // is needed.
                BridgeCommand::AppGetVersion(_) => {}
                BridgeCommand::PluginStorageRead(payload) => {
                    // Shape check only (presence + byte caps); charset,
                    // traversal, drive/UNC, and containment rules live in
                    // `plugin_storage` and are re-enforced at dispatch.
                    if payload.plugin_id.is_empty()
                        || payload.plugin_id.len() > MAX_PLUGIN_ID_BYTES
                        || payload.relative_path.is_empty()
                        || payload.relative_path.len() > MAX_PLUGIN_REL_PATH_BYTES
                    {
                        return Err("invalid plugin storage read".to_owned());
                    }
                }
                BridgeCommand::PluginStorageWrite(payload) => {
                    if payload.plugin_id.is_empty()
                        || payload.plugin_id.len() > MAX_PLUGIN_ID_BYTES
                        || payload.relative_path.is_empty()
                        || payload.relative_path.len() > MAX_PLUGIN_REL_PATH_BYTES
                        || payload.data.len() > MAX_PLUGIN_FILE_BYTES
                    {
                        return Err("invalid plugin storage write".to_owned());
                    }
                }
            }
        }
        BridgeMessage::Event(event) => {
            if event.version != PROTOCOL_VERSION {
                return Err("unsupported bridge version".to_owned());
            }
            match &event.event {
                BridgeEvent::RoundTripAck(payload) => {
                    if payload.nonce.is_empty() || payload.nonce.len() > 128 {
                        return Err("invalid round-trip acknowledgement".to_owned());
                    }
                }
                BridgeEvent::CssProbe(payload) => {
                    if payload.value.len() > 128 {
                        return Err("CSS probe result exceeds the size limit".to_owned());
                    }
                }
                BridgeEvent::RendererError(payload) => {
                    if payload.kind.is_empty()
                        || payload.kind.len() > 64
                        || payload.message.len() > MAX_ERROR_BYTES
                    {
                        return Err("invalid renderer error report".to_owned());
                    }
                }
                BridgeEvent::BootstrapMetrics(payload) => {
                    if !payload.execution_ms.is_finite()
                        || !(0.0..=60_000.0).contains(&payload.execution_ms)
                    {
                        return Err("invalid bootstrap execution time".to_owned());
                    }
                }
                BridgeEvent::VencordCssResult(payload) => {
                    if !valid_request_id(&payload.request_id)
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid Vencord CSS result".to_owned());
                    }
                }
                BridgeEvent::VencordBootstrapResult(payload) => {
                    if !valid_request_id(&payload.request_id)
                        || payload.marker.len() > 64
                        || !payload.execution_ms.is_finite()
                        || !(0.0..=60_000.0).contains(&payload.execution_ms)
                        || !matches!(
                            payload.local_storage_api.as_str(),
                            "available" | "missing" | "blocked"
                        )
                        || !matches!(
                            payload.settings_state.as_str(),
                            "missing"
                                | "valid_json"
                                | "invalid_json"
                                | "read_error"
                                | "storage_unavailable"
                        )
                        || !matches!(
                            payload.local_storage_api.as_str(),
                            "available" | "missing" | "blocked"
                        )
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid Vencord bootstrap result".to_owned());
                    }
                }
                BridgeEvent::VencordAdapterError(payload) => {
                    if payload.message.is_empty() || payload.message.len() > MAX_ERROR_BYTES {
                        return Err("invalid Vencord adapter error".to_owned());
                    }
                }
                BridgeEvent::VencordResponseAck(payload) => {
                    if !valid_request_id(&payload.request_id) {
                        return Err("invalid Vencord response acknowledgement".to_owned());
                    }
                }
                BridgeEvent::InteractionProbe(payload) => {
                    if !matches!(
                        payload.phase.as_str(),
                        "channel_missing"
                            | "channel_navigation_requested"
                            | "composer_missing"
                            | "composer_not_empty"
                            | "focus_failed"
                            | "composer_focused"
                            | "input_observed"
                            | "character_erased"
                    ) {
                        return Err("invalid interaction diagnostic phase".to_owned());
                    }
                }
                BridgeEvent::SettingsProbe(payload) => {
                    if !matches!(
                        payload.source.as_str(),
                        // "unknown" is renderer-observed (the facade resolves the
                        // settings object only); the authoritative source is in
                        // the host `[Settings]` log for the same request.
                        "file" | "defaults" | "unavailable" | "unknown"
                    ) || payload.payload_bytes > 1024 * 1024
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid settings probe result".to_owned());
                    }
                }
                BridgeEvent::SettingsMutationProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "mutated" | "restored")
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid settings mutation probe result".to_owned());
                    }
                }
                BridgeEvent::GetLangProbe(payload) => {
                    if payload.lang.len() > crate::localization::MAX_LANG_CODE_BYTES
                        || payload.value.len() > crate::localization::MAX_LANG_VALUE_BYTES
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid localization probe result".to_owned());
                    }
                }
                BridgeEvent::AppVersionProbe(payload) => {
                    if payload.version.len() > crate::app::MAX_APP_VERSION_BYTES
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                    {
                        return Err("invalid app version probe result".to_owned());
                    }
                }
                BridgeEvent::PluginStorageProbe(payload) => {
                    if !matches!(payload.stage.as_str(), "write_read")
                        || payload.payload_bytes > MAX_PLUGIN_FILE_BYTES
                        || payload.attacks_blocked > 8
                        || payload
                            .error
                            .as_ref()
                            .is_some_and(|error| error.len() > MAX_ERROR_BYTES)
                        || payload.ok == payload.error.is_some()
                        || (payload.ok && (!payload.round_trip || payload.attacks_blocked != 3))
                    {
                        return Err("invalid plugin storage probe result".to_owned());
                    }
                }
            }
        }
    }

    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::{
        parse_bridge_message, validate_mod_bundle_id, BridgeCommand, BridgeEvent, BridgeMessage,
        BridgeRequest, BridgeResponse, BridgeResponsePayload, ModBundleResponse, RequestTracker,
        MAX_MOD_BUNDLE_ID_BYTES, PROTOCOL_VERSION,
    };

    #[test]
    fn accepts_a_valid_typed_command() {
        let message = r#"{"version":1,"request_id":"test-1","command":{"type":"ping","payload":{"nonce":"hello"}}}"#;
        assert!(matches!(
            parse_bridge_message(message),
            Ok(BridgeMessage::Request(_))
        ));
    }

    #[test]
    fn rejects_unknown_commands_and_extra_payload_fields() {
        let unknown = r#"{"version":1,"request_id":"test-1","command":{"type":"invoke","payload":{"method":"anything"}}}"#;
        let extra = r#"{"version":1,"request_id":"test-1","command":{"type":"ping","payload":{"nonce":"hello","host_api":"filesystem"}}}"#;
        assert!(parse_bridge_message(unknown).is_err());
        assert!(parse_bridge_message(extra).is_err());
    }

    #[test]
    fn rejects_malformed_payloads_and_wrong_versions() {
        let malformed = r#"{"version":1,"request_id":"test-1","command":{"type":"ping","payload":{"nonce":false}}}"#;
        let wrong_version = r#"{"version":2,"request_id":"test-1","command":{"type":"ping","payload":{"nonce":"hello"}}}"#;
        assert!(parse_bridge_message(malformed).is_err());
        assert!(parse_bridge_message(wrong_version).is_err());
    }

    #[test]
    fn accepts_a_scoped_bundle_id_and_rejects_renderer_source_parameters() {
        let request = r#"{"version":1,"request_id":"vencord-1","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord"}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(BridgeRequest {
                command: BridgeCommand::ModGetBundle(_),
                ..
            }))
        ));

        // A URL, path, filename, or executable-source member is never a
        // valid member of this command: it cannot select what the host reads.
        let url = r#"{"version":1,"request_id":"vencord-2","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord","url":"https://attacker.invalid/bundle.js"}}}"#;
        let path = r#"{"version":1,"request_id":"vencord-3","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord","path":"C:\\secret"}}}"#;
        let file = r#"{"version":1,"request_id":"vencord-4","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord","file":"vencord.js"}}}"#;
        let source = r#"{"version":1,"request_id":"vencord-5","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord","source":"globalThis.x=1"}}}"#;
        let channel = r#"{"version":1,"request_id":"vencord-6","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord","channel":"getVencordBundle"}}}"#;
        let generic = r#"{"version":1,"request_id":"vencord-7","command":{"type":"mod_get_bundle","payload":{"mod_id":"vencord","method":"readFile"}}}"#;
        for message in [url, path, file, source, channel, generic] {
            assert!(
                parse_bridge_message(message).is_err(),
                "must reject {message}"
            );
        }
    }

    #[test]
    fn rejects_bundle_ids_that_could_escape_the_registry() {
        // Path, URL, and scheme shapes are rejected at parse time by the
        // charset bound, so no id can become a filesystem or URL selector.
        for hostile in [
            r#"{"version":1,"request_id":"m-1","command":{"type":"mod_get_bundle","payload":{"mod_id":"../vencord"}}}"#,
            r#"{"version":1,"request_id":"m-2","command":{"type":"mod_get_bundle","payload":{"mod_id":"..\\vencord"}}}"#,
            r#"{"version":1,"request_id":"m-3","command":{"type":"mod_get_bundle","payload":{"mod_id":"C:\\vencord.js"}}}"#,
            r#"{"version":1,"request_id":"m-4","command":{"type":"mod_get_bundle","payload":{"mod_id":"//server/share/vencord.js"}}}"#,
            r#"{"version":1,"request_id":"m-5","command":{"type":"mod_get_bundle","payload":{"mod_id":"https://attacker.invalid/vencord.js"}}}"#,
            r#"{"version":1,"request_id":"m-6","command":{"type":"mod_get_bundle","payload":{"mod_id":"javascript:alert(1)"}}}"#,
            r#"{"version":1,"request_id":"m-7","command":{"type":"mod_get_bundle","payload":{"mod_id":""}}}"#,
            r#"{"version":1,"request_id":"m-9","command":{"type":"mod_get_bundle","payload":{"mod_id":"ven cord"}}}"#,
        ] {
            assert!(
                parse_bridge_message(hostile).is_err(),
                "must reject {hostile}"
            );
        }
        // Charset-valid but UNREGISTERED ids parse here and are rejected at
        // dispatch with UNKNOWN_MOD: exact registry match only, so a file
        // name or an unregistered mod can never become a source selector.
        for unregistered in ["shelter", "vencord.js", "equicord", "custom", "Vencord"] {
            let message = format!(
                r#"{{"version":1,"request_id":"m-x","command":{{"type":"mod_get_bundle","payload":{{"mod_id":"{unregistered}"}}}}}}"#
            );
            assert!(
                matches!(
                    parse_bridge_message(&message),
                    Ok(BridgeMessage::Request(BridgeRequest {
                        command: BridgeCommand::ModGetBundle(_),
                        ..
                    }))
                ),
                "{unregistered} must parse and be rejected by the host registry"
            );
            assert!(validate_mod_bundle_id(unregistered).is_ok());
        }
    }

    #[test]
    fn validates_bundle_id_bounds_and_charset() {
        assert!(validate_mod_bundle_id("vencord").is_ok());
        assert!(validate_mod_bundle_id("my.mod_2").is_ok());
        assert!(validate_mod_bundle_id("").is_err());
        assert!(validate_mod_bundle_id(&"a".repeat(MAX_MOD_BUNDLE_ID_BYTES + 1)).is_err());
        assert!(validate_mod_bundle_id("../x").is_err());
        assert!(validate_mod_bundle_id("x/y").is_err());
        assert!(validate_mod_bundle_id("x\\y").is_err());
        assert!(validate_mod_bundle_id("https://a.invalid/x").is_err());
        assert!(validate_mod_bundle_id("javascript:alert(1)").is_err());
    }

    #[test]
    fn accepts_renderer_error_event() {
        let error = r#"{"version":1,"event":{"type":"renderer_error","payload":{"kind":"uncaught","message":"failed"}}}"#;
        assert!(matches!(
            parse_bridge_message(error),
            Ok(BridgeMessage::Event(ref message))
                if matches!(&message.event, BridgeEvent::RendererError(_))
        ));
    }

    #[test]
    fn validates_interaction_diagnostic_phases() {
        let focused = r#"{"version":1,"event":{"type":"interaction_probe","payload":{"phase":"composer_focused"}}}"#;
        let erased = r#"{"version":1,"event":{"type":"interaction_probe","payload":{"phase":"character_erased"}}}"#;
        let invalid = r#"{"version":1,"event":{"type":"interaction_probe","payload":{"phase":"send_message"}}}"#;
        assert!(matches!(
            parse_bridge_message(focused),
            Ok(BridgeMessage::Event(ref message))
                if matches!(&message.event, BridgeEvent::InteractionProbe(_))
        ));
        assert!(matches!(
            parse_bridge_message(erased),
            Ok(BridgeMessage::Event(ref message))
                if matches!(&message.event, BridgeEvent::InteractionProbe(_))
        ));
        assert!(parse_bridge_message(invalid).is_err());
    }

    #[test]
    fn validates_vencord_completion_events() {
        let css = r#"{"version":1,"event":{"type":"vencord_css_result","payload":{"request_id":"vencord-1","ok":true,"error":null}}}"#;
        let bootstrap = r#"{"version":1,"event":{"type":"vencord_bootstrap_result","payload":{"request_id":"vencord-1","ok":true,"marker":"vencord-bootstrap-executed","execution_ms":12.5,"local_storage_api":"available","settings_state":"missing","error":null}}}"#;
        let malformed = r#"{"version":1,"event":{"type":"vencord_bootstrap_result","payload":{"request_id":"vencord-1","ok":true,"marker":"vencord-bootstrap-executed","execution_ms":-1,"local_storage_api":"other","settings_state":"missing","error":null}}}"#;
        assert!(matches!(
            parse_bridge_message(css),
            Ok(BridgeMessage::Event(ref message))
                if matches!(&message.event, BridgeEvent::VencordCssResult(_))
        ));
        assert!(matches!(
            parse_bridge_message(bootstrap),
            Ok(BridgeMessage::Event(ref message))
                if matches!(&message.event, BridgeEvent::VencordBootstrapResult(_))
        ));
        assert!(parse_bridge_message(malformed).is_err());
    }

    #[test]
    fn serializes_a_typed_bundle_response_without_bundle_source() {
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "vencord-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::ModBundle(ModBundleResponse {
                mod_id: "vencord",
                bundle_version: "test-build".to_owned(),
                bundle_bytes: 1024,
                css_bytes: 256,
                request_latency_ms: 20.0,
                retrieval_ms: 10.0,
                execution_ms: 5.0,
                css_injection_ms: 2.0,
                marker: "vencord-bootstrap-executed",
                document_execution_count: 1,
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "mod_bundle");
        assert_eq!(value["payload"]["payload"]["mod_id"], "vencord");
        assert_eq!(
            value["payload"]["payload"]["marker"],
            "vencord-bootstrap-executed"
        );
        assert_eq!(value["payload"]["payload"]["document_execution_count"], 1);
        // Metadata only: no source, no URL, no cache path, no layout.
        assert!(value["payload"]["payload"].get("js").is_none());
        assert!(value["payload"]["payload"].get("css").is_none());
        assert!(value["payload"]["payload"].get("url").is_none());
        assert!(value["payload"]["payload"].get("path").is_none());
        assert!(value["payload"]["payload"].get("js_url").is_none());
        assert!(value["payload"]["payload"].get("js_file").is_none());
        assert!(value["payload"]["payload"].get("source").is_none());
        assert!(value["payload"]["payload"].get("profile").is_none());
        assert_eq!(
            value["payload"]["payload"]
                .as_object()
                .map(|object| object.len()),
            Some(10)
        );
    }

    #[test]
    fn correlates_responses_and_rejects_unknown_or_duplicate_ids() {
        let mut tracker = RequestTracker::default();
        assert!(tracker.begin("request-1").is_ok());
        assert!(tracker.begin("request-1").is_err());
        assert!(tracker.complete("request-unknown").is_err());
        assert!(tracker.complete("request-1").is_ok());
        assert!(tracker.complete("request-1").is_err());
    }

    #[test]
    fn accepts_the_settings_capability_without_renderer_parameters() {
        let request = r#"{"version":1,"request_id":"settings-1","command":{"type":"settings_get_config","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        // No config key, filesystem path, or selector may be supplied.
        let with_key = r#"{"version":1,"request_id":"settings-2","command":{"type":"settings_get_config","payload":{"key":"mods"}}}"#;
        let with_path = r#"{"version":1,"request_id":"settings-3","command":{"type":"settings_get_config","payload":{"path":"C:\\secret"}}}"#;
        assert!(parse_bridge_message(with_key).is_err());
        assert!(parse_bridge_message(with_path).is_err());
    }

    #[test]
    fn rejects_settings_mutation_and_generic_invocation() {
        let set_config = r#"{"version":1,"request_id":"settings-1","command":{"type":"settings_set_config","payload":{}}}"#;
        let invoke = r#"{"version":1,"request_id":"settings-2","command":{"type":"invoke","payload":{"method":"settings.getConfig"}}}"#;
        assert!(parse_bridge_message(set_config).is_err());
        assert!(parse_bridge_message(invoke).is_err());
    }

    #[test]
    fn accepts_only_the_allowlisted_block_power_saving_mutation() {
        let valid = r#"{"version":1,"request_id":"settings-1","command":{"type":"settings_set_block_power_saving","payload":{"value":true}}}"#;
        assert!(matches!(
            parse_bridge_message(valid),
            Ok(BridgeMessage::Request(_))
        ));

        // Wrong value types are rejected before dispatch.
        let string_value = r#"{"version":1,"request_id":"settings-2","command":{"type":"settings_set_block_power_saving","payload":{"value":"yes"}}}"#;
        let null_value = r#"{"version":1,"request_id":"settings-3","command":{"type":"settings_set_block_power_saving","payload":{"value":null}}}"#;
        let int_value = r#"{"version":1,"request_id":"settings-4","command":{"type":"settings_set_block_power_saving","payload":{"value":1}}}"#;
        let missing_value = r#"{"version":1,"request_id":"settings-5","command":{"type":"settings_set_block_power_saving","payload":{}}}"#;
        assert!(parse_bridge_message(string_value).is_err());
        assert!(parse_bridge_message(null_value).is_err());
        assert!(parse_bridge_message(int_value).is_err());
        assert!(parse_bridge_message(missing_value).is_err());
    }

    #[test]
    fn rejects_unauthorized_mutation_shapes() {
        // No key, field name, path, JSON patch, or object may select a
        // different setting: the mutated field is fixed at compile time.
        let with_key = r#"{"version":1,"request_id":"settings-1","command":{"type":"settings_set_block_power_saving","payload":{"value":true,"key":"minimizeToTray"}}}"#;
        let with_field = r#"{"version":1,"request_id":"settings-2","command":{"type":"settings_set_block_power_saving","payload":{"value":true,"field":"tray"}}}"#;
        let with_path = r#"{"version":1,"request_id":"settings-3","command":{"type":"settings_set_block_power_saving","payload":{"value":true,"path":"C:\\secret"}}}"#;
        let with_object = r#"{"version":1,"request_id":"settings-4","command":{"type":"settings_set_block_power_saving","payload":{"settings":{"tray":"disabled"}}}}}"#;
        assert!(parse_bridge_message(with_key).is_err());
        assert!(parse_bridge_message(with_field).is_err());
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_object).is_err());
    }

    #[test]
    fn serializes_a_typed_settings_mutation_response() {
        use super::SettingsMutationResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "settings-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::SettingsMutation(
                SettingsMutationResponse {
                    block_power_saving_in_voice_chat: true,
                    source: "file",
                    payload_bytes: 2048,
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "settings_mutation");
        assert_eq!(
            value["payload"]["payload"]["block_power_saving_in_voice_chat"],
            true
        );
        assert_eq!(value["payload"]["payload"]["source"], "file");
        // The response carries only the applied field, never a key/path.
        assert!(value["payload"]["payload"].get("key").is_none());
        assert!(value["payload"]["payload"].get("path").is_none());
    }

    #[test]
    fn validates_settings_mutation_probe_diagnostics() {
        let mutated = r#"{"version":1,"event":{"type":"settings_mutation_probe","payload":{"ok":true,"stage":"mutated","before":false,"value":true,"reread_matches":true,"error":null}}}"#;
        let restored = r#"{"version":1,"event":{"type":"settings_mutation_probe","payload":{"ok":true,"stage":"restored","before":true,"value":false,"reread_matches":true,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"settings_mutation_probe","payload":{"ok":false,"stage":"mutated","before":false,"value":false,"reread_matches":false,"error":"timeout"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"settings_mutation_probe","payload":{"ok":true,"stage":"deleted","before":false,"value":true,"reread_matches":true,"error":null}}}"#;
        let inconsistent = r#"{"version":1,"event":{"type":"settings_mutation_probe","payload":{"ok":true,"stage":"mutated","before":false,"value":true,"reread_matches":true,"error":"extra"}}}"#;
        assert!(parse_bridge_message(mutated).is_ok());
        assert!(parse_bridge_message(restored).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
    }

    #[test]
    fn mutation_request_ids_reset_with_the_document() {
        let mut tracker = RequestTracker::default();
        assert!(tracker.begin("settings-mutate-1").is_ok());
        // A duplicate within one document is rejected (stale/double-submit).
        assert!(tracker.begin("settings-mutate-1").is_err());
        tracker.clear();
        assert!(tracker.begin("settings-mutate-1").is_ok());
        assert!(tracker.complete("settings-mutate-1").is_ok());
    }
    #[test]
    fn serializes_a_typed_settings_snapshot_response() {
        use super::SettingsConfigResponse;
        use crate::settings::SettingsSnapshot;
        let snapshot = SettingsSnapshot::default();
        let payload_bytes = serde_json::to_string(&snapshot).unwrap().len();
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "settings-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::SettingsConfig(Box::new(
                SettingsConfigResponse {
                    settings: snapshot,
                    source: "defaults",
                    payload_bytes,
                },
            ))),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "settings_config");
        assert_eq!(
            value["payload"]["payload"]["settings"]["blockPowerSavingInVoiceChat"],
            false
        );
        assert_eq!(value["payload"]["payload"]["source"], "defaults");
    }

    #[test]
    fn settings_request_ids_reset_with_the_document() {
        let mut tracker = RequestTracker::default();
        assert!(tracker.begin("settings-1").is_ok());
        tracker.clear();
        // After a document reset the same id shape is usable again and stale
        // completions no longer correlate.
        assert!(tracker.begin("settings-1").is_ok());
        assert!(tracker.complete("settings-1").is_ok());
    }

    #[test]
    fn validates_settings_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"settings_probe","payload":{"ok":true,"source":"file","payload_bytes":2048,"block_power_saving":false,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"settings_probe","payload":{"ok":false,"source":"unavailable","payload_bytes":0,"block_power_saving":false,"error":"timeout"}}}"#;
        let bad_source = r#"{"version":1,"event":{"type":"settings_probe","payload":{"ok":true,"source":"electron","payload_bytes":10,"block_power_saving":false,"error":null}}}"#;
        let inconsistent = r#"{"version":1,"event":{"type":"settings_probe","payload":{"ok":true,"source":"file","payload_bytes":10,"block_power_saving":false,"error":"extra"}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_source).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
    }

    #[test]
    fn accepts_a_valid_get_lang_request() {
        let message = r#"{"version":1,"request_id":"lang-1","command":{"type":"localization_get_lang","payload":{"key":"invite-goBackToApp"}}}"#;
        assert!(matches!(
            parse_bridge_message(message),
            Ok(BridgeMessage::Request(_))
        ));
    }

    #[test]
    fn rejects_empty_and_oversized_localization_keys() {
        let empty = r#"{"version":1,"request_id":"lang-1","command":{"type":"localization_get_lang","payload":{"key":""}}}"#;
        let oversized = format!(
            r#"{{"version":1,"request_id":"lang-2","command":{{"type":"localization_get_lang","payload":{{"key":"{}"}}}}}}"#,
            "k".repeat(129)
        );
        assert!(parse_bridge_message(empty).is_err());
        assert!(parse_bridge_message(&oversized).is_err());
    }

    #[test]
    fn rejects_non_key_localization_parameters() {
        let numeric = r#"{"version":1,"request_id":"lang-1","command":{"type":"localization_get_lang","payload":{"key":42}}}"#;
        let with_path = r#"{"version":1,"request_id":"lang-2","command":{"type":"localization_get_lang","payload":{"key":"a","path":"C:\\secret"}}}"#;
        let with_locale = r#"{"version":1,"request_id":"lang-3","command":{"type":"localization_get_lang","payload":{"key":"a","locale":"en-US"}}}"#;
        assert!(parse_bridge_message(numeric).is_err());
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_locale).is_err());
    }

    #[test]
    fn serializes_a_typed_localization_response() {
        use super::LocalizationStringResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "lang-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::LocalizationString(
                LocalizationStringResponse {
                    value: "Go back to the Discord app".to_owned(),
                    lang: "en-US".to_owned(),
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "localization_string");
        assert_eq!(
            value["payload"]["payload"]["value"],
            "Go back to the Discord app"
        );
        assert_eq!(value["payload"]["payload"]["lang"], "en-US");
    }

    #[test]
    fn localization_request_ids_reset_with_the_document() {
        let mut tracker = RequestTracker::default();
        assert!(tracker.begin("lang-1").is_ok());
        tracker.clear();
        assert!(tracker.begin("lang-1").is_ok());
        assert!(tracker.complete("lang-1").is_ok());
    }

    #[test]
    fn validates_localization_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"get_lang_probe","payload":{"ok":true,"lang":"en-US","value":"Go back to the Discord app","missing_is_empty":true,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"get_lang_probe","payload":{"ok":false,"lang":"en-US","value":"","missing_is_empty":false,"error":"timeout"}}}"#;
        let oversized = format!(
            r#"{{"version":1,"event":{{"type":"get_lang_probe","payload":{{"ok":true,"lang":"en-US","value":"{}","missing_is_empty":true,"error":null}}}}}}"#,
            "v".repeat(5000)
        );
        let inconsistent = r#"{"version":1,"event":{"type":"get_lang_probe","payload":{"ok":true,"lang":"en-US","value":"x","missing_is_empty":true,"error":"extra"}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(&oversized).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
    }

    #[test]
    fn accepts_the_app_version_capability_without_renderer_parameters() {
        let request = r#"{"version":1,"request_id":"app-1","command":{"type":"app_get_version","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        // No channel, key, path, or selector may be supplied.
        let with_channel = r#"{"version":1,"request_id":"app-2","command":{"type":"app_get_version","payload":{"channel":"get-app-version"}}}"#;
        let with_key = r#"{"version":1,"request_id":"app-3","command":{"type":"app_get_version","payload":{"key":"version"}}}"#;
        let with_path = r#"{"version":1,"request_id":"app-4","command":{"type":"app_get_version","payload":{"path":"C:\\secret"}}}"#;
        assert!(parse_bridge_message(with_channel).is_err());
        assert!(parse_bridge_message(with_key).is_err());
        assert!(parse_bridge_message(with_path).is_err());
    }

    #[test]
    fn rejects_generic_invocation_for_the_app_version() {
        let invoke = r#"{"version":1,"request_id":"app-1","command":{"type":"invoke","payload":{"method":"app.getVersion"}}}"#;
        let version_channel = r#"{"version":1,"request_id":"app-2","command":{"type":"get-app-version","payload":{}}}"#;
        assert!(parse_bridge_message(invoke).is_err());
        assert!(parse_bridge_message(version_channel).is_err());
    }

    #[test]
    fn serializes_a_typed_app_version_response() {
        use super::AppVersionResponse;
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "app-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::AppVersion(AppVersionResponse {
                version: "1.3.0".to_owned(),
            })),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "app_version");
        assert_eq!(value["payload"]["payload"]["version"], "1.3.0");
        // The response carries only the version string, never a
        // channel/key/path selector.
        assert!(value["payload"]["payload"].get("key").is_none());
        assert!(value["payload"]["payload"].get("path").is_none());
    }

    #[test]
    fn app_version_request_ids_reset_with_the_document() {
        let mut tracker = RequestTracker::default();
        assert!(tracker.begin("app-1").is_ok());
        // A duplicate within one document is rejected (stale/double-submit).
        assert!(tracker.begin("app-1").is_err());
        tracker.clear();
        assert!(tracker.begin("app-1").is_ok());
        assert!(tracker.complete("app-1").is_ok());
    }

    #[test]
    fn validates_app_version_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"app_version_probe","payload":{"ok":true,"version":"1.3.0","error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"app_version_probe","payload":{"ok":false,"version":"","error":"timeout"}}}"#;
        let oversized = format!(
            r#"{{"version":1,"event":{{"type":"app_version_probe","payload":{{"ok":true,"version":"{}","error":null}}}}}}"#,
            "1".repeat(65)
        );
        let inconsistent = r#"{"version":1,"event":{"type":"app_version_probe","payload":{"ok":true,"version":"1.3.0","error":"extra"}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(&oversized).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
    }

    #[test]
    fn accepts_scoped_plugin_storage_requests() {
        let read = r#"{"version":1,"request_id":"ps-1","command":{"type":"plugin_storage_read","payload":{"plugin_id":"my-plugin","relative_path":"cache/note.txt"}}}"#;
        let write = r#"{"version":1,"request_id":"ps-2","command":{"type":"plugin_storage_write","payload":{"plugin_id":"my-plugin","relative_path":"cache/note.txt","data":"hello"}}}"#;
        assert!(matches!(
            parse_bridge_message(read),
            Ok(BridgeMessage::Request(_))
        ));
        assert!(matches!(
            parse_bridge_message(write),
            Ok(BridgeMessage::Request(_))
        ));
    }

    #[test]
    fn rejects_generic_filesystem_shapes_for_plugin_storage() {
        // No absolute path, URL, channel, key, or generic invoke surface.
        let absolute = r#"{"version":1,"request_id":"ps-1","command":{"type":"plugin_storage_read","payload":{"plugin_id":"p","relative_path":"C:\\secret"}}}"#;
        // Shape-level: absolute-looking input passes the parser byte check
        // (strict traversal rejection lives in `plugin_storage`); unknown
        // members must fail here.
        assert!(parse_bridge_message(absolute).is_ok());
        let with_path = r#"{"version":1,"request_id":"ps-2","command":{"type":"plugin_storage_read","payload":{"plugin_id":"p","relative_path":"a.txt","path":"C:\\secret"}}}"#;
        let with_url = r#"{"version":1,"request_id":"ps-3","command":{"type":"plugin_storage_write","payload":{"plugin_id":"p","relative_path":"a.txt","data":"x","url":"https://attacker.invalid"}}}"#;
        let invoke = r#"{"version":1,"request_id":"ps-4","command":{"type":"invoke","payload":{"method":"fs.readFile"}}}"#;
        let fs_read = r#"{"version":1,"request_id":"ps-5","command":{"type":"fs_read_file","payload":{"path":"C:\\secret"}}}"#;
        assert!(parse_bridge_message(with_path).is_err());
        assert!(parse_bridge_message(with_url).is_err());
        assert!(parse_bridge_message(invoke).is_err());
        assert!(parse_bridge_message(fs_read).is_err());
    }

    #[test]
    fn rejects_malformed_plugin_storage_payloads() {
        let empty_id = r#"{"version":1,"request_id":"ps-1","command":{"type":"plugin_storage_read","payload":{"plugin_id":"","relative_path":"a.txt"}}}"#;
        let empty_path = r#"{"version":1,"request_id":"ps-2","command":{"type":"plugin_storage_read","payload":{"plugin_id":"p","relative_path":""}}}}"#;
        let missing_data = r#"{"version":1,"request_id":"ps-3","command":{"type":"plugin_storage_write","payload":{"plugin_id":"p","relative_path":"a.txt"}}}"#;
        let numeric_data = r#"{"version":1,"request_id":"ps-4","command":{"type":"plugin_storage_write","payload":{"plugin_id":"p","relative_path":"a.txt","data":42}}}"#;
        let oversized_id = format!(
            r#"{{"version":1,"request_id":"ps-5","command":{{"type":"plugin_storage_read","payload":{{"plugin_id":"{}","relative_path":"a.txt"}}}}}}"#,
            "p".repeat(65)
        );
        assert!(parse_bridge_message(empty_id).is_err());
        assert!(parse_bridge_message(empty_path).is_err());
        assert!(parse_bridge_message(missing_data).is_err());
        assert!(parse_bridge_message(numeric_data).is_err());
        assert!(parse_bridge_message(&oversized_id).is_err());
    }

    #[test]
    fn serializes_typed_plugin_storage_responses_without_paths() {
        use super::{PluginStorageReadResponse, PluginStorageWriteResponse};
        let read = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "ps-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::PluginStorageRead(
                PluginStorageReadResponse {
                    data: "hello".to_owned(),
                    payload_bytes: 5,
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(read).unwrap();
        assert_eq!(value["payload"]["type"], "plugin_storage_read");
        assert_eq!(value["payload"]["payload"]["data"], "hello");
        assert!(value["payload"]["payload"].get("path").is_none());

        let write = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "ps-2".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::PluginStorageWrite(
                PluginStorageWriteResponse { payload_bytes: 5 },
            )),
            error: None,
        };
        let value = serde_json::to_value(write).unwrap();
        assert_eq!(value["payload"]["type"], "plugin_storage_write");
        assert_eq!(value["payload"]["payload"]["payload_bytes"], 5);
        assert!(value["payload"]["payload"].get("data").is_none());
        assert!(value["payload"]["payload"].get("path").is_none());
    }

    #[test]
    fn validates_plugin_storage_probe_diagnostics() {
        let ok = r#"{"version":1,"event":{"type":"plugin_storage_probe","payload":{"ok":true,"stage":"write_read","payload_bytes":32,"round_trip":true,"attacks_blocked":3,"error":null}}}"#;
        let failed = r#"{"version":1,"event":{"type":"plugin_storage_probe","payload":{"ok":false,"stage":"write_read","payload_bytes":0,"round_trip":false,"attacks_blocked":0,"error":"timeout"}}}"#;
        let bad_stage = r#"{"version":1,"event":{"type":"plugin_storage_probe","payload":{"ok":true,"stage":"deleted","payload_bytes":1,"round_trip":true,"attacks_blocked":3,"error":null}}}"#;
        let inconsistent = r#"{"version":1,"event":{"type":"plugin_storage_probe","payload":{"ok":true,"stage":"write_read","payload_bytes":1,"round_trip":true,"attacks_blocked":3,"error":"extra"}}}"#;
        let missing_attacks = r#"{"version":1,"event":{"type":"plugin_storage_probe","payload":{"ok":true,"stage":"write_read","payload_bytes":1,"round_trip":true,"attacks_blocked":2,"error":null}}}"#;
        assert!(parse_bridge_message(ok).is_ok());
        assert!(parse_bridge_message(failed).is_ok());
        assert!(parse_bridge_message(bad_stage).is_err());
        assert!(parse_bridge_message(inconsistent).is_err());
        assert!(parse_bridge_message(missing_attacks).is_err());
    }
}

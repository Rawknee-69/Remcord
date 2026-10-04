use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::localization::MAX_LANG_KEY_BYTES;
use crate::settings::SettingsSnapshot;

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_IPC_MESSAGE_BYTES: usize = 8 * 1024;
pub const MAX_REQUEST_ID_BYTES: usize = 128;
pub const MAX_ERROR_BYTES: usize = 2 * 1024;
pub const MAX_TRACKED_REQUEST_IDS: usize = 4096;

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
    ModVencordGetBundle(VencordBundleRequest),
    SettingsGetConfig(SettingsGetConfigRequest),
    LocalizationGetLang(LocalizationGetLangRequest),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PingRequest {
    pub nonce: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VencordBundleRequest {}

/// Empty request for the read-only `settings.get_config` capability. There
/// are no renderer-supplied parameters by design: no config key, no file
/// path, no selector. Any extra field is rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsGetConfigRequest {}

/// The single allowed `localization.get_lang` parameter: one catalog key.
/// No locale, file, path, URL, or object may be supplied.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalizationGetLangRequest {
    pub key: String,
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
    /// DEV-only diagnostic posted by the localization smoke probe
    /// (`LEGCORD_NATIVE_GETLANG_SMOKE=1`). Never sent by shipped adapters.
    GetLangProbe(GetLangProbe),
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetLangProbe {
    pub ok: bool,
    pub lang: String,
    pub value: String,
    pub missing_is_empty: bool,
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
    VencordBundle(VencordBundleResponse),
    SettingsConfig(Box<SettingsConfigResponse>),
    LocalizationString(LocalizationStringResponse),
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

#[derive(Serialize)]
pub struct VencordBundleResponse {
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

/// Single localized string response. `value` is `""` for a missing key
/// (Electron parity); `lang` is the resolved language selection.
#[derive(Serialize)]
pub struct LocalizationStringResponse {
    pub value: String,
    pub lang: String,
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
                BridgeCommand::ModVencordGetBundle(_) => {}
                BridgeCommand::SettingsGetConfig(_) => {}
                BridgeCommand::LocalizationGetLang(payload) => {
                    if payload.key.is_empty() || payload.key.len() > MAX_LANG_KEY_BYTES {
                        return Err("invalid localization key".to_owned());
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
            }
        }
    }

    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::{
        parse_bridge_message, BridgeEvent, BridgeMessage, BridgeResponse, BridgeResponsePayload,
        RequestTracker, VencordBundleResponse, PROTOCOL_VERSION,
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
    fn accepts_the_vencord_bundle_capability_without_renderer_parameters() {
        let request = r#"{"version":1,"request_id":"vencord-1","command":{"type":"mod_vencord_get_bundle","payload":{}}}"#;
        assert!(matches!(
            parse_bridge_message(request),
            Ok(BridgeMessage::Request(_))
        ));

        let arbitrary_url = r#"{"version":1,"request_id":"vencord-2","command":{"type":"mod_vencord_get_bundle","payload":{"url":"https://attacker.invalid/bundle.js"}}}"#;
        let arbitrary_path = r#"{"version":1,"request_id":"vencord-3","command":{"type":"mod_vencord_get_bundle","payload":{"path":"C:\\secret"}}}"#;
        assert!(parse_bridge_message(arbitrary_url).is_err());
        assert!(parse_bridge_message(arbitrary_path).is_err());
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
    fn serializes_a_typed_vencord_response_without_bundle_source() {
        let response = BridgeResponse {
            version: PROTOCOL_VERSION,
            request_id: "vencord-1".to_owned(),
            ok: true,
            payload: Some(BridgeResponsePayload::VencordBundle(
                VencordBundleResponse {
                    bundle_version: "test-build".to_owned(),
                    bundle_bytes: 1024,
                    css_bytes: 256,
                    request_latency_ms: 20.0,
                    retrieval_ms: 10.0,
                    execution_ms: 5.0,
                    css_injection_ms: 2.0,
                    marker: "vencord-bootstrap-executed",
                    document_execution_count: 1,
                },
            )),
            error: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["payload"]["type"], "vencord_bundle");
        assert_eq!(
            value["payload"]["payload"]["marker"],
            "vencord-bootstrap-executed"
        );
        assert_eq!(value["payload"]["payload"]["document_execution_count"], 1);
        assert!(value["payload"]["payload"].get("js").is_none());
        assert!(value["payload"]["payload"].get("css").is_none());
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
}

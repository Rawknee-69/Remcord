use std::collections::HashSet;

use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PingRequest {
    pub nonce: String,
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
    pub payload: Option<PingResponse>,
    pub error: Option<&'static str>,
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
            if request.request_id.is_empty()
                || request.request_id.len() > MAX_REQUEST_ID_BYTES
                || !request
                    .request_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err("invalid request id".to_owned());
            }
            match &request.command {
                BridgeCommand::Ping(payload) => {
                    if payload.nonce.is_empty() || payload.nonce.len() > 128 {
                        return Err("invalid nonce".to_owned());
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
            }
        }
    }

    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::{parse_bridge_message, BridgeEvent, BridgeMessage, RequestTracker};

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
    fn accepts_renderer_error_event() {
        let error = r#"{"version":1,"event":{"type":"renderer_error","payload":{"kind":"uncaught","message":"failed"}}}"#;
        assert!(matches!(
            parse_bridge_message(error),
            Ok(BridgeMessage::Event(ref message))
                if matches!(&message.event, BridgeEvent::RendererError(_))
        ));
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
}

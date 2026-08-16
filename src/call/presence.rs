use serde::{Deserialize, Serialize};

pub const TOPIC: &str = "miaow";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PresenceKind {
    #[serde(rename = "presence.request")]
    Request,
    #[serde(rename = "presence.ready")]
    Ready,
    #[serde(rename = "presence.state")]
    State,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceMessage {
    #[serde(rename = "type")]
    pub kind: PresenceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub microphone_muted: Option<bool>,
}

impl PresenceMessage {
    pub fn request() -> Self {
        Self {
            kind: PresenceKind::Request,
            microphone_muted: None,
        }
    }

    pub fn ready(muted: bool) -> Self {
        Self {
            kind: PresenceKind::Ready,
            microphone_muted: Some(muted),
        }
    }

    pub fn state(muted: bool) -> Self {
        Self {
            kind: PresenceKind::State,
            microphone_muted: Some(muted),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_matches_call_channel_wire_format() {
        assert_eq!(
            serde_json::to_string(&PresenceMessage::request()).unwrap(),
            r#"{"type":"presence.request"}"#
        );
    }

    #[test]
    fn ready_matches_call_channel_wire_format() {
        assert_eq!(
            serde_json::to_string(&PresenceMessage::ready(true)).unwrap(),
            r#"{"type":"presence.ready","microphoneMuted":true}"#
        );
    }
}

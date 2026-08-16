use serde::{Deserialize, Serialize};

pub const TOPIC: &str = "miaow";
pub const FILE_TOPIC: &str = "miaow.file";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReactionKind {
    #[serde(rename = "reaction.event")]
    Event,
    #[serde(rename = "reaction.request")]
    Request,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionMessage {
    #[serde(rename = "type")]
    pub kind: ReactionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    pub hash: String,
}

impl ReactionMessage {
    pub fn event(
        name: impl Into<String>,
        emoji: impl Into<String>,
        hash: impl Into<String>,
    ) -> Self {
        Self {
            kind: ReactionKind::Event,
            name: Some(name.into()),
            emoji: Some(emoji.into()),
            hash: hash.into(),
        }
    }

    pub fn request(hash: impl Into<String>) -> Self {
        Self {
            kind: ReactionKind::Request,
            name: None,
            emoji: None,
            hash: hash.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_json_matches_swift() {
        let message = ReactionMessage::event(
            "miaow!",
            "😸",
            "20d4a0a731796f9562dcfbb2e075df3da28ea15a795401962ef3e96c8a748e11",
        );
        assert_eq!(
            serde_json::to_string(&message).unwrap(),
            r#"{"type":"reaction.event","name":"miaow!","emoji":"😸","hash":"20d4a0a731796f9562dcfbb2e075df3da28ea15a795401962ef3e96c8a748e11"}"#
        );
    }

    #[test]
    fn request_json_matches_call_channel_wire_format() {
        let message = ReactionMessage::request(
            "20d4a0a731796f9562dcfbb2e075df3da28ea15a795401962ef3e96c8a748e11",
        );
        assert_eq!(
            serde_json::to_string(&message).unwrap(),
            r#"{"type":"reaction.request","hash":"20d4a0a731796f9562dcfbb2e075df3da28ea15a795401962ef3e96c8a748e11"}"#
        );
    }
}

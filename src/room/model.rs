use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedRoom {
    pub id: Uuid,
    pub server_url: String,
    pub token: String,
    pub room_name: String,
    pub local_identity: String,
    pub remote_identity: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingRoom {
    pub server_url: String,
    pub token: String,
    pub room_name: String,
    pub local_identity: String,
    pub remote_identity: String,
    pub expires_at: DateTime<Utc>,
}

impl PendingRoom {
    pub fn confirmation_message(&self) -> String {
        format!(
            "Would you like to connect with {} as {}?",
            self.remote_identity, self.local_identity
        )
    }

    pub fn into_saved(self, id: Uuid) -> SavedRoom {
        SavedRoom {
            id,
            server_url: self.server_url,
            token: self.token,
            room_name: self.room_name,
            local_identity: self.local_identity,
            remote_identity: self.remote_identity,
            expires_at: self.expires_at,
        }
    }
}

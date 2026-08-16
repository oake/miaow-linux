use anyhow::{Context, Result};
use chrono::Utc;
use uuid::Uuid;

use super::{PendingRoom, SavedRoom, secrets};

/// A successfully loaded secret. Mutations are committed in memory only after
/// Secret Service accepts the replacement, so failed writes can be retried.
#[derive(Debug)]
pub struct RoomStore {
    rooms: Vec<SavedRoom>,
    selected_id: Option<Uuid>,
}

impl RoomStore {
    pub async fn load(selected_id: Option<Uuid>, temporary_remote: Option<&str>) -> Result<Self> {
        let loaded_rooms = secrets::load_rooms().await?;
        let now = Utc::now();
        let mut rooms: Vec<_> = loaded_rooms
            .iter()
            .filter(|room| room.expires_at > now)
            .cloned()
            .collect();
        rooms.sort_by_key(|room| room.remote_identity.to_lowercase());

        if rooms.len() != loaded_rooms.len() {
            secrets::save_rooms(&rooms).await?;
        }
        let temporary = temporary_remote
            .and_then(|name| rooms.iter().find(|room| room.remote_identity == name))
            .map(|room| room.id);
        let selected_id = temporary
            .or_else(|| selected_id.filter(|id| rooms.iter().any(|room| room.id == *id)))
            .or_else(|| rooms.first().map(|room| room.id));
        Ok(Self { rooms, selected_id })
    }

    pub fn rooms(&self) -> &[SavedRoom] {
        &self.rooms
    }

    pub fn selected(&self) -> Option<&SavedRoom> {
        self.selected_id
            .and_then(|id| self.rooms.iter().find(|room| room.id == id))
    }

    pub fn select(&mut self, id: Uuid) -> Option<SavedRoom> {
        let room = self.rooms.iter().find(|room| room.id == id)?.clone();
        self.selected_id = Some(id);
        Some(room)
    }

    pub fn exact_token(&self, token: &str) -> Option<&SavedRoom> {
        self.rooms.iter().find(|room| room.token == token.trim())
    }

    pub async fn add(&mut self, pending: PendingRoom) -> Result<SavedRoom> {
        self.add_with(pending, secrets::save_rooms).await
    }

    async fn add_with(
        &mut self,
        pending: PendingRoom,
        save: impl AsyncFnOnce(&[SavedRoom]) -> Result<()>,
    ) -> Result<SavedRoom> {
        let existing_id = self
            .rooms
            .iter()
            .find(|room| {
                room.room_name == pending.room_name && room.local_identity == pending.local_identity
            })
            .map(|room| room.id)
            .unwrap_or_else(Uuid::new_v4);
        let mut rooms = self.rooms.clone();
        rooms.retain(|room| {
            !(room.room_name == pending.room_name && room.local_identity == pending.local_identity)
        });
        let room = pending.into_saved(existing_id);
        rooms.push(room.clone());
        rooms.sort_by_key(|room| room.remote_identity.to_lowercase());
        save(&rooms).await.context("Could not save room")?;
        self.rooms = rooms;
        self.selected_id = Some(room.id);
        Ok(room)
    }

    pub async fn delete_selected(&mut self) -> Result<Option<SavedRoom>> {
        self.delete_selected_with(secrets::save_rooms).await
    }

    async fn delete_selected_with(
        &mut self,
        save: impl AsyncFnOnce(&[SavedRoom]) -> Result<()>,
    ) -> Result<Option<SavedRoom>> {
        let Some(selected_id) = self.selected_id else {
            return Ok(None);
        };
        let rooms = self
            .rooms
            .iter()
            .filter(|room| room.id != selected_id)
            .cloned()
            .collect::<Vec<_>>();
        save(&rooms).await.context("Could not delete room")?;
        self.rooms = rooms;
        self.selected_id = self.rooms.first().map(|room| room.id);
        Ok(self.selected().cloned())
    }

    pub async fn purge_expired(&mut self) -> Result<bool> {
        self.purge_expired_with(secrets::save_rooms).await
    }

    async fn purge_expired_with(
        &mut self,
        save: impl AsyncFnOnce(&[SavedRoom]) -> Result<()>,
    ) -> Result<bool> {
        let now = Utc::now();
        let rooms = self
            .rooms
            .iter()
            .filter(|room| room.expires_at > now)
            .cloned()
            .collect::<Vec<_>>();
        if rooms.len() == self.rooms.len() {
            return Ok(false);
        }
        save(&rooms)
            .await
            .context("Could not remove expired rooms")?;
        self.rooms = rooms;
        if self
            .selected_id
            .is_some_and(|id| !self.rooms.iter().any(|room| room.id == id))
        {
            self.selected_id = self.rooms.first().map(|room| room.id);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(remote: &str) -> PendingRoom {
        PendingRoom {
            server_url: "wss://example.com".into(),
            token: format!("token-{remote}"),
            room_name: format!("alice+{remote}"),
            local_identity: "alice".into(),
            remote_identity: remote.into(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
        }
    }

    fn store() -> RoomStore {
        let room = pending("bob").into_saved(Uuid::new_v4());
        RoomStore {
            selected_id: Some(room.id),
            rooms: vec![room],
        }
    }

    async fn reject(_: &[SavedRoom]) -> Result<()> {
        anyhow::bail!("keyring unavailable")
    }

    async fn accept(_: &[SavedRoom]) -> Result<()> {
        Ok(())
    }

    #[tokio::test]
    async fn failed_add_preserves_rooms_and_can_be_retried() {
        let mut store = store();
        let before = store.rooms.clone();
        let selected = store.selected_id;
        let mut replacement = pending("bob");
        replacement.token = "refreshed-token".into();
        assert!(store.add_with(replacement.clone(), reject).await.is_err());
        assert_eq!(store.rooms, before);
        assert_eq!(store.selected_id, selected);
        let saved = store.add_with(replacement, accept).await.unwrap();
        assert_eq!(saved.id, before[0].id);
        assert_eq!(store.rooms.len(), 1);
        assert_eq!(store.selected().unwrap().token, "refreshed-token");
    }

    #[tokio::test]
    async fn failed_delete_preserves_selection_and_can_be_retried() {
        let mut store = store();
        let before = store.rooms.clone();
        assert!(store.delete_selected_with(reject).await.is_err());
        assert_eq!(store.rooms, before);
        assert_eq!(store.selected_id, Some(before[0].id));
        assert!(store.delete_selected_with(accept).await.unwrap().is_none());
        assert!(store.rooms.is_empty());
    }

    #[tokio::test]
    async fn failed_expiry_cleanup_is_retried_before_selecting_a_survivor() {
        let mut store = store();
        store.rooms[0].expires_at = Utc::now() - chrono::Duration::seconds(1);
        let survivor = pending("carol").into_saved(Uuid::new_v4());
        store.rooms.push(survivor.clone());
        let before = store.rooms.clone();
        assert!(store.purge_expired_with(reject).await.is_err());
        assert_eq!(store.rooms, before);
        assert_eq!(store.selected_id, Some(before[0].id));
        assert!(store.purge_expired_with(accept).await.unwrap());
        assert_eq!(store.selected(), Some(&survivor));
        // With no expired rooms, the keyring need not be contacted.
        assert!(!store.purge_expired_with(reject).await.unwrap());
    }
}

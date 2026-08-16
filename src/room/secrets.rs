use std::collections::HashMap;

use anyhow::{Context, Result};
use secret_service::{EncryptionType, SecretService};

use super::SavedRoom;

const SERVICE: &str = "ke.oa.miaow";
const ACCOUNT: &str = "rooms-v1";

fn attributes() -> HashMap<&'static str, &'static str> {
    HashMap::from([("service", SERVICE), ("account", ACCOUNT)])
}

pub async fn load_rooms() -> Result<Vec<SavedRoom>> {
    let service = SecretService::connect(EncryptionType::Dh)
        .await
        .context("Could not connect to Secret Service")?;
    let result = service
        .search_items(attributes())
        .await
        .context("Could not search Secret Service")?;
    let item = if let Some(item) = result.unlocked.first() {
        item
    } else if let Some(item) = result.locked.first() {
        item.unlock()
            .await
            .context("Could not unlock saved rooms")?;
        item
    } else {
        return Ok(Vec::new());
    };
    let secret = item
        .get_secret()
        .await
        .context("Could not read saved rooms")?;
    serde_json::from_slice(&secret).context("Saved rooms are corrupted")
}

pub async fn save_rooms(rooms: &[SavedRoom]) -> Result<()> {
    let service = SecretService::connect(EncryptionType::Dh)
        .await
        .context("Could not connect to Secret Service")?;
    let collection = service
        .get_default_collection()
        .await
        .context("Could not open the default keyring")?;
    collection
        .unlock()
        .await
        .context("Could not unlock the default keyring")?;
    let secret = serde_json::to_vec(rooms).context("Could not encode saved rooms")?;
    collection
        .create_item(
            "miaow rooms",
            attributes(),
            &secret,
            true,
            "application/json",
        )
        .await
        .context("Could not save rooms")?;
    Ok(())
}

use std::path::Path;

use anyhow::{Context, Result, bail};
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};

use crate::{
    reaction::{MAXIMUM_BYTES, RemoteReactionSlot, is_valid_hash},
    room::SavedRoom,
};

#[derive(Deserialize)]
struct SlotList {
    slots: Vec<RemoteReactionSlot>,
}

#[derive(Serialize)]
struct SlotBody<'a> {
    name: &'a str,
    emoji: &'a str,
    hash: &'a str,
}

#[derive(Clone)]
pub struct ReactionSync {
    client: Client,
}

impl ReactionSync {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn pull(&self, room: &SavedRoom) -> Result<Vec<RemoteReactionSlot>> {
        let response = self
            .request(Method::GET, room, "/api/reactions")?
            .send()
            .await?;
        Ok(response.error_for_status()?.json::<SlotList>().await?.slots)
    }

    pub async fn download_missing(&self, room: &SavedRoom, hash: &str) -> Result<()> {
        if crate::reaction::cached_file_path(hash).is_some() {
            return Ok(());
        }
        if !is_valid_hash(hash) {
            bail!("invalid reaction hash");
        }
        let mut response = self
            .request(Method::GET, room, &format!("/api/reactions/files/{hash}"))?
            .send()
            .await?
            .error_for_status()?;
        if response
            .content_length()
            .is_some_and(|length| length > MAXIMUM_BYTES as u64)
        {
            bail!("reaction file is too large");
        }
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if chunk.len() > MAXIMUM_BYTES - data.len() {
                bail!("reaction file is too large");
            }
            data.extend_from_slice(&chunk);
        }
        crate::reaction::store_received_file(&data, hash)
            .context("server returned an invalid reaction file")?;
        Ok(())
    }

    pub async fn assign(
        &self,
        room: &SavedRoom,
        slot: &RemoteReactionSlot,
        path: &Path,
    ) -> Result<()> {
        let file_path = format!("/api/reactions/files/{}", slot.hash);
        let exists = self
            .request(Method::HEAD, room, &file_path)?
            .send()
            .await?
            .status();
        if exists == StatusCode::NOT_FOUND {
            self.upload(room, &file_path, path).await?;
        } else if !exists.is_success() {
            bail!("reaction file check returned HTTP {exists}");
        }

        let mut response = self.put_assignment(room, slot).await?;
        if response.status() == StatusCode::CONFLICT {
            self.upload(room, &file_path, path).await?;
            response = self.put_assignment(room, slot).await?;
        }
        response.error_for_status()?;
        Ok(())
    }

    async fn upload(&self, room: &SavedRoom, file_path: &str, path: &Path) -> Result<()> {
        let data = std::fs::read(path)?;
        self.request(Method::PUT, room, file_path)?
            .header(reqwest::header::CONTENT_TYPE, "audio/mp4")
            .body(data)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn put_assignment(
        &self,
        room: &SavedRoom,
        slot: &RemoteReactionSlot,
    ) -> Result<reqwest::Response> {
        Ok(self
            .request(
                Method::PUT,
                room,
                &format!("/api/reactions/{}", slot.number),
            )?
            .json(&SlotBody {
                name: &slot.name,
                emoji: &slot.emoji,
                hash: &slot.hash,
            })
            .send()
            .await?)
    }

    pub async fn remove(&self, room: &SavedRoom, number: u8) -> Result<()> {
        self.request(Method::DELETE, room, &format!("/api/reactions/{number}"))?
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    fn request(
        &self,
        method: Method,
        room: &SavedRoom,
        path: &str,
    ) -> Result<reqwest::RequestBuilder> {
        Ok(self
            .client
            .request(method, endpoint(&room.server_url, path)?)
            .bearer_auth(&room.token)
            .header(reqwest::header::CACHE_CONTROL, "no-cache"))
    }
}

fn endpoint(server: &str, path: &str) -> Result<url::Url> {
    let mut endpoint = url::Url::parse(server).context("invalid reaction server URL")?;
    if endpoint.host().is_none() {
        bail!("reaction server URL has no host");
    }
    let scheme = match endpoint.scheme() {
        "wss" | "https" => "https",
        "ws" | "http" => "http",
        _ => bail!("unsupported reaction server URL scheme"),
    };
    endpoint
        .set_scheme(scheme)
        .map_err(|()| anyhow::anyhow!("could not set reaction endpoint scheme"))?;
    endpoint.set_path(path);
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn livekit_urls_map_to_companion_http_urls() {
        assert_eq!(
            endpoint("wss://call.oa.ke/rtc?foo=bar", "/api/reactions")
                .unwrap()
                .as_str(),
            "https://call.oa.ke/api/reactions"
        );
        assert_eq!(
            endpoint("ws://localhost:7880/", "/api/reactions")
                .unwrap()
                .as_str(),
            "http://localhost:7880/api/reactions"
        );
    }
}

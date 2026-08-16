use base64::Engine;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use thiserror::Error;
use url::Url;

use super::PendingRoom;

#[derive(Debug, Error, Clone, Eq, PartialEq)]
pub enum RoomConfigurationError {
    #[error("This is not a valid LiveKit token.")]
    MalformedToken,
    #[error("The token does not contain a usable room-join grant.")]
    MissingRoomGrant,
    #[error("This token has expired.")]
    ExpiredToken,
    #[error("This token is not active yet.")]
    TokenNotActiveYet,
    #[error("The room name must contain exactly two different names separated by +.")]
    InvalidOneToOneRoom,
    #[error("The names must be alphabetically sorted. Use room {0} in both tokens.")]
    RoomNameNotCanonical(String),
    #[error("Token identity {0} is not one of the two names in room {1}.")]
    IdentityNotInRoom(String, String),
    #[error("Enter a valid LiveKit server.")]
    InvalidServer,
    #[error("This is not a valid join link.")]
    InvalidJoinLink,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedRoomToken {
    pub room_name: String,
    pub local_identity: String,
    pub remote_identity: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct Claims {
    exp: f64,
    nbf: Option<f64>,
    sub: String,
    video: VideoGrant,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoGrant {
    room: String,
    room_join: bool,
}

pub fn parse_room_token(
    token: &str,
    now: DateTime<Utc>,
) -> Result<ParsedRoomToken, RoomConfigurationError> {
    let token = token.trim();
    let mut parts = token.split('.');
    let (Some(_), Some(encoded_payload), Some(_), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(RoomConfigurationError::MalformedToken);
    };

    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded_payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(encoded_payload))
        .map_err(|_| RoomConfigurationError::MalformedToken)?;
    let claims: Claims =
        serde_json::from_slice(&payload).map_err(|_| RoomConfigurationError::MalformedToken)?;

    let identity = claims.sub.trim().to_owned();
    let room_name = claims.video.room.trim().to_owned();
    if identity.is_empty() || room_name.is_empty() || !claims.video.room_join {
        return Err(RoomConfigurationError::MissingRoomGrant);
    }

    let expires_at = Utc
        .timestamp_millis_opt((claims.exp * 1000.0) as i64)
        .single()
        .ok_or(RoomConfigurationError::MalformedToken)?;
    if expires_at <= now {
        return Err(RoomConfigurationError::ExpiredToken);
    }
    if claims
        .nbf
        .is_some_and(|not_before| not_before > now.timestamp_millis() as f64 / 1000.0)
    {
        return Err(RoomConfigurationError::TokenNotActiveYet);
    }

    let Some((first, second)) = room_name.split_once('+') else {
        return Err(RoomConfigurationError::InvalidOneToOneRoom);
    };
    if first.is_empty() || second.is_empty() || second.contains('+') || first == second {
        return Err(RoomConfigurationError::InvalidOneToOneRoom);
    }
    if first > second {
        return Err(RoomConfigurationError::RoomNameNotCanonical(format!(
            "{second}+{first}"
        )));
    }
    let remote_identity = if identity == first {
        second.to_owned()
    } else if identity == second {
        first.to_owned()
    } else {
        return Err(RoomConfigurationError::IdentityNotInRoom(
            identity, room_name,
        ));
    };

    Ok(ParsedRoomToken {
        room_name,
        local_identity: identity,
        remote_identity,
        expires_at,
    })
}

pub fn normalize_server(input: &str) -> Result<String, RoomConfigurationError> {
    let value = input.trim();
    if value.is_empty() {
        return Err(RoomConfigurationError::InvalidServer);
    }
    let value = if value.contains("://") {
        value.to_owned()
    } else {
        format!("wss://{value}")
    };
    let mut url = Url::parse(&value).map_err(|_| RoomConfigurationError::InvalidServer)?;
    if url.host_str().is_none_or(str::is_empty) {
        return Err(RoomConfigurationError::InvalidServer);
    }
    let scheme = match url.scheme() {
        "wss" | "https" => "wss",
        "ws" | "http" => "ws",
        _ => return Err(RoomConfigurationError::InvalidServer),
    };
    url.set_scheme(scheme)
        .map_err(|_| RoomConfigurationError::InvalidServer)?;
    url.set_query(None);
    url.set_fragment(None);
    if url.path() == "/" {
        url.set_path("");
    }
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

pub fn pending_room(server: &str, token: &str) -> Result<PendingRoom, RoomConfigurationError> {
    let server_url = normalize_server(server)?;
    let token = token.trim().to_owned();
    let parsed = parse_room_token(&token, Utc::now())?;
    Ok(PendingRoom {
        server_url,
        token,
        room_name: parsed.room_name,
        local_identity: parsed.local_identity,
        remote_identity: parsed.remote_identity,
        expires_at: parsed.expires_at,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JoinLink {
    pub server: String,
    pub token: String,
}

pub fn parse_join_link(input: &str) -> Result<JoinLink, RoomConfigurationError> {
    let url = Url::parse(input).map_err(|_| RoomConfigurationError::InvalidJoinLink)?;
    if url.scheme() != "miaow"
        || url
            .host_str()
            .is_none_or(|host| !host.eq_ignore_ascii_case("join"))
        || !url.path().trim_matches('/').is_empty()
    {
        return Err(RoomConfigurationError::InvalidJoinLink);
    }
    let mut server = None;
    let mut token = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "server" if server.is_none() => server = Some(value.trim().to_owned()),
            "token" if token.is_none() => token = Some(value.trim().to_owned()),
            _ => {}
        }
    }
    match (server, token) {
        (Some(server), Some(token)) if !server.is_empty() && !token.is_empty() => {
            Ok(JoinLink { server, token })
        }
        _ => Err(RoomConfigurationError::InvalidJoinLink),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use chrono::Duration;
    use serde_json::json;

    fn token(sub: &str, room: &str, exp: i64, room_join: bool) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "exp": exp,
                "sub": sub,
                "video": { "room": room, "roomJoin": room_join }
            }))
            .unwrap(),
        );
        format!("{header}.{payload}.signature")
    }

    #[test]
    fn parses_canonical_one_to_one_token() {
        let now = Utc::now();
        let parsed = parse_room_token(
            &token(
                "anna",
                "anna+maeve",
                (now + Duration::hours(1)).timestamp(),
                true,
            ),
            now,
        )
        .unwrap();
        assert_eq!(parsed.local_identity, "anna");
        assert_eq!(parsed.remote_identity, "maeve");
        assert_eq!(parsed.room_name, "anna+maeve");
    }

    #[test]
    fn rejects_noncanonical_room() {
        let now = Utc::now();
        let error = parse_room_token(
            &token(
                "anna",
                "maeve+anna",
                (now + Duration::hours(1)).timestamp(),
                true,
            ),
            now,
        )
        .unwrap_err();
        assert_eq!(
            error,
            RoomConfigurationError::RoomNameNotCanonical("anna+maeve".into())
        );
    }

    #[test]
    fn rejects_expired_and_missing_grant() {
        let now = Utc::now();
        assert_eq!(
            parse_room_token(
                &token(
                    "anna",
                    "anna+maeve",
                    (now - Duration::seconds(1)).timestamp(),
                    true
                ),
                now
            ),
            Err(RoomConfigurationError::ExpiredToken)
        );
        assert_eq!(
            parse_room_token(
                &token(
                    "anna",
                    "anna+maeve",
                    (now + Duration::hours(1)).timestamp(),
                    false
                ),
                now
            ),
            Err(RoomConfigurationError::MissingRoomGrant)
        );
    }

    #[test]
    fn normalizes_servers_like_macos() {
        assert_eq!(normalize_server("call.oa.ke").unwrap(), "wss://call.oa.ke");
        assert_eq!(
            normalize_server("https://call.oa.ke/?ignored=yes").unwrap(),
            "wss://call.oa.ke"
        );
        assert_eq!(
            normalize_server("http://localhost:7880").unwrap(),
            "ws://localhost:7880"
        );
    }

    #[test]
    fn accepts_only_exact_join_shape() {
        assert_eq!(
            parse_join_link("miaow://join/?server=call.oa.ke&token=abc").unwrap(),
            JoinLink {
                server: "call.oa.ke".into(),
                token: "abc".into()
            }
        );
        assert!(parse_join_link("miaow://join/path?server=x&token=y").is_err());
        assert!(parse_join_link("https://call.oa.ke/join/token").is_err());
    }
}

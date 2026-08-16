mod mp4;
pub(crate) use mp4::aac_priming_samples;
use mp4::duration_seconds as mp4_duration_seconds;

use std::{
    fs,
    path::{Path, PathBuf},
};

use gio::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

pub const SLOT_COUNT: u8 = 9;
pub const BUILTIN_NUMBER: u8 = 5;
pub const BUILTIN_NAME: &str = "miaow!";
pub const BUILTIN_EMOJI: &str = "😸";
pub const BUILTIN_HASH: &str = "20d4a0a731796f9562dcfbb2e075df3da28ea15a795401962ef3e96c8a748e11";
pub const MAXIMUM_BYTES: usize = 128 * 1024;
pub const MAXIMUM_DURATION: f64 = 5.0;
const DEFAULT_EMOJI: &str = "📣";
const BUILTIN_BYTES: &[u8] = include_bytes!(
    "../data/reactions/20d4a0a731796f9562dcfbb2e075df3da28ea15a795401962ef3e96c8a748e11.m4a"
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReactionSlot {
    pub number: u8,
    pub name: Option<String>,
    pub emoji: Option<String>,
    pub hash: Option<String>,
}

impl ReactionSlot {
    pub fn empty(number: u8) -> Self {
        Self {
            number,
            name: None,
            emoji: None,
            hash: None,
        }
    }

    pub fn built_in() -> Self {
        Self {
            number: BUILTIN_NUMBER,
            name: Some(BUILTIN_NAME.to_owned()),
            emoji: Some(BUILTIN_EMOJI.to_owned()),
            hash: Some(BUILTIN_HASH.to_owned()),
        }
    }

    pub fn is_assigned(&self) -> bool {
        self.name.is_some() && self.emoji.is_some() && self.hash.is_some()
    }

    pub fn is_built_in(&self) -> bool {
        self.number == BUILTIN_NUMBER
    }

    pub fn menu_label(&self) -> String {
        match (&self.emoji, &self.name) {
            (Some(emoji), Some(name)) => format!("{}  •  {emoji}  {name}", self.number),
            _ => format!("{}  •  unassigned", self.number),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedReactionFile {
    pub path: PathBuf,
    pub hash: String,
    pub is_temporary: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentlyHeardReaction {
    pub name: String,
    pub emoji: String,
    pub hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredReactionAssignment {
    slot: u8,
    name: String,
    emoji: String,
    hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RemoteReactionSlot {
    pub number: u8,
    pub name: String,
    pub emoji: String,
    pub hash: String,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ReactionImportError {
    #[error("That audio file could not be read.")]
    Unreadable,
    #[error("The encoded sound is longer than 5 seconds.")]
    TooLong,
    #[error("The encoded sound is larger than 128 KB.")]
    TooLarge,
    #[error("The sound could not be encoded: {0}")]
    EncodingFailed(String),
}

pub struct ReactionStore {
    slots: [ReactionSlot; SLOT_COUNT as usize],
    directory: PathBuf,
    persist: bool,
}

impl ReactionStore {
    pub fn load() -> Self {
        let json = gio::Settings::new("ke.oa.miaow").string("reaction-slots");
        let assignments = serde_json::from_str(&json).unwrap_or_default();
        Self::from_assignments(cache_directory(), assignments, true)
    }

    fn from_assignments(
        directory: PathBuf,
        assignments: Vec<StoredReactionAssignment>,
        persist: bool,
    ) -> Self {
        let _ = fs::create_dir_all(&directory);
        extract_builtin(&directory.join(format!("{BUILTIN_HASH}.m4a")));
        let mut slots = std::array::from_fn(|index| {
            let number = index as u8 + 1;
            if number == BUILTIN_NUMBER {
                ReactionSlot::built_in()
            } else {
                ReactionSlot::empty(number)
            }
        });
        for assignment in assignments {
            if !matches!(assignment.slot, 1..=SLOT_COUNT) || assignment.slot == BUILTIN_NUMBER {
                continue;
            }
            if !is_valid_hash(&assignment.hash) {
                continue;
            }
            slots[assignment.slot as usize - 1] = ReactionSlot {
                number: assignment.slot,
                name: Some(assignment.name),
                emoji: Some(assignment.emoji),
                hash: Some(assignment.hash),
            };
        }
        let store = Self {
            slots,
            directory,
            persist,
        };
        store.persist();
        store
    }

    pub fn slots(&self) -> &[ReactionSlot; SLOT_COUNT as usize] {
        &self.slots
    }

    pub fn slot(&self, number: u8) -> Option<&ReactionSlot> {
        self.slots.iter().find(|slot| slot.number == number)
    }

    pub fn file_for_slot(&self, number: u8) -> Option<PathBuf> {
        let slot = self.slot(number)?.clone();
        let hash = slot.hash.as_deref()?;
        if slot.is_built_in() {
            return cached_file_path(hash);
        }
        let path = self.directory.join(format!("{hash}.m4a"));
        path.is_file().then_some(path)
    }

    pub fn prepared_file(&self, number: u8) -> Option<PreparedReactionFile> {
        let slot = self.slot(number)?;
        let hash = slot.hash.clone()?;
        let path = cached_file_path(&hash)?;
        Some(PreparedReactionFile {
            path,
            hash,
            is_temporary: false,
        })
    }

    pub fn assign(
        &mut self,
        number: u8,
        name: &str,
        emoji: &str,
        file: &PreparedReactionFile,
    ) -> Result<(), ReactionImportError> {
        if number == BUILTIN_NUMBER || !matches!(number, 1..=SLOT_COUNT) {
            return Err(ReactionImportError::Unreadable);
        }
        let trimmed_name = normalize_name(name);
        if trimmed_name.is_empty() {
            return Err(ReactionImportError::Unreadable);
        }
        let normalized_emoji = normalize_emoji(emoji);
        if !is_valid_hash(&file.hash) {
            return Err(ReactionImportError::Unreadable);
        }
        let destination = self.directory.join(format!("{}.m4a", file.hash));
        let old_hash = self.slots[number as usize - 1].hash.clone();
        if file.path != destination {
            if !destination.is_file() {
                if fs::rename(&file.path, &destination).is_err() {
                    fs::copy(&file.path, &destination)
                        .map_err(|_| ReactionImportError::Unreadable)?;
                    if file.is_temporary {
                        let _ = fs::remove_file(&file.path);
                    }
                }
            } else if file.is_temporary {
                let _ = fs::remove_file(&file.path);
            }
        }
        self.slots[number as usize - 1] = ReactionSlot {
            number,
            name: Some(trimmed_name),
            emoji: Some(normalized_emoji),
            hash: Some(file.hash.clone()),
        };
        self.persist();
        if old_hash.as_deref() != Some(file.hash.as_str())
            && let Some(old_hash) = old_hash
        {
            self.remove_cached_file_if_unreferenced(&old_hash);
        }
        Ok(())
    }

    pub fn remove(&mut self, number: u8) {
        if number == BUILTIN_NUMBER {
            return;
        }
        let old_hash = self
            .slots
            .iter()
            .find(|slot| slot.number == number)
            .and_then(|slot| slot.hash.clone());
        self.unassign(number);
        if let Some(old_hash) = old_hash {
            self.remove_cached_file_if_unreferenced(&old_hash);
        }
    }

    pub fn apply_remote_slots(
        &mut self,
        remote: &[RemoteReactionSlot],
        dirty: &std::collections::HashSet<u8>,
    ) {
        let old_hashes: std::collections::HashSet<_> = self
            .slots
            .iter()
            .filter_map(|slot| slot.hash.clone())
            .collect();
        let remote_by_number: std::collections::HashMap<_, _> = remote
            .iter()
            .filter(|slot| {
                matches!(slot.number, 1..=SLOT_COUNT)
                    && slot.number != BUILTIN_NUMBER
                    && is_valid_hash(&slot.hash)
            })
            .map(|slot| (slot.number, slot))
            .collect();
        for number in 1..=SLOT_COUNT {
            if number == BUILTIN_NUMBER || dirty.contains(&number) {
                continue;
            }
            self.slots[number as usize - 1] = remote_by_number
                .get(&number)
                .and_then(|slot| {
                    let name = normalize_name(&slot.name);
                    (!name.is_empty()).then(|| ReactionSlot {
                        number,
                        name: Some(name),
                        emoji: Some(normalize_emoji(&slot.emoji)),
                        hash: Some(slot.hash.clone()),
                    })
                })
                .unwrap_or_else(|| ReactionSlot::empty(number));
        }
        self.persist();
        for hash in old_hashes {
            self.remove_cached_file_if_unreferenced(&hash);
        }
    }

    fn unassign(&mut self, number: u8) {
        if number == BUILTIN_NUMBER {
            return;
        }
        if let Some(slot) = self.slots.iter_mut().find(|slot| slot.number == number) {
            *slot = ReactionSlot::empty(number);
            self.persist();
        }
    }

    fn remove_cached_file_if_unreferenced(&self, hash: &str) {
        if hash == BUILTIN_HASH
            || self
                .slots
                .iter()
                .any(|slot| slot.hash.as_deref() == Some(hash))
        {
            return;
        }
        let _ = fs::remove_file(self.directory.join(format!("{hash}.m4a")));
    }

    fn persist(&self) {
        if !self.persist {
            return;
        }
        let assignments: Vec<StoredReactionAssignment> = self
            .slots
            .iter()
            .filter(|slot| !slot.is_built_in())
            .filter_map(|slot| {
                Some(StoredReactionAssignment {
                    slot: slot.number,
                    name: slot.name.clone()?,
                    emoji: slot.emoji.clone()?,
                    hash: slot.hash.clone()?,
                })
            })
            .collect();
        if let Ok(json) = serde_json::to_string(&assignments) {
            let _ = gio::Settings::new("ke.oa.miaow").set_string("reaction-slots", &json);
        }
    }
}

pub fn prepare(source: &Path) -> Result<PreparedReactionFile, ReactionImportError> {
    if !source.is_file() {
        return Err(ReactionImportError::Unreadable);
    }
    let bytes = fs::read(source).map_err(|_| ReactionImportError::Unreadable)?;
    if let Some(prepared) = accept_existing_m4a(&bytes) {
        let output = std::env::temp_dir().join(format!("miaow-reaction-{}.m4a", Uuid::new_v4()));
        fs::write(&output, &bytes).map_err(|_| ReactionImportError::Unreadable)?;
        return Ok(PreparedReactionFile {
            path: output,
            hash: prepared,
            is_temporary: true,
        });
    }

    let mut buffer = oxiaudio::decode_file(source).map_err(|_| ReactionImportError::Unreadable)?;
    if !buffer.duration_secs().is_finite() {
        return Err(ReactionImportError::Unreadable);
    }
    if buffer.duration_secs() > MAXIMUM_DURATION {
        return Err(ReactionImportError::TooLong);
    }
    if buffer.channels.channel_count() > 2 {
        buffer = oxiaudio::downmix_to_mono(&buffer);
    }

    let output = std::env::temp_dir().join(format!("miaow-reaction-{}.m4a", Uuid::new_v4()));
    if oxiaudio::encode_m4a_file(&buffer, &output).is_err() {
        let resampled = buffer.resample_linear(44_100);
        oxiaudio::encode_m4a_file(&resampled, &output).map_err(|error| {
            let _ = fs::remove_file(&output);
            ReactionImportError::EncodingFailed(error.to_string())
        })?;
    }
    let encoded = fs::read(&output).map_err(|_| {
        let _ = fs::remove_file(&output);
        ReactionImportError::Unreadable
    })?;
    if encoded.len() > MAXIMUM_BYTES {
        let _ = fs::remove_file(&output);
        return Err(ReactionImportError::TooLarge);
    }
    if mp4_duration_seconds(&encoded).is_none() {
        let _ = fs::remove_file(&output);
        return Err(ReactionImportError::Unreadable);
    }
    Ok(PreparedReactionFile {
        path: output,
        hash: sha256_hex(&encoded),
        is_temporary: true,
    })
}

fn accept_existing_m4a(bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAXIMUM_BYTES {
        return None;
    }
    let duration = mp4_duration_seconds(bytes)?;
    (duration.is_finite() && duration > 0.0 && duration <= MAXIMUM_DURATION)
        .then(|| sha256_hex(bytes))
}

pub fn store_received_file(
    data: &[u8],
    expected_hash: &str,
) -> Result<PathBuf, ReactionImportError> {
    if !is_valid_hash(expected_hash) {
        return Err(ReactionImportError::Unreadable);
    }
    if data.len() > MAXIMUM_BYTES {
        return Err(ReactionImportError::TooLarge);
    }
    if sha256_hex(data) != expected_hash {
        return Err(ReactionImportError::Unreadable);
    }
    let duration = mp4_duration_seconds(data).ok_or(ReactionImportError::Unreadable)?;
    if duration > MAXIMUM_DURATION {
        return Err(ReactionImportError::TooLong);
    }
    let directory = cache_directory();
    fs::create_dir_all(&directory).map_err(|_| ReactionImportError::Unreadable)?;
    let destination = directory.join(format!("{expected_hash}.m4a"));
    if !destination.is_file() {
        fs::write(&destination, data).map_err(|_| ReactionImportError::Unreadable)?;
    }
    Ok(destination)
}

pub fn cached_file_path(hash: &str) -> Option<PathBuf> {
    if !is_valid_hash(hash) {
        return None;
    }
    let directory = cache_directory();
    let _ = fs::create_dir_all(&directory);
    let path = directory.join(format!("{hash}.m4a"));
    if hash == BUILTIN_HASH {
        extract_builtin(&path);
        return path.is_file().then_some(path);
    }
    path.is_file().then_some(path)
}

pub fn is_valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn normalize_name(name: &str) -> String {
    name.trim().chars().take(12).collect()
}

pub fn normalize_emoji(emoji: &str) -> String {
    emoji
        .chars()
        .next()
        .map(String::from)
        .unwrap_or_else(|| DEFAULT_EMOJI.to_owned())
}

pub fn cache_directory() -> PathBuf {
    glib::user_data_dir().join("miaow").join("reactions-cache")
}

fn extract_builtin(path: &Path) {
    if path.is_file() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, BUILTIN_BYTES);
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_hash_matches_bundled_bytes() {
        assert_eq!(sha256_hex(BUILTIN_BYTES), BUILTIN_HASH);
    }

    #[test]
    fn bundled_miaow_is_within_limits() {
        let duration = mp4_duration_seconds(BUILTIN_BYTES).unwrap();
        assert!(duration > 0.0 && duration <= MAXIMUM_DURATION);
        assert!(BUILTIN_BYTES.len() <= MAXIMUM_BYTES);
        assert_eq!(
            accept_existing_m4a(BUILTIN_BYTES).as_deref(),
            Some(BUILTIN_HASH)
        );
    }

    #[test]
    fn hash_validation_matches_swift() {
        assert!(is_valid_hash(BUILTIN_HASH));
        assert!(!is_valid_hash(&BUILTIN_HASH.to_uppercase()));
        assert!(!is_valid_hash("abc"));
        assert!(!is_valid_hash(&format!("{}G", &BUILTIN_HASH[..63])));
    }

    #[test]
    fn name_and_emoji_are_normalized() {
        assert_eq!(normalize_name("  hello world!! "), "hello world!");
        assert_eq!(normalize_emoji("😸xyz"), "😸");
        assert_eq!(normalize_emoji(""), "📣");
    }

    #[test]
    fn builtin_slot_cannot_be_removed_or_replaced() {
        let directory =
            std::env::temp_dir().join(format!("miaow-reaction-test-{}", Uuid::new_v4()));
        let mut store = ReactionStore::from_assignments(directory.clone(), Vec::new(), false);
        store.remove(BUILTIN_NUMBER);
        assert!(store.slot(BUILTIN_NUMBER).unwrap().is_assigned());
        let prepared = PreparedReactionFile {
            path: directory.join("missing.m4a"),
            hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            is_temporary: true,
        };
        assert_eq!(
            store.assign(BUILTIN_NUMBER, "nope", "🎉", &prepared),
            Err(ReactionImportError::Unreadable)
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn assignments_survive_a_missing_cache_file_for_later_sync() {
        let directory =
            std::env::temp_dir().join(format!("miaow-reaction-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let hash = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        fs::write(directory.join(format!("{hash}.m4a")), b"not-parsed-here").unwrap();
        let store = ReactionStore::from_assignments(
            directory.clone(),
            vec![StoredReactionAssignment {
                slot: 2,
                name: "clap".into(),
                emoji: "👏".into(),
                hash: hash.into(),
            }],
            false,
        );
        let slot = store.slot(2).unwrap();
        assert_eq!(slot.name.as_deref(), Some("clap"));
        assert_eq!(slot.emoji.as_deref(), Some("👏"));
        let missing = ReactionStore::from_assignments(
            directory.clone(),
            vec![StoredReactionAssignment {
                slot: 3,
                name: "gone".into(),
                emoji: "💨".into(),
                hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into(),
            }],
            false,
        );
        assert!(missing.slot(3).unwrap().is_assigned());
        let _ = fs::remove_dir_all(directory);
    }
}

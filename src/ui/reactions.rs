use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    io::Cursor,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use adw::prelude::*;
use async_channel::Sender;
use gtk::gdk;
use rodio::Source;
use uuid::Uuid;

use crate::{
    backend::BackendCommand,
    call::{CallCommand, ReactionKind, ReactionMessage},
    reaction::{ReactionStore, RecentlyHeardReaction, RemoteReactionSlot},
};

use super::{
    reaction_editor::present_reaction_editor,
    reaction_explosion::{LaunchEdge, ReactionExplosion},
};

#[derive(Clone)]
pub struct Reactions {
    overlay: gtk::Overlay,
    window: adw::ApplicationWindow,
    sender: Sender<BackendCommand>,
    store: Rc<RefCell<ReactionStore>>,
    pending: Rc<RefCell<HashMap<String, Vec<ReactionMessage>>>>,
    request_ids: Rc<RefCell<HashMap<String, Uuid>>>,
    recent: Rc<RefCell<Option<RecentlyHeardReaction>>>,
    last_edge: Rc<Cell<Option<LaunchEdge>>>,
    playback: Rc<Option<ReactionPlayback>>,
    editor_open: Rc<Cell<bool>>,
    held_numbers: Rc<RefCell<HashSet<u8>>>,
    dirty_numbers: Rc<RefCell<HashSet<u8>>>,
    sync_identity: Rc<RefCell<Option<String>>>,
    in_call: Rc<Cell<bool>>,
    reference_loudness: Rc<Cell<Option<f64>>>,
}

struct ReactionPlayback {
    stream: rodio::OutputStream,
}

impl Reactions {
    pub fn new(
        overlay: &gtk::Overlay,
        window: &adw::ApplicationWindow,
        sender: Sender<BackendCommand>,
    ) -> Self {
        Self {
            overlay: overlay.clone(),
            window: window.clone(),
            sender,
            store: Rc::new(RefCell::new(ReactionStore::load())),
            pending: Rc::new(RefCell::new(HashMap::new())),
            request_ids: Rc::new(RefCell::new(HashMap::new())),
            recent: Rc::new(RefCell::new(None)),
            last_edge: Rc::new(Cell::new(None)),
            playback: Rc::new(open_playback()),
            editor_open: Rc::new(Cell::new(false)),
            held_numbers: Rc::new(RefCell::new(HashSet::new())),
            dirty_numbers: Rc::new(RefCell::new(HashSet::new())),
            sync_identity: Rc::new(RefCell::new(None)),
            in_call: Rc::new(Cell::new(false)),
            reference_loudness: Rc::new(Cell::new(None)),
        }
    }

    pub fn store(&self) -> Rc<RefCell<ReactionStore>> {
        self.store.clone()
    }

    pub fn editor_is_open(&self) -> bool {
        self.editor_open.get()
    }

    pub fn set_in_call(&self, in_call: bool) {
        self.in_call.set(in_call);
    }

    pub fn play_number(&self, number: u8) {
        let (emoji, path, event) = {
            let store = self.store.borrow();
            let slot = store.slot(number).cloned();
            let path = store.file_for_slot(number);
            let Some(slot) = slot else {
                return;
            };
            let Some(emoji) = slot.emoji.clone() else {
                return;
            };
            let Some(path) = path else {
                return;
            };
            let event = match (slot.name.clone(), slot.hash.clone()) {
                (Some(name), Some(hash)) => Some(ReactionMessage::event(name, emoji.clone(), hash)),
                _ => None,
            };
            (emoji, path, event)
        };
        self.play_file(&path, true);
        self.show_explosion(&emoji);
        if let Some(event) = event {
            let _ = self
                .sender
                .try_send(BackendCommand::Call(CallCommand::PublishReaction(event)));
        }
    }

    pub fn preview(&self, path: &Path) {
        self.play_file(path, false);
    }

    pub fn handle_remote(&self, message: ReactionMessage) {
        match message.kind {
            ReactionKind::Event => {
                let Some(emoji) = message.emoji.clone() else {
                    return;
                };
                if let Some(path) = crate::reaction::cached_file_path(&message.hash) {
                    self.play_file(&path, true);
                    self.show_explosion(&emoji);
                    self.record_heard(&message);
                } else {
                    self.pending
                        .borrow_mut()
                        .entry(message.hash.clone())
                        .or_default()
                        .push(message.clone());
                    if self.request_ids.borrow().contains_key(&message.hash) {
                        return;
                    }
                    let request_id = Uuid::new_v4();
                    self.request_ids
                        .borrow_mut()
                        .insert(message.hash.clone(), request_id);
                    let _ =
                        self.sender
                            .try_send(BackendCommand::Call(CallCommand::PublishReaction(
                                ReactionMessage::request(message.hash.clone()),
                            )));
                    let request_ids = self.request_ids.clone();
                    let hash = message.hash;
                    glib::timeout_add_local_once(std::time::Duration::from_secs(10), move || {
                        let mut ids = request_ids.borrow_mut();
                        if ids.get(&hash) == Some(&request_id) {
                            ids.remove(&hash);
                        }
                    });
                }
            }
            ReactionKind::Request => {}
        }
    }

    pub fn handle_file_ready(&self, hash: &str) {
        self.request_ids.borrow_mut().remove(hash);
        let pending = self.pending.borrow_mut().remove(hash).unwrap_or_default();
        let Some(path) = crate::reaction::cached_file_path(hash) else {
            return;
        };
        for message in pending {
            let Some(emoji) = message.emoji.clone() else {
                continue;
            };
            self.play_file(&path, true);
            self.show_explosion(&emoji);
            self.record_heard(&message);
        }
    }

    pub fn reset_network(&self) {
        self.recent.replace(None);
        self.pending.borrow_mut().clear();
        self.request_ids.borrow_mut().clear();
    }

    pub fn select_identity(&self, identity: Option<&str>) {
        if self.sync_identity.borrow().as_deref() == identity {
            return;
        }
        *self.sync_identity.borrow_mut() = identity.map(str::to_owned);
        self.dirty_numbers.borrow_mut().clear();
        self.store
            .borrow_mut()
            .apply_remote_slots(&[], &HashSet::new());
    }

    pub fn restore_identity(&self, identity: &str) {
        if self.sync_identity.borrow().is_none() {
            *self.sync_identity.borrow_mut() = Some(identity.to_owned());
        }
    }

    pub fn apply_remote_slots(&self, identity: &str, slots: &[RemoteReactionSlot]) {
        if self.sync_identity.borrow().as_deref() != Some(identity) {
            return;
        }
        self.store
            .borrow_mut()
            .apply_remote_slots(slots, &self.dirty_numbers.borrow());
    }

    pub fn finish_sync(&self, number: u8) {
        self.dirty_numbers.borrow_mut().remove(&number);
    }

    fn sync_slot(&self, number: u8) {
        self.dirty_numbers.borrow_mut().insert(number);
        let assignment = self.store.borrow().slot(number).and_then(|slot| {
            Some(RemoteReactionSlot {
                number,
                name: slot.name.clone()?,
                emoji: slot.emoji.clone()?,
                hash: slot.hash.clone()?,
            })
        });
        let _ = self
            .sender
            .try_send(BackendCommand::SetReactionSlot { number, assignment });
    }

    pub fn edit(&self, number: u8, on_menu_changed: impl Fn() + Clone + 'static) {
        if self
            .store
            .borrow()
            .slot(number)
            .is_some_and(|slot| slot.is_built_in())
        {
            return;
        }
        self.editor_open.set(true);
        let store = self.store.clone();
        let recent = self.recent.borrow().clone();
        let this = self.clone();
        let editor_open = self.editor_open.clone();
        let on_menu_changed_for_assign = on_menu_changed.clone();
        present_reaction_editor(
            &self.window,
            number,
            store,
            recent,
            move || {
                if let Some(recent) = this.recent.borrow().as_ref() {
                    let assigned = this
                        .store
                        .borrow()
                        .slots()
                        .iter()
                        .any(|slot| slot.hash.as_deref() == Some(recent.hash.as_str()));
                    if assigned {
                        this.recent.replace(None);
                    }
                }
                this.sync_slot(number);
                on_menu_changed_for_assign();
            },
            {
                let this = self.clone();
                move |path: PathBuf| this.preview(&path)
            },
            move || editor_open.set(false),
        );
    }

    pub fn press_number(&self, number: u8) -> bool {
        if !self.held_numbers.borrow_mut().insert(number) {
            return true;
        }
        self.play_number(number);
        true
    }

    pub fn release_number(&self, number: u8) {
        self.held_numbers.borrow_mut().remove(&number);
    }

    fn play_file(&self, path: &Path, normalize_in_call: bool) {
        let Some(playback) = self.playback.as_ref() else {
            return;
        };
        let Ok(source) = decode_reaction(path) else {
            return;
        };
        if !self.in_call.get() || !normalize_in_call {
            playback.stream.mixer().add(source);
            return;
        }

        let mut samples: Vec<f32> =
            rodio::source::UniformSourceIterator::new(source, 2, 48_000).collect();
        let channels = 2;
        let (loudness, peak) = measure_loudness(&samples, channels);
        let reference = self.reference_loudness.get().unwrap_or_else(|| {
            let value = crate::reaction::cached_file_path(crate::reaction::BUILTIN_HASH)
                .and_then(|path| decode_reaction(&path).ok())
                .map(|source| {
                    let samples = rodio::source::UniformSourceIterator::new(source, 2, 48_000)
                        .collect::<Vec<f32>>();
                    measure_loudness(&samples, 2).0
                })
                .filter(|value| *value > 1e-8)
                .unwrap_or(loudness);
            self.reference_loudness.set(Some(value));
            value
        });
        let gain = normalization_gain(loudness, peak, reference) * 0.65;
        for sample in &mut samples {
            *sample *= gain;
        }
        playback
            .stream
            .mixer()
            .add(rodio::buffer::SamplesBuffer::new(2, 48_000, samples));
    }

    fn show_explosion(&self, emoji: &str) {
        let launch_edge = self
            .last_edge
            .get()
            .map(LaunchEdge::opposite)
            .unwrap_or(LaunchEdge::Bottom);
        self.last_edge.set(Some(launch_edge));
        let explosion = ReactionExplosion::new(emoji, launch_edge);
        explosion.set_halign(gtk::Align::Fill);
        explosion.set_valign(gtk::Align::Fill);
        explosion.set_can_target(false);
        self.overlay.add_overlay(&explosion);
    }

    fn record_heard(&self, message: &ReactionMessage) {
        let Some(raw_name) = message.name.as_deref() else {
            return;
        };
        let Some(raw_emoji) = message.emoji.as_deref() else {
            return;
        };
        if crate::reaction::cached_file_path(&message.hash).is_none() {
            return;
        }
        let name = crate::reaction::normalize_name(raw_name);
        let assigned = self
            .store
            .borrow()
            .slots()
            .iter()
            .any(|slot| slot.hash.as_deref() == Some(message.hash.as_str()));
        if name.is_empty() || assigned {
            return;
        }
        let emoji = crate::reaction::normalize_emoji(raw_emoji);
        *self.recent.borrow_mut() = Some(RecentlyHeardReaction {
            name,
            emoji,
            hash: message.hash.clone(),
        });
    }
}

fn decode_reaction(
    path: &Path,
) -> anyhow::Result<impl Source<Item = f32> + Send + 'static + use<>> {
    let bytes = std::fs::read(path)?;
    let priming = crate::reaction::aac_priming_samples(&bytes).unwrap_or(0);
    let source = rodio::Decoder::builder()
        .with_byte_len(bytes.len() as u64)
        .with_data(Cursor::new(bytes))
        .build()?;
    let delay = Duration::from_secs_f64(f64::from(priming) / f64::from(source.sample_rate()));
    Ok(source.skip_duration(delay))
}

fn measure_loudness(samples: &[f32], channels: u16) -> (f64, f32) {
    let channels = usize::from(channels.max(1));
    let mut energies = Vec::with_capacity(samples.len() / channels);
    let mut total = 0.0;
    let mut peak = 0.0_f32;
    for frame in samples.chunks_exact(channels) {
        let mut energy = 0.0;
        for &sample in frame {
            peak = peak.max(sample.abs());
            energy += f64::from(sample * sample);
        }
        energy /= channels as f64;
        energies.push(energy);
        total += energy;
    }
    if energies.is_empty() {
        return (0.0, peak);
    }
    let gate = (total / energies.len() as f64 * 0.1).max(1e-10);
    let mut gated = 0.0;
    let mut count = 0usize;
    for energy in energies.into_iter().filter(|energy| *energy >= gate) {
        gated += energy;
        count += 1;
    }
    if count == 0 {
        (0.0, peak)
    } else {
        ((gated / count as f64).sqrt(), peak)
    }
}

fn normalization_gain(source: f64, peak: f32, reference: f64) -> f32 {
    if source <= 1e-8 || reference <= 1e-8 || peak <= 0.0 {
        return 1.0;
    }
    let desired = (reference / source) as f32;
    desired
        .clamp(0.01, 10_f32.powf(18.0 / 20.0))
        .min(1.0 / peak)
}

fn open_playback() -> Option<ReactionPlayback> {
    let stream = rodio::OutputStreamBuilder::open_default_stream().ok()?;
    Some(ReactionPlayback { stream })
}

pub fn reaction_number(keyval: gdk::Key, keycode: u32) -> Option<u8> {
    match keyval {
        gdk::Key::_1 | gdk::Key::KP_1 => Some(1),
        gdk::Key::_2 | gdk::Key::KP_2 => Some(2),
        gdk::Key::_3 | gdk::Key::KP_3 => Some(3),
        gdk::Key::_4 | gdk::Key::KP_4 => Some(4),
        gdk::Key::_5 | gdk::Key::KP_5 => Some(5),
        gdk::Key::_6 | gdk::Key::KP_6 => Some(6),
        gdk::Key::_7 | gdk::Key::KP_7 => Some(7),
        gdk::Key::_8 | gdk::Key::KP_8 => Some(8),
        gdk::Key::_9 | gdk::Key::KP_9 => Some(9),
        _ => match keycode {
            10..=18 => Some((keycode - 9) as u8),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_playback_skips_declared_encoder_padding() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "data/reactions/{}.m4a",
            crate::reaction::BUILTIN_HASH
        ));
        let raw = rodio::Decoder::try_from(std::fs::File::open(&path).unwrap()).unwrap();
        let rate = raw.sample_rate();
        let channels = usize::from(raw.channels());
        assert_eq!(rate, 24_000);
        let raw: Vec<_> = raw.collect();
        let corrected: Vec<_> = decode_reaction(&path).unwrap().collect();
        assert_eq!(corrected, raw[2112 * channels..]);
    }

    #[test]
    fn gated_loudness_ignores_silence_around_a_short_effect() {
        let mut samples = vec![0.0; 200];
        samples.extend(std::iter::repeat_n(0.5, 200));
        samples.extend(std::iter::repeat_n(0.0, 200));
        let (loudness, peak) = measure_loudness(&samples, 1);
        assert!((loudness - 0.5).abs() < 1e-6);
        assert_eq!(peak, 0.5);
    }

    #[test]
    fn normalization_is_reference_matched_but_peak_safe() {
        assert!((normalization_gain(0.1, 0.4, 0.2) - 2.0).abs() < 1e-6);
        assert!((normalization_gain(0.1, 0.8, 0.2) - 1.25).abs() < 1e-6);
    }
}

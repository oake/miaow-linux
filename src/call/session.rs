use std::{collections::HashMap, time::Duration};

use async_channel::{Receiver, Sender};
use livekit::{
    DataPacket, Room, RoomEvent, RoomOptions, StreamByteOptions, StreamReader,
    options::TrackPublishOptions,
    prelude::{
        AudioProcessingOptions, ConnectionQuality, LocalAudioTrack, LocalTrack, PlayoutDeviceId,
        RecordingDeviceId, RemoteTrack, TrackSource,
    },
};

use crate::room::SavedRoom;

use super::{
    camera,
    presence::{PresenceKind, PresenceMessage, TOPIC},
    reaction::{self, ReactionKind, ReactionMessage},
    screen_share,
};

use super::{
    audio::{ConfiguredAudio, audio_device_guid, configure_platform_audio},
    remote_video::{
        RemoteVideo, apply_remote_video_quality, remote_video_kind, spawn_remote_video_renderer,
    },
    stats::format_debug_stats,
    types::*,
};

pub async fn run_call_actor(commands: Receiver<CallCommand>, events: Sender<CallEvent>) {
    let mut target: Option<SavedRoom> = None;
    let mut muted = false;
    let mut wants_camera = true;
    let mut wants_screen_share = false;
    let mut selected_microphone = String::new();
    let mut selected_speaker = String::new();
    let mut selected_camera = String::new();
    let mut noise_suppression = true;
    let mut echo_cancellation = true;
    let mut debug_stats = false;
    let mut remote_video_preference = RemoteVideoPreference::default();
    loop {
        if target.is_none() {
            let _ = events.send(CallEvent::Idle).await;
            match commands.recv().await {
                Ok(CallCommand::Select(room)) => target = room,
                Ok(CallCommand::SetMicrophoneMuted(value)) => muted = value,
                Ok(CallCommand::SetCameraEnabled(value)) => wants_camera = value,
                Ok(CallCommand::SetScreenShare(value)) => wants_screen_share = value,
                Ok(CallCommand::SetMicrophoneDevice(id)) => selected_microphone = id,
                Ok(CallCommand::SetSpeakerDevice(id)) => selected_speaker = id,
                Ok(CallCommand::SetCameraDevice(id)) => selected_camera = id,
                Ok(CallCommand::SetNoiseSuppression(value)) => noise_suppression = value,
                Ok(CallCommand::SetEchoCancellation(value)) => echo_cancellation = value,
                Ok(CallCommand::SetDebugStats(value)) => debug_stats = value,
                Ok(CallCommand::SetRemoteVideoPreference(value)) => remote_video_preference = value,
                Ok(CallCommand::PublishReaction(_)) => {}
                Ok(CallCommand::Shutdown) | Err(_) => break,
            }
            continue;
        }

        let room_target = target.clone().expect("checked above");
        let mut attempt = 0usize;
        'reconnect: loop {
            let _ = events
                .send(CallEvent::Connecting {
                    remote: room_target.remote_identity.clone(),
                })
                .await;

            // PlatformAudio must acquire/configure WebRTC's ADM before the
            // peer connection is created. Otherwise Linux can successfully
            // publish an audio track backed by an inactive device module.
            let platform_audio = match configure_platform_audio(
                &selected_microphone,
                &selected_speaker,
                noise_suppression,
                echo_cancellation,
            ) {
                Ok(ConfiguredAudio {
                    audio,
                    microphones,
                    speakers,
                    selected_microphone: microphone,
                    selected_speaker: speaker,
                }) => {
                    selected_microphone = microphone;
                    selected_speaker = speaker;
                    let cameras = camera::devices();
                    if selected_camera.is_empty() {
                        // Preserve the pre-selector behavior: Nokhwa's index 0
                        // is the platform-selected/default camera even when its
                        // enumeration order contains auxiliary Surface sensors first.
                        selected_camera = "0".to_owned();
                    } else if !cameras.iter().any(|device| device.id == selected_camera) {
                        selected_camera = "0".to_owned();
                    }
                    let _ = events
                        .send(CallEvent::MediaDevices {
                            microphones,
                            speakers,
                            cameras,
                            selected_microphone: selected_microphone.clone(),
                            selected_speaker: selected_speaker.clone(),
                            selected_camera: selected_camera.clone(),
                            noise_suppression,
                            echo_cancellation,
                        })
                        .await;
                    Some(audio)
                }
                Err(error) => {
                    let _ = events
                        .send(CallEvent::Error(format!(
                            "Could not start the audio devices: {error}"
                        )))
                        .await;
                    None
                }
            };

            let mut options = RoomOptions::default();
            options.dynacast = true;
            let connection =
                Room::connect(&room_target.server_url, &room_target.token, options).await;
            let (room, mut room_events) = match connection {
                Ok(value) => value,
                Err(error) => {
                    let _ = events.send(CallEvent::Error(error.to_string())).await;
                    let delays = [1, 2, 4, 8, 15, 30];
                    let delay = Duration::from_secs(delays[attempt.min(delays.len() - 1)]);
                    attempt += 1;
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => continue,
                        command = commands.recv() => match command {
                            Ok(CallCommand::Select(room)) => { target = room; break 'reconnect; }
                            Ok(CallCommand::SetMicrophoneMuted(value)) => muted = value,
                            Ok(CallCommand::SetCameraEnabled(value)) => wants_camera = value,
                            Ok(CallCommand::SetScreenShare(value)) => wants_screen_share = value,
                            Ok(CallCommand::SetMicrophoneDevice(id)) => selected_microphone = id,
                            Ok(CallCommand::SetSpeakerDevice(id)) => selected_speaker = id,
                            Ok(CallCommand::SetCameraDevice(id)) => selected_camera = id,
                            Ok(CallCommand::SetNoiseSuppression(value)) => noise_suppression = value,
                            Ok(CallCommand::SetEchoCancellation(value)) => echo_cancellation = value,
                            Ok(CallCommand::SetDebugStats(value)) => debug_stats = value,
                            Ok(CallCommand::SetRemoteVideoPreference(value)) => {
                                remote_video_preference = value
                            },
                            Ok(CallCommand::PublishReaction(_)) => {},
                            Ok(CallCommand::Shutdown) | Err(_) => return,
                        }
                    }
                    continue;
                }
            };

            let microphone_track = if let Some(audio) = platform_audio.as_ref() {
                let track = LocalAudioTrack::create_audio_track("microphone", audio.rtc_source());
                if muted {
                    track.mute();
                } else {
                    track.unmute();
                }
                match room
                    .local_participant()
                    .publish_track(
                        LocalTrack::Audio(track.clone()),
                        TrackPublishOptions {
                            source: TrackSource::Microphone,
                            ..Default::default()
                        },
                    )
                    .await
                {
                    Ok(_) => Some(track),
                    Err(error) => {
                        let _ = events
                            .send(CallEvent::Error(format!(
                                "Could not publish the microphone: {error}"
                            )))
                            .await;
                        None
                    }
                }
            } else {
                None
            };
            let mut active_camera = None;
            if wants_camera {
                match camera::start(&room, events.clone(), Some(selected_camera.clone())).await {
                    Ok(camera) => {
                        active_camera = Some(camera);
                        let _ = events.send(CallEvent::Camera(true)).await;
                    }
                    Err(error) => {
                        wants_camera = false;
                        let _ = events.send(CallEvent::Camera(false)).await;
                        let _ = events
                            .send(CallEvent::Error(format!(
                                "Could not start the camera: {error:#}"
                            )))
                            .await;
                    }
                }
            }
            let mut active_screen_share = None;
            if wants_screen_share {
                match screen_share::start(&room, events.clone()).await {
                    Ok(share) => {
                        active_screen_share = Some(share);
                        let _ = events.send(CallEvent::ScreenShare(true)).await;
                    }
                    Err(error) => {
                        wants_screen_share = false;
                        let _ = events.send(CallEvent::ScreenShare(false)).await;
                        let _ = events
                            .send(CallEvent::Error(format!(
                                "Could not share the screen: {error:#}"
                            )))
                            .await;
                    }
                }
            }

            let _ = events
                .send(CallEvent::Waiting {
                    remote: room_target.remote_identity.clone(),
                })
                .await;
            publish_presence(&room, PresenceMessage::ready(muted)).await;
            publish_presence(&room, PresenceMessage::request()).await;
            let stable = tokio::time::sleep(Duration::from_secs(20));
            tokio::pin!(stable);
            let mut connection_stable = false;
            let mut stats_tick = tokio::time::interval(Duration::from_millis(500));
            stats_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            stats_tick.tick().await;
            let mut stats_history = HashMap::new();
            let mut remote_camera_task: Option<RemoteVideo> = None;
            let mut remote_screen_task: Option<RemoteVideo> = None;

            loop {
                tokio::select! {
                    _ = &mut stable, if !connection_stable => {
                        attempt = 0;
                        connection_stable = true;
                    }
                    _ = stats_tick.tick() => {
                        if debug_stats && let Ok(stats) = room.get_stats().await {
                            let mut outgoing = Vec::new();
                            if let Some(camera) = active_camera.as_ref() {
                                outgoing.extend(camera.stats().await);
                            }
                            if let Some(screen) = active_screen_share.as_ref() {
                                outgoing.extend(screen.stats().await);
                            }
                            let mut incoming = Vec::new();
                            if let Some(video) = remote_camera_task.as_ref() {
                                incoming.extend(video.track.get_stats().await.unwrap_or_default());
                            }
                            if let Some(video) = remote_screen_task.as_ref() {
                                incoming.extend(video.track.get_stats().await.unwrap_or_default());
                            }
                            let text = format_debug_stats(
                                &outgoing,
                                &stats.publisher_stats,
                                &incoming,
                                &mut stats_history,
                            );
                            let _ = events.send(CallEvent::DebugStats(text)).await;
                          }
                    }
                    command = commands.recv() => match command {
                        Ok(CallCommand::Select(room_target_new)) => {
                            target = room_target_new;
                            let _ = tokio::time::timeout(Duration::from_secs(2), room.close()).await;
                            break 'reconnect;
                        }
                        Ok(CallCommand::SetMicrophoneMuted(value)) => {
                            muted = value;
                            if let Some(track) = microphone_track.as_ref() {
                                if muted {
                                    track.mute();
                                } else {
                                    track.unmute();
                                }
                            }
                            publish_presence(&room, PresenceMessage::state(muted)).await;
                            let _ = events.send(CallEvent::MicrophoneMuted(muted)).await;
                        }
                        Ok(CallCommand::SetCameraEnabled(value)) => {
                            wants_camera = value;
                            if value && active_camera.is_none() {
                                match camera::start(&room, events.clone(), Some(selected_camera.clone())).await {
                                    Ok(camera) => {
                                        active_camera = Some(camera);
                                        let _ = events.send(CallEvent::Camera(true)).await;
                                    }
                                    Err(error) => {
                                        wants_camera = false;
                                        let _ = events.send(CallEvent::Camera(false)).await;
                                        let _ = events.send(CallEvent::Error(format!(
                                            "Could not start the camera: {error:#}"
                                        ))).await;
                                    }
                                }
                            } else if !value {
                                if let Some(camera) = active_camera.take() {
                                    camera.stop(&room).await;
                                }
                                let _ = events.send(CallEvent::Camera(false)).await;
                            }
                        }
                        Ok(CallCommand::SetScreenShare(value)) => {
                            wants_screen_share = value;
                            if value && active_screen_share.is_none() {
                                match screen_share::start(&room, events.clone()).await {
                                    Ok(share) => {
                                        active_screen_share = Some(share);
                                        let _ = events.send(CallEvent::ScreenShare(true)).await;
                                    }
                                    Err(error) => {
                                        wants_screen_share = false;
                                        let _ = events.send(CallEvent::ScreenShare(false)).await;
                                        let _ = events.send(CallEvent::Error(format!(
                                            "Could not share the screen: {error:#}"
                                        ))).await;
                                    }
                                }
                            } else if !value {
                                if let Some(share) = active_screen_share.take() {
                                    share.stop(&room).await;
                                }
                                let _ = events.send(CallEvent::ScreenShare(false)).await;
                            }
                        }
                        Ok(CallCommand::SetMicrophoneDevice(id)) => {
                            if let Some(audio) = platform_audio.as_ref() {
                                let device = RecordingDeviceId::from_unchecked_guid(audio_device_guid(&id));
                                match audio.switch_recording_device(&device) {
                                    Ok(()) => selected_microphone = id,
                                    Err(error) => {
                                        let _ = events.send(CallEvent::Error(format!(
                                            "Could not change microphone: {error}"
                                        ))).await;
                                    }
                                }
                            }
                        }
                        Ok(CallCommand::SetSpeakerDevice(id)) => {
                            if let Some(audio) = platform_audio.as_ref() {
                                let device = PlayoutDeviceId::from_unchecked_guid(audio_device_guid(&id));
                                match audio.switch_playout_device(&device) {
                                    Ok(()) => selected_speaker = id,
                                    Err(error) => {
                                        let _ = events.send(CallEvent::Error(format!(
                                            "Could not change speaker: {error}"
                                        ))).await;
                                    }
                                }
                            }
                        }
                        Ok(CallCommand::SetCameraDevice(id)) => {
                            if selected_camera != id {
                                selected_camera = id;
                                if let Some(camera) = active_camera.take() {
                                    camera.stop(&room).await;
                                }
                                if wants_camera {
                                    match camera::start(
                                        &room,
                                        events.clone(),
                                        Some(selected_camera.clone()),
                                    ).await {
                                        Ok(camera) => active_camera = Some(camera),
                                        Err(error) => {
                                            let _ = events.send(CallEvent::Error(format!(
                                                "Could not change camera: {error:#}"
                                            ))).await;
                                        }
                                    }
                                }
                            }
                        }
                        Ok(CallCommand::SetNoiseSuppression(value)) => {
                            noise_suppression = value;
                            if let Some(audio) = platform_audio.as_ref() {
                                let options = AudioProcessingOptions {
                                    noise_suppression: value,
                                    echo_cancellation,
                                    ..Default::default()
                                };
                                if let Err(error) = audio.configure_audio_processing(options) {
                                    let _ = events.send(CallEvent::Error(format!(
                                        "Could not update noise suppression: {error}"
                                    ))).await;
                                }
                            }
                        }
                        Ok(CallCommand::SetEchoCancellation(value)) => {
                            echo_cancellation = value;
                            if let Some(audio) = platform_audio.as_ref() {
                                let options = AudioProcessingOptions {
                                    echo_cancellation: value,
                                    noise_suppression,
                                    ..Default::default()
                                };
                                if let Err(error) = audio.configure_audio_processing(options) {
                                    let _ = events.send(CallEvent::Error(format!(
                                        "Could not update echo cancellation: {error}"
                                    ))).await;
                                }
                            }
                        }
                        Ok(CallCommand::SetDebugStats(value)) => {
                            debug_stats = value;
                            if !value {
                                stats_history.clear();
                            }
                        }
                        Ok(CallCommand::SetRemoteVideoPreference(value)) => {
                            remote_video_preference = value;
                            if let Some(video) = remote_camera_task.as_ref() {
                                apply_remote_video_quality(&video.publication, value);
                            }
                        }
                        Ok(CallCommand::PublishReaction(message)) => {
                            publish_reaction(&room, &message).await;
                        }
                        Ok(CallCommand::Shutdown) | Err(_) => {
                            if let Some(camera) = active_camera.take() {
                                camera.stop(&room).await;
                            }
                            if let Some(share) = active_screen_share.take() {
                                share.stop(&room).await;
                            }
                            let _ = tokio::time::timeout(Duration::from_secs(2), room.close()).await;
                            return;
                        }
                    },
                    event = room_events.recv() => match event {
                        Some(RoomEvent::ParticipantActive(participant))
                            if participant.identity().to_string() == room_target.remote_identity => {
                                let _ = events.send(CallEvent::RemoteJoining {
                                    remote: room_target.remote_identity.clone(),
                                }).await;
                                publish_presence(&room, PresenceMessage::request()).await;
                                publish_presence(&room, PresenceMessage::ready(muted)).await;
                            }
                        Some(RoomEvent::ParticipantDisconnected(participant))
                            if participant.identity().to_string() == room_target.remote_identity => {
                                remote_camera_task.take();
                                remote_screen_task.take();
                                let _ = events.send(CallEvent::VideoEnded(VideoKind::Camera)).await;
                                let _ = events.send(CallEvent::VideoEnded(VideoKind::ScreenShare)).await;
                                let _ = events.send(CallEvent::Waiting {
                                    remote: room_target.remote_identity.clone(),
                                }).await;
                            }
                        Some(RoomEvent::ConnectionQualityChanged { quality: ConnectionQuality::Lost, participant })
                            if participant.identity().to_string() == room_target.remote_identity => {
                                remote_camera_task.take();
                                remote_screen_task.take();
                                let _ = events.send(CallEvent::VideoEnded(VideoKind::Camera)).await;
                                let _ = events.send(CallEvent::VideoEnded(VideoKind::ScreenShare)).await;
                                let _ = events.send(CallEvent::Waiting {
                                    remote: room_target.remote_identity.clone(),
                                }).await;
                            }
                        Some(RoomEvent::DataReceived { payload, topic: Some(topic), participant: Some(participant), .. })
                            if participant.identity().to_string() == room_target.remote_identity => {
                                if topic == TOPIC {
                                    if let Ok(message) = serde_json::from_slice::<PresenceMessage>(&payload) {
                                        match message.kind {
                                            PresenceKind::Request => publish_presence(&room, PresenceMessage::ready(muted)).await,
                                            PresenceKind::Ready | PresenceKind::State => {
                                                let _ = events.send(CallEvent::Ready {
                                                    remote: room_target.remote_identity.clone(),
                                                    remote_muted: message.microphone_muted.unwrap_or(false),
                                                }).await;
                                            }

                                        }
                                    } else if let Ok(message) = serde_json::from_slice::<ReactionMessage>(&payload) {
                                        match message.kind {
                                            ReactionKind::Event => {
                                                let _ = events.send(CallEvent::Reaction(message)).await;
                                            }
                                            ReactionKind::Request => {
                                                send_reaction_file(
                                                    &room,
                                                    &message.hash,
                                                    participant.identity().to_string(),
                                                ).await;
                                            }
                                        }
                                    }
                                }
                            }
                        Some(RoomEvent::ByteStreamOpened { reader, topic, participant_identity })
                            if topic == reaction::FILE_TOPIC
                                && participant_identity.as_str() == room_target.remote_identity => {
                                if let Some(reader) = reader.take() {
                                    let hash = reader.info().attributes().get("hash").cloned();
                                    let length = reader.info().total_length;
                                    if let (Some(hash), Some(length)) = (hash, length) && length <= crate::reaction::MAXIMUM_BYTES as u64 {
                                            match reader.read_all().await {
                                                Ok(data) => {
                                                    if crate::reaction::store_received_file(&data, &hash).is_ok() {
                                                        let _ = events.send(CallEvent::ReactionFileReady { hash }).await;
                                                    }
                                                }
                                                Err(error) => {
                                                    log::warn!("reaction file stream failed: {error}");
                                                }
                                            }
                                        }
                                }
                            }
                        Some(RoomEvent::TrackSubscribed { track: RemoteTrack::Video(track), publication, participant })
                            if participant.identity().to_string() == room_target.remote_identity => {
                                let kind = if publication.source() == TrackSource::Screenshare {
                                    VideoKind::ScreenShare
                                } else {
                                    VideoKind::Camera
                                };
                                let sid = track.sid().to_string();
                                log::info!(
                                    "remote {:?} subscribed ({sid}, muted={})",
                                    publication.source(),
                                    publication.is_muted()
                                );
                                let task_slot = if kind == VideoKind::ScreenShare {
                                    &mut remote_screen_task
                                } else {
                                    &mut remote_camera_task
                                };
                                if let Some(video) = task_slot.take() {
                                    video.stop().await;
                                }
                                apply_remote_video_quality(&publication, remote_video_preference);
                                let stats_track = track.clone();
                                // A NativeVideoStream attached while a publication is muted does
                                // not reliably start receiving when that publication is unmuted.
                                // Wait for TrackUnmuted and attach the sink at that point instead.
                                let task = if publication.is_muted() {
                                    None
                                } else {
                                    Some(spawn_remote_video_renderer(
                                        track,
                                        publication.clone(),
                                        kind,
                                        events.clone(),
                                    ))
                                };
                                *task_slot = Some(RemoteVideo { sid, track: stats_track, publication, renderer: task });
                            }
                        Some(RoomEvent::TrackSubscribed { track: RemoteTrack::Audio(track), participant, .. })
                            if participant.identity().to_string() == room_target.remote_identity => {
                                track.enable();
                                log::info!("remote microphone subscribed and enabled");
                            }
                        Some(RoomEvent::TrackMuted { publication, participant })
                            if participant.identity().to_string() == room_target.remote_identity => {
                                if let Some(kind) = remote_video_kind(publication.source()) {
                                    let task_slot = if kind == VideoKind::ScreenShare {
                                        &mut remote_screen_task
                                    } else {
                                        &mut remote_camera_task
                                    };
                                    if let Some(video) = task_slot.as_mut()
                                        && video.sid == publication.sid().to_string()
                                    {
                                        video.stop_rendering().await;
                                        let _ = events.send(CallEvent::VideoEnded(kind)).await;
                                    }
                                }
                            }
                        Some(RoomEvent::TrackUnmuted { publication, participant })
                            if participant.identity().to_string() == room_target.remote_identity => {
                                let livekit::prelude::TrackPublication::Remote(publication) = publication else {
                                    continue;
                                };
                                publication.set_subscribed(true);
                                log::info!(
                                    "remote {:?} unmuted ({}), requesting subscription; track attached={}",
                                    publication.source(),
                                    publication.sid(),
                                    publication.track().is_some()
                                );
                                let Some(kind) = remote_video_kind(publication.source()) else {
                                    continue;
                                };
                                let task_slot = if kind == VideoKind::ScreenShare {
                                    &mut remote_screen_task
                                } else {
                                    &mut remote_camera_task
                                };
                                let Some(RemoteTrack::Video(track)) = publication.track() else {
                                    continue;
                                };
                                if let Some(video) = task_slot.take() {
                                    video.stop().await;
                                }
                                apply_remote_video_quality(&publication, remote_video_preference);
                                let sid = track.sid().to_string();
                                let stats_track = track.clone();
                                let task = Some(spawn_remote_video_renderer(
                                    track,
                                    publication.clone(),
                                    kind,
                                    events.clone(),
                                ));
                                *task_slot = Some(RemoteVideo { sid, track: stats_track, publication, renderer: task });
                            }
                        Some(RoomEvent::TrackPublished { publication, participant })
                            if participant.identity().to_string() == room_target.remote_identity
                                && remote_video_kind(publication.source()).is_some() => {
                                log::info!(
                                    "remote {:?} published ({}), requesting subscription; muted={}",
                                    publication.source(),
                                    publication.sid(),
                                    publication.is_muted()
                                );
                                publication.set_subscribed(true);
                                apply_remote_video_quality(&publication, remote_video_preference);
                            }
                        Some(RoomEvent::TrackUnsubscribed { track: RemoteTrack::Video(track), publication, participant })
                            if participant.identity().to_string() == room_target.remote_identity => {
                                if let Some(kind) = remote_video_kind(publication.source()) {
                                    let sid = track.sid().to_string();
                                    let task_slot = if kind == VideoKind::ScreenShare {
                                        &mut remote_screen_task
                                    } else {
                                        &mut remote_camera_task
                                    };
                                    if task_slot.as_ref().is_some_and(|video| video.sid == sid) {
                                        if let Some(video) = task_slot.take() {
                                            video.stop().await;
                                        }
                                        let _ = events.send(CallEvent::VideoEnded(kind)).await;
                                    }
                                }
                            }
                        Some(RoomEvent::TrackUnpublished { publication, participant })
                            if participant.identity().to_string() == room_target.remote_identity => {
                                if let Some(kind) = remote_video_kind(publication.source()) {
                                    let sid = publication.sid().to_string();
                                    let task_slot = if kind == VideoKind::ScreenShare {
                                        &mut remote_screen_task
                                    } else {
                                        &mut remote_camera_task
                                    };
                                    if task_slot.as_ref().is_some_and(|video| video.sid == sid) {
                                        if let Some(video) = task_slot.take() {
                                            video.stop().await;
                                        }
                                        let _ = events.send(CallEvent::VideoEnded(kind)).await;
                                    }
                                }
                            }
                        Some(RoomEvent::Disconnected { reason }) => {
                            let _ = events.send(CallEvent::Error(format!("Disconnected: {reason:?}"))).await;
                            break;
                        }
                        None => break,
                        _ => {}
                    }
                }
            }

            if let Some(video) = remote_camera_task.take() {
                video.stop().await;
            }
            if let Some(video) = remote_screen_task.take() {
                video.stop().await;
            }
            let _ = events.send(CallEvent::VideoEnded(VideoKind::Camera)).await;
            let _ = events
                .send(CallEvent::VideoEnded(VideoKind::ScreenShare))
                .await;

            if target
                .as_ref()
                .is_none_or(|new_target| new_target.id != room_target.id)
            {
                break;
            }
            let delays = [1, 2, 4, 8, 15, 30];
            let delay = Duration::from_secs(delays[attempt.min(delays.len() - 1)]);
            attempt += 1;
            tokio::time::sleep(delay).await;
        }
    }
}

async fn publish_reaction(room: &Room, message: &ReactionMessage) {
    let Ok(payload) = serde_json::to_vec(message) else {
        return;
    };
    let _ = room
        .local_participant()
        .publish_data(DataPacket {
            payload,
            topic: Some(reaction::TOPIC.to_owned()),
            reliable: true,
            ..Default::default()
        })
        .await;
}

async fn send_reaction_file(room: &Room, hash: &str, destination: String) {
    let Some(path) = crate::reaction::cached_file_path(hash) else {
        return;
    };
    let options = StreamByteOptions::new_with_topic(reaction::FILE_TOPIC)
        .with_attribute("hash", hash.to_owned())
        .with_destination_identity(destination)
        .with_mime_type("audio/mp4")
        .with_name(format!("{hash}.m4a"));
    if let Err(error) = room.local_participant().send_file(path, options).await {
        log::warn!("could not send reaction file: {error}");
    }
}

async fn publish_presence(room: &Room, message: PresenceMessage) {
    let Ok(payload) = serde_json::to_vec(&message) else {
        return;
    };
    let _ = room
        .local_participant()
        .publish_data(DataPacket {
            payload,
            topic: Some(TOPIC.to_owned()),
            reliable: true,
            ..Default::default()
        })
        .await;
}

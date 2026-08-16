use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use async_channel::Sender;
use livekit::{
    Room,
    options::{DegradationPreference, TrackPublishOptions, VideoEncoding},
    prelude::{LocalTrack, LocalVideoTrack, TrackSource},
    webrtc::{
        desktop_capturer::{
            CaptureError, DesktopCaptureSourceType, DesktopCapturer, DesktopCapturerOptions,
            DesktopFrame,
        },
        native::yuv_helper,
        prelude::{
            I420Buffer, RtcVideoSource, VideoBuffer, VideoFrame, VideoResolution, VideoRotation,
        },
        video_source::native::NativeVideoSource,
    },
};

use super::{CallEvent, VideoKind, video_encoding::preferred_video_encoding};

enum CaptureCommand {
    Stop,
}

type ResolutionSender = tokio::sync::oneshot::Sender<Result<VideoResolution, String>>;
type VideoSourceSlot = Arc<Mutex<Option<NativeVideoSource>>>;

pub struct ActiveScreenShare {
    command: mpsc::Sender<CaptureCommand>,
    thread: Option<thread::JoinHandle<()>>,
    track: LocalVideoTrack,
}

impl ActiveScreenShare {
    pub async fn stats(&self) -> Vec<livekit::webrtc::stats::RtcStats> {
        self.track.get_stats().await.unwrap_or_default()
    }

    pub async fn stop(mut self, room: &Room) {
        let _ = room
            .local_participant()
            .unpublish_track(&self.track.sid())
            .await;
        let _ = self.command.send(CaptureCommand::Stop);
        if let Some(handle) = self.thread.take() {
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
        }
    }
}

impl Drop for ActiveScreenShare {
    fn drop(&mut self) {
        let _ = self.command.send(CaptureCommand::Stop);
    }
}

pub async fn start(room: &Room, events: Sender<CallEvent>) -> Result<ActiveScreenShare> {
    let (resolution_tx, resolution_rx) = tokio::sync::oneshot::channel();
    let video_source_slot: VideoSourceSlot = Arc::new(Mutex::new(None));
    let (command, thread) = spawn_capture_thread(resolution_tx, video_source_slot.clone(), events);

    let resolution = tokio::time::timeout(Duration::from_secs(120), resolution_rx)
        .await
        .context("The screen picker timed out")?
        .context("Screen capture stopped before producing a frame")?
        .map_err(anyhow::Error::msg)?;
    let source = NativeVideoSource::new(resolution.clone(), true);
    *video_source_slot.lock().expect("screen source mutex") = Some(source.clone());
    let track = LocalVideoTrack::create_video_track("screen_share", RtcVideoSource::Native(source));

    let pixels = u64::from(resolution.width) * u64::from(resolution.height);
    let bitrate = ((pixels * 20_000_000).div_ceil(1920 * 1080)).clamp(2_000_000, 40_000_000);
    let encoding = preferred_video_encoding();
    let options = TrackPublishOptions {
        source: TrackSource::Screenshare,
        video_codec: encoding.codec,
        video_encoder: encoding.encoder,
        video_encoding: Some(VideoEncoding {
            max_bitrate: bitrate,
            max_framerate: 30.0,
        }),
        simulcast: false,
        degradation_preference: Some(DegradationPreference::Balanced),
        ..Default::default()
    };
    let publication = room
        .local_participant()
        .publish_track(LocalTrack::Video(track.clone()), options)
        .await
        .context("Could not publish screen share")?;
    log::info!(
        "screen-share track published as {:?} via {:?} ({})",
        encoding.codec,
        encoding.encoder,
        publication.sid()
    );

    Ok(ActiveScreenShare {
        command,
        thread: Some(thread),
        track,
    })
}

fn spawn_capture_thread(
    resolution: ResolutionSender,
    video_source_slot: VideoSourceSlot,
    events: Sender<CallEvent>,
) -> (mpsc::Sender<CaptureCommand>, thread::JoinHandle<()>) {
    let (command, commands) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut options = DesktopCapturerOptions::new(DesktopCaptureSourceType::Generic);
        options.set_include_cursor(true);
        let mut capturer = match DesktopCapturer::new(options) {
            Some(capturer) => capturer,
            None => {
                let _ = resolution.send(Err("Could not open the screen picker".to_owned()));
                return;
            }
        };
        let callback = {
            let mut resolution = Some(resolution);
            let events = events.clone();
            let started = Instant::now();
            let mut frame_buffer = VideoFrame {
                rotation: VideoRotation::VideoRotation0,
                timestamp_us: 0,
                frame_metadata: None,
                buffer: I420Buffer::new(1, 1),
            };
            move |result: Result<DesktopFrame, CaptureError>| {
                let frame = match result {
                    Ok(frame) => frame,
                    Err(CaptureError::Temporary) => return,
                    Err(CaptureError::Permanent) => {
                        if let Some(sender) = resolution.take() {
                            let _ =
                                sender.send(Err("Screen capture stopped unexpectedly".to_owned()));
                        }
                        return;
                    }
                };
                let width = frame.width();
                let height = frame.height();
                if let Some(sender) = resolution.take() {
                    let _ = sender.send(Ok(VideoResolution {
                        width: width as u32,
                        height: height as u32,
                    }));
                }

                if frame_buffer.buffer.width() as i32 != width
                    || frame_buffer.buffer.height() as i32 != height
                {
                    frame_buffer.buffer = I420Buffer::new(width as u32, height as u32);
                }
                frame_buffer.timestamp_us = started.elapsed().as_micros() as i64;
                let (stride_y, stride_u, stride_v) = frame_buffer.buffer.strides();
                let (y, u, v) = frame_buffer.buffer.data_mut();
                yuv_helper::argb_to_i420(
                    frame.data(),
                    frame.stride(),
                    y,
                    stride_y,
                    u,
                    stride_u,
                    v,
                    stride_v,
                    width,
                    height,
                );
                if let Some(source) = video_source_slot
                    .lock()
                    .expect("screen source mutex")
                    .as_ref()
                {
                    source.capture_frame(&frame_buffer);
                }

                let (stride_y, stride_u, stride_v) = frame_buffer.buffer.strides();
                let (y, u, v) = frame_buffer.buffer.data();
                let mut rgba = vec![0; width as usize * height as usize * 4];
                // Libyuv's names describe a 32-bit word. ABGR is the function
                // whose little-endian byte output is R, G, B, A.
                yuv_helper::i420_to_abgr(
                    y,
                    stride_y,
                    u,
                    stride_u,
                    v,
                    stride_v,
                    &mut rgba,
                    width as u32 * 4,
                    width,
                    height,
                );
                let _ = events.try_send(CallEvent::VideoFrame {
                    kind: VideoKind::LocalScreenShare,
                    width: width as u32,
                    height: height as u32,
                    rgba,
                });
            }
        };

        let selected = capturer.get_source_list().first().cloned();
        capturer.start_capture(selected, callback);
        loop {
            match commands.recv_timeout(Duration::from_millis(33)) {
                Ok(CaptureCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => capturer.capture_frame(),
            }
        }
        let _ = events.try_send(CallEvent::VideoEnded(VideoKind::LocalScreenShare));
    });
    (command, handle)
}

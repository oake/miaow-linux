use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Instant,
};

use anyhow::{Context, Result};
use async_channel::Sender;
use livekit::{
    Room,
    options::{DegradationPreference, TrackPublishOptions, VideoEncoding, VideoPreset},
    prelude::{LocalTrack, LocalVideoTrack, TrackSource},
    webrtc::{
        native::yuv_helper,
        prelude::{I420Buffer, RtcVideoSource, VideoFrame, VideoResolution, VideoRotation},
        video_source::native::NativeVideoSource,
    },
};
use nokhwa::{
    Camera,
    pixel_format::RgbAFormat,
    query,
    utils::{
        ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
        Resolution,
    },
};

use super::{
    CallEvent, MediaDevice, VideoKind,
    video_encoding::{camera_max_bitrate, preferred_video_encoding},
};

pub fn devices() -> Vec<MediaDevice> {
    query(ApiBackend::Auto)
        .unwrap_or_default()
        .into_iter()
        .map(|camera| MediaDevice {
            id: camera.index().as_string(),
            name: camera.human_name(),
        })
        .collect()
}

pub struct ActiveCamera {
    running: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    track: LocalVideoTrack,
}

impl ActiveCamera {
    pub async fn stats(&self) -> Vec<livekit::webrtc::stats::RtcStats> {
        self.track.get_stats().await.unwrap_or_default()
    }

    pub async fn stop(mut self, room: &Room) {
        // Stop feeding native frames before tearing down the sender/encoder.
        self.running.store(false, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
        }
        let _ = room
            .local_participant()
            .unpublish_track(&self.track.sid())
            .await;
    }
}

impl Drop for ActiveCamera {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
    }
}

pub async fn start(
    room: &Room,
    events: Sender<CallEvent>,
    device_id: Option<String>,
) -> Result<ActiveCamera> {
    let (camera, width, height) = tokio::task::spawn_blocking(move || open_camera(device_id))
        .await
        .context("Camera setup task failed")??;
    let source = NativeVideoSource::new(VideoResolution { width, height }, false);
    let track =
        LocalVideoTrack::create_video_track("camera", RtcVideoSource::Native(source.clone()));
    let encoding = preferred_video_encoding();
    let publish_options = TrackPublishOptions {
        source: TrackSource::Camera,
        video_codec: encoding.codec,
        video_encoder: encoding.encoder,
        video_encoding: Some(VideoEncoding {
            max_bitrate: camera_max_bitrate(encoding.codec),
            max_framerate: 60.0,
        }),
        simulcast: true,
        simulcast_layers: Some(vec![
            VideoPreset::new(640, 360, 800_000, 30.0),
            VideoPreset::new(960, 540, 2_500_000, 30.0),
        ]),
        degradation_preference: Some(DegradationPreference::MaintainFramerate),
        ..Default::default()
    };
    let publication = room
        .local_participant()
        .publish_track(LocalTrack::Video(track.clone()), publish_options)
        .await
        .context("Could not publish camera")?;
    log::info!(
        "camera track published as {:?} via {:?} at {} bps ({})",
        encoding.codec,
        encoding.encoder,
        camera_max_bitrate(encoding.codec),
        publication.sid()
    );

    let running = Arc::new(AtomicBool::new(true));
    let running_for_thread = running.clone();
    let thread =
        thread::spawn(move || capture(camera, source, width, height, running_for_thread, events));
    Ok(ActiveCamera {
        running,
        thread: Some(thread),
        track,
    })
}

fn open_camera(device_id: Option<String>) -> Result<(Camera, u32, u32)> {
    let requested = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::Exact(
        CameraFormat::new(Resolution::new(1920, 1080), FrameFormat::MJPEG, 60),
    ));
    let index = device_id
        .map(|id| {
            id.parse::<u32>()
                .map(CameraIndex::Index)
                .unwrap_or(CameraIndex::String(id))
        })
        .unwrap_or(CameraIndex::Index(0));
    let mut camera = Camera::new(index.clone(), requested)
        .or_else(|_| {
            Camera::new(
                index,
                RequestedFormat::new::<RgbAFormat>(RequestedFormatType::AbsoluteHighestFrameRate),
            )
        })
        .context("Could not open the default camera")?;
    camera.open_stream().context("Could not start the camera")?;
    let format = camera.camera_format();
    log::info!(
        "camera opened at {}x{} {} fps ({})",
        format.width(),
        format.height(),
        format.frame_rate(),
        format.format()
    );
    Ok((camera, format.width(), format.height()))
}

fn capture(
    mut camera: Camera,
    source: NativeVideoSource,
    width: u32,
    height: u32,
    running: Arc<AtomicBool>,
    events: Sender<CallEvent>,
) {
    let started = Instant::now();
    while running.load(Ordering::Acquire) {
        let frame = match camera.frame() {
            Ok(frame) => frame,
            Err(error) => {
                log::warn!("camera frame failed: {error}");
                continue;
            }
        };
        let rgba = match frame.decode_image::<RgbAFormat>() {
            Ok(image) => image,
            Err(error) => {
                log::warn!("camera frame decode failed: {error}");
                continue;
            }
        };
        if rgba.width() != width || rgba.height() != height {
            continue;
        }
        let mut video = VideoFrame {
            rotation: VideoRotation::VideoRotation0,
            timestamp_us: started.elapsed().as_micros() as i64,
            frame_metadata: None,
            buffer: I420Buffer::new(width, height),
        };
        let (stride_y, stride_u, stride_v) = video.buffer.strides();
        let (y, u, v) = video.buffer.data_mut();
        yuv_helper::abgr_to_i420(
            rgba.as_raw(),
            width * 4,
            y,
            stride_y,
            u,
            stride_u,
            v,
            stride_v,
            width as i32,
            height as i32,
        );
        source.capture_frame(&video);
        let _ = events.try_send(CallEvent::VideoFrame {
            kind: VideoKind::LocalCamera,
            width,
            height,
            rgba: rgba.into_raw(),
        });
    }
    let _ = events.try_send(CallEvent::VideoEnded(VideoKind::LocalCamera));
    let _ = camera.stop_stream();
}

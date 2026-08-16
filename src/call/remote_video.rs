use super::{CallEvent, RemoteVideoPreference, VideoKind};
use async_channel::Sender;
use futures_util::StreamExt;
use livekit::{
    prelude::{RemoteTrackPublication, RemoteVideoTrack, TrackSource},
    track::VideoQuality,
    webrtc::{native::yuv_helper, video_stream::native::NativeVideoStream},
};

/// A subscribed track and its optional sink. Dropping it always cancels the
/// renderer, including early returns when switching rooms or shutting down.
pub(super) struct RemoteVideo {
    pub sid: String,
    pub track: RemoteVideoTrack,
    pub publication: RemoteTrackPublication,
    pub renderer: Option<tokio::task::JoinHandle<()>>,
}

impl RemoteVideo {
    pub async fn stop(mut self) {
        self.stop_rendering().await;
    }

    pub async fn stop_rendering(&mut self) {
        if let Some(task) = self.renderer.take() {
            task.abort();
            // Await cancellation so the native sink is detached before a
            // replacement attaches or the UI receives VideoEnded.
            let _ = task.await;
        }
    }
}

impl Drop for RemoteVideo {
    fn drop(&mut self) {
        if let Some(task) = self.renderer.take() {
            task.abort();
        }
    }
}

pub(super) fn remote_video_kind(source: TrackSource) -> Option<VideoKind> {
    match source {
        TrackSource::Camera => Some(VideoKind::Camera),
        TrackSource::Screenshare => Some(VideoKind::ScreenShare),
        _ => None,
    }
}

pub(super) fn spawn_remote_video_renderer(
    track: RemoteVideoTrack,
    publication: RemoteTrackPublication,
    kind: VideoKind,
    events: Sender<CallEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // A republished camera can reuse a WebRTC receiver that the previous
        // publication disabled. Re-enable it when attaching the replacement.
        publication.set_enabled(true);
        track.enable();
        let mut stream = NativeVideoStream::new(track.rtc_track());
        let mut received_first_frame = false;
        while let Some(frame) = stream.next().await {
            // LiveKit updates the publication's mute state on later participant
            // updates, but the RemoteVideoTrack mute flag can retain its value
            // from subscription time. The publication is therefore the source
            // of truth for whether incoming frames should be displayed.
            if publication.is_muted() {
                continue;
            }
            if !received_first_frame {
                log::info!("remote {kind:?} renderer received its first frame");
                received_first_frame = true;
            }
            let width = frame.buffer.width();
            let height = frame.buffer.height();
            let i420 = frame.buffer.to_i420();
            let (stride_y, stride_u, stride_v) = i420.strides();
            let (y, u, v) = i420.data();
            let mut rgba = vec![0; (width * height * 4) as usize];
            yuv_helper::i420_to_abgr(
                y,
                stride_y,
                u,
                stride_u,
                v,
                stride_v,
                &mut rgba,
                width * 4,
                width as i32,
                height as i32,
            );
            if let Err(error) = events.try_send(CallEvent::VideoFrame {
                kind,
                width,
                height,
                rgba,
            }) && error.is_closed()
            {
                return;
            }
        }
        log::info!("remote {kind:?} renderer stream ended");
        let _ = events.send(CallEvent::VideoEnded(kind)).await;
    })
}

pub(super) fn apply_remote_video_quality(
    publication: &RemoteTrackPublication,
    preference: RemoteVideoPreference,
) {
    if publication.source() != TrackSource::Camera || !publication.simulcasted() {
        return;
    }
    let quality = if preference.popup_mode {
        VideoQuality::Low
    } else if !preference.showing_remote_screen_on_main {
        if preference.large_window {
            VideoQuality::High
        } else {
            VideoQuality::Medium
        }
    } else if preference.remote_pip_enabled {
        VideoQuality::Medium
    } else {
        VideoQuality::Low
    };
    publication.set_video_quality(quality);
    log::debug!(
        "requested remote camera quality {quality:?} for {}",
        publication.sid()
    );
}

use super::ReactionMessage;
use crate::room::SavedRoom;

#[derive(Clone, Debug)]
pub enum CallCommand {
    Select(Option<SavedRoom>),
    SetMicrophoneMuted(bool),
    SetCameraEnabled(bool),
    SetScreenShare(bool),
    SetMicrophoneDevice(String),
    SetSpeakerDevice(String),
    SetCameraDevice(String),
    SetNoiseSuppression(bool),
    SetEchoCancellation(bool),
    SetDebugStats(bool),
    SetRemoteVideoPreference(RemoteVideoPreference),
    PublishReaction(ReactionMessage),
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum CallEvent {
    Idle,
    Connecting {
        remote: String,
    },
    Waiting {
        remote: String,
    },
    RemoteJoining {
        remote: String,
    },
    Ready {
        remote: String,
        remote_muted: bool,
    },
    MicrophoneMuted(bool),
    Camera(bool),
    ScreenShare(bool),
    MediaDevices {
        microphones: Vec<MediaDevice>,
        speakers: Vec<MediaDevice>,
        cameras: Vec<MediaDevice>,
        selected_microphone: String,
        selected_speaker: String,
        selected_camera: String,
        noise_suppression: bool,
        echo_cancellation: bool,
    },
    VideoFrame {
        kind: VideoKind,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    VideoEnded(VideoKind),
    DebugStats(String),
    Reaction(ReactionMessage),
    ReactionFileReady {
        hash: String,
    },
    Error(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaDevice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoKind {
    LocalCamera,
    LocalScreenShare,
    Camera,
    ScreenShare,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RemoteVideoPreference {
    pub popup_mode: bool,
    pub showing_remote_screen_on_main: bool,
    pub remote_pip_enabled: bool,
    pub large_window: bool,
}

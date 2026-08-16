use livekit::options::{VideoCodec, VideoEncoderBackend};

/// Camera send bitrate for 1080p60.
///
/// macOS publishes HEVC at 5 Mbps, which holds motion. Linux often lands on
/// H.264 (VA-API / OpenH264), which needs a much higher ceiling or movement
/// turns into blocks. HEVC still gets a raise so NVENC/Jetson motion holds.
pub fn camera_max_bitrate(codec: VideoCodec) -> u64 {
    match codec {
        VideoCodec::H265 => 8_000_000,
        _ => 20_000_000,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VideoEncodingChoice {
    pub codec: VideoCodec,
    pub encoder: VideoEncoderBackend,
}

pub fn preferred_video_encoding() -> VideoEncodingChoice {
    let available: Vec<_> = VideoEncoderBackend::list_available().into_iter().collect();
    let choice = choose_video_encoding(&available);
    log::info!(
        "video encoders available: {available:?}; selected {:?} via {:?}",
        choice.codec,
        choice.encoder
    );
    choice
}

fn choose_video_encoding(available: &[VideoEncoderBackend]) -> VideoEncodingChoice {
    if available.contains(&VideoEncoderBackend::Nvenc) {
        VideoEncodingChoice {
            codec: VideoCodec::H265,
            encoder: VideoEncoderBackend::Nvenc,
        }
    } else if available.contains(&VideoEncoderBackend::Vaapi) {
        // The pinned LiveKit VA-API H.264 wrapper crashes in Intel's
        // CodechalEncodeAvcBase::SetPictureStructs after camera restarts.
        // A native SIGSEGV cannot fall back at runtime. Explicit software
        // selection also prevents the generic Hardware factory selecting VA-API.
        VideoEncodingChoice {
            codec: VideoCodec::H264,
            encoder: VideoEncoderBackend::Software,
        }
    } else if available.contains(&VideoEncoderBackend::Hardware) {
        // Jetson and other platform hardware factories prefer HEVC and fall
        // back to another compatible implementation when it is unavailable.
        VideoEncodingChoice {
            codec: VideoCodec::H265,
            encoder: VideoEncoderBackend::Hardware,
        }
    } else {
        VideoEncodingChoice {
            codec: VideoCodec::H264,
            encoder: VideoEncoderBackend::Software,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vaapi_uses_software_even_when_generic_hardware_is_available() {
        let choice =
            choose_video_encoding(&[VideoEncoderBackend::Vaapi, VideoEncoderBackend::Hardware]);
        assert_eq!(choice.codec, VideoCodec::H264);
        assert_eq!(choice.encoder, VideoEncoderBackend::Software);
    }

    #[test]
    fn nvenc_remains_preferred() {
        let choice =
            choose_video_encoding(&[VideoEncoderBackend::Nvenc, VideoEncoderBackend::Vaapi]);
        assert_eq!(choice.codec, VideoCodec::H265);
        assert_eq!(choice.encoder, VideoEncoderBackend::Nvenc);
    }
}

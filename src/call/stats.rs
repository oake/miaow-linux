use livekit::webrtc::stats::{QualityLimitationReason, RtcStats};
use std::collections::HashMap;

pub(super) fn format_debug_stats(
    publisher: &[RtcStats],
    network: &[RtcStats],
    subscriber: &[RtcStats],
    history: &mut HashMap<String, (i64, u64)>,
) -> String {
    let mut sections = Vec::new();
    append_video_stats("WATCHING", subscriber, history, false, &mut sections);
    append_video_stats("SENDING", publisher, history, true, &mut sections);

    let path = network
        .iter()
        .chain(subscriber)
        .filter_map(|stat| match stat {
            RtcStats::CandidatePair(pair) if pair.candidate_pair.nominated => Some(pair),
            _ => None,
        })
        .max_by(|left, right| {
            left.candidate_pair
                .available_outgoing_bitrate
                .total_cmp(&right.candidate_pair.available_outgoing_bitrate)
        });
    if let Some(path) = path {
        sections.push(format!(
            "PATH\nRound trip  {}\nUplink      {}",
            format_duration(path.candidate_pair.current_round_trip_time),
            format_rate(path.candidate_pair.available_outgoing_bitrate),
        ));
    }

    if sections.is_empty() {
        "Waiting for media statistics…".to_owned()
    } else {
        sections.join("\n\n")
    }
}

fn append_video_stats(
    heading: &str,
    report: &[RtcStats],
    history: &mut HashMap<String, (i64, u64)>,
    outbound: bool,
    sections: &mut Vec<String>,
) {
    let codecs: HashMap<&str, &str> = report
        .iter()
        .filter_map(|stat| match stat {
            RtcStats::Codec(codec) => Some((codec.rtc.id.as_str(), codec.codec.mime_type.as_str())),
            _ => None,
        })
        .collect();
    let sources: HashMap<&str, &str> = report
        .iter()
        .filter_map(|stat| match stat {
            RtcStats::MediaSource(source) => Some((
                source.rtc.id.as_str(),
                source.source.track_identifier.as_str(),
            )),
            _ => None,
        })
        .collect();

    if outbound {
        let mut streams: HashMap<&str, Vec<&livekit::webrtc::stats::OutboundRtpStats>> =
            HashMap::new();
        for stat in report {
            let RtcStats::OutboundRtp(video) = stat else {
                continue;
            };
            if video.stream.kind != "video" {
                continue;
            }
            streams
                .entry(video.outbound.media_source_id.as_str())
                .or_default()
                .push(video);
        }
        for (source_id, layers) in streams {
            let active: Vec<_> = layers
                .iter()
                .copied()
                .filter(|video| {
                    video.outbound.active
                        && video.outbound.frame_width > 0
                        && video.outbound.frame_height > 0
                        && video.outbound.frames_encoded > 0
                })
                .collect();
            if active.is_empty() {
                continue;
            }
            let Some(video) = active.iter().copied().max_by_key(|video| {
                u64::from(video.outbound.frame_width) * u64::from(video.outbound.frame_height)
            }) else {
                continue;
            };
            let source = sources.get(source_id).copied().unwrap_or("VIDEO");
            let rate = bitrate(
                history,
                &video.rtc.id,
                video.rtc.timestamp,
                video.sent.bytes_sent,
            );
            let codec = format_codec(
                codecs.get(video.stream.codec_id.as_str()).copied(),
                &video.outbound.encoder_implementation,
                video.outbound.power_efficient_encoder,
            );
            let held = match video.outbound.quality_limitation_reason {
                QualityLimitationReason::None => "Nothing",
                QualityLimitationReason::Cpu => "CPU",
                QualityLimitationReason::Bandwidth => "Bandwidth",
                QualityLimitationReason::Other => "Encoder",
            };
            let mut layer_names = active
                .iter()
                .map(|video| match video.outbound.rid.as_str() {
                    "q" => "360p",
                    "h" => "540p",
                    "f" => "1080p",
                    "" => "one stream",
                    rid => rid,
                })
                .collect::<Vec<_>>();
            layer_names.sort_by_key(|layer| match *layer {
                "360p" => 0,
                "540p" => 1,
                "1080p" => 2,
                _ => 3,
            });
            layer_names.dedup();
            let layer_names = layer_names.join(" + ");
            sections.push(format!(
                "{} {}\nCodec       {}\nPicture     {}×{} at {:.0} fps\nRate        {}\nTarget      {}\nLayers      {}\nHeld back   {}",
                heading,
                source_label(source),
                codec,
                video.outbound.frame_width,
                video.outbound.frame_height,
                video.outbound.frames_per_second,
                format_rate(rate),
                format_rate(video.outbound.target_bitrate),
                layer_names,
                held,
            ));
        }
    } else {
        let round_trip = report.iter().find_map(|stat| match stat {
            RtcStats::RemoteOutboundRtp(remote) if remote.stream.kind == "video" => {
                Some(remote.remote_outbound.round_trip_time)
            }
            _ => None,
        });
        for stat in report {
            let RtcStats::InboundRtp(video) = stat else {
                continue;
            };
            if video.stream.kind != "video" {
                continue;
            }
            let rate = bitrate(
                history,
                &video.rtc.id,
                video.rtc.timestamp,
                video.inbound.bytes_received,
            );
            let codec = format_codec(
                codecs.get(video.stream.codec_id.as_str()).copied(),
                &video.inbound.decoder_implementation,
                video.inbound.power_efficient_decoder,
            );
            let packets = video.received.packets_received as i64 + video.received.packets_lost;
            let loss = if packets > 0 {
                100.0 * video.received.packets_lost.max(0) as f64 / packets as f64
            } else {
                0.0
            };
            sections.push(format!(
                "{} {}\nCodec       {}\nPicture     {}×{}  {:.0} fps\nRate        {}\nDelay       {} · {} jitter\nLoss        {:.1}%",
                heading,
                source_label(&video.inbound.track_identifier),
                codec,
                video.inbound.frame_width,
                video.inbound.frame_height,
                video.inbound.frames_per_second,
                format_rate(rate),
                round_trip.map_or_else(|| "—".to_owned(), format_duration),
                format_duration(video.received.jitter),
                loss,
            ));
        }
    }
}

fn bitrate(history: &mut HashMap<String, (i64, u64)>, id: &str, time: i64, bytes: u64) -> f64 {
    let previous = history.insert(id.to_owned(), (time, bytes));
    let Some((old_time, old_bytes)) = previous else {
        return 0.0;
    };
    let delta = time.saturating_sub(old_time) as f64;
    if delta <= 0.0 || bytes < old_bytes {
        return 0.0;
    }
    // Native WebRTC stats use microseconds since the Unix epoch. The
    // sampling interval does not change that unit.
    let seconds = delta / 1_000_000.0;
    (bytes - old_bytes) as f64 * 8.0 / seconds
}

fn format_codec(codec: Option<&str>, implementation: &str, power_efficient: bool) -> String {
    let codec = codec
        .and_then(|codec| codec.rsplit('/').next())
        .map(|codec| match codec.to_ascii_uppercase().as_str() {
            "H264" => "H.264".to_owned(),
            "H265" | "HEVC" => "H.265".to_owned(),
            other => other.to_owned(),
        })
        .unwrap_or_else(|| "—".to_owned());
    let normalized = implementation.to_ascii_lowercase();
    let acceleration = if normalized.contains("vaapi")
        || normalized.contains("nvenc")
        || normalized.contains("nvdec")
        || normalized.contains("nvidia")
        || normalized.contains("videotoolbox")
        || normalized.contains("jetson")
        || power_efficient
    {
        "HARDWARE"
    } else if normalized.contains("ffmpeg")
        || normalized.contains("openh264")
        || normalized.contains("libvpx")
        || normalized.contains("libaom")
        || normalized.contains("dav1d")
        || normalized.contains("software")
    {
        "SOFTWARE"
    } else {
        "UNKNOWN"
    };
    if implementation.is_empty() {
        format!("{codec} · {acceleration}")
    } else {
        format!("{codec} · {acceleration} ({implementation})")
    }
}

fn source_label(source: &str) -> &'static str {
    if source.to_ascii_lowercase().contains("screen") {
        "SCREEN"
    } else {
        "CAMERA"
    }
}

fn format_duration(seconds: f64) -> String {
    if seconds > 0.0 {
        format!("{:.0} ms", seconds * 1000.0)
    } else {
        "—".to_owned()
    }
}

fn format_rate(bits_per_second: f64) -> String {
    if bits_per_second >= 1_000_000.0 {
        format!("{:.2} Mbps", bits_per_second / 1_000_000.0)
    } else if bits_per_second > 0.0 {
        format!("{:.0} kbps", bits_per_second / 1000.0)
    } else {
        "—".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrate_uses_microseconds_for_both_regular_and_short_intervals() {
        let mut history = HashMap::new();
        assert_eq!(bitrate(&mut history, "camera", 1_000_000, 0), 0.0);
        assert_eq!(
            bitrate(&mut history, "camera", 1_500_000, 125_000),
            2_000_000.0
        );
        assert_eq!(
            bitrate(&mut history, "camera", 1_501_000, 125_250),
            2_000_000.0
        );
    }

    #[test]
    fn bitrate_handles_counter_resets_and_repeated_timestamps() {
        let mut history = HashMap::new();
        bitrate(&mut history, "camera", 1_000_000, 100_000);
        assert_eq!(bitrate(&mut history, "camera", 1_000_000, 125_000), 0.0);
        assert_eq!(bitrate(&mut history, "camera", 1_500_000, 0), 0.0);
        assert_eq!(
            bitrate(&mut history, "camera", 2_000_000, 125_000),
            2_000_000.0
        );
        assert_eq!(bitrate(&mut history, "screen", 2_000_000, 125_000), 0.0);
    }
}

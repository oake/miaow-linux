use super::MediaDevice;
use livekit::prelude::{AudioProcessingOptions, PlatformAudio};

pub(super) struct ConfiguredAudio {
    pub audio: PlatformAudio,
    pub microphones: Vec<MediaDevice>,
    pub speakers: Vec<MediaDevice>,
    pub selected_microphone: String,
    pub selected_speaker: String,
}

pub(super) fn configure_platform_audio(
    requested_microphone: &str,
    requested_speaker: &str,
    noise_suppression: bool,
    echo_cancellation: bool,
) -> Result<ConfiguredAudio, String> {
    let audio = PlatformAudio::new().map_err(|error| error.to_string())?;

    let recording_devices = audio.recording_devices().collect::<Vec<_>>();
    let playout_devices = audio.playout_devices().collect::<Vec<_>>();
    log::info!(
        "platform audio found {} microphone(s) and {} speaker(s)",
        recording_devices.len(),
        playout_devices.len()
    );
    for device in &recording_devices {
        log::info!(
            "microphone [{}]: {} ({})",
            device.index,
            device.name,
            device.id
        );
    }
    for device in &playout_devices {
        log::info!(
            "speaker [{}]: {} ({})",
            device.index,
            device.name,
            device.id
        );
    }

    let microphone = recording_devices
        .iter()
        .find(|device| audio_device_key(device.index, device.id.as_str()) == requested_microphone)
        .or_else(|| recording_devices.first())
        .ok_or_else(|| "No microphone was found".to_owned())?;
    audio
        .set_recording_device(&microphone.id)
        .map_err(|error| format!("Could not select {}: {error}", microphone.name))?;

    let speaker = playout_devices
        .iter()
        .find(|device| audio_device_key(device.index, device.id.as_str()) == requested_speaker)
        .or_else(|| playout_devices.first())
        .ok_or_else(|| "No speaker was found".to_owned())?;
    audio
        .set_playout_device(&speaker.id)
        .map_err(|error| format!("Could not select {}: {error}", speaker.name))?;

    audio
        .configure_audio_processing(AudioProcessingOptions {
            noise_suppression,
            echo_cancellation,
            ..Default::default()
        })
        .map_err(|error| format!("Could not configure audio processing: {error}"))?;
    audio
        .start_recording()
        .map_err(|error| format!("Could not start microphone capture: {error}"))?;
    let selected_microphone = audio_device_key(microphone.index, microphone.id.as_str());
    let selected_speaker = audio_device_key(speaker.index, speaker.id.as_str());
    let microphones = recording_devices
        .into_iter()
        .map(|device| MediaDevice {
            id: audio_device_key(device.index, device.id.as_str()),
            name: device.name,
        })
        .collect();
    let speakers = playout_devices
        .into_iter()
        .map(|device| MediaDevice {
            id: audio_device_key(device.index, device.id.as_str()),
            name: device.name,
        })
        .collect();
    Ok(ConfiguredAudio {
        audio,
        microphones,
        speakers,
        selected_microphone,
        selected_speaker,
    })
}

fn audio_device_key(index: usize, guid: &str) -> String {
    format!("{index}|{guid}")
}

pub(super) fn audio_device_guid(key: &str) -> &str {
    key.split_once('|').map_or(key, |(_, guid)| guid)
}

/// Read movie duration without decoding audio. Container traversal is iterative
/// because imported and received files can contain arbitrarily nested boxes.
pub(super) fn duration_seconds(data: &[u8]) -> Option<f64> {
    let mut remaining = vec![data];
    while let Some(container) = remaining.pop() {
        for (kind, payload) in boxes(container) {
            if kind == b"moov" {
                remaining.push(payload);
            } else if kind == b"mvhd"
                && let Some(duration) = mvhd_duration(payload)
            {
                return Some(duration);
            }
        }
    }
    None
}

fn mvhd_duration(payload: &[u8]) -> Option<f64> {
    let version = *payload.first()?;
    let (timescale, duration) = if version == 1 {
        if payload.len() < 32 {
            return None;
        }
        (
            u32::from_be_bytes(payload[20..24].try_into().ok()?) as f64,
            u64::from_be_bytes(payload[24..32].try_into().ok()?) as f64,
        )
    } else {
        if payload.len() < 20 {
            return None;
        }
        (
            u32::from_be_bytes(payload[12..16].try_into().ok()?) as f64,
            u32::from_be_bytes(payload[16..20].try_into().ok()?) as f64,
        )
    };
    if timescale <= 0.0 {
        return None;
    }
    Some(duration / timescale)
}

/// Apple AAC files can store encoder priming in iTunSMPB rather than an
/// edit list. Symphonia 0.5 (used by Rodio) does not apply this metadata.
pub(crate) fn aac_priming_samples(data: &[u8]) -> Option<u32> {
    let moov = find_box(data, b"moov")?;
    let udta = find_box(moov, b"udta")?;
    let meta = find_box(udta, b"meta")?;
    let ilst = find_box(meta.get(4..)?, b"ilst")?;
    for (kind, item) in boxes(ilst) {
        if kind != b"----" {
            continue;
        }
        if find_box(item, b"mean")?.get(4..)? != b"com.apple.iTunes"
            || find_box(item, b"name")?.get(4..)? != b"iTunSMPB"
        {
            continue;
        }
        // data has a four-byte type indicator and four-byte locale.
        let value = std::str::from_utf8(find_box(item, b"data")?.get(8..)?).ok()?;
        let delay = value.split_ascii_whitespace().nth(1)?;
        return u32::from_str_radix(delay, 16).ok();
    }
    None
}

fn find_box<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data)
        .find(|(name, _)| *name == kind)
        .map(|(_, payload)| payload)
}

fn boxes(mut data: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    std::iter::from_fn(move || {
        let header = data.get(..8)?;
        let size = u32::from_be_bytes(header[..4].try_into().ok()?);
        let (header_size, total) = match size {
            0 => (8, data.len()),
            1 => (
                16,
                usize::try_from(u64::from_be_bytes(data.get(8..16)?.try_into().ok()?)).ok()?,
            ),
            size => (8, size as usize),
        };
        let payload = data.get(header_size..total)?;
        data = data.get(total..)?;
        Some((&header[4..8], payload))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie_header() -> Vec<u8> {
        let mut payload = vec![0; 20];
        payload[12..16].copy_from_slice(&1000u32.to_be_bytes());
        payload[16..20].copy_from_slice(&2500u32.to_be_bytes());
        box_bytes(b"mvhd", &payload)
    }

    fn box_bytes(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn reads_builtin_aac_priming() {
        assert_eq!(aac_priming_samples(super::super::BUILTIN_BYTES), Some(2112));
    }

    #[test]
    fn truncated_metadata_does_not_panic_or_invent_padding() {
        let bytes = super::super::BUILTIN_BYTES;
        for length in 0..bytes.len() {
            assert_eq!(aac_priming_samples(&bytes[..length]), None);
        }
        assert_eq!(aac_priming_samples(&box_bytes(b"moov", &[])), None);
    }

    #[test]
    fn reads_normal_extended_and_open_ended_boxes() {
        let header = movie_header();
        assert_eq!(duration_seconds(&box_bytes(b"moov", &header)), Some(2.5));
        let mut extended = 1u32.to_be_bytes().to_vec();
        extended.extend_from_slice(b"moov");
        extended.extend_from_slice(&((16 + header.len()) as u64).to_be_bytes());
        extended.extend_from_slice(&header);
        assert_eq!(duration_seconds(&extended), Some(2.5));
        let mut open = box_bytes(b"moov", &header);
        open[..4].fill(0);
        assert_eq!(duration_seconds(&open), Some(2.5));
    }

    #[test]
    fn rejects_truncated_and_overflowing_sizes_without_panicking() {
        let valid = box_bytes(b"moov", &movie_header());
        for length in 0..valid.len() {
            assert_eq!(duration_seconds(&valid[..length]), None);
        }
        let mut overflow = box_bytes(b"free", &[]);
        overflow.extend_from_slice(&1u32.to_be_bytes());
        overflow.extend_from_slice(b"moov");
        overflow.extend_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(duration_seconds(&overflow), None);
    }

    #[test]
    fn nested_containers_do_not_use_the_call_stack() {
        let depth = 10_000;
        let header = movie_header();
        let mut data = Vec::new();
        for level in (1..=depth).rev() {
            data.extend_from_slice(&((level * 8 + header.len()) as u32).to_be_bytes());
            data.extend_from_slice(b"moov");
        }
        data.extend_from_slice(&header);
        assert_eq!(duration_seconds(&data), Some(2.5));
    }
}

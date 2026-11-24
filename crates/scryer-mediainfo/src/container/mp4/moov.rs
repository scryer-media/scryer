//! Track metadata read from the moov box tree: sample entries, channel
//! layouts, languages, kinds and names.

use super::*;

pub(super) fn mp4_audio_channels(
    codec_id: &str,
    codec_private: Option<&[u8]>,
    sample_entry_channels: Option<u32>,
) -> Option<i32> {
    let codec_specific_channels = codec_private.and_then(|codec_private| match codec_id {
        "mp4a" => parse_aac_audio_specific_config_channels(codec_private),
        "ac-3" => parse_dac3_channels(codec_private),
        "ec-3" => parse_dec3_channels(codec_private),
        _ => None,
    });
    let sample_entry_channels = sample_entry_channels
        .filter(|channels| *channels > 0)
        .map(|channels| channels as i32);

    match (codec_specific_channels, sample_entry_channels) {
        (Some(codec_specific_channels), Some(sample_entry_channels)) => {
            Some(codec_specific_channels.max(sample_entry_channels))
        }
        (Some(codec_specific_channels), None) => Some(codec_specific_channels),
        (None, Some(sample_entry_channels)) => Some(sample_entry_channels),
        (None, None) => None,
    }
}

pub(super) fn parse_aac_audio_specific_config_channels(data: &[u8]) -> Option<i32> {
    const AAC_CHANNEL_CONFIGS: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 8, 0, 0, 0, 7, 8, 0, 8, 0];

    let mut bits = BitCursor::new(data);
    let audio_object_type = bits.read_aac_audio_object_type()?;
    bits.read_aac_sample_rate()?;
    let mut channel_config = bits.read_bits(4)? as usize;

    if matches!(audio_object_type, 5 | 29) {
        bits.read_aac_sample_rate()?;
        let ext_audio_object_type = bits.read_aac_audio_object_type()?;
        if ext_audio_object_type == 22 {
            channel_config = bits.read_bits(4)? as usize;
        }
    }

    let channels = *AAC_CHANNEL_CONFIGS.get(channel_config)?;
    (channels > 0).then_some(i32::from(channels))
}

pub(super) fn parse_dac3_channels(data: &[u8]) -> Option<i32> {
    let mut bits = BitCursor::new(data);
    bits.read_bits(2)?;
    bits.read_bits(5)?;
    bits.read_bits(3)?;
    let acmod = bits.read_bits(3)? as usize;
    let lfeon = bits.read_bits(1)? as i32;
    Some(ac3_channel_count(acmod) + lfeon)
}

pub(super) fn parse_dec3_channels(data: &[u8]) -> Option<i32> {
    let mut bits = BitCursor::new(data);
    bits.read_bits(13)?;
    let num_ind_sub = bits.read_bits(3)? as usize + 1;
    if num_ind_sub == 0 {
        return None;
    }

    bits.read_bits(2)?;
    bits.read_bits(5)?;
    bits.read_bits(1)?;
    bits.read_bits(1)?;
    bits.read_bits(3)?;
    let acmod = bits.read_bits(3)? as usize;
    let lfeon = bits.read_bits(1)? as i32;
    Some(ac3_channel_count(acmod) + lfeon)
}

pub(super) fn ac3_channel_count(acmod: usize) -> i32 {
    const AC3_CHANNELS_BY_ACMOD: [i32; 8] = [2, 1, 2, 3, 3, 4, 4, 5];
    AC3_CHANNELS_BY_ACMOD.get(acmod).copied().unwrap_or(0)
}

pub(super) struct BitCursor<'a> {
    pub(super) data: &'a [u8],
    pub(super) bit_offset: usize,
}

impl<'a> BitCursor<'a> {
    pub(super) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_offset: 0,
        }
    }

    pub(super) fn read_bits(&mut self, count: usize) -> Option<u32> {
        if count == 0 || count > 32 {
            return None;
        }

        let mut value = 0_u32;
        for _ in 0..count {
            let byte = *self.data.get(self.bit_offset / 8)?;
            let shift = 7 - (self.bit_offset % 8);
            value = (value << 1) | u32::from((byte >> shift) & 1);
            self.bit_offset += 1;
        }
        Some(value)
    }

    pub(super) fn read_aac_audio_object_type(&mut self) -> Option<u8> {
        let object_type = self.read_bits(5)? as u8;
        if object_type == 31 {
            Some(32 + self.read_bits(6)? as u8)
        } else {
            Some(object_type)
        }
    }

    pub(super) fn read_aac_sample_rate(&mut self) -> Option<u32> {
        const AAC_SAMPLE_RATES: [u32; 13] = [
            96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025,
            8_000, 7_350,
        ];

        let sample_rate_index = self.read_bits(4)? as usize;
        if sample_rate_index == 0xF {
            self.read_bits(24)
        } else {
            AAC_SAMPLE_RATES.get(sample_rate_index).copied()
        }
    }
}

pub(super) fn parse_moov(data: &[u8], metadata: &mut HashMap<u32, Mp4TrackMetadata>) {
    if !mp4_box_name_present(data, &[*b"trak"]) {
        return;
    }
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"trak" {
            let track = parse_trak(payload);
            if let Some(track_id) = track.track_id {
                metadata.insert(track_id, track);
            }
        }
    });
}

pub(super) fn parse_trak(data: &[u8]) -> Mp4TrackMetadata {
    let mut track = Mp4TrackMetadata::default();
    if mp4_box_name_present(data, &[*b"tkhd", *b"mdia", *b"udta"]) {
        for_each_mp4_box(data, |header, payload| match &header.name {
            b"tkhd" => {
                apply_tkhd_metadata(payload, &mut track);
            }
            b"mdia" => parse_mdia(payload, &mut track),
            b"udta" => parse_udta(payload, &mut track),
            _ => {}
        });
    }
    parse_kind_boxes(data, &mut track);
    track.details.id = track.track_id.map(|id| id.to_string());
    track.details.original_language = track.language.clone();
    if track.language.is_some() {
        track.details.language_provenance = scryer_media_model::Provenance::Container;
    }
    track.details.disposition.default = Some(track.default_track);
    track.details.disposition.forced = Some(track.forced);
    track
}

pub(super) fn parse_kind_boxes(data: &[u8], track: &mut Mp4TrackMetadata) {
    if !mp4_box_name_present(data, &[*b"kind"]) {
        return;
    }
    walk_mp4_boxes(
        data,
        MP4_BOX_MAX_DEPTH,
        |header, payload, _depth| match &header.name {
            b"mdia" | b"minf" | b"stbl" | b"udta" => Some(payload),
            b"meta" => payload.get(4..),
            _ => None,
        },
        |header, payload, _depth| {
            if &header.name == b"kind" {
                apply_kind_metadata(payload, track);
            }
        },
    );
}

pub(super) fn parse_mdia(data: &[u8], track: &mut Mp4TrackMetadata) {
    if !mp4_box_name_present(data, &[*b"mdhd", *b"hdlr", *b"minf", *b"elng"]) {
        return;
    }
    let mut extended_language = None;
    for_each_mp4_box(data, |header, payload| match &header.name {
        b"mdhd" => {
            track.language = parse_mdhd_language(payload);
        }
        b"hdlr" => {
            track.handler_type = parse_hdlr_type(payload);
        }
        b"minf" => parse_minf(payload, track),
        b"elng" => {
            extended_language = payload
                .get(..4)
                .filter(|flags| *flags == [0, 0, 0, 0])
                .and_then(|_| payload.get(4..))
                .and_then(|bytes| bytes.strip_suffix(&[0]))
                .filter(|bytes| {
                    !bytes.is_empty()
                        && bytes.len() <= 255
                        && bytes
                            .iter()
                            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
                })
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .map(str::to_owned);
        }
        _ => {}
    });
    if extended_language.is_some() {
        track.language = extended_language;
    }
}

pub(super) fn parse_udta(data: &[u8], track: &mut Mp4TrackMetadata) {
    if !mp4_box_name_present(data, &[*b"name"]) {
        return;
    }
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"name" {
            track.name = parse_name_box(payload);
        }
    });
}

pub(super) fn parse_minf(data: &[u8], track: &mut Mp4TrackMetadata) {
    if !mp4_box_name_present(data, &[*b"stbl"]) {
        return;
    }
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"stbl" {
            parse_stbl(payload, track);
        }
    });
}

pub(super) fn parse_stbl(data: &[u8], track: &mut Mp4TrackMetadata) {
    if !mp4_box_name_present(data, &[*b"stsd"]) {
        return;
    }
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"stsd" {
            parse_stsd(payload, track);
        }
    });
}

pub(super) fn parse_stsd(data: &[u8], track: &mut Mp4TrackMetadata) {
    if data.len() < 8 {
        return;
    }
    let offset = 8; // version/flags + entry_count
    if let Some(header) = read_box_header_from_bytes(&data[offset..]) {
        let size = header.size as usize;
        if size < header.header_size || offset + size > data.len() {
            return;
        }
        let sample_entry = &data[offset..offset + size];
        let payload = &sample_entry[header.header_size..];
        let fourcc = fourcc_to_string(header.name);
        track.sample_entry_fourcc = Some(fourcc.clone());

        if is_video_sample_entry(&fourcc) {
            let child_offset = 78;
            if payload.len() >= child_offset {
                let children = &payload[child_offset..];
                if mp4_box_name_present(
                    children,
                    &[
                        *b"avcC", *b"hvcC", *b"av1C", *b"dvcC", *b"dvvC", *b"dvwC", *b"colr",
                        *b"pasp", *b"vpcC", *b"st3d",
                    ],
                ) {
                    for_each_mp4_box(children, |child, child_payload| {
                        let child_name = fourcc_to_string(child.name);
                        match child_name.as_str() {
                            "avcC" | "hvcC" | "av1C" | "vpcC" => {
                                track.codec_private = Some(child_payload.to_vec());
                            }
                            "st3d" if child_payload.len() >= 5 && child_payload[..4] == [0; 4] => {
                                // FullBox version 0: mono=0, stereo modes=1..4.
                                track.details.is_3d |= matches!(child_payload[4], 1..=4);
                            }
                            "colr" => {
                                if child_payload.len() >= 10
                                    && matches!(&child_payload[..4], b"nclx" | b"nclc")
                                {
                                    track.details.color = scryer_media_model::ColorMetadata {
                                        primaries: read_be_u16(&child_payload[4..6])
                                            .map(u32::from)
                                            .filter(|value| *value != 2),
                                        transfer: read_be_u16(&child_payload[6..8])
                                            .map(u32::from)
                                            .filter(|value| *value != 2),
                                        matrix: read_be_u16(&child_payload[8..10])
                                            .map(u32::from)
                                            .filter(|value| *value != 2),
                                        full_range: (child_payload[..4] == *b"nclx")
                                            .then(|| {
                                                child_payload.get(10).map(|byte| byte & 128 != 0)
                                            })
                                            .flatten(),
                                        provenance: scryer_media_model::Provenance::Container,
                                        ..Default::default()
                                    };
                                }
                            }
                            "pasp" if child_payload.len() >= 8 => {
                                track.details.sample_aspect_ratio =
                                    read_be_u32(&child_payload[..4])
                                        .zip(read_be_u32(&child_payload[4..8]))
                                        .and_then(|(width, height)| {
                                            scryer_media_model::Rational::new(
                                                i64::from(width),
                                                u64::from(height),
                                            )
                                        });
                            }
                            t if MP4_DOVI_TYPES.contains(&t)
                                && crate::codec::dolby_vision::parse_dovi_config(child_payload)
                                    .is_some() =>
                            {
                                track.dovi_config = Some(child_payload.to_vec());
                            }
                            _ => {}
                        }
                    });
                }
            }
        } else if is_audio_sample_entry(&fourcc)
            && let Some(child_offset) = audio_sample_entry_child_offset(payload)
        {
            let children = &payload[child_offset..];
            if mp4_box_name_present(children, &[*b"dac3", *b"dec3", *b"dOps", *b"dfLa"]) {
                for_each_mp4_box(children, |child, child_payload| {
                    let child_name = fourcc_to_string(child.name);
                    if matches!(child_name.as_str(), "dac3" | "dec3" | "dOps") {
                        track.codec_private = Some(child_payload.to_vec());
                    } else if child_name == "dfLa" && child_payload.starts_with(&[0, 0, 0, 0]) {
                        let mut metadata = b"fLaC".to_vec();
                        metadata.extend_from_slice(&child_payload[4..]);
                        track.codec_private = Some(metadata);
                    }
                });
            }
        }
    }
}

pub(super) fn audio_sample_entry_child_offset(payload: &[u8]) -> Option<usize> {
    if payload.len() < 28 {
        return None;
    }
    let version = read_be_u16(&payload[8..10])?;
    let offset = match version {
        0 => 28,
        1 => 44,
        2 => 64,
        _ => return None,
    };
    payload.get(offset..).map(|_| offset)
}

pub(super) fn apply_tkhd_metadata(data: &[u8], track: &mut Mp4TrackMetadata) {
    track.track_id = parse_tkhd_track_id(data);
    track.default_track =
        parse_full_box_flags(data).is_some_and(|flags| (flags & MOV_TKHD_FLAG_ENABLED) != 0);
}

pub(super) fn parse_tkhd_track_id(data: &[u8]) -> Option<u32> {
    if data.len() < 24 {
        return None;
    }
    match data[0] {
        1 if data.len() >= 32 => Some(read_be_u32(&data[20..24])?),
        _ => Some(read_be_u32(&data[12..16])?),
    }
}

pub(super) fn parse_full_box_flags(data: &[u8]) -> Option<u32> {
    if data.len() < 4 {
        return None;
    }
    Some(u32::from_be_bytes([0, data[1], data[2], data[3]]))
}

pub(super) fn parse_mdhd_language(data: &[u8]) -> Option<String> {
    if data.len() < 24 {
        return None;
    }
    let language_offset = match data[0] {
        1 if data.len() >= 34 => 32,
        _ => 20,
    };
    let code = read_be_u16(&data[language_offset..language_offset + 2])?;
    decode_mdhd_language(code)
}

pub(super) fn parse_hdlr_type(data: &[u8]) -> Option<[u8; 4]> {
    data.get(8..12)?.try_into().ok()
}

pub(super) fn parse_name_box(data: &[u8]) -> Option<String> {
    let raw = if data.len() > 4 { &data[4..] } else { data };
    let name = std::str::from_utf8(raw).ok()?.trim_end_matches('\0').trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

pub(super) fn apply_kind_metadata(data: &[u8], track: &mut Mp4TrackMetadata) {
    if data.len() < 6 {
        return;
    }
    if !matches!(data.get(0..4), Some([0, 0, 0, 0])) {
        return;
    }
    let Some(payload) = data.get(4..) else {
        return;
    };
    let Some((scheme, value)) = parse_kind_strings(payload) else {
        return;
    };
    if scheme == "urn:mpeg:dash:role:2011" && value.starts_with("forced-subtitle") {
        track.forced = true;
    }
}

pub(super) fn parse_kind_strings(data: &[u8]) -> Option<(String, String)> {
    let scheme_end = data.iter().position(|&byte| byte == 0)?;
    let scheme = std::str::from_utf8(&data[..scheme_end]).ok()?.to_owned();
    let rest = data.get(scheme_end + 1..)?;
    let value_end = rest
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(rest.len());
    let value = std::str::from_utf8(&rest[..value_end]).ok()?.to_owned();
    Some((scheme, value))
}

pub(super) fn decode_mdhd_language(code: u16) -> Option<String> {
    if code < 0x400 {
        return match code {
            0 => Some("eng".to_string()),
            6 => Some("spa".to_string()),
            11 => Some("jpn".to_string()),
            _ => None,
        };
    }

    let chars = [
        (((code >> 10) & 0x1F) as u8).saturating_add(0x60),
        (((code >> 5) & 0x1F) as u8).saturating_add(0x60),
        ((code & 0x1F) as u8).saturating_add(0x60),
    ];
    if chars.iter().all(|c| c.is_ascii_lowercase()) {
        Some(String::from_utf8_lossy(&chars).into_owned())
    } else {
        None
    }
}

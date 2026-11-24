//! Fills track facts from a sample of an elementary stream: video sequence
//! headers and audio frame headers. Used by containers that carry raw
//! elementary streams (transport, program and AVI streams).

use super::aac::{find_adts_header, find_latm_header};
use super::ac3::{find_ac3_header, find_eac3_header};
use super::audio_profile::{detect_audio_profile_from_payload, merge_audio_profile};
use super::bits::BitReader;
use super::dts::{detect_dts_channels_from_probe_bytes, find_dts_header};
use super::find_start_code_payload;
use super::mpeg_audio::find_mpeg_audio_header;
use super::mpeg_video::find_mpeg_video_sequence_header;
use crate::raw::RawTrack;
use crate::scan;

pub(crate) fn probe_h264_track(data: &[u8], track: &mut RawTrack) {
    let Some(sps_nal) = find_annexb_nal(data, |nal| (nal[0] & 0x1F) == 7) else {
        return;
    };
    let rbsp = crate::scan::h2645_unescape_rbsp(sps_nal);
    let Ok(sps) = scuffle_h264::Sps::parse(std::io::Cursor::new(rbsp.as_ref())) else {
        return;
    };

    track.width = Some(sps.width() as i32);
    track.height = Some(sps.height() as i32);
    track.frame_rate_fps = sps.frame_rate();
    track.codec_private = Some(sps_nal.to_vec());
    track.color_transfer = sps.color_config.as_ref().and_then(|color| {
        let transfer = color.transfer_characteristics as u32;
        if transfer > 0 && transfer != 2 {
            Some(transfer)
        } else {
            None
        }
    });
}

pub(crate) fn probe_hevc_track(data: &[u8], track: &mut RawTrack) {
    let Some(sps_nal) = find_annexb_nal(data, |nal| ((nal[0] >> 1) & 0x3F) == 33) else {
        return;
    };
    let Ok(sps) = scuffle_h265::SpsNALUnit::parse(std::io::Cursor::new(sps_nal)) else {
        return;
    };

    track.width = Some(sps.rbsp.cropped_width() as i32);
    track.height = Some(sps.rbsp.cropped_height() as i32);
    track.codec_private = Some(sps_nal.to_vec());
    track.color_transfer = sps.rbsp.vui_parameters.as_ref().and_then(|vui| {
        let transfer = vui.video_signal_type.transfer_characteristics;
        if transfer > 0 && transfer != 2 {
            Some(transfer as u32)
        } else {
            None
        }
    });
}

pub(crate) fn probe_vc1_track(data: &[u8], track: &mut RawTrack) {
    let Some(sequence) = find_start_code_payload(data, 0x0F) else {
        return;
    };
    let Some((width, height)) = parse_vc1_advanced_sequence_dimensions(sequence) else {
        return;
    };

    track.width = Some(width as i32);
    track.height = Some(height as i32);
    track.codec_private = Some(sequence[..sequence.len().min(64)].to_vec());
}

fn parse_vc1_advanced_sequence_dimensions(data: &[u8]) -> Option<(u16, u16)> {
    let mut bits = BitReader::new(data);
    let profile = bits.read_bits(2)?;
    if profile != 3 {
        return None;
    }

    bits.skip_bits(3)?; // level
    bits.skip_bits(2)?; // chroma format
    bits.skip_bits(3)?; // frame-rate post-processing quality
    bits.skip_bits(5)?; // bit-rate post-processing quality
    bits.skip_bits(1)?; // post-processing flag
    let width = ((bits.read_bits(12)? + 1) * 2) as u16;
    let height = ((bits.read_bits(12)? + 1) * 2) as u16;

    (width > 0 && height > 0).then_some((width, height))
}

pub(crate) fn probe_aac_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_adts_header(data) else {
        return;
    };
    track.channels = Some(header.channels as i32);
    track.metadata.sample_rate = Some(header.sample_rate);
    track.metadata.channel_layout = Some(header.channel_layout.into());
    if let Some(bit_rate_bps) = header.bit_rate_bps {
        track.metadata.estimated_bitrate_bps = Some(u64::from(bit_rate_bps));
    }
    merge_audio_profile(
        &mut track.audio_profile,
        detect_audio_profile_from_payload(track.codec_name.as_deref(), data),
    );
}

pub(crate) fn probe_latm_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_latm_header(data) else {
        return;
    };
    track.channels = Some(header.channels as i32);
    merge_audio_profile(
        &mut track.audio_profile,
        detect_audio_profile_from_payload(track.codec_name.as_deref(), data),
    );
}

pub(crate) fn probe_elementary_stream(data: &[u8], track: &mut RawTrack) {
    crate::codec::picture_headers::enrich(track, data);
    match track.codec_name.as_deref() {
        Some("mpeg1video" | "mpeg2video") => probe_mpeg_video_track(data, track),
        Some("h264") => probe_h264_track(data, track),
        Some("hevc") => probe_hevc_track(data, track),
        Some("vc1") => probe_vc1_track(data, track),
        Some("mp1" | "mp2" | "mp3") => probe_mpeg_audio_track(data, track),
        Some("ac3") => probe_ac3_track(data, track),
        Some("eac3") => probe_eac3_track(data, track),
        Some("dts") => probe_dts_track(data, track),
        Some("truehd") => probe_truehd_track(data, track),
        Some("aac") => probe_aac_track(data, track),
        _ => {}
    }
}

pub(crate) fn probe_mpeg_video_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_mpeg_video_sequence_header(data) else {
        return;
    };
    track.width = Some(header.width as i32);
    track.height = Some(header.height as i32);
    track.frame_rate_fps = header.frame_rate_fps;
    if track.bit_rate_bps.is_none() {
        track.bit_rate_bps = header.bit_rate_bps.map(i64::from);
    }
    crate::codec::picture_headers::enrich(track, data);
}

pub(crate) fn probe_mpeg_audio_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_mpeg_audio_header(data) else {
        return;
    };
    header.apply(track);
}

pub(crate) fn probe_unknown_audio_track(data: &[u8], track: &mut RawTrack) {
    if find_adts_header(data).is_some() {
        track.codec_name = Some("aac".to_owned());
        probe_aac_track(data, track);
        return;
    }
    if find_latm_header(data).is_some() {
        track.codec_name = Some("aac_latm".to_owned());
        probe_latm_track(data, track);
        return;
    }
    if find_mpeg_audio_header(data).is_some() {
        probe_mpeg_audio_track(data, track);
        return;
    }
    if find_ac3_header(data).is_some() {
        track.codec_name = Some("ac3".to_owned());
        probe_ac3_track(data, track);
        return;
    }
    if find_eac3_header(data).is_some() {
        track.codec_name = Some("eac3".to_owned());
        probe_eac3_track(data, track);
    }
}

pub(crate) fn probe_ac3_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_ac3_header(data) else {
        return;
    };
    header.apply(track);
}

pub(crate) fn probe_eac3_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_eac3_header(data) else {
        return;
    };
    header.apply(track);
    merge_audio_profile(
        &mut track.audio_profile,
        detect_audio_profile_from_payload(track.codec_name.as_deref(), data),
    );
}

pub(crate) fn probe_truehd_track(data: &[u8], track: &mut RawTrack) {
    merge_audio_profile(
        &mut track.audio_profile,
        detect_audio_profile_from_payload(track.codec_name.as_deref(), data),
    );
}

pub(crate) fn probe_dts_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_dts_header(data) else {
        return;
    };
    let core_channels = i32::from(header.channels);
    track.channels = Some(track.channels.unwrap_or(0).max(core_channels));
    let extension_channels = detect_dts_channels_from_probe_bytes(data);
    if let Some(channels) = extension_channels {
        track.channels = Some(
            track
                .channels
                .map_or(channels, |existing| existing.max(channels)),
        );
    }
    merge_audio_profile(
        &mut track.audio_profile,
        detect_audio_profile_from_payload(track.codec_name.as_deref(), data),
    );
    if track.audio_profile.as_deref() == Some("DTS-ES") && track.channels == Some(6) {
        track.channels = Some(7);
    }
    // The core describes the complete presentation only when no extension is
    // known. In particular, its bitrate is not the bitrate of a DTS-HD track.
    if extension_channels.is_none()
        && matches!(track.audio_profile.as_deref(), None | Some("DTS"))
        && track.channels == Some(core_channels)
    {
        track.metadata.sample_rate = Some(header.sample_rate);
        track.metadata.channel_layout = header.channel_layout.map(str::to_owned);
        if header.bit_rate_bps > 3 && track.bit_rate_bps.is_none() {
            track.bit_rate_bps = Some(i64::from(header.bit_rate_bps));
            track.metadata.bitrate_provenance = scryer_media_model::Provenance::Bitstream;
        }
    }
}

fn find_annexb_nal(data: &[u8], predicate: impl Fn(&[u8]) -> bool) -> Option<&[u8]> {
    let mut i = 0;
    while let Some(start_code) = scan::find_annexb_start_code(data, i) {
        let nal_start = start_code.end;
        let nal_end = scan::find_annexb_start_code(data, nal_start)
            .map(|next_start_code| next_start_code.start)
            .unwrap_or(data.len());

        if nal_end > nal_start {
            let nal = &data[nal_start..nal_end];
            if !nal.is_empty() && predicate(nal) {
                return Some(nal);
            }
        }
        i = nal_end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dts_core_metadata_preserves_extension_declarations() {
        let data = [
            0x7F, 0xFE, 0x80, 0x01, 0x7C, 0x7C, 0x05, 0xF2, 0x77, 0x00, 0x02,
        ];
        let mut core = RawTrack {
            codec_name: Some("dts".into()),
            ..Default::default()
        };
        probe_dts_track(&data, &mut core);
        assert_eq!(core.channels, Some(6));
        assert_eq!(core.metadata.sample_rate, Some(48_000));
        assert_eq!(core.metadata.channel_layout.as_deref(), Some("5.1(side)"));
        assert_eq!(core.bit_rate_bps, Some(1_536_000));
        assert_eq!(
            core.metadata.bitrate_provenance,
            scryer_media_model::Provenance::Bitstream
        );

        let mut extension = RawTrack {
            codec_name: Some("dts".into()),
            audio_profile: Some("DTS-HD MA".into()),
            channels: Some(8),
            ..Default::default()
        };
        extension.metadata.sample_rate = Some(96_000);
        extension.metadata.channel_layout = Some("7.1".into());
        probe_dts_track(&data, &mut extension);
        assert_eq!(extension.channels, Some(8));
        assert_eq!(extension.metadata.sample_rate, Some(96_000));
        assert_eq!(extension.metadata.channel_layout.as_deref(), Some("7.1"));
        assert_eq!(extension.bit_rate_bps, None);
    }
}

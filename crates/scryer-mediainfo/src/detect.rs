//! Container identification from leading bytes, with the file extension as a
//! fallback when no signature matches.

use crate::container::{asf, dv, elementary};

pub(crate) fn resolve_container_format(
    ext: &str,
    sniffed: Option<ContainerFormat>,
) -> Option<ContainerFormat> {
    sniffed.or_else(|| container_format_from_extension(ext))
}

fn container_format_from_extension(ext: &str) -> Option<ContainerFormat> {
    use scryer_media_model::formats::{
        MediaFormat, MediaProbeCapability, media_format_for_extension,
    };
    let registration = media_format_for_extension(ext)?;
    if registration.probe != MediaProbeCapability::Native {
        return None;
    }
    match registration.format {
        MediaFormat::Matroska | MediaFormat::WebM => Some(ContainerFormat::Matroska),
        MediaFormat::Mp4 | MediaFormat::QuickTime => Some(ContainerFormat::Mp4),
        MediaFormat::Avi => Some(ContainerFormat::Avi),
        MediaFormat::MpegTs => Some(ContainerFormat::Ts),
        MediaFormat::MpegProgram => Some(ContainerFormat::Ps),
        MediaFormat::Asf => Some(ContainerFormat::Asf),
        MediaFormat::Ogg | MediaFormat::Ogm => Some(ContainerFormat::Ogg),
        MediaFormat::Flv => Some(ContainerFormat::Flv),
        MediaFormat::RealMedia => Some(ContainerFormat::RealMedia),
        MediaFormat::Dv => Some(ContainerFormat::Dv),
        MediaFormat::H264 => Some(ContainerFormat::H264),
        MediaFormat::MpegVideo => Some(ContainerFormat::MpegVideo),
        MediaFormat::StreamLink
        | MediaFormat::DiscImage
        | MediaFormat::Playlist
        | MediaFormat::Companion
        | MediaFormat::Unsupported
        | MediaFormat::Svq3 => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContainerFormat {
    Matroska,
    Mp4,
    Avi,
    Ts,
    Asf,
    Ogg,
    Flv,
    Ps,
    RealMedia,
    Dv,
    H264,
    MpegVideo,
    FlacAudio,
    WaveAudio,
}

pub(crate) fn sniff_container_format_from_bytes(data: &[u8]) -> Option<ContainerFormat> {
    if data.starts_with(b"fLaC") {
        return Some(ContainerFormat::FlacAudio);
    }
    if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WAVE") {
        return Some(ContainerFormat::WaveAudio);
    }
    if dv::sniff(data) {
        return Some(ContainerFormat::Dv);
    }
    if data.starts_with(b".RMF") {
        return Some(ContainerFormat::RealMedia);
    }
    if data.starts_with(&[0, 0, 1, 0xb3]) {
        return Some(ContainerFormat::MpegVideo);
    }
    if data.starts_with(&[0, 0, 1, 0xba]) {
        return Some(ContainerFormat::Ps);
    }
    if data.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some(ContainerFormat::Matroska);
    }

    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"AVI " {
        return Some(ContainerFormat::Avi);
    }

    if data.starts_with(&asf::ASF_HEADER_GUID) {
        return Some(ContainerFormat::Asf);
    }

    if data.len() >= 5 && &data[..4] == b"OggS" && data[4] == 0 {
        return Some(ContainerFormat::Ogg);
    }

    if data.len() >= 9 && &data[..3] == b"FLV" && data[3] < 5 {
        let data_offset = u32::from_be_bytes([data[5], data[6], data[7], data[8]]);
        if data_offset >= 9 {
            return Some(ContainerFormat::Flv);
        }
    }

    if looks_like_transport_stream(data) {
        return Some(ContainerFormat::Ts);
    }

    if looks_like_mp4(data) {
        return Some(ContainerFormat::Mp4);
    }

    if elementary::sniff_h264(data) {
        return Some(ContainerFormat::H264);
    }
    None
}

fn looks_like_transport_stream(data: &[u8]) -> bool {
    const TS_PACKET_SIZE: usize = 188;

    [0_usize, 4].into_iter().any(|offset| {
        data.len() > offset + TS_PACKET_SIZE * 2 && {
            data[offset] == 0x47
                && data[offset + TS_PACKET_SIZE] == 0x47
                && data[offset + TS_PACKET_SIZE * 2] == 0x47
        }
    })
}

fn looks_like_mp4(data: &[u8]) -> bool {
    if data.len() < 8 {
        return false;
    }

    let name = &data[4..8];
    let printable_name = name.iter().all(u8::is_ascii_alphanumeric)
        || matches!(name, b"ac-3" | b"ec-3" | b"mp4a" | b".mp3");

    printable_name
        && matches!(
            name,
            b"ftyp" | b"moov" | b"moof" | b"mdat" | b"free" | b"skip" | b"wide" | b"styp"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_native_format_has_a_parser_dispatch() {
        use scryer_media_model::formats::{MEDIA_FORMATS, MediaProbeCapability};
        for entry in MEDIA_FORMATS {
            assert_eq!(
                container_format_from_extension(entry.extension).is_some(),
                entry.probe == MediaProbeCapability::Native,
                "{}",
                entry.extension
            );
        }
    }

    #[test]
    fn container_signatures_win_over_embedded_elementary_headers() {
        let sps = [0, 0, 1, 0x67, 66, 0, 30, 244, 5, 1, 236, 128];
        for (prefix, expected) in [
            (vec![0, 0, 1, 0xba], ContainerFormat::Ps),
            (
                vec![0, 0, 0, 24, b'f', b't', b'y', b'p'],
                ContainerFormat::Mp4,
            ),
            (vec![0x1a, 0x45, 0xdf, 0xa3], ContainerFormat::Matroska),
        ] {
            let bytes = [prefix, sps.to_vec()].concat();
            assert_eq!(
                resolve_container_format("avc", sniff_container_format_from_bytes(&bytes)),
                Some(expected)
            );
        }
    }

    #[test]
    fn content_only_aliases_never_force_a_container() {
        for extension in ["dat", "svq3", "bivx", "divx", "xvid"] {
            assert_eq!(resolve_container_format(extension, None), None);
            assert_eq!(
                resolve_container_format(extension, Some(ContainerFormat::Avi)),
                Some(ContainerFormat::Avi)
            );
        }
    }

    #[test]
    fn sniff_container_format_prefers_matroska_magic_over_extension_hint() {
        assert_eq!(
            resolve_container_format(
                "mp4",
                sniff_container_format_from_bytes(&[0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0, 0])
            ),
            Some(ContainerFormat::Matroska)
        );
    }

    #[test]
    fn sniff_container_format_detects_avi_and_transport_stream() {
        assert_eq!(
            sniff_container_format_from_bytes(b"RIFF\0\0\0\0AVI LIST"),
            Some(ContainerFormat::Avi)
        );

        let mut ts = vec![0_u8; 564];
        ts[0] = 0x47;
        ts[188] = 0x47;
        ts[376] = 0x47;
        assert_eq!(
            sniff_container_format_from_bytes(&ts),
            Some(ContainerFormat::Ts)
        );
    }

    #[test]
    fn sniff_container_format_detects_mp4_box_headers() {
        let mut bytes = vec![0_u8; 16];
        bytes[..4].copy_from_slice(&16_u32.to_be_bytes());
        bytes[4..8].copy_from_slice(b"ftyp");
        assert_eq!(
            sniff_container_format_from_bytes(&bytes),
            Some(ContainerFormat::Mp4)
        );
    }

    #[test]
    fn sniff_container_format_detects_asf_ogg_and_flv() {
        assert_eq!(
            sniff_container_format_from_bytes(&asf::ASF_HEADER_GUID),
            Some(ContainerFormat::Asf)
        );
        assert_eq!(
            sniff_container_format_from_bytes(b"OggS\0"),
            Some(ContainerFormat::Ogg)
        );

        let mut flv = *b"FLV\x01\x05\0\0\0\x09";
        assert_eq!(
            sniff_container_format_from_bytes(&flv),
            Some(ContainerFormat::Flv)
        );
        flv[8] = 8;
        assert_eq!(sniff_container_format_from_bytes(&flv), None);
    }
}

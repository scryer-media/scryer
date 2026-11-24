//! Codec configuration and bitstream header readers shared by every
//! container parser.

pub(crate) mod aac;
pub(crate) mod ac3;
pub(crate) mod audio_config;
pub(crate) mod audio_profile;
pub(crate) mod av1;
pub(crate) mod bits;
pub(crate) mod dolby_vision;
pub(crate) mod dts;
pub(crate) mod h264;
pub(crate) mod hdr10plus;
pub(crate) mod hevc;
pub(crate) mod mjpeg;
pub(crate) mod mpeg_audio;
pub(crate) mod mpeg_video;
pub(crate) mod parameter_sets;
pub(crate) mod picture_headers;
pub(crate) mod sei;
pub(crate) mod stream_probe;
pub(crate) mod vp9;

use scryer_media_model::Codec;

use crate::scan;

/// Codec profile and bit-depth information extracted from bitstream headers.
#[derive(Debug, Clone, Default)]
pub(crate) struct CodecInfo {
    pub profile: Option<String>,
    pub bit_depth: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub frame_rate_fps: Option<f64>,
    /// ITU-T H.273 TransferCharacteristics value extracted from bitstream VUI
    /// (e.g. 16 = SMPTE 2084/PQ, 18 = HLG). Used for HDR detection when the
    /// container doesn't carry this information.
    pub color_transfer: Option<u32>,
}

/// Maps a container-level codec identifier to its normalized codec name.
///
/// Handles MKV codec IDs (e.g. `V_MPEG4/ISO/AVC`), MP4 FourCC codes (e.g.
/// `"avc1"`), and returns `None` for identifiers that require further
/// container-level disambiguation (e.g. `V_MS/VFW/FOURCC`).
pub(crate) fn normalize_codec_name(codec_id: &str) -> Option<Codec> {
    let name = match codec_id {
        // --- MKV video ---
        "V_MPEG4/ISO/AVC" => Codec::H264,
        "V_MPEGH/ISO/HEVC" => Codec::Hevc,
        "V_AV1" => Codec::Av1,
        "V_VP8" => Codec::Vp8,
        "V_VP9" => Codec::Vp9,
        "V_MPEG4/ISO/SP" | "V_MPEG4/ISO/ASP" | "V_MPEG4/ISO/AP" => Codec::Mpeg4,
        "V_MPEG2" => Codec::Mpeg2Video,
        "V_MPEG1" => Codec::Mpeg1Video,
        "V_MJPEG" => Codec::Mjpeg,
        "V_MS/VFW/FOURCC" => return None,

        // --- MKV audio ---
        "A_AC3" => Codec::Ac3,
        "A_EAC3" => Codec::Eac3,
        "A_TRUEHD" => Codec::TrueHd,
        "A_DTS" => Codec::Dts,
        "A_FLAC" => Codec::Flac,
        "A_OPUS" => Codec::Opus,
        "A_VORBIS" => Codec::Vorbis,
        "A_MPEG/L3" => Codec::Mp3,
        "A_MPEG/L2" => Codec::Mp2,
        "A_MPEG/L1" => Codec::Mp1,

        // --- MKV subtitle ---
        "S_TEXT/UTF8" => Codec::Subrip,
        "S_TEXT/ASS" | "S_TEXT/SSA" => Codec::Ass,
        "S_HDMV/PGS" => Codec::HdmvPgsSubtitle,
        "S_VOBSUB" => Codec::DvdSubtitle,
        "S_TEXT/WEBVTT" => Codec::WebVtt,
        "D_WEBVTT/SUBTITLES" | "D_WEBVTT/CAPTIONS" => Codec::WebVtt,

        // --- MP4 FourCC ---
        "avc1" | "avc3" => Codec::H264,
        "dva1" | "dvav" => Codec::H264,
        "hvc1" | "hev1" => Codec::Hevc,
        "dvh1" | "dvhe" => Codec::Hevc,
        "av01" => Codec::Av1,
        "vp09" => Codec::Vp9,
        "mp4v" => Codec::Mpeg4,
        "jpeg" | "mjpg" => Codec::Mjpeg,
        "png " => Codec::Png,
        "mp4a" => Codec::Aac,
        "ac-3" => Codec::Ac3,
        "ec-3" => Codec::Eac3,
        "fLaC" => Codec::Flac,
        "Opus" => Codec::Opus,
        "text" | "tx3g" => Codec::MovText,
        "wvtt" => Codec::WebVtt,
        "stpp" => Codec::Ttml,

        // MKV AAC variants and PCM wildcard
        other => {
            if other == "A_AAC" || other.starts_with("A_AAC/") {
                return Some(Codec::Aac);
            }
            if other == "A_DTS"
                || other.starts_with("A_DTS/")
                || other == "A_DTS/LOSSY"
                || other == "A_DTS/LOSSLESS"
                || other == "A_DTS/EXPRESS"
            {
                return Some(Codec::Dts);
            }
            if other.starts_with("A_PCM/") {
                return Some(Codec::Pcm);
            }
            return None;
        }
    };
    Some(name)
}

pub(crate) fn normalize_pcm_codec_name(codec_id: &str, bit_depth: Option<i32>) -> Option<Codec> {
    if !codec_id.starts_with("A_PCM/") {
        return None;
    }

    let depth = bit_depth.unwrap_or_default();
    let codec_name = match (
        codec_id.contains("/FLOAT"),
        codec_id.contains("/BIG"),
        depth,
    ) {
        (true, true, 32) => Codec::PcmF32Be,
        (true, false, 32) => Codec::PcmF32Le,
        (true, true, 64) => Codec::PcmF64Be,
        (true, false, 64) => Codec::PcmF64Le,
        (false, _, 8) => Codec::PcmS8,
        (false, true, 16) => Codec::PcmS16Be,
        (false, false, 16) => Codec::PcmS16Le,
        (false, true, 24) => Codec::PcmS24Be,
        (false, false, 24) => Codec::PcmS24Le,
        (false, true, 32) => Codec::PcmS32Be,
        (false, false, 32) => Codec::PcmS32Le,
        _ => Codec::Pcm,
    };

    Some(codec_name)
}

pub(crate) fn normalize_video_fourcc_codec_name(fourcc: &str) -> Option<Codec> {
    let codec_name = match fourcc.trim_end_matches('\0') {
        "H264" | "h264" | "X264" | "x264" | "avc1" | "AVC1" => Codec::H264,
        "HEVC" | "hevc" | "H265" | "h265" | "hvc1" | "HVC1" | "hev1" | "HEV1" => Codec::Hevc,
        "XVID" | "xvid" | "DX50" | "dx50" | "DIVX" | "divx" | "DIV3" | "div3" | "DIV4" | "div4"
        | "DIV5" | "div5" | "MP4V" | "mp4v" | "FMP4" | "fmp4" => Codec::Mpeg4,
        "MJPG" | "mjpg" => Codec::Mjpeg,
        "WVC1" | "wvc1" => Codec::Vc1,
        "WMV3" | "wmv3" => Codec::Wmv3,
        "MP2V" | "mp2v" | "mpg2" | "MPG2" => Codec::Mpeg2Video,
        "MP1V" | "mp1v" | "mpg1" | "MPG1" => Codec::Mpeg1Video,
        "VP80" | "vp80" => Codec::Vp8,
        "VP90" | "vp90" => Codec::Vp9,
        _ => return None,
    };

    Some(codec_name)
}

pub(crate) fn normalize_vfw_codec_name(codec_private: Option<&[u8]>) -> Option<Codec> {
    let compression = codec_private
        .and_then(|data| data.get(16..20))
        .and_then(|fourcc| std::str::from_utf8(fourcc).ok())?;
    normalize_video_fourcc_codec_name(compression)
}

/// The bytes after the first `00 00 01 <code>` start code.
pub(crate) fn find_start_code_payload(data: &[u8], code: u8) -> Option<&[u8]> {
    scan::find_mpeg_start_code(data, code).and_then(|i| data.get(i + 4..))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_mkv_video_codecs() {
        assert_eq!(
            normalize_codec_name("V_MPEG4/ISO/AVC")
                .as_ref()
                .map(Codec::as_str),
            Some("h264")
        );
        assert_eq!(
            normalize_codec_name("V_MPEGH/ISO/HEVC")
                .as_ref()
                .map(Codec::as_str),
            Some("hevc")
        );
        assert_eq!(
            normalize_codec_name("V_AV1").as_ref().map(Codec::as_str),
            Some("av1")
        );
        assert_eq!(
            normalize_codec_name("V_VP9").as_ref().map(Codec::as_str),
            Some("vp9")
        );
        assert_eq!(
            normalize_codec_name("V_MPEG4/ISO/SP")
                .as_ref()
                .map(Codec::as_str),
            Some("mpeg4")
        );
        assert_eq!(normalize_codec_name("V_MS/VFW/FOURCC"), None);
    }

    #[test]
    fn normalize_mkv_audio_codecs() {
        assert_eq!(
            normalize_codec_name("A_AAC").as_ref().map(Codec::as_str),
            Some("aac")
        );
        assert_eq!(
            normalize_codec_name("A_AAC/MPEG2/LC")
                .as_ref()
                .map(Codec::as_str),
            Some("aac")
        );
        assert_eq!(
            normalize_codec_name("A_AAC/MPEG4/LC/SBR")
                .as_ref()
                .map(Codec::as_str),
            Some("aac")
        );
        assert_eq!(
            normalize_codec_name("A_AC3").as_ref().map(Codec::as_str),
            Some("ac3")
        );
        assert_eq!(
            normalize_codec_name("A_EAC3").as_ref().map(Codec::as_str),
            Some("eac3")
        );
        assert_eq!(
            normalize_codec_name("A_TRUEHD").as_ref().map(Codec::as_str),
            Some("truehd")
        );
        assert_eq!(
            normalize_codec_name("A_DTS").as_ref().map(Codec::as_str),
            Some("dts")
        );
        assert_eq!(
            normalize_codec_name("A_FLAC").as_ref().map(Codec::as_str),
            Some("flac")
        );
        assert_eq!(
            normalize_codec_name("A_OPUS").as_ref().map(Codec::as_str),
            Some("opus")
        );
        assert_eq!(
            normalize_codec_name("A_VORBIS").as_ref().map(Codec::as_str),
            Some("vorbis")
        );
        assert_eq!(
            normalize_codec_name("A_PCM/INT/LIT")
                .as_ref()
                .map(Codec::as_str),
            Some("pcm")
        );
        assert_eq!(
            normalize_codec_name("A_PCM/FLOAT/IEEE")
                .as_ref()
                .map(Codec::as_str),
            Some("pcm")
        );
        assert_eq!(
            normalize_codec_name("A_MPEG/L3")
                .as_ref()
                .map(Codec::as_str),
            Some("mp3")
        );
    }

    #[test]
    fn normalize_mkv_subtitle_codecs() {
        assert_eq!(
            normalize_codec_name("S_TEXT/UTF8")
                .as_ref()
                .map(Codec::as_str),
            Some("subrip")
        );
        assert_eq!(
            normalize_codec_name("S_TEXT/ASS")
                .as_ref()
                .map(Codec::as_str),
            Some("ass")
        );
        assert_eq!(
            normalize_codec_name("S_TEXT/SSA")
                .as_ref()
                .map(Codec::as_str),
            Some("ass")
        );
        assert_eq!(
            normalize_codec_name("S_HDMV/PGS")
                .as_ref()
                .map(Codec::as_str),
            Some("hdmv_pgs_subtitle")
        );
        assert_eq!(
            normalize_codec_name("S_VOBSUB").as_ref().map(Codec::as_str),
            Some("dvd_subtitle")
        );
        assert_eq!(
            normalize_codec_name("S_TEXT/WEBVTT")
                .as_ref()
                .map(Codec::as_str),
            Some("webvtt")
        );
    }

    #[test]
    fn normalize_mp4_fourcc() {
        assert_eq!(
            normalize_codec_name("avc1").as_ref().map(Codec::as_str),
            Some("h264")
        );
        assert_eq!(
            normalize_codec_name("avc3").as_ref().map(Codec::as_str),
            Some("h264")
        );
        assert_eq!(
            normalize_codec_name("hvc1").as_ref().map(Codec::as_str),
            Some("hevc")
        );
        assert_eq!(
            normalize_codec_name("hev1").as_ref().map(Codec::as_str),
            Some("hevc")
        );
        assert_eq!(
            normalize_codec_name("av01").as_ref().map(Codec::as_str),
            Some("av1")
        );
        assert_eq!(
            normalize_codec_name("vp09").as_ref().map(Codec::as_str),
            Some("vp9")
        );
        assert_eq!(
            normalize_codec_name("mp4a").as_ref().map(Codec::as_str),
            Some("aac")
        );
        assert_eq!(
            normalize_codec_name("ac-3").as_ref().map(Codec::as_str),
            Some("ac3")
        );
        assert_eq!(
            normalize_codec_name("ec-3").as_ref().map(Codec::as_str),
            Some("eac3")
        );
        assert_eq!(
            normalize_codec_name("fLaC").as_ref().map(Codec::as_str),
            Some("flac")
        );
        assert_eq!(
            normalize_codec_name("Opus").as_ref().map(Codec::as_str),
            Some("opus")
        );
        assert_eq!(
            normalize_codec_name("tx3g").as_ref().map(Codec::as_str),
            Some("mov_text")
        );
        assert_eq!(
            normalize_codec_name("wvtt").as_ref().map(Codec::as_str),
            Some("webvtt")
        );
        assert_eq!(
            normalize_codec_name("stpp").as_ref().map(Codec::as_str),
            Some("ttml")
        );
    }

    #[test]
    fn normalize_unknown_returns_none() {
        assert_eq!(normalize_codec_name("XYZZY"), None);
        assert_eq!(normalize_codec_name(""), None);
    }

    #[test]
    fn normalizes_pcm_codec_names_from_depth_and_endianness() {
        assert_eq!(
            normalize_pcm_codec_name("A_PCM/INT/LIT", Some(24))
                .as_ref()
                .map(Codec::as_str),
            Some("pcm_s24le")
        );
        assert_eq!(
            normalize_pcm_codec_name("A_PCM/INT/BIG", Some(16))
                .as_ref()
                .map(Codec::as_str),
            Some("pcm_s16be")
        );
        assert_eq!(
            normalize_pcm_codec_name("A_PCM/FLOAT/IEEE", Some(32))
                .as_ref()
                .map(Codec::as_str),
            Some("pcm_f32le")
        );
    }

    #[test]
    fn normalizes_vfw_fourcc_video_codecs() {
        assert_eq!(
            normalize_codec_name("V_MPEG2").as_ref().map(Codec::as_str),
            Some("mpeg2video")
        );
        assert_eq!(
            normalize_video_fourcc_codec_name("MPG2")
                .as_ref()
                .map(Codec::as_str),
            Some("mpeg2video")
        );
        assert_eq!(
            normalize_vfw_codec_name(Some(&[
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b'M', b'P', b'G', b'2'
            ]))
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
            Some("mpeg2video")
        );
    }
}

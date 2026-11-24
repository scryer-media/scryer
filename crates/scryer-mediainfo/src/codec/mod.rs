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
pub(crate) fn normalize_codec_name(codec_id: &str) -> Option<String> {
    let name = match codec_id {
        // --- MKV video ---
        "V_MPEG4/ISO/AVC" => "h264",
        "V_MPEGH/ISO/HEVC" => "hevc",
        "V_AV1" => "av1",
        "V_VP8" => "vp8",
        "V_VP9" => "vp9",
        "V_MPEG4/ISO/SP" | "V_MPEG4/ISO/ASP" | "V_MPEG4/ISO/AP" => "mpeg4",
        "V_MPEG2" => "mpeg2video",
        "V_MPEG1" => "mpeg1video",
        "V_MJPEG" => "mjpeg",
        "V_MS/VFW/FOURCC" => return None,

        // --- MKV audio ---
        "A_AC3" => "ac3",
        "A_EAC3" => "eac3",
        "A_TRUEHD" => "truehd",
        "A_DTS" => "dts",
        "A_FLAC" => "flac",
        "A_OPUS" => "opus",
        "A_VORBIS" => "vorbis",
        "A_MPEG/L3" => "mp3",
        "A_MPEG/L2" => "mp2",
        "A_MPEG/L1" => "mp1",

        // --- MKV subtitle ---
        "S_TEXT/UTF8" => "subrip",
        "S_TEXT/ASS" | "S_TEXT/SSA" => "ass",
        "S_HDMV/PGS" => "hdmv_pgs_subtitle",
        "S_VOBSUB" => "dvd_subtitle",
        "S_TEXT/WEBVTT" => "webvtt",
        "D_WEBVTT/SUBTITLES" | "D_WEBVTT/CAPTIONS" => "webvtt",

        // --- MP4 FourCC ---
        "avc1" | "avc3" => "h264",
        "dva1" | "dvav" => "h264",
        "hvc1" | "hev1" => "hevc",
        "dvh1" | "dvhe" => "hevc",
        "av01" => "av1",
        "vp09" => "vp9",
        "mp4v" => "mpeg4",
        "jpeg" | "mjpg" => "mjpeg",
        "png " => "png",
        "mp4a" => "aac",
        "ac-3" => "ac3",
        "ec-3" => "eac3",
        "fLaC" => "flac",
        "Opus" => "opus",
        "text" | "tx3g" => "mov_text",
        "wvtt" => "webvtt",
        "stpp" => "ttml",

        // MKV AAC variants and PCM wildcard
        other => {
            if other == "A_AAC" || other.starts_with("A_AAC/") {
                return Some("aac".into());
            }
            if other == "A_DTS"
                || other.starts_with("A_DTS/")
                || other == "A_DTS/LOSSY"
                || other == "A_DTS/LOSSLESS"
                || other == "A_DTS/EXPRESS"
            {
                return Some("dts".into());
            }
            if other.starts_with("A_PCM/") {
                return Some("pcm".into());
            }
            return None;
        }
    };
    Some(name.into())
}

pub(crate) fn normalize_pcm_codec_name(codec_id: &str, bit_depth: Option<i32>) -> Option<String> {
    if !codec_id.starts_with("A_PCM/") {
        return None;
    }

    let depth = bit_depth.unwrap_or_default();
    let codec_name = match (
        codec_id.contains("/FLOAT"),
        codec_id.contains("/BIG"),
        depth,
    ) {
        (true, true, 32) => "pcm_f32be",
        (true, false, 32) => "pcm_f32le",
        (true, true, 64) => "pcm_f64be",
        (true, false, 64) => "pcm_f64le",
        (false, _, 8) => "pcm_s8",
        (false, true, 16) => "pcm_s16be",
        (false, false, 16) => "pcm_s16le",
        (false, true, 24) => "pcm_s24be",
        (false, false, 24) => "pcm_s24le",
        (false, true, 32) => "pcm_s32be",
        (false, false, 32) => "pcm_s32le",
        _ => "pcm",
    };

    Some(codec_name.to_owned())
}

pub(crate) fn normalize_video_fourcc_codec_name(fourcc: &str) -> Option<String> {
    let codec_name = match fourcc.trim_end_matches('\0') {
        "H264" | "h264" | "X264" | "x264" | "avc1" | "AVC1" => "h264",
        "HEVC" | "hevc" | "H265" | "h265" | "hvc1" | "HVC1" | "hev1" | "HEV1" => "hevc",
        "XVID" | "xvid" | "DX50" | "dx50" | "DIVX" | "divx" | "DIV3" | "div3" | "DIV4" | "div4"
        | "DIV5" | "div5" | "MP4V" | "mp4v" | "FMP4" | "fmp4" => "mpeg4",
        "MJPG" | "mjpg" => "mjpeg",
        "WVC1" | "wvc1" => "vc1",
        "WMV3" | "wmv3" => "wmv3",
        "MP2V" | "mp2v" | "mpg2" | "MPG2" => "mpeg2video",
        "MP1V" | "mp1v" | "mpg1" | "MPG1" => "mpeg1video",
        "VP80" | "vp80" => "vp8",
        "VP90" | "vp90" => "vp9",
        _ => return None,
    };

    Some(codec_name.to_owned())
}

pub(crate) fn normalize_vfw_codec_name(codec_private: Option<&[u8]>) -> Option<String> {
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
            normalize_codec_name("V_MPEG4/ISO/AVC").as_deref(),
            Some("h264")
        );
        assert_eq!(
            normalize_codec_name("V_MPEGH/ISO/HEVC").as_deref(),
            Some("hevc")
        );
        assert_eq!(normalize_codec_name("V_AV1").as_deref(), Some("av1"));
        assert_eq!(normalize_codec_name("V_VP9").as_deref(), Some("vp9"));
        assert_eq!(
            normalize_codec_name("V_MPEG4/ISO/SP").as_deref(),
            Some("mpeg4")
        );
        assert_eq!(normalize_codec_name("V_MS/VFW/FOURCC"), None);
    }

    #[test]
    fn normalize_mkv_audio_codecs() {
        assert_eq!(normalize_codec_name("A_AAC").as_deref(), Some("aac"));
        assert_eq!(
            normalize_codec_name("A_AAC/MPEG2/LC").as_deref(),
            Some("aac")
        );
        assert_eq!(
            normalize_codec_name("A_AAC/MPEG4/LC/SBR").as_deref(),
            Some("aac")
        );
        assert_eq!(normalize_codec_name("A_AC3").as_deref(), Some("ac3"));
        assert_eq!(normalize_codec_name("A_EAC3").as_deref(), Some("eac3"));
        assert_eq!(normalize_codec_name("A_TRUEHD").as_deref(), Some("truehd"));
        assert_eq!(normalize_codec_name("A_DTS").as_deref(), Some("dts"));
        assert_eq!(normalize_codec_name("A_FLAC").as_deref(), Some("flac"));
        assert_eq!(normalize_codec_name("A_OPUS").as_deref(), Some("opus"));
        assert_eq!(normalize_codec_name("A_VORBIS").as_deref(), Some("vorbis"));
        assert_eq!(
            normalize_codec_name("A_PCM/INT/LIT").as_deref(),
            Some("pcm")
        );
        assert_eq!(
            normalize_codec_name("A_PCM/FLOAT/IEEE").as_deref(),
            Some("pcm")
        );
        assert_eq!(normalize_codec_name("A_MPEG/L3").as_deref(), Some("mp3"));
    }

    #[test]
    fn normalize_mkv_subtitle_codecs() {
        assert_eq!(
            normalize_codec_name("S_TEXT/UTF8").as_deref(),
            Some("subrip")
        );
        assert_eq!(normalize_codec_name("S_TEXT/ASS").as_deref(), Some("ass"));
        assert_eq!(normalize_codec_name("S_TEXT/SSA").as_deref(), Some("ass"));
        assert_eq!(
            normalize_codec_name("S_HDMV/PGS").as_deref(),
            Some("hdmv_pgs_subtitle")
        );
        assert_eq!(
            normalize_codec_name("S_VOBSUB").as_deref(),
            Some("dvd_subtitle")
        );
        assert_eq!(
            normalize_codec_name("S_TEXT/WEBVTT").as_deref(),
            Some("webvtt")
        );
    }

    #[test]
    fn normalize_mp4_fourcc() {
        assert_eq!(normalize_codec_name("avc1").as_deref(), Some("h264"));
        assert_eq!(normalize_codec_name("avc3").as_deref(), Some("h264"));
        assert_eq!(normalize_codec_name("hvc1").as_deref(), Some("hevc"));
        assert_eq!(normalize_codec_name("hev1").as_deref(), Some("hevc"));
        assert_eq!(normalize_codec_name("av01").as_deref(), Some("av1"));
        assert_eq!(normalize_codec_name("vp09").as_deref(), Some("vp9"));
        assert_eq!(normalize_codec_name("mp4a").as_deref(), Some("aac"));
        assert_eq!(normalize_codec_name("ac-3").as_deref(), Some("ac3"));
        assert_eq!(normalize_codec_name("ec-3").as_deref(), Some("eac3"));
        assert_eq!(normalize_codec_name("fLaC").as_deref(), Some("flac"));
        assert_eq!(normalize_codec_name("Opus").as_deref(), Some("opus"));
        assert_eq!(normalize_codec_name("tx3g").as_deref(), Some("mov_text"));
        assert_eq!(normalize_codec_name("wvtt").as_deref(), Some("webvtt"));
        assert_eq!(normalize_codec_name("stpp").as_deref(), Some("ttml"));
    }

    #[test]
    fn normalize_unknown_returns_none() {
        assert_eq!(normalize_codec_name("XYZZY"), None);
        assert_eq!(normalize_codec_name(""), None);
    }

    #[test]
    fn normalizes_pcm_codec_names_from_depth_and_endianness() {
        assert_eq!(
            normalize_pcm_codec_name("A_PCM/INT/LIT", Some(24)).as_deref(),
            Some("pcm_s24le")
        );
        assert_eq!(
            normalize_pcm_codec_name("A_PCM/INT/BIG", Some(16)).as_deref(),
            Some("pcm_s16be")
        );
        assert_eq!(
            normalize_pcm_codec_name("A_PCM/FLOAT/IEEE", Some(32)).as_deref(),
            Some("pcm_f32le")
        );
    }

    #[test]
    fn normalizes_vfw_fourcc_video_codecs() {
        assert_eq!(
            normalize_codec_name("V_MPEG2").as_deref(),
            Some("mpeg2video")
        );
        assert_eq!(
            normalize_video_fourcc_codec_name("MPG2").as_deref(),
            Some("mpeg2video")
        );
        assert_eq!(
            normalize_vfw_codec_name(Some(&[
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b'M', b'P', b'G', b'2'
            ]))
            .as_deref(),
            Some("mpeg2video")
        );
    }
}

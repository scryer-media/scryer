//! The codec a stream is coded with.

use std::convert::Infallible;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::StreamKind;

macro_rules! codecs {
    ($($(#[doc = $doc:literal])* $variant:ident => $name:literal, $kind:expr;)*) => {
        /// A codec the parser can name, or [`Codec::Other`] for any other name.
        ///
        /// The string form is the short codec name ffprobe uses, such as `h264`,
        /// `eac3` or `subrip`, and it is what serialization writes and reads.
        /// Every name the parser reports has its own variant; a name that has
        /// none, for example one written by a newer version, parses as
        /// [`Codec::Other`] and serializes back unchanged.
        ///
        /// Build a value with [`From<&str>`](Codec::from) or [`str::parse`]
        /// rather than by writing `Codec::Other` directly, so a known name
        /// always lands on its own variant and compares equal to it.
        ///
        /// ```
        /// use scryer_media_model::{Codec, StreamKind};
        ///
        /// let codec = Codec::from("eac3");
        /// assert_eq!(codec, Codec::Eac3);
        /// assert_eq!(codec.as_str(), "eac3");
        /// assert_eq!(codec.kind(), Some(StreamKind::Audio));
        /// assert_eq!(Codec::from("future_codec"), Codec::Other("future_codec".into()));
        /// ```
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum Codec {
            $($(#[doc = $doc])* $variant,)*
            /// A codec name with no variant of its own, kept exactly as read.
            Other(String),
        }

        impl Codec {
            /// Every named variant, in declaration order; [`Codec::Other`] is
            /// not included.
            pub const KNOWN: &'static [Codec] = &[$(Codec::$variant,)*];

            /// The codec's short name, for example `"h264"`. Never allocates.
            #[must_use]
            pub fn as_str(&self) -> &str {
                match self {
                    $(Codec::$variant => $name,)*
                    Codec::Other(name) => name,
                }
            }

            /// The kind of stream this codec carries, when the codec alone
            /// decides it. `None` for [`Codec::Unknown`], for
            /// [`Codec::HdmvInteractiveGraphics`] (menus, which are neither
            /// picture nor subtitle), and for [`Codec::Other`].
            #[must_use]
            pub fn kind(&self) -> Option<StreamKind> {
                match self {
                    $(Codec::$variant => $kind,)*
                    Codec::Other(_) => None,
                }
            }

            fn known(name: &str) -> Option<Codec> {
                match name {
                    $($name => Some(Codec::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

const VIDEO: Option<StreamKind> = Some(StreamKind::Video);
const AUDIO: Option<StreamKind> = Some(StreamKind::Audio);
const SUBTITLE: Option<StreamKind> = Some(StreamKind::Subtitle);

codecs! {
    /// H.264 / AVC.
    H264 => "h264", VIDEO;
    /// H.265 / HEVC.
    Hevc => "hevc", VIDEO;
    /// AV1.
    Av1 => "av1", VIDEO;
    /// VP8.
    Vp8 => "vp8", VIDEO;
    /// VP9.
    Vp9 => "vp9", VIDEO;
    /// On2 VP6 as carried in Flash video.
    Vp6f => "vp6f", VIDEO;
    /// On2 VP6 with an alpha channel, as carried in Flash video.
    Vp6a => "vp6a", VIDEO;
    /// SMPTE VC-1 (including Windows Media Video 9 Advanced Profile).
    Vc1 => "vc1", VIDEO;
    /// Windows Media Video 7.
    Wmv1 => "wmv1", VIDEO;
    /// Windows Media Video 8.
    Wmv2 => "wmv2", VIDEO;
    /// Windows Media Video 9.
    Wmv3 => "wmv3", VIDEO;
    /// MPEG-1 video.
    Mpeg1Video => "mpeg1video", VIDEO;
    /// MPEG-2 video.
    Mpeg2Video => "mpeg2video", VIDEO;
    /// MPEG-4 Part 2 video (DivX, Xvid and similar).
    Mpeg4 => "mpeg4", VIDEO;
    /// Motion JPEG; also cover art stored as a JPEG picture.
    Mjpeg => "mjpeg", VIDEO;
    /// PNG; cover art stored as a PNG picture.
    Png => "png", VIDEO;
    /// Theora.
    Theora => "theora", VIDEO;
    /// Sorenson Spark (H.263 as carried in Flash video).
    Flv1 => "flv1", VIDEO;
    /// RealVideo 1.0.
    Rv10 => "rv10", VIDEO;
    /// RealVideo 2.0.
    Rv20 => "rv20", VIDEO;
    /// RealVideo 3.0.
    Rv30 => "rv30", VIDEO;
    /// RealVideo 4.0.
    Rv40 => "rv40", VIDEO;
    /// DV video.
    DvVideo => "dvvideo", VIDEO;
    /// Chinese AVS (AVS1-P2).
    Cavs => "cavs", VIDEO;
    /// AVS2.
    Avs2 => "avs2", VIDEO;
    /// AVS3.
    Avs3 => "avs3", VIDEO;

    /// AAC in ADTS or a container's own framing.
    Aac => "aac", AUDIO;
    /// AAC in LATM/LOAS framing.
    AacLatm => "aac_latm", AUDIO;
    /// Dolby Digital (AC-3).
    Ac3 => "ac3", AUDIO;
    /// Dolby Digital Plus (E-AC-3).
    Eac3 => "eac3", AUDIO;
    /// Dolby TrueHD.
    TrueHd => "truehd", AUDIO;
    /// DTS, including its extensions.
    Dts => "dts", AUDIO;
    /// FLAC.
    Flac => "flac", AUDIO;
    /// Opus.
    Opus => "opus", AUDIO;
    /// Vorbis.
    Vorbis => "vorbis", AUDIO;
    /// Speex.
    Speex => "speex", AUDIO;
    /// MPEG audio layer I.
    Mp1 => "mp1", AUDIO;
    /// MPEG audio layer II.
    Mp2 => "mp2", AUDIO;
    /// MPEG audio layer III.
    Mp3 => "mp3", AUDIO;
    /// Nellymoser Asao.
    Nellymoser => "nellymoser", AUDIO;
    /// Flash ADPCM.
    AdpcmSwf => "adpcm_swf", AUDIO;
    /// Windows Media Audio 1.
    WmaV1 => "wmav1", AUDIO;
    /// Windows Media Audio 2.
    WmaV2 => "wmav2", AUDIO;
    /// Windows Media Audio Professional.
    WmaPro => "wmapro", AUDIO;
    /// Windows Media Audio Lossless.
    WmaLossless => "wmalossless", AUDIO;
    /// Windows Media Audio Voice.
    WmaVoice => "wmavoice", AUDIO;
    /// RealAudio Cook.
    Cook => "cook", AUDIO;
    /// RealAudio Sipro.
    Sipr => "sipr", AUDIO;
    /// Sony ATRAC3.
    Atrac3 => "atrac3", AUDIO;
    /// RealAudio 1.0 (14.4 kbit/s).
    Ra144 => "ra_144", AUDIO;
    /// RealAudio 2.0 (28.8 kbit/s).
    Ra288 => "ra_288", AUDIO;
    /// PCM whose sample representation the container does not state.
    Pcm => "pcm", AUDIO;
    /// PCM, unsigned 8-bit.
    PcmU8 => "pcm_u8", AUDIO;
    /// PCM, signed 8-bit.
    PcmS8 => "pcm_s8", AUDIO;
    /// PCM, signed 16-bit in the writer's native byte order, as Flash video
    /// declares it.
    PcmS16 => "pcm_s16", AUDIO;
    /// PCM, signed 16-bit little-endian.
    PcmS16Le => "pcm_s16le", AUDIO;
    /// PCM, signed 16-bit big-endian.
    PcmS16Be => "pcm_s16be", AUDIO;
    /// PCM, signed 24-bit little-endian.
    PcmS24Le => "pcm_s24le", AUDIO;
    /// PCM, signed 24-bit big-endian.
    PcmS24Be => "pcm_s24be", AUDIO;
    /// PCM, signed 32-bit little-endian.
    PcmS32Le => "pcm_s32le", AUDIO;
    /// PCM, signed 32-bit big-endian.
    PcmS32Be => "pcm_s32be", AUDIO;
    /// PCM, signed 64-bit little-endian.
    PcmS64Le => "pcm_s64le", AUDIO;
    /// PCM, 32-bit float little-endian.
    PcmF32Le => "pcm_f32le", AUDIO;
    /// PCM, 32-bit float big-endian.
    PcmF32Be => "pcm_f32be", AUDIO;
    /// PCM, 64-bit float little-endian.
    PcmF64Le => "pcm_f64le", AUDIO;
    /// PCM, 64-bit float big-endian.
    PcmF64Be => "pcm_f64be", AUDIO;
    /// G.711 A-law.
    PcmAlaw => "pcm_alaw", AUDIO;
    /// G.711 mu-law.
    PcmMulaw => "pcm_mulaw", AUDIO;
    /// DVD-Video LPCM.
    PcmDvd => "pcm_dvd", AUDIO;
    /// Blu-ray LPCM.
    PcmBluray => "pcm_bluray", AUDIO;

    /// SubRip / plain UTF-8 text subtitles.
    Subrip => "subrip", SUBTITLE;
    /// SubStation Alpha and Advanced SubStation Alpha.
    Ass => "ass", SUBTITLE;
    /// WebVTT.
    WebVtt => "webvtt", SUBTITLE;
    /// MPEG-4 Timed Text (3GPP text).
    MovText => "mov_text", SUBTITLE;
    /// TTML.
    Ttml => "ttml", SUBTITLE;
    /// Plain text subtitles without further markup.
    Text => "text", SUBTITLE;
    /// DVD bitmap subtitles (VobSub).
    DvdSubtitle => "dvd_subtitle", SUBTITLE;
    /// Blu-ray presentation graphics (PGS).
    HdmvPgsSubtitle => "hdmv_pgs_subtitle", SUBTITLE;
    /// Blu-ray text subtitles.
    HdmvTextSubtitle => "hdmv_text_subtitle", SUBTITLE;
    /// Blu-ray interactive graphics (menus).
    HdmvInteractiveGraphics => "hdmv_interactive_graphics", None;
    /// DVB bitmap subtitles.
    DvbSubtitle => "dvb_subtitle", SUBTITLE;
    /// DVB teletext.
    DvbTeletext => "dvb_teletext", SUBTITLE;

    /// A stream the container declares but whose codec it does not identify.
    Unknown => "unknown", None;
}

/// Codecs order by name, as the names themselves do.
impl Ord for Codec {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for Codec {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for Codec {
    fn from(name: &str) -> Self {
        Codec::known(name).unwrap_or_else(|| Codec::Other(name.to_owned()))
    }
}

impl From<String> for Codec {
    /// Reuses the string's allocation when the name has no variant.
    fn from(name: String) -> Self {
        Codec::known(&name).unwrap_or(Codec::Other(name))
    }
}

impl From<Codec> for String {
    /// Reuses the allocation of an [`Other`](Codec::Other) name.
    fn from(codec: Codec) -> Self {
        match codec {
            Codec::Other(name) => name,
            known => known.as_str().to_owned(),
        }
    }
}

impl FromStr for Codec {
    type Err = Infallible;

    /// Never fails: an unrecognized name becomes [`Codec::Other`].
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Ok(Codec::from(name))
    }
}

impl AsRef<str> for Codec {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<str> for Codec {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Codec {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl Serialize for Codec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Codec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = Codec;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a codec name")
            }

            fn visit_str<E: serde::de::Error>(self, name: &str) -> Result<Codec, E> {
                Ok(Codec::from(name))
            }

            fn visit_string<E: serde::de::Error>(self, name: String) -> Result<Codec, E> {
                Ok(Codec::from(name))
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde::de::IntoDeserializer;
    use serde::de::value::{Error, StringDeserializer};

    fn from_borrowed(name: &str) -> Codec {
        Codec::deserialize(IntoDeserializer::<Error>::into_deserializer(name)).unwrap()
    }

    fn from_owned(name: &str) -> Codec {
        let deserializer: StringDeserializer<Error> = name.to_owned().into_deserializer();
        Codec::deserialize(deserializer).unwrap()
    }

    // JSON output is checked in the parser crate, which has a JSON serializer.
    #[test]
    fn every_variant_round_trips_through_its_name() {
        let mut names = std::collections::HashSet::new();
        for codec in Codec::KNOWN {
            assert!(names.insert(codec.as_str()), "duplicate {codec}");
            assert_eq!(&Codec::from(codec.as_str()), codec);
            assert_eq!(&codec.as_str().parse::<Codec>().unwrap(), codec);
            assert_eq!(&Codec::from(codec.as_str().to_owned()), codec);
            assert_eq!(&from_borrowed(codec.as_str()), codec);
            assert_eq!(&from_owned(codec.as_str()), codec);
            assert_eq!(codec.to_string(), codec.as_str());
        }
    }

    #[test]
    fn unknown_names_are_kept_verbatim() {
        let codec = Codec::from("Some Future Codec");
        assert_eq!(codec, Codec::Other("Some Future Codec".into()));
        assert_eq!(codec.kind(), None);
        assert_eq!(codec.as_str(), "Some Future Codec");
        assert_eq!(from_borrowed("Some Future Codec"), codec);
        assert_eq!(from_owned("Some Future Codec"), codec);
        assert_eq!(String::from(codec), "Some Future Codec");
        assert_eq!(String::from(Codec::Hevc), "hevc");
        // Matching is exact: a different spelling is a different codec.
        assert_eq!(Codec::from("H264"), Codec::Other("H264".into()));
    }

    #[test]
    fn kinds_follow_the_codec() {
        assert_eq!(Codec::Hevc.kind(), Some(StreamKind::Video));
        assert_eq!(Codec::TrueHd.kind(), Some(StreamKind::Audio));
        assert_eq!(Codec::Subrip.kind(), Some(StreamKind::Subtitle));
        assert_eq!(Codec::Unknown.kind(), None);
        assert_eq!(Codec::HdmvInteractiveGraphics.kind(), None);
    }
}

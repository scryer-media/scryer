use chrono::NaiveDate;
use core::range::Range;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use smallvec::SmallVec;
use std::fmt;

use crate::lex::{ReleaseCst, TextSpan, Token};

/// Token range within the lossless token stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TokenRange {
    pub start_token: usize,
    pub end_token: usize,
}

impl TokenRange {
    pub fn new(start_token: usize, end_token: usize) -> Self {
        Self {
            start_token,
            end_token,
        }
    }

    pub fn indices(self) -> Range<usize> {
        Range {
            start: self.start_token,
            end: self.end_token,
        }
    }

    pub fn len(self) -> usize {
        self.end_token.saturating_sub(self.start_token)
    }

    pub fn is_empty(self) -> bool {
        self.start_token >= self.end_token
    }
}

/// Family selected by the beam parser for a candidate release interpretation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseFamily {
    Movie,
    StandardEpisode,
    DailyEpisode,
    AnimeAbsolute,
    SeasonPack,
    EpisodeRangePack,
    Special,
    #[default]
    Unknown,
}

/// Special episode kinds recognized by the parser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParsedSpecialKind {
    #[default]
    Special,
    Ova,
    Oad,
    Ncop,
    Nced,
    Extra,
}

impl ParsedSpecialKind {
    pub const OVA: Self = Self::Ova;
    pub const OAD: Self = Self::Oad;
    pub const OVD: Self = Self::Oad;
    pub const NCOP: Self = Self::Ncop;
    pub const NCED: Self = Self::Nced;
}

/// Episodic release type recognized by the parser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParsedEpisodeReleaseType {
    SingleEpisode,
    MultiEpisode,
    SeasonPack,
    RangePack,
    Daily,
    #[default]
    Unknown,
}

/// Structured episodic metadata projected from the winning parse candidate.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ParsedEpisodeMetadata {
    pub season: Option<u32>,
    /// Every explicitly named season in a pack. Empty means a complete-series
    /// marker that covers the title rather than an enumerated season range.
    pub season_numbers: Vec<u32>,
    pub episode_numbers: Vec<u32>,
    pub absolute_episode: Option<u32>,
    pub absolute_episode_numbers: Vec<u32>,
    pub special_absolute_episode_numbers: Vec<u32>,
    pub air_date: Option<NaiveDate>,
    pub daily_part: Option<u32>,
    pub full_season: bool,
    pub is_partial_season: bool,
    pub is_multi_season: bool,
    /// A release that packages a whole series or several explicit seasons.
    pub is_series_pack: bool,
    pub season_part: Option<u32>,
    pub is_season_extra: bool,
    pub is_split_episode: bool,
    pub is_mini_series: bool,
    pub special_kind: Option<ParsedSpecialKind>,
    pub release_type: ParsedEpisodeReleaseType,
    pub raw: Option<String>,
}

impl ParsedEpisodeMetadata {
    #[must_use]
    pub fn first_episode(&self) -> Option<u32> {
        self.episode_numbers
            .first()
            .copied()
            .or_else(|| self.absolute_episode_numbers.first().copied())
            .or_else(|| self.special_absolute_episode_numbers.first().copied())
    }
}

/// Parsed external id projected from raw metadata tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ParsedExternalId {
    pub source: ExternalIdSource,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReleaseSource {
    WebDl,
    WebRip,
    BluRay,
    BrDisk,
    Dvd,
    Hdtv,
    Cam,
    Telesync,
    Telecine,
    DvdScr,
    Workprint,
}

impl ReleaseSource {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::WebDl => "WEB-DL",
            Self::WebRip => "WEBRip",
            Self::BluRay => "BluRay",
            Self::BrDisk => "BRDISK",
            Self::Dvd => "DVD",
            Self::Hdtv => "HDTV",
            Self::Cam => "CAM",
            Self::Telesync => "TELESYNC",
            Self::Telecine => "TELECINE",
            Self::DvdScr => "DVDSCR",
            Self::Workprint => "WORKPRINT",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let value = raw
            .trim()
            .to_ascii_uppercase()
            .replace(['-', '_', '.', ' '], "");
        match value.as_str() {
            "WEBDL" | "WEB" => Some(Self::WebDl),
            "WEBRIP" | "WEBRI" => Some(Self::WebRip),
            "BLURAY" | "BLU" | "BD" | "UHD" | "BDRIP" | "BRRIP" | "BDREMUX" | "BDRIO" => {
                Some(Self::BluRay)
            }
            "BRDISK" | "BDMV" | "BDISO" | "BD25" | "BD50" | "BD66" | "BD100" => Some(Self::BrDisk),
            "DVD" | "DVDRIP" => Some(Self::Dvd),
            "HDTV" | "RAWHD" => Some(Self::Hdtv),
            "CAM" | "HQCAM" | "CAMRIP" | "HDCAM" => Some(Self::Cam),
            "TELESYNC" | "TS" => Some(Self::Telesync),
            "TELECINE" | "TC" => Some(Self::Telecine),
            "DVDSCR" | "DVDSCREENER" => Some(Self::DvdScr),
            "WORKPRINT" | "WP" => Some(Self::Workprint),
            _ => None,
        }
    }
}

impl fmt::Display for ReleaseSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ReleaseSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ReleaseSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown release source label: {raw}")))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    H264,
    H265,
    Av1,
    Vp9,
    Vc1,
    Mpeg2,
    Mpeg4,
    Xvid,
    Divx,
    Vvc,
}

impl VideoCodec {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::H264 => "H.264",
            Self::H265 => "H.265",
            Self::Av1 => "AV1",
            Self::Vp9 => "VP9",
            Self::Vc1 => "VC1",
            Self::Mpeg2 => "MPEG2",
            Self::Mpeg4 => "MPEG4",
            Self::Xvid => "XVID",
            Self::Divx => "DIVX",
            Self::Vvc => "VVC",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let value = raw.trim();
        match value.to_ascii_uppercase().as_str() {
            "AVC" | "AVC1" | "H264" | "H.264" | "X264" => Some(Self::H264),
            "HEVC" | "HEV1" | "H265" | "H.265" | "HVC1" | "X265" => Some(Self::H265),
            "AV1" | "AV01" => Some(Self::Av1),
            "VP9" => Some(Self::Vp9),
            "VC1" | "VC-1" => Some(Self::Vc1),
            "MPEG2" | "MPEG-2" => Some(Self::Mpeg2),
            "MPEG4" | "MPEG-4" | "MP4V" => Some(Self::Mpeg4),
            "XVID" => Some(Self::Xvid),
            "DIVX" => Some(Self::Divx),
            "VVC" | "H266" | "H.266" => Some(Self::Vvc),
            _ => None,
        }
    }
}

impl fmt::Display for VideoCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for VideoCodec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for VideoCodec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown video codec label: {raw}")))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    Ddp,
    Eac3,
    Ac3,
    Aac,
    TrueHd,
    DtsMa,
    DtsX,
    DtsHd,
    Dts,
    Flac,
    Opus,
    Vorbis,
    Mp3,
    Pcm,
}

impl AudioCodec {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Ddp => "DDP",
            Self::Eac3 => "EAC3",
            Self::Ac3 => "AC3",
            Self::Aac => "AAC",
            Self::TrueHd => "TRUEHD",
            Self::DtsMa => "DTSMA",
            Self::DtsX => "DTSX",
            Self::DtsHd => "DTSHD",
            Self::Dts => "DTS",
            Self::Flac => "FLAC",
            Self::Opus => "OPUS",
            Self::Vorbis => "VORBIS",
            Self::Mp3 => "MP3",
            Self::Pcm => "PCM",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let value = raw.trim().to_ascii_uppercase();
        if matches!(
            value.as_str(),
            "DD+" | "DD PLUS" | "DOLBY DIGITAL+" | "DOLBYDIGITAL+"
        ) {
            return Some(Self::Ddp);
        }
        let compact = value.replace(['-', '_', '.', ' ', ':', '+'], "");
        match compact.as_str() {
            "DDP" | "DDPLUS" | "DOLBYDIGITALPLUS" => Some(Self::Ddp),
            "EAC3" | "EAC" | "EC3" => Some(Self::Eac3),
            "AC3" | "DD" | "DOLBYDIGITAL" => Some(Self::Ac3),
            "AAC" | "AACLC" | "HEAAC" => Some(Self::Aac),
            "TRUEHD" | "DOLBYTRUEHD" => Some(Self::TrueHd),
            "DTSMA" | "DTSHDMA" | "DTSHDMASTER" | "DTSHDMASTERAUDIO" => Some(Self::DtsMa),
            "DTSX" => Some(Self::DtsX),
            "DTSHD" => Some(Self::DtsHd),
            "DTS" => Some(Self::Dts),
            "FLAC" => Some(Self::Flac),
            "OPUS" => Some(Self::Opus),
            "VORBIS" => Some(Self::Vorbis),
            "MP3" | "MPEG3" | "MPEGAUDIOLAYER3" => Some(Self::Mp3),
            "PCM" | "LPCM" => Some(Self::Pcm),
            _ => None,
        }
    }
}

impl fmt::Display for AudioCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for AudioCodec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AudioCodec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown audio codec label: {raw}")))
    }
}

/// A streaming service a release can be tagged with.
///
/// The membership follows the distilled TRaSH alias table plus the curated
/// supplement; `service_display_names_round_trip_through_parse`
/// keeps the two in step, so a service admitted to the alias table can always be
/// projected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StreamingService {
    Amazon,
    Netflix,
    AppleTvPlus,
    DisneyPlus,
    HboMax,
    ParamountPlus,
    Peacock,
    Hulu,
    Crunchyroll,
    Funimation,
    Hidive,
    Stan,
    Itunes,
    Bilibili,
    Hotstar,
    BbcIplayer,
    Youtube,
    Abema,
    AbcIview,
    Adn,
    Atv,
    Bcore,
    BGlobal,
    CanalPlus,
    CbcGem,
    Channel4,
    CoupangPlay,
    Crave,
    DcUniverse,
    DiscoveryPlus,
    DisneyPlusHotstar,
    DmmTv,
    Fod,
    FridayVideo,
    HamiVideo,
    Iqiyi,
    Itvx,
    Kktv,
    Kocowa,
    LineTv,
    My5,
    MyTvSuper,
    Nlziet,
    Now,
    OvidTv,
    PatheThuis,
    Play,
    Quibi,
    RokuChannel,
    Salto,
    Showtime,
    StarPlus,
    Syfy,
    Tver,
    Tving,
    Videoland,
    Viki,
    Viu,
    Vrv,
    Wakanim,
    Wavve,
    WeTv,
    Youku,
    YoutubePremium,
}

impl StreamingService {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Amazon => "Amazon",
            Self::Netflix => "Netflix",
            Self::AppleTvPlus => "Apple TV+",
            Self::DisneyPlus => "Disney+",
            Self::HboMax => "HBO Max",
            Self::ParamountPlus => "Paramount+",
            Self::Peacock => "Peacock",
            Self::Hulu => "Hulu",
            Self::Crunchyroll => "Crunchyroll",
            Self::Funimation => "Funimation",
            Self::Hidive => "HIDIVE",
            Self::Stan => "Stan",
            Self::Itunes => "iTunes",
            Self::Bilibili => "Bilibili",
            Self::Hotstar => "Hotstar",
            Self::BbcIplayer => "BBC iPlayer",
            Self::Youtube => "YouTube",
            Self::Abema => "ABEMA",
            Self::AbcIview => "ABC iview",
            Self::Adn => "ADN",
            Self::Atv => "ATV",
            Self::Bcore => "BCORE",
            Self::BGlobal => "B-Global",
            Self::CanalPlus => "CANAL+",
            Self::CbcGem => "CBC Gem",
            Self::Channel4 => "Channel 4",
            Self::CoupangPlay => "Coupang Play",
            Self::Crave => "Crave",
            Self::DcUniverse => "DC Universe",
            Self::DiscoveryPlus => "Discovery+",
            Self::DisneyPlusHotstar => "Disney+ Hotstar",
            Self::DmmTv => "DMM TV",
            Self::Fod => "FOD",
            Self::FridayVideo => "friDay Video",
            Self::HamiVideo => "Hami Video",
            Self::Iqiyi => "iQIYI",
            Self::Itvx => "ITVX",
            Self::Kktv => "KKTV",
            Self::Kocowa => "KOCOWA",
            Self::LineTv => "LINE TV",
            Self::My5 => "My5",
            Self::MyTvSuper => "myTV SUPER",
            Self::Nlziet => "NLZiet",
            Self::Now => "NOW",
            Self::OvidTv => "OVID.tv",
            Self::PatheThuis => "Pathé Thuis",
            Self::Play => "PLAY",
            Self::Quibi => "Quibi",
            Self::RokuChannel => "The Roku Channel",
            Self::Salto => "Salto",
            Self::Showtime => "Showtime",
            Self::StarPlus => "Star+",
            Self::Syfy => "SYFY",
            Self::Tver => "TVer",
            Self::Tving => "TVING",
            Self::Videoland => "Videoland",
            Self::Viki => "Viki",
            Self::Viu => "Viu",
            Self::Vrv => "VRV",
            Self::Wakanim => "Wakanim",
            Self::Wavve => "Wavve",
            Self::WeTv => "WeTV",
            Self::Youku => "Youku",
            Self::YoutubePremium => "YouTube Premium",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let value = raw
            .trim()
            .to_ascii_uppercase()
            .replace(['-', '_', '.', ' ', '+'], "");
        match value.as_str() {
            "AMZN" | "AMAZON" | "AMAZONPRIME" | "PRIMEVIDEO" => Some(Self::Amazon),
            "NF" | "NETFLIX" => Some(Self::Netflix),
            "ATVP" | "APTV" | "APPLETV" | "APPLETVPLUS" => Some(Self::AppleTvPlus),
            "DSNP" | "DNSP" | "DISNEY" | "DISNEYPLUS" => Some(Self::DisneyPlus),
            "MAX" | "HMAX" | "HBO" | "HBOMAX" => Some(Self::HboMax),
            "PMTP" | "PARAMOUNT" | "PARAMOUNTPLUS" => Some(Self::ParamountPlus),
            "PCOK" | "PEACOCK" => Some(Self::Peacock),
            "HULU" => Some(Self::Hulu),
            "CR" | "CRUNCHYROLL" => Some(Self::Crunchyroll),
            "FUNI" | "FUNIMATION" => Some(Self::Funimation),
            "HIDIVE" => Some(Self::Hidive),
            "STAN" => Some(Self::Stan),
            "ITUNES" => Some(Self::Itunes),
            "BILI" | "BILIBILI" => Some(Self::Bilibili),
            "HOTSTAR" => Some(Self::Hotstar),
            "BBC" | "BBCI" | "IPLAYER" | "BBCIPLAYER" => Some(Self::BbcIplayer),
            "YOUTUBE" => Some(Self::Youtube),
            "ABEMA" | "ABEMATV" => Some(Self::Abema),
            "ABCIVIEW" | "AUBC" => Some(Self::AbcIview),
            "ADN" => Some(Self::Adn),
            "ATV" => Some(Self::Atv),
            "BCORE" => Some(Self::Bcore),
            "BGLOBAL" => Some(Self::BGlobal),
            "CANAL" | "CNLP" => Some(Self::CanalPlus),
            "CBC" | "CBCGEM" => Some(Self::CbcGem),
            "CHANNEL4" | "4OD" | "ALL4" => Some(Self::Channel4),
            "COUPANGPLAY" | "CPNG" => Some(Self::CoupangPlay),
            "CRAV" | "CRAVE" => Some(Self::Crave),
            "DCU" | "DCUNIVERSE" => Some(Self::DcUniverse),
            "DISCOVERY" | "DSCP" | "DSCV" => Some(Self::DiscoveryPlus),
            "DISNEYHOTSTAR" | "DSNPHS" | "HTSR" => Some(Self::DisneyPlusHotstar),
            "DMM" | "DMMTV" => Some(Self::DmmTv),
            "FOD" => Some(Self::Fod),
            "FRIDAY" | "FRIDAYVIDEO" => Some(Self::FridayVideo),
            "HAMI" | "HAMIVIDEO" => Some(Self::HamiVideo),
            "IQIY" | "IQIYI" => Some(Self::Iqiyi),
            "ITV" | "ITVX" => Some(Self::Itvx),
            "KKTV" => Some(Self::Kktv),
            "KCW" | "KOCOWA" => Some(Self::Kocowa),
            "LINETV" => Some(Self::LineTv),
            "MY5" => Some(Self::My5),
            "MYTVSUPER" => Some(Self::MyTvSuper),
            "NLZ" | "NLZIET" => Some(Self::Nlziet),
            "NOW" => Some(Self::Now),
            "OVID" | "OVIDTV" => Some(Self::OvidTv),
            // `Pathé Thuis` keeps its accent through `to_ascii_uppercase`, so
            // the display name normalizes with a lowercase `é`.
            "PATHE" | "PATHETHUIS" | "PATHéTHUIS" => Some(Self::PatheThuis),
            "PLAY" => Some(Self::Play),
            "QIBI" | "QUIBI" => Some(Self::Quibi),
            "ROKU" | "THEROKUCHANNEL" => Some(Self::RokuChannel),
            "SALTO" => Some(Self::Salto),
            "SHO" | "SHOWTIME" => Some(Self::Showtime),
            "STAR" | "STRP" => Some(Self::StarPlus),
            "SYFY" => Some(Self::Syfy),
            "TVER" => Some(Self::Tver),
            "TVING" => Some(Self::Tving),
            "VDL" | "VIDEOLAND" => Some(Self::Videoland),
            "VIKI" => Some(Self::Viki),
            "VIU" => Some(Self::Viu),
            "VRV" => Some(Self::Vrv),
            "WAKA" | "WAKANIM" | "WKN" => Some(Self::Wakanim),
            "WAVVE" => Some(Self::Wavve),
            "WETV" => Some(Self::WeTv),
            "YOUKU" => Some(Self::Youku),
            "RED" | "YOUTUBEPREMIUM" => Some(Self::YoutubePremium),
            _ => None,
        }
    }
}

impl fmt::Display for StreamingService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for StreamingService {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StreamingService {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!("unknown streaming service label: {raw}"))
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExternalIdSource {
    Imdb,
    Tmdb,
    Tvdb,
    AniDb,
    AniDbEpisode,
    AniList,
    Mal,
}

impl ExternalIdSource {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Imdb => "imdb",
            Self::Tmdb => "tmdb",
            Self::Tvdb => "tvdb",
            Self::AniDb => "anidb",
            Self::AniDbEpisode => "anidb_episode",
            Self::AniList => "anilist",
            Self::Mal => "mal",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let value = raw.trim().to_ascii_lowercase();
        let compact = value.replace(['-', '_', '.', ' '], "");
        match compact.as_str() {
            "imdb" | "imdbid" => Some(Self::Imdb),
            "tmdb" | "tmdbid" => Some(Self::Tmdb),
            "tvdb" | "tvdbid" => Some(Self::Tvdb),
            "anidb" | "anidbid" => Some(Self::AniDb),
            "anidbepisode" | "anidbepisodeid" => Some(Self::AniDbEpisode),
            "anilist" | "anilistid" => Some(Self::AniList),
            "mal" | "malid" => Some(Self::Mal),
            _ => None,
        }
    }
}

impl fmt::Display for ExternalIdSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ExternalIdSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ExternalIdSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!("unknown external id source label: {raw}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioCodec, ExternalIdSource, ReleaseSource, StreamingService, VideoCodec};

    #[test]
    fn video_codec_serializes_to_canonical_label() {
        let encoded = serde_json::to_string(&VideoCodec::parse("HEVC").expect("parse codec"))
            .expect("serialize codec");
        assert_eq!(encoded, "\"H.265\"");
    }

    #[test]
    fn video_codec_deserializes_legacy_alias_to_canonical_variant() {
        let decoded: VideoCodec = serde_json::from_str("\"AVC1\"").expect("deserialize codec");
        assert_eq!(decoded, VideoCodec::H264);
        assert_eq!(decoded.to_string(), "H.264");
    }

    #[test]
    fn video_codec_rejects_unknown_label() {
        assert!(VideoCodec::parse("some-weird-codec").is_none());
        assert!(serde_json::from_str::<VideoCodec>("\"some-weird-codec\"").is_err());
    }

    #[test]
    fn strict_metadata_enums_serialize_to_canonical_labels() {
        assert_eq!(
            serde_json::to_string(&ReleaseSource::parse("bluray").expect("parse source"))
                .expect("serialize source"),
            "\"BluRay\""
        );
        assert_eq!(
            serde_json::to_string(&AudioCodec::parse("DD+").expect("parse audio"))
                .expect("serialize audio"),
            "\"DDP\""
        );
        assert_eq!(
            serde_json::to_string(&StreamingService::parse("ATVP").expect("parse service"))
                .expect("serialize service"),
            "\"Apple TV+\""
        );
        assert_eq!(
            serde_json::to_string(&ExternalIdSource::parse("IMDBID").expect("parse id source"))
                .expect("serialize id source"),
            "\"imdb\""
        );
    }

    #[test]
    fn strict_metadata_enums_reject_unknown_labels() {
        assert!(serde_json::from_str::<ReleaseSource>("\"laserdisc\"").is_err());
        assert!(serde_json::from_str::<AudioCodec>("\"some-audio-codec\"").is_err());
        assert!(serde_json::from_str::<StreamingService>("\"some-service\"").is_err());
        assert!(serde_json::from_str::<ExternalIdSource>("\"some-id-source\"").is_err());
    }
}

/// Overall disposition of a parse attempt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseDisposition {
    #[default]
    Parsed,
    Ambiguous,
    Unparseable,
}

/// Stable TRaSH Guides fact derived from a raw release title.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct GuideFact {
    pub code: String,
}

/// Structured release parse returned by the v2 parser.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ParsedReleaseMetadata {
    pub raw_title: String,
    pub guide_facts: Vec<GuideFact>,
    pub normalized_title: String,
    pub normalized_title_variants: Vec<String>,
    pub release_group: Option<String>,
    pub languages_audio: Vec<String>,
    pub languages_subtitles: Vec<String>,
    pub external_ids: Vec<ParsedExternalId>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub year: Option<i32>,
    pub quality: Option<String>,
    pub source: Option<ReleaseSource>,
    pub video_codec: Option<VideoCodec>,
    pub video_encoding: Option<String>,
    pub audio: Option<AudioCodec>,
    pub audio_codecs: Vec<AudioCodec>,
    pub audio_channels: Option<String>,
    pub is_dual_audio: bool,
    pub is_atmos: bool,
    pub is_dolby_vision: bool,
    pub detected_hdr: bool,
    pub has_hdr_fallback: bool,
    pub is_hdr10plus: bool,
    pub is_hlg: bool,
    pub is_10bit: bool,
    pub fps: Option<f32>,
    pub is_proper_upload: bool,
    pub is_repack: bool,
    pub is_remux: bool,
    pub is_bd_disk: bool,
    pub is_ai_enhanced: bool,
    pub is_hardcoded_subs: bool,
    pub is_uncensored: bool,
    pub is_dubs_only: bool,
    pub streaming_service: Option<StreamingService>,
    pub edition: Option<String>,
    pub anime_version: Option<u32>,
    pub episode: Option<ParsedEpisodeMetadata>,
    pub parser_version: &'static str,
    pub scoring_model_version: u16,
    pub parse_confidence: f32,
    pub ambiguity_margin: i32,
    pub is_ambiguous: bool,
    pub disposition: ParseDisposition,
    pub parse_family: ParseFamily,
    pub missing_fields: Vec<String>,
    pub parse_hints: Vec<String>,
}

impl ParsedReleaseMetadata {
    /// Build an empty parse projection for irrecoverable parse failures.
    #[must_use]
    pub fn empty(raw: &str, parser_version: &'static str) -> Self {
        Self {
            raw_title: raw.to_string(),
            guide_facts: Vec::new(),
            normalized_title: String::new(),
            normalized_title_variants: Vec::new(),
            release_group: None,
            languages_audio: Vec::new(),
            languages_subtitles: Vec::new(),
            external_ids: Vec::new(),
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            year: None,
            quality: None,
            source: None,
            video_codec: None,
            video_encoding: None,
            audio: None,
            audio_codecs: Vec::new(),
            audio_channels: None,
            is_dual_audio: false,
            is_atmos: false,
            is_dolby_vision: false,
            detected_hdr: false,
            has_hdr_fallback: false,
            is_hdr10plus: false,
            is_hlg: false,
            is_10bit: false,
            fps: None,
            is_proper_upload: false,
            is_repack: false,
            is_remux: false,
            is_bd_disk: false,
            is_ai_enhanced: false,
            is_hardcoded_subs: false,
            is_uncensored: false,
            is_dubs_only: false,
            streaming_service: None,
            edition: None,
            anime_version: None,
            episode: None,
            parser_version,
            scoring_model_version: 0,
            parse_confidence: 0.0,
            ambiguity_margin: 0,
            is_ambiguous: true,
            disposition: ParseDisposition::Unparseable,
            parse_family: ParseFamily::Unknown,
            missing_fields: Vec::new(),
            parse_hints: vec!["no_candidate".to_string()],
        }
    }
}

impl Default for ParsedReleaseMetadata {
    fn default() -> Self {
        Self::empty("", "unknown")
    }
}

/// Role assigned to a token by the bounded annotator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenRole {
    Year,
    Quality,
    Source,
    StreamingService,
    VideoCodec,
    AudioCodec,
    AudioChannels,
    Language,
    Edition,
    ReleaseFlag,
    EpisodeMarker,
    SeasonMarker,
    AbsoluteEpisodeMarker,
    DateMarker,
    PackMarker,
    SpecialMarker,
    VersionMarker,
    ExternalId,
    ReleaseGroupCandidate,
    ChecksumOrHash,
    Noise,
    #[default]
    TitleWord,
}

/// Bounded role annotation for a token.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TokenAnnotations {
    pub primary_role: TokenRole,
    pub alternate_roles: SmallVec<[TokenRole; 2]>,
    pub may_be_title_word: bool,
    pub role_confidence: u8,
    pub role_pruned: bool,
}

/// Parse reason emitted by the scorer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ParseReason {
    pub code: String,
    pub delta: i32,
    pub detail: Option<String>,
}

/// Title segment emitted by a parse candidate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleSegmentKind {
    #[default]
    ObservedPrimary,
    ObservedAlternate,
    ContextMatchedAlias,
    Connector,
}

/// Title-bearing span selected by the parser.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TitleSegment {
    pub kind: TitleSegmentKind,
    pub token_start: usize,
    pub token_end: usize,
    pub raw: String,
    pub normalized: String,
}

/// The kind of context evidence that matched a title-bearing token span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextTitleMatchKind {
    CanonicalTitle,
    TitleAlias,
    EpisodeTitle,
}

/// A typed context match retained before target projection rewrites the title.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContextTitleMatch {
    pub kind: ContextTitleMatchKind,
    pub token_range: TokenRange,
    pub raw: String,
    pub normalized: String,
}

/// Metadata AST collected before projection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MetadataAst {
    pub year: Option<i32>,
    pub quality: Option<String>,
    pub source: Option<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<String>,
    pub streaming_service: Option<String>,
    pub edition: Option<String>,
    pub external_ids: Vec<ParsedExternalId>,
    pub token_indices: Vec<usize>,
    pub year_span: Option<TokenRange>,
    pub quality_span: Option<TokenRange>,
    pub source_span: Option<TokenRange>,
    pub video_codec_span: Option<TokenRange>,
    pub audio_codec_span: Option<TokenRange>,
    pub audio_channels_span: Option<TokenRange>,
    pub streaming_service_span: Option<TokenRange>,
    pub edition_span: Option<TokenRange>,
    pub external_id_spans: Vec<TokenRange>,
}

/// Semantic identity extracted from a release candidate.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReleaseIdentity {
    MovieIdentity,
    StandardEpisodeIdentity {
        season: Option<u32>,
        episode_numbers: Vec<u32>,
    },
    DailyIdentity {
        air_date: NaiveDate,
        part: Option<u32>,
    },
    AbsoluteIdentity {
        absolute_episode_numbers: Vec<u32>,
        version: Option<u32>,
        season_hint: Option<u32>,
    },
    SeasonPackIdentity {
        seasons: Vec<u32>,
        is_partial: bool,
        season_part: Option<u32>,
        is_series_pack: bool,
    },
    RangePackIdentity {
        season: Option<u32>,
        range_start: u32,
        range_end: u32,
    },
    SpecialIdentity {
        special_kind: ParsedSpecialKind,
        season_hint: Option<u32>,
        episode_hint: Option<u32>,
    },
    #[default]
    Unknown,
}

/// Candidate parse emitted by the beam search.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReleaseParseCandidate {
    pub family: ParseFamily,
    pub title_segments: Vec<TitleSegment>,
    pub context_title_matches: Vec<ContextTitleMatch>,
    pub identity: ReleaseIdentity,
    pub metadata: MetadataAst,
    pub zones: CandidateZones,
    pub release_group: Option<String>,
    pub unconsumed_tokens: Vec<TextSpan>,
    pub reasons: Vec<ParseReason>,
    pub raw_evidence: Vec<String>,
    pub context_evidence: Vec<String>,
    pub raw_score: i32,
    pub enrichment: Option<MetadataEnrichment>,
    pub projected: ParsedReleaseMetadata,
}

/// Explicit token zones handed off from the beam to metadata enrichment.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CandidateZones {
    pub title_zones: Vec<TokenRange>,
    pub metadata_zone: Option<TokenRange>,
    pub trailing_zone: Option<TokenRange>,
    pub source_span: Option<TokenRange>,
    pub service_span: Option<TokenRange>,
    pub video_span: Option<TokenRange>,
    pub audio_span: Option<TokenRange>,
    pub language_span: Option<TokenRange>,
    pub edition_span: Option<TokenRange>,
    pub release_group_span: Option<TokenRange>,
}

/// Local metadata classification emitted by the deterministic enrichment pass.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MetadataEnrichment {
    pub languages_audio: Vec<String>,
    pub languages_subtitles: Vec<String>,
    pub external_ids: Vec<ParsedExternalId>,
    pub tmdb_id: Option<String>,
    pub video_codec: Option<String>,
    pub video_encoding: Option<String>,
    pub audio: Option<String>,
    pub audio_codecs: Vec<String>,
    pub audio_channels: Option<String>,
    pub is_dual_audio: bool,
    pub is_atmos: bool,
    pub is_dolby_vision: bool,
    pub detected_hdr: bool,
    pub has_hdr_fallback: bool,
    pub is_hdr10plus: bool,
    pub is_hlg: bool,
    pub is_10bit: bool,
    pub fps: Option<f32>,
    pub is_proper_upload: bool,
    pub is_repack: bool,
    pub is_bd_disk: bool,
    pub is_ai_enhanced: bool,
    pub is_hardcoded_subs: bool,
    pub is_uncensored: bool,
    pub is_dubs_only: bool,
    pub edition: Option<String>,
    pub anime_version: Option<u32>,
    pub normalized_source: Option<String>,
    pub parse_hints: Vec<String>,
}

/// Target-aware analysis result for one release string.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReleaseParseAnalysis {
    pub raw_input: String,
    pub sanitized_input: String,
    pub guide_facts: Vec<GuideFact>,
    pub parse_hints: Vec<String>,
    pub tokens: Vec<Token>,
    pub annotations: Vec<TokenAnnotations>,
    pub cst: ReleaseCst,
    pub candidates: Vec<ReleaseParseCandidate>,
    pub best_candidate_index: Option<usize>,
    pub parser_version: &'static str,
    pub scoring_model_version: u16,
    pub ambiguity_margin: i32,
    pub is_ambiguous: bool,
    pub disposition: ParseDisposition,
}

impl ReleaseParseAnalysis {
    /// Return the highest-scoring parse candidate, if one exists.
    #[must_use]
    pub fn best_candidate(&self) -> Option<&ReleaseParseCandidate> {
        self.best_candidate_index
            .and_then(|index| self.candidates.get(index))
    }

    /// Return whether the parser found no viable candidate.
    #[must_use]
    pub fn is_unparseable(&self) -> bool {
        matches!(self.disposition, ParseDisposition::Unparseable)
    }
}

/// One target-specific analysis result with a stable target index.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TargetScoredAnalysis {
    pub target_index: usize,
    pub analysis: ReleaseParseAnalysis,
    pub best_score: i32,
}

/// Multi-target analysis result for one raw release string.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TargetedReleaseParseAnalysis {
    pub targets: Vec<TargetScoredAnalysis>,
    pub best_target_index: Option<usize>,
}

impl TargetedReleaseParseAnalysis {
    /// Return the highest-scoring target analysis, if one exists.
    #[must_use]
    pub fn best_target(&self) -> Option<&TargetScoredAnalysis> {
        let best_index = self.best_target_index?;
        self.targets
            .iter()
            .find(|target| target.target_index == best_index)
    }

    /// Return the score margin between the best and second-best target contexts.
    #[must_use]
    pub fn ambiguity_margin(&self) -> i32 {
        let Some(best_target) = self.best_target() else {
            return 0;
        };
        let second_best = self
            .targets
            .iter()
            .filter(|target| target.target_index != best_target.target_index)
            .filter(|target| !target.analysis.is_unparseable())
            .map(|target| target.best_score)
            .max();
        second_best.map_or(i32::MAX, |score| {
            best_target.best_score.saturating_sub(score)
        })
    }

    /// Return whether the best target choice is ambiguous.
    #[must_use]
    pub fn is_ambiguous(&self) -> bool {
        let Some(best_target) = self.best_target() else {
            return true;
        };
        if best_target.analysis.is_unparseable() || best_target.analysis.is_ambiguous {
            return true;
        }
        let parsed_target_count = self
            .targets
            .iter()
            .filter(|target| !target.analysis.is_unparseable())
            .count();
        parsed_target_count > 1 && self.ambiguity_margin() < 10
    }
}

//! Accepted media suffixes and their native container families.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFormat {
    Matroska,
    Mp4,
    Avi,
    Asf,
    QuickTime,
    MpegTs,
    WebM,
    Flv,
    Ogg,
    Ogm,
    RealMedia,
    Dv,
    H264,
    MpegVideo,
    Svq3,
    MpegProgram,
    StreamLink,
    DiscImage,
    Playlist,
    Companion,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaHandling {
    Native,
    StreamLink,
    DiscImage,
    Playlist,
    Companion,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaProbeCapability {
    Native,
    ContentOnly,
    Unsupported,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaFormatRegistration {
    pub extension: &'static str,
    pub format: MediaFormat,
    pub container_aliases: &'static [&'static str],
    pub handling: MediaHandling,
    pub probe: MediaProbeCapability,
}

macro_rules! media_formats {
    ($(($extension:literal, $format:ident, $handling:ident, $probe:ident, [$($alias:literal),*])),* $(,)?) => {
        pub const MEDIA_FORMATS: &[MediaFormatRegistration] = &[$(
            MediaFormatRegistration {
                extension: $extension,
                format: MediaFormat::$format,
                container_aliases: &[$($alias),*],
                handling: MediaHandling::$handling,
                probe: MediaProbeCapability::$probe,
            },
        )*];
        pub const VIDEO_EXTENSIONS: &[&str] = &[$($extension),*];
    };
}

// Container aliases belong to the preferred suffix; filename aliases retain
// their own suffix when an existing file is imported.
media_formats! {
    ("mkv", Matroska, Native, Native, ["matroska", "mkv"]),
    ("mp4", Mp4, Native, Native, ["mp4"]),
    ("avi", Avi, Native, Native, ["avi"]),
    ("wmv", Asf, Native, Native, ["asf", "wmv"]),
    ("mov", QuickTime, Native, Native, ["mov"]),
    ("m4v", Mp4, Native, Native, ["m4v"]),
    ("ts", MpegTs, Native, Native, ["mpegts", "ts"]),
    ("m2ts", MpegTs, Native, Native, ["m2ts"]),
    ("webm", WebM, Native, Native, ["webm"]),
    ("flv", Flv, Native, Native, ["flv"]),
    ("ogv", Ogg, Native, Native, ["ogg", "ogv"]),
    ("ogm", Ogm, Native, Native, ["ogm"]),
    ("strm", StreamLink, StreamLink, NotApplicable, []),
    ("mpg", MpegProgram, Native, Native, ["mpeg", "mpg"]),
    ("mpeg", MpegProgram, Native, Native, []),
    ("vob", MpegProgram, Native, Native, ["vob"]),
    ("iso", DiscImage, DiscImage, NotApplicable, []),
    ("3gp", Mp4, Native, Native, []),
    ("asf", Asf, Native, Native, []),
    ("asx", Playlist, Playlist, NotApplicable, []),
    ("avc", H264, Native, Native, ["h264", "avc"]),
    ("bin", DiscImage, DiscImage, NotApplicable, []),
    ("bivx", Avi, Native, ContentOnly, []),
    ("dat", MpegProgram, Native, ContentOnly, []),
    ("divx", Avi, Native, ContentOnly, []),
    ("dvr-ms", Asf, Native, Native, []),
    ("dv", Dv, Native, Native, ["dv"]),
    ("fli", Unsupported, Unsupported, Unsupported, []),
    ("ifo", Companion, Companion, NotApplicable, []),
    ("img", DiscImage, DiscImage, NotApplicable, []),
    ("m2t", MpegTs, Native, Native, []),
    ("m2v", MpegVideo, Native, Native, ["mpegvideo", "m2v"]),
    ("m3u", Playlist, Playlist, NotApplicable, []),
    ("mts", MpegTs, Native, Native, []),
    ("nrg", DiscImage, DiscImage, NotApplicable, []),
    ("nsv", Unsupported, Unsupported, Unsupported, []),
    ("nuv", Unsupported, Unsupported, Unsupported, []),
    ("ogg", Ogg, Native, Native, []),
    ("ogx", Ogg, Native, Native, []),
    ("pva", Unsupported, Unsupported, Unsupported, []),
    ("qt", QuickTime, Native, Native, []),
    ("rm", RealMedia, Native, Native, ["realmedia", "rm"]),
    ("rmvb", RealMedia, Native, Native, []),
    ("svq3", Svq3, Native, ContentOnly, []),
    ("ty", Unsupported, Unsupported, Unsupported, []),
    ("viv", Unsupported, Unsupported, Unsupported, []),
    ("vp3", Unsupported, Unsupported, Unsupported, []),
    ("wpl", Playlist, Playlist, NotApplicable, []),
    ("wtv", Unsupported, Unsupported, Unsupported, []),
    ("xvid", Avi, Native, ContentOnly, []),
}

pub fn media_format_for_extension(extension: &str) -> Option<&'static MediaFormatRegistration> {
    MEDIA_FORMATS
        .iter()
        .find(|entry| entry.extension.eq_ignore_ascii_case(extension))
}

/// Normalizes only the established trailing filename junk, never an inner suffix.
pub fn canonical_video_extension(path: &Path) -> Option<&'static str> {
    let name = path
        .file_name()?
        .to_str()?
        .trim()
        .trim_end_matches(|ch: char| {
            ch.is_ascii_whitespace() || matches!(ch, '"' | '\'' | '`' | '_' | '.')
        });
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    media_format_for_extension(extension).map(|entry| entry.extension)
}

pub fn media_format_for_path(path: &Path) -> Option<&'static MediaFormatRegistration> {
    media_format_for_extension(canonical_video_extension(path)?)
}

pub fn preferred_video_extension_for_container(container: &str) -> Option<&'static str> {
    MEDIA_FORMATS.iter().find_map(|entry| {
        entry
            .container_aliases
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(container))
            .then_some(entry.extension)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn recognition_probe_capability_and_handling_are_independent() {
        for extension in ["dat", "bivx", "divx", "xvid", "svq3"] {
            let entry = media_format_for_extension(extension).unwrap();
            assert_eq!(entry.handling, MediaHandling::Native);
            assert_eq!(entry.probe, MediaProbeCapability::ContentOnly);
        }
        for extension in ["fli", "nsv", "nuv", "pva", "ty", "viv", "vp3", "wtv"] {
            let entry = media_format_for_extension(extension).unwrap();
            assert_eq!(entry.handling, MediaHandling::Unsupported);
            assert_eq!(entry.probe, MediaProbeCapability::Unsupported);
        }
        for extension in ["asx", "m3u", "wpl"] {
            assert_eq!(
                media_format_for_extension(extension).unwrap().handling,
                MediaHandling::Playlist
            );
        }
        assert_eq!(
            media_format_for_extension("ifo").unwrap().handling,
            MediaHandling::Companion
        );
    }

    #[test]
    fn registry_is_complete_and_unambiguous() {
        assert_eq!(MEDIA_FORMATS.len(), 50);
        assert_eq!(VIDEO_EXTENSIONS.len(), MEDIA_FORMATS.len());
        let mut extensions = HashSet::new();
        let mut aliases = HashSet::new();
        for entry in MEDIA_FORMATS {
            assert!(extensions.insert(entry.extension));
            assert_eq!(entry.extension, entry.extension.to_ascii_lowercase());
            assert_eq!(media_format_for_extension(entry.extension), Some(entry));
            for alias in entry.container_aliases {
                assert!(aliases.insert(*alias), "duplicate container alias {alias}");
                assert_eq!(
                    preferred_video_extension_for_container(alias),
                    Some(entry.extension)
                );
            }
            for suffix in [entry.extension.to_owned(), entry.extension.to_uppercase()] {
                for junk in ["", "_", "\"", ". ", "'`_\t"] {
                    let path = format!("Release.{suffix}{junk}");
                    assert_eq!(
                        canonical_video_extension(Path::new(&path)),
                        Some(entry.extension)
                    );
                }
                for path in [
                    format!("Release.{suffix}.exe"),
                    format!("Release.{suffix}x"),
                    format!(".{suffix}"),
                ] {
                    assert_eq!(canonical_video_extension(Path::new(&path)), None);
                }
            }
        }
        assert_eq!(preferred_video_extension_for_container("OGM"), Some("ogm"));
        assert_eq!(preferred_video_extension_for_container("ogg"), Some("ogv"));
        assert_eq!(
            media_format_for_extension("strm").unwrap().handling,
            MediaHandling::StreamLink
        );
        assert_eq!(
            media_format_for_extension("iso").unwrap().handling,
            MediaHandling::DiscImage
        );
    }
}

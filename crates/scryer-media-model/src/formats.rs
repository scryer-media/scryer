//! Accepted media suffixes and their native container families.

use std::path::Path;

/// The container family a suffix belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)] // Each variant is the format it names.
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
    /// A raw H.264 elementary stream.
    H264,
    /// A raw MPEG-1 or MPEG-2 video elementary stream.
    MpegVideo,
    Svq3,
    /// An MPEG program stream, including DVD VOB files.
    MpegProgram,
    /// A text file holding a link to a stream.
    StreamLink,
    /// An optical disc image.
    DiscImage,
    /// A playlist of other files.
    Playlist,
    /// A file that accompanies media rather than holding it, such as a DVD
    /// `.ifo`.
    Companion,
    /// A recognized suffix whose format is not supported.
    Unsupported,
}

/// How a file with a given suffix is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaHandling {
    /// A media file the parser reads directly.
    Native,
    /// A text file holding a link to a stream.
    StreamLink,
    /// A disc image, read through its navigation structures.
    DiscImage,
    /// A playlist; not media itself.
    Playlist,
    /// A file that accompanies media; not media itself.
    Companion,
    /// Recognized but not supported.
    Unsupported,
}

/// What a probe can do with a file of a given suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaProbeCapability {
    /// The suffix names a format the parser reads.
    Native,
    /// The suffix says nothing reliable; the format is recognized from the
    /// file's content.
    ContentOnly,
    /// The format cannot be probed.
    Unsupported,
    /// Probing does not apply (playlists, links, disc images, companions).
    NotApplicable,
}

/// One recognized suffix and what it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaFormatRegistration {
    /// The suffix, lowercase and without the dot.
    pub extension: &'static str,
    /// The container family.
    pub format: MediaFormat,
    /// Container names for which this suffix is the preferred one.
    pub container_aliases: &'static [&'static str],
    /// How a file with this suffix is treated.
    pub handling: MediaHandling,
    /// What a probe can do with it.
    pub probe: MediaProbeCapability,
}

macro_rules! media_formats {
    ($(($extension:literal, $format:ident, $handling:ident, $probe:ident, [$($alias:literal),*])),* $(,)?) => {
        /// Every recognized suffix.
        pub const MEDIA_FORMATS: &[MediaFormatRegistration] = &[$(
            MediaFormatRegistration {
                extension: $extension,
                format: MediaFormat::$format,
                container_aliases: &[$($alias),*],
                handling: MediaHandling::$handling,
                probe: MediaProbeCapability::$probe,
            },
        )*];
        /// Every recognized suffix, in the order of [`MEDIA_FORMATS`].
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

/// The registration for a suffix, ignoring ASCII case; `None` when the
/// suffix is not recognized.
#[must_use]
pub fn media_format_for_extension(extension: &str) -> Option<&'static MediaFormatRegistration> {
    MEDIA_FORMATS
        .iter()
        .find(|entry| entry.extension.eq_ignore_ascii_case(extension))
}

/// The recognized suffix of a file name, lowercase; `None` when it has none.
///
/// Normalizes only the established trailing filename junk, never an inner suffix.
///
/// Every registered extension is ASCII, so a stem that is not valid UTF-8 is
/// still recognized: invalid sequences become replacement characters, which
/// keeps the stem non-empty and leaves the extension bytes untouched.
#[must_use]
pub fn canonical_video_extension(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_string_lossy();
    let name = name.trim().trim_end_matches(|ch: char| {
        ch.is_ascii_whitespace() || matches!(ch, '"' | '\'' | '`' | '_' | '.')
    });
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    media_format_for_extension(extension).map(|entry| entry.extension)
}

/// The registration for a path's suffix, after
/// [`canonical_video_extension`]; `None` when it is not recognized.
#[must_use]
pub fn media_format_for_path(path: &Path) -> Option<&'static MediaFormatRegistration> {
    media_format_for_extension(canonical_video_extension(path)?)
}

/// The preferred suffix for a container name such as `matroska`, ignoring
/// ASCII case; `None` when the name is not recognized.
#[must_use]
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

    #[cfg(unix)]
    #[test]
    fn non_utf8_stems_keep_their_video_extension() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let path = Path::new(OsStr::from_bytes(b"/library/Synthetic-\xFF.mkv"));
        assert_eq!(canonical_video_extension(path), Some("mkv"));
        let junk = Path::new(OsStr::from_bytes(b"/library/\xFE\xFF.MKV_ "));
        assert_eq!(canonical_video_extension(junk), Some("mkv"));
        let not_video = Path::new(OsStr::from_bytes(b"/library/Synthetic-\xFF.nfo"));
        assert_eq!(canonical_video_extension(not_video), None);
        let bare = Path::new(OsStr::from_bytes(b"/library/.mkv\xFF"));
        assert_eq!(canonical_video_extension(bare), None);
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

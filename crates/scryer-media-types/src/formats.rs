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

/// Whether a recognized suffix may represent its own library item.
///
/// The library list follows Sonarr's `MediaFileExtensions` video list. Suffixes
/// outside it stay registered for probing and classification only, and disc
/// companions are never library items on their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaLibraryRole {
    Standalone,
    ProbeOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaFormatRegistration {
    pub extension: &'static str,
    pub format: MediaFormat,
    pub container_aliases: &'static [&'static str],
    pub handling: MediaHandling,
    pub probe: MediaProbeCapability,
    pub library: MediaLibraryRole,
}

impl MediaFormatRegistration {
    /// A recognized video that can represent its own library item.
    pub fn is_standalone_video(&self) -> bool {
        self.library == MediaLibraryRole::Standalone && self.handling != MediaHandling::Companion
    }
}

macro_rules! media_formats {
    ($(($extension:literal, $format:ident, $handling:ident, $probe:ident, $library:ident, [$($alias:literal),*])),* $(,)?) => {
        pub const MEDIA_FORMATS: &[MediaFormatRegistration] = &[$(
            MediaFormatRegistration {
                extension: $extension,
                format: MediaFormat::$format,
                container_aliases: &[$($alias),*],
                handling: MediaHandling::$handling,
                probe: MediaProbeCapability::$probe,
                library: MediaLibraryRole::$library,
            },
        )*];
        pub const VIDEO_EXTENSIONS: &[&str] = &[$($extension),*];
    };
}

// Container aliases belong to the preferred suffix; filename aliases retain
// their own suffix when an existing file is imported.
media_formats! {
    ("mkv", Matroska, Native, Native, Standalone, ["matroska", "mkv"]),
    ("mp4", Mp4, Native, Native, Standalone, ["mp4"]),
    ("avi", Avi, Native, Native, Standalone, ["avi"]),
    ("wmv", Asf, Native, Native, Standalone, ["asf", "wmv"]),
    ("mov", QuickTime, Native, Native, Standalone, ["mov"]),
    ("m4v", Mp4, Native, Native, Standalone, ["m4v"]),
    ("ts", MpegTs, Native, Native, Standalone, ["mpegts", "ts"]),
    ("m2ts", MpegTs, Native, Native, Standalone, ["m2ts"]),
    ("webm", WebM, Native, Native, Standalone, ["webm"]),
    ("flv", Flv, Native, Native, Standalone, ["flv"]),
    ("ogv", Ogg, Native, Native, Standalone, ["ogg", "ogv"]),
    ("ogm", Ogm, Native, Native, Standalone, ["ogm"]),
    ("strm", StreamLink, StreamLink, NotApplicable, Standalone, []),
    ("mpg", MpegProgram, Native, Native, Standalone, ["mpeg", "mpg"]),
    ("mpeg", MpegProgram, Native, Native, Standalone, []),
    ("vob", MpegProgram, Native, Native, Standalone, ["vob"]),
    ("iso", DiscImage, DiscImage, NotApplicable, Standalone, []),
    ("3gp", Mp4, Native, Native, Standalone, []),
    ("asf", Asf, Native, Native, Standalone, []),
    ("asx", Playlist, Playlist, NotApplicable, Standalone, []),
    ("avc", H264, Native, Native, Standalone, ["h264", "avc"]),
    ("bin", DiscImage, DiscImage, NotApplicable, Standalone, []),
    ("bivx", Avi, Native, ContentOnly, Standalone, []),
    ("dat", MpegProgram, Native, ContentOnly, Standalone, []),
    ("divx", Avi, Native, ContentOnly, Standalone, []),
    ("dvr-ms", Asf, Native, Native, Standalone, []),
    ("dv", Dv, Native, Native, Standalone, ["dv"]),
    ("fli", Unsupported, Unsupported, Unsupported, Standalone, []),
    ("ifo", Companion, Companion, NotApplicable, ProbeOnly, []),
    ("img", DiscImage, DiscImage, NotApplicable, Standalone, []),
    ("m2t", MpegTs, Native, Native, ProbeOnly, []),
    ("m2v", MpegVideo, Native, Native, Standalone, ["mpegvideo", "m2v"]),
    ("m3u", Playlist, Playlist, NotApplicable, Standalone, []),
    ("mts", MpegTs, Native, Native, ProbeOnly, []),
    ("nrg", DiscImage, DiscImage, NotApplicable, Standalone, []),
    ("nsv", Unsupported, Unsupported, Unsupported, Standalone, []),
    ("nuv", Unsupported, Unsupported, Unsupported, Standalone, []),
    ("ogg", Ogg, Native, Native, ProbeOnly, []),
    ("ogx", Ogg, Native, Native, ProbeOnly, []),
    ("pva", Unsupported, Unsupported, Unsupported, Standalone, []),
    ("qt", QuickTime, Native, Native, Standalone, []),
    ("rm", RealMedia, Native, Native, Standalone, ["realmedia", "rm"]),
    ("rmvb", RealMedia, Native, Native, Standalone, []),
    ("svq3", Svq3, Native, ContentOnly, Standalone, []),
    ("ty", Unsupported, Unsupported, Unsupported, Standalone, []),
    ("viv", Unsupported, Unsupported, Unsupported, Standalone, []),
    ("vp3", Unsupported, Unsupported, Unsupported, Standalone, []),
    ("wpl", Playlist, Playlist, NotApplicable, Standalone, []),
    ("wtv", Unsupported, Unsupported, Unsupported, Standalone, []),
    ("xvid", Avi, Native, ContentOnly, Standalone, []),
}

pub fn media_format_for_extension(extension: &str) -> Option<&'static MediaFormatRegistration> {
    MEDIA_FORMATS
        .iter()
        .find(|entry| entry.extension.eq_ignore_ascii_case(extension))
}

/// Normalizes only the established trailing filename junk, never an inner suffix.
///
/// Every registered extension is ASCII, so a stem that is not valid UTF-8 is
/// still recognized: invalid sequences become replacement characters, which
/// keeps the stem non-empty and leaves the extension bytes untouched.
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

pub fn media_format_for_path(path: &Path) -> Option<&'static MediaFormatRegistration> {
    media_format_for_extension(canonical_video_extension(path)?)
}

/// Suffixes that can be standalone library items, in registry order.
pub fn standalone_video_extensions() -> impl Iterator<Item = &'static str> {
    MEDIA_FORMATS
        .iter()
        .filter(|entry| entry.is_standalone_video())
        .map(|entry| entry.extension)
}

/// A recognized video that can represent its own library item, rather than a
/// companion or a suffix registered for probing only.
pub fn is_standalone_video_path(path: &Path) -> bool {
    media_format_for_path(path).is_some_and(MediaFormatRegistration::is_standalone_video)
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
    fn standalone_library_suffixes_follow_sonarr_without_disc_companions() {
        // Sonarr src/NzbDrone.Core/MediaFiles/MediaFileExtensions.cs:9-66, minus
        // ".ifo": a DVD companion is never a library item on its own.
        let sonarr: HashSet<&str> = [
            "webm", "m4v", "3gp", "nsv", "ty", "strm", "rm", "rmvb", "m3u", "mov", "qt", "divx",
            "xvid", "bivx", "nrg", "pva", "wmv", "asf", "asx", "ogm", "ogv", "m2v", "avi", "bin",
            "dat", "dvr-ms", "mpg", "mpeg", "mp4", "avc", "vp3", "svq3", "nuv", "viv", "dv", "fli",
            "flv", "wpl", "img", "iso", "vob", "mkv", "ts", "wtv", "m2ts",
        ]
        .into_iter()
        .collect();
        let standalone: HashSet<&str> = standalone_video_extensions().collect();
        assert_eq!(standalone, sonarr);
        for probe_only in ["ifo", "ogg", "ogx", "m2t", "mts"] {
            assert!(media_format_for_extension(probe_only).is_some());
            assert!(!is_standalone_video_path(Path::new(&format!(
                "Release.{probe_only}"
            ))));
        }
        assert!(is_standalone_video_path(Path::new("Release.MKV_")));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_stems_keep_their_video_extension() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let path = Path::new(OsStr::from_bytes(b"/library/Synthetic-\xFF.mkv"));
        assert_eq!(canonical_video_extension(path), Some("mkv"));
        assert!(is_standalone_video_path(path));
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

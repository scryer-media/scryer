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
    MpegProgram,
    StreamLink,
    DiscImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaHandling {
    Native,
    StreamLink,
    DiscImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaFormatRegistration {
    pub extension: &'static str,
    pub format: MediaFormat,
    pub container_aliases: &'static [&'static str],
    pub handling: MediaHandling,
}

macro_rules! media_formats {
    ($(($extension:literal, $format:ident, $handling:ident, [$($alias:literal),*])),* $(,)?) => {
        pub const MEDIA_FORMATS: &[MediaFormatRegistration] = &[$(
            MediaFormatRegistration {
                extension: $extension,
                format: MediaFormat::$format,
                container_aliases: &[$($alias),*],
                handling: MediaHandling::$handling,
            },
        )*];
        pub const VIDEO_EXTENSIONS: &[&str] = &[$($extension),*];
    };
}

// Container aliases belong to the preferred suffix; filename aliases retain
// their own suffix when an existing file is imported.
media_formats! {
    ("mkv", Matroska, Native, ["matroska", "mkv"]),
    ("mp4", Mp4, Native, ["mp4"]),
    ("avi", Avi, Native, ["avi"]),
    ("wmv", Asf, Native, ["asf", "wmv"]),
    ("mov", QuickTime, Native, ["mov"]),
    ("m4v", Mp4, Native, ["m4v"]),
    ("ts", MpegTs, Native, ["mpegts", "ts"]),
    ("m2ts", MpegTs, Native, ["m2ts"]),
    ("webm", WebM, Native, ["webm"]),
    ("flv", Flv, Native, ["flv"]),
    ("ogv", Ogg, Native, ["ogg", "ogv"]),
    ("ogm", Ogm, Native, ["ogm"]),
    ("strm", StreamLink, StreamLink, []),
    ("mpg", MpegProgram, Native, ["mpeg", "mpg"]),
    ("mpeg", MpegProgram, Native, []),
    ("vob", MpegProgram, Native, ["vob"]),
    ("iso", DiscImage, DiscImage, []),
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
    fn registry_is_complete_and_unambiguous() {
        assert_eq!(MEDIA_FORMATS.len(), 17);
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

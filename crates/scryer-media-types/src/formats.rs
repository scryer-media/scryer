//! Accepted media suffixes, their native container families, and which of them
//! Scryer treats as standalone library items.

use std::path::Path;

pub use scryer_media_model::formats::*;

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

macro_rules! library_roles {
    ($(($extension:literal, $role:ident)),* $(,)?) => {
        /// The library role of every registered suffix, in registry order.
        pub const MEDIA_LIBRARY_ROLES: &[(&str, MediaLibraryRole)] = &[$(
            ($extension, MediaLibraryRole::$role),
        )*];
    };
}

library_roles! {
    ("mkv", Standalone),
    ("mp4", Standalone),
    ("avi", Standalone),
    ("wmv", Standalone),
    ("mov", Standalone),
    ("m4v", Standalone),
    ("ts", Standalone),
    ("m2ts", Standalone),
    ("webm", Standalone),
    ("flv", Standalone),
    ("ogv", Standalone),
    ("ogm", Standalone),
    ("strm", Standalone),
    ("mpg", Standalone),
    ("mpeg", Standalone),
    ("vob", Standalone),
    ("iso", Standalone),
    ("3gp", Standalone),
    ("asf", Standalone),
    ("asx", Standalone),
    ("avc", Standalone),
    ("bin", Standalone),
    ("bivx", Standalone),
    ("dat", Standalone),
    ("divx", Standalone),
    ("dvr-ms", Standalone),
    ("dv", Standalone),
    ("fli", Standalone),
    ("ifo", ProbeOnly),
    ("img", Standalone),
    ("m2t", ProbeOnly),
    ("m2v", Standalone),
    ("m3u", Standalone),
    ("mts", ProbeOnly),
    ("nrg", Standalone),
    ("nsv", Standalone),
    ("nuv", Standalone),
    ("ogg", ProbeOnly),
    ("ogx", ProbeOnly),
    ("pva", Standalone),
    ("qt", Standalone),
    ("rm", Standalone),
    ("rmvb", Standalone),
    ("svq3", Standalone),
    ("ty", Standalone),
    ("viv", Standalone),
    ("vp3", Standalone),
    ("wpl", Standalone),
    ("wtv", Standalone),
    ("xvid", Standalone),
}

/// The library role of a registered suffix, matched against the registry's
/// canonical lowercase spelling.
pub fn library_role(registration: &MediaFormatRegistration) -> Option<MediaLibraryRole> {
    MEDIA_LIBRARY_ROLES
        .iter()
        .find(|(extension, _)| *extension == registration.extension)
        .map(|(_, role)| *role)
}

/// A recognized video that can represent its own library item.
pub fn is_standalone_video_format(registration: &MediaFormatRegistration) -> bool {
    library_role(registration) == Some(MediaLibraryRole::Standalone)
        && registration.handling != MediaHandling::Companion
}

/// Suffixes that can be standalone library items, in registry order.
pub fn standalone_video_extensions() -> impl Iterator<Item = &'static str> {
    MEDIA_FORMATS
        .iter()
        .filter(|entry| is_standalone_video_format(entry))
        .map(|entry| entry.extension)
}

/// A recognized video that can represent its own library item, rather than a
/// companion or a suffix registered for probing only.
pub fn is_standalone_video_path(path: &Path) -> bool {
    media_format_for_path(path).is_some_and(is_standalone_video_format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_registered_extension_has_exactly_one_library_role() {
        assert_eq!(MEDIA_LIBRARY_ROLES.len(), MEDIA_FORMATS.len());
        for entry in MEDIA_FORMATS {
            assert_eq!(
                MEDIA_LIBRARY_ROLES
                    .iter()
                    .filter(|(extension, _)| *extension == entry.extension)
                    .count(),
                1,
                "{}",
                entry.extension
            );
        }
        for (extension, _) in MEDIA_LIBRARY_ROLES {
            assert!(
                MEDIA_FORMATS
                    .iter()
                    .any(|entry| entry.extension == *extension),
                "{extension} has a library role but no registration"
            );
        }
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
    fn non_utf8_stems_keep_their_standalone_role() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let path = Path::new(OsStr::from_bytes(b"/library/Synthetic-\xFF.mkv"));
        assert!(is_standalone_video_path(path));
    }
}

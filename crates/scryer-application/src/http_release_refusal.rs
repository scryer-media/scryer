//! Stable names for the ways an HTTP-family release cannot be grabbed or
//! downloaded.
//!
//! Each travels beside the English sentence as `extensions.reason` on a
//! `VALIDATION_ERROR`, so a client can tell them apart and say them in the
//! operator's language. A name states the condition, never the wording, and
//! is API once released.

/// The grab carries no HTTP release description: the listing that offered
/// it has expired, and the release must be found by a new search.
pub const DETAILS_UNAVAILABLE: &str = "HTTP_RELEASE_DETAILS_UNAVAILABLE";
/// The provider reports the release as a live stream.
pub const LIVE: &str = "HTTP_RELEASE_LIVE";
/// The provider reports the release as announced but not yet available.
pub const UPCOMING: &str = "HTTP_RELEASE_UPCOMING";
/// The provider reports the release as removed.
pub const REMOVED: &str = "HTTP_RELEASE_REMOVED";
/// The release offers no transport the HTTP downloader fetches.
pub const UNSUPPORTED_TRANSPORT: &str = "HTTP_RELEASE_UNSUPPORTED_TRANSPORT";
/// The release offers only separate audio and video tracks, which would
/// need muxing.
pub const SEPARATE_TRACKS: &str = "HTTP_RELEASE_SEPARATE_TRACKS";
/// The selection holds no direct media file.
pub const NO_MEDIA_RESOURCE: &str = "HTTP_RELEASE_NO_MEDIA_RESOURCE";
/// Subtitle sidecars are required and none in a wanted language is offered.
pub const NO_SUBTITLE_RESOURCE: &str = "HTTP_RELEASE_NO_SUBTITLE_RESOURCE";
/// No complete resource satisfies the required audio languages.
pub const NO_AUDIO_RESOURCE: &str = "HTTP_RELEASE_NO_AUDIO_RESOURCE";
/// The resource requires provider authentication the client cannot supply.
pub const UNSUPPORTED_AUTH: &str = "HTTP_RELEASE_UNSUPPORTED_AUTH";
/// An opaque payload could not be identified as a supported video container.
pub const UNKNOWN_CONTAINER: &str = "HTTP_RELEASE_UNKNOWN_CONTAINER";
/// A destination is already occupied by a different file.
pub const DESTINATION_EXISTS: &str = "HTTP_RELEASE_DESTINATION_EXISTS";
/// The filesystem cannot publish the partial without replacing a destination.
pub const PUBLICATION_FAILED: &str = "HTTP_RELEASE_PUBLICATION_FAILED";
/// An existing output lacks matching durable publication evidence.
pub const RECOVERY_UNVERIFIED: &str = "HTTP_RELEASE_RECOVERY_UNVERIFIED";

/// Every reason an HTTP release refusal can name. Clients key translations
/// on these, so adding, renaming, or removing one must be deliberate.
pub const ALL: &[&str] = &[
    DETAILS_UNAVAILABLE,
    LIVE,
    UPCOMING,
    REMOVED,
    UNSUPPORTED_TRANSPORT,
    SEPARATE_TRACKS,
    NO_MEDIA_RESOURCE,
    NO_SUBTITLE_RESOURCE,
    NO_AUDIO_RESOURCE,
    UNSUPPORTED_AUTH,
    UNKNOWN_CONTAINER,
    DESTINATION_EXISTS,
    PUBLICATION_FAILED,
    RECOVERY_UNVERIFIED,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_reason_is_a_distinct_http_release_name_in_screaming_snake() {
        let distinct = ALL.iter().collect::<BTreeSet<_>>();
        assert_eq!(distinct.len(), ALL.len(), "a reason is listed twice");
        for reason in ALL {
            assert!(reason.starts_with("HTTP_RELEASE_"), "{reason}");
            assert!(
                reason
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "{reason}"
            );
            assert!(!reason.ends_with('_') && !reason.contains("__"), "{reason}");
        }
    }
}

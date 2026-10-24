//! Stable names for the ways an indexer's sign-in challenge cannot be shown.
//!
//! Each travels beside the English sentence as `extensions.reason` on a
//! `VALIDATION_ERROR`, so the settings form can say it in the operator's
//! language. A failure the plugin itself reports keeps the plugin's own code
//! instead. A name states the condition, never the wording, and is API once
//! released.

/// The plugin's challenge reply could not be read.
pub const MALFORMED: &str = "INDEXER_LOGIN_CHALLENGE_MALFORMED";
/// The plugin asked for a kind of challenge the host cannot show.
pub const KIND_UNSUPPORTED: &str = "INDEXER_LOGIN_CHALLENGE_KIND_UNSUPPORTED";
/// The challenge image is not a raster image type the host shows.
pub const IMAGE_TYPE_UNSUPPORTED: &str = "INDEXER_LOGIN_CHALLENGE_IMAGE_TYPE_UNSUPPORTED";
/// The challenge image is larger than the host shows.
pub const IMAGE_TOO_LARGE: &str = "INDEXER_LOGIN_CHALLENGE_IMAGE_TOO_LARGE";

/// Every reason a sign-in challenge refusal can name. Clients key
/// translations on these, so adding, renaming, or removing one must be
/// deliberate.
pub const ALL: &[&str] = &[
    MALFORMED,
    KIND_UNSUPPORTED,
    IMAGE_TYPE_UNSUPPORTED,
    IMAGE_TOO_LARGE,
];

/// Largest challenge image the host shows, decoded.
pub const MAX_IMAGE_BYTES: usize = 1024 * 1024;

/// Media types the host shows a challenge image as. SVG is left out: it is a
/// document, not a picture.
pub const IMAGE_CONTENT_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/bmp",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_reason_is_a_distinct_login_challenge_name_in_screaming_snake() {
        let distinct = ALL.iter().collect::<BTreeSet<_>>();
        assert_eq!(distinct.len(), ALL.len(), "a reason is listed twice");
        for reason in ALL {
            assert!(reason.starts_with("INDEXER_LOGIN_CHALLENGE_"), "{reason}");
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

//! Stable names for the ways TorBox refuses, or cannot yet take, a request.
//!
//! Each travels beside the English sentence as `extensions.reason`: on a
//! `VALIDATION_ERROR` for a refusal the operator acts on, on a
//! `NOT_FOUND` for an item TorBox no longer holds, and on a
//! `TEMPORARY_UNAVAILABLE` for a condition that clears with time. A client can
//! tell them apart and say them in the operator's language. A name states the
//! condition, never the wording, and is API once released.

/// TorBox did not accept the API token: it is missing, wrong, or revoked.
pub const TOKEN_REJECTED: &str = "TORBOX_TOKEN_REJECTED";
/// The account's TorBox plan does not include the requested feature.
pub const PLAN_RESTRICTED: &str = "TORBOX_PLAN_RESTRICTED";
/// The account already has as many active downloads as its plan allows.
pub const ACTIVE_LIMIT: &str = "TORBOX_ACTIVE_LIMIT";
/// TorBox asks the account to wait before adding another download.
pub const COOLDOWN_LIMIT: &str = "TORBOX_COOLDOWN_LIMIT";
/// The account has used its monthly allowance.
pub const MONTHLY_LIMIT: &str = "TORBOX_MONTHLY_LIMIT";
/// This instance has sent as many creates of this kind as TorBox allows in
/// an hour.
pub const HOURLY_LIMIT: &str = "TORBOX_HOURLY_LIMIT";
/// The download is larger than the account's plan allows.
pub const DOWNLOAD_TOO_LARGE: &str = "TORBOX_DOWNLOAD_TOO_LARGE";
/// The download was only to be added if TorBox already had it cached, and it
/// did not.
pub const NOT_CACHED: &str = "TORBOX_NOT_CACHED";
/// TorBox could not read the torrent or magnet it was given.
pub const INVALID_TORRENT: &str = "TORBOX_INVALID_TORRENT";
/// TorBox could not read the NZB it was given.
pub const INVALID_NZB: &str = "TORBOX_INVALID_NZB";
/// The account already holds this download.
pub const DUPLICATE_ITEM: &str = "TORBOX_DUPLICATE_ITEM";
/// TorBox holds no item with the given id.
pub const ITEM_NOT_FOUND: &str = "TORBOX_ITEM_NOT_FOUND";
/// The link TorBox was given to fetch is no longer reachable.
pub const LINK_OFFLINE: &str = "TORBOX_LINK_OFFLINE";
/// TorBox asked for fewer requests.
pub const RATE_LIMITED: &str = "TORBOX_RATE_LIMITED";
/// TorBox could not serve the request because of a fault on its side.
pub const UNAVAILABLE: &str = "TORBOX_UNAVAILABLE";
/// TorBox flagged a file of the download as infected, so it is not fetched.
pub const FILE_INFECTED: &str = "TORBOX_FILE_INFECTED";
/// TorBox holds a file of the download only inside a zip, which Scryer does
/// not fetch.
pub const FILE_ZIPPED: &str = "TORBOX_FILE_ZIPPED";
/// The finished download holds no video file to fetch.
pub const NO_VIDEO_FILE: &str = "TORBOX_NO_VIDEO_FILE";
/// TorBox reports that it could not finish the download.
pub const REMOTE_FAILED: &str = "TORBOX_REMOTE_FAILED";
/// The download is no longer in the TorBox account.
pub const ITEM_MISSING: &str = "TORBOX_ITEM_MISSING";
/// A torrent has stopped making progress because no peer has its data.
pub const STALLED: &str = "TORBOX_STALLED";
/// The file paths TorBox reports cannot be laid out safely on disk.
pub const FILE_PATHS_UNUSABLE: &str = "TORBOX_FILE_PATHS_UNUSABLE";
/// The request carries no torrent, magnet, or NZB this client can send.
pub const SOURCE_MISSING: &str = "TORBOX_SOURCE_MISSING";
/// TorBox offers no way to do this to a download in its current phase.
pub const ACTION_UNSUPPORTED: &str = "TORBOX_ACTION_UNSUPPORTED";
/// The client has no download with the given id.
pub const JOB_NOT_FOUND: &str = "TORBOX_JOB_NOT_FOUND";
/// The client configuration has no API token.
pub const TOKEN_REQUIRED: &str = "TORBOX_TOKEN_REQUIRED";

/// Every reason a TorBox refusal can name. Clients key translations on these,
/// so adding, renaming, or removing one must be deliberate.
pub const ALL: &[&str] = &[
    TOKEN_REJECTED,
    PLAN_RESTRICTED,
    ACTIVE_LIMIT,
    COOLDOWN_LIMIT,
    MONTHLY_LIMIT,
    HOURLY_LIMIT,
    DOWNLOAD_TOO_LARGE,
    NOT_CACHED,
    INVALID_TORRENT,
    INVALID_NZB,
    DUPLICATE_ITEM,
    ITEM_NOT_FOUND,
    LINK_OFFLINE,
    RATE_LIMITED,
    UNAVAILABLE,
    FILE_INFECTED,
    FILE_ZIPPED,
    NO_VIDEO_FILE,
    REMOTE_FAILED,
    ITEM_MISSING,
    STALLED,
    FILE_PATHS_UNUSABLE,
    SOURCE_MISSING,
    ACTION_UNSUPPORTED,
    JOB_NOT_FOUND,
    TOKEN_REQUIRED,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_reason_is_a_distinct_torbox_name_in_screaming_snake() {
        let distinct = ALL.iter().collect::<BTreeSet<_>>();
        assert_eq!(distinct.len(), ALL.len(), "a reason is listed twice");
        for reason in ALL {
            assert!(reason.starts_with("TORBOX_"), "{reason}");
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

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

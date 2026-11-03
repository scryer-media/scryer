//! Stable names for why a watch-history sweep of one media-server connection
//! was incomplete.
//!
//! Each one is stored beside the English sentence in
//! `media_server_signal_sync_state.last_error_code` and served as
//! `lastErrorCode`, so a client explains the condition in its own language
//! instead of showing or matching the sentence. A name states the condition,
//! never the wording, and is API once released: reword a sentence freely, but
//! do not rename or reuse its code.
//!
//! A row written before codes were stored has no code; it is read through its
//! sentence alone.

/// The connection has no stored credential to read with.
pub const CREDENTIAL_MISSING: &str = "WATCH_SYNC_CREDENTIAL_MISSING";
/// The connection's provider has no linked-account identity to read for.
pub const PROVIDER_UNSUPPORTED: &str = "WATCH_SYNC_PROVIDER_UNSUPPORTED";
/// The stored server address cannot be used to build a request.
pub const BASE_URL_INVALID: &str = "WATCH_SYNC_BASE_URL_INVALID";
/// A Plex connection names no server, so there is no library to read.
pub const PLEX_SERVER_NOT_SELECTED: &str = "WATCH_SYNC_PLEX_SERVER_NOT_SELECTED";
/// The selected Plex server is not in the account's resources, or lists no
/// address to reach it at.
pub const PLEX_SERVER_UNREACHABLE: &str = "WATCH_SYNC_PLEX_SERVER_UNREACHABLE";
/// A linked Plex account's id is not an account id the server can filter on.
pub const PLEX_ACCOUNT_ID_INVALID: &str = "WATCH_SYNC_PLEX_ACCOUNT_ID_INVALID";
/// A linked account has no provider user id to read for.
pub const PARTICIPANT_ID_MISSING: &str = "WATCH_SYNC_PARTICIPANT_ID_MISSING";
/// The server refused the credential.
pub const ACCESS_DENIED: &str = "WATCH_SYNC_ACCESS_DENIED";
/// The server answered with another non-success status.
pub const REQUEST_REJECTED: &str = "WATCH_SYNC_REQUEST_REJECTED";
/// The request got no answer: timeout, connection failure, or similar.
pub const REQUEST_FAILED: &str = "WATCH_SYNC_REQUEST_FAILED";
/// The server answered, but not with the shape a listing has.
pub const RESPONSE_UNREADABLE: &str = "WATCH_SYNC_RESPONSE_UNREADABLE";
/// Played items came back that could not be read as a movie or episode.
pub const ITEMS_UNREADABLE: &str = "WATCH_SYNC_ITEMS_UNREADABLE";
/// The server did not page as asked: it answered a later page from elsewhere,
/// or stopped before the total it reported.
pub const PAGING_NOT_HONOURED: &str = "WATCH_SYNC_PAGING_NOT_HONOURED";
/// The watched-set listing returned unwatched items, so its filter was not
/// applied and the listing cannot be trusted as a played set.
pub const FILTER_NOT_HONOURED: &str = "WATCH_SYNC_FILTER_NOT_HONOURED";
/// Played movies could not be matched to a Scryer title and nothing showed them
/// to be outside Scryer (no usable external id, or a file inside a Scryer
/// library), so a played Scryer title may be among them.
pub const UNMAPPED_PLAYED_MOVIES: &str = "WATCH_SYNC_UNMAPPED_PLAYED_MOVIES";
/// Scryer's own store could not read or write this connection's signals.
pub const STORE_FAILED: &str = "WATCH_SYNC_STORE_FAILED";
/// The signal source is not configured in this build.
pub const SOURCE_UNAVAILABLE: &str = "WATCH_SYNC_SOURCE_UNAVAILABLE";

/// Every code, for the distinctness test and for clients that enumerate them.
pub const ALL: &[&str] = &[
    CREDENTIAL_MISSING,
    PROVIDER_UNSUPPORTED,
    BASE_URL_INVALID,
    PLEX_SERVER_NOT_SELECTED,
    PLEX_SERVER_UNREACHABLE,
    PLEX_ACCOUNT_ID_INVALID,
    PARTICIPANT_ID_MISSING,
    ACCESS_DENIED,
    REQUEST_REJECTED,
    REQUEST_FAILED,
    RESPONSE_UNREADABLE,
    ITEMS_UNREADABLE,
    PAGING_NOT_HONOURED,
    FILTER_NOT_HONOURED,
    UNMAPPED_PLAYED_MOVIES,
    STORE_FAILED,
    SOURCE_UNAVAILABLE,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_reason_is_a_distinct_watch_sync_name_in_screaming_snake() {
        let distinct = ALL.iter().collect::<BTreeSet<_>>();
        assert_eq!(distinct.len(), ALL.len(), "a reason is listed twice");
        for reason in ALL {
            assert!(reason.starts_with("WATCH_SYNC_"), "{reason}");
            assert!(
                reason
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "{reason}"
            );
            assert!(!reason.ends_with('_') && !reason.contains("__"), "{reason}");
        }
    }

    /// The web client keeps a literal copy of this list in
    /// `apps/scryer-web/lib/utils/watch-sync-reason.test.ts`; adding a reason
    /// here means adding it, and its translation, there too.
    #[test]
    fn the_reason_list_is_the_one_the_web_client_mirrors() {
        assert_eq!(ALL.len(), 17);
    }
}

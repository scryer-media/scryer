//! TorBox's `download_state` strings, read once into a typed state.
//!
//! TorBox reports the state as free text that differs between torrents
//! (qBittorrent-style words such as `metaDL`, `stalled (no seeds)` or
//! `uploading`) and usenet downloads (`verifying`, `repairing`,
//! `extracting`). Everything downstream decides on [`TorboxRemoteState`],
//! never on the text.

/// Where TorBox says an item stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TorboxRemoteState {
    /// Waiting for a slot, fetching metadata, or checking data.
    Queued,
    Downloading,
    /// Downloading without peers or sources to fetch from.
    Stalled,
    Paused,
    /// Finished and uploading to peers.
    Seeding,
    /// Finished and not uploading.
    Completed,
    /// Usenet post-processing: verifying, repairing or extracting.
    Processing,
    Failed,
    /// A state this build does not know.
    Unknown,
}

impl TorboxRemoteState {
    pub(crate) fn parse(raw: Option<&str>) -> Self {
        let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
            return Self::Unknown;
        };
        let state = raw.to_ascii_lowercase();
        let state = state.as_str();
        if state.contains("error") || state.contains("fail") {
            return Self::Failed;
        }
        // `stalledUP` is a finished torrent with nobody to upload to.
        if state.starts_with("stalledup") || state.starts_with("stalled up") {
            return Self::Seeding;
        }
        if state.starts_with("stalled") {
            return Self::Stalled;
        }
        if state.starts_with("paused") {
            return Self::Paused;
        }
        if state.starts_with("uploading") || state.starts_with("seeding") {
            return Self::Seeding;
        }
        if matches!(state, "completed" | "cached") || state.starts_with("completed") {
            return Self::Completed;
        }
        if state.starts_with("downloading") || state.starts_with("forceddl") {
            return Self::Downloading;
        }
        if state.starts_with("queued")
            || state.starts_with("metadl")
            || state.starts_with("checking")
            || state.starts_with("allocating")
            || state.starts_with("moving")
        {
            return Self::Queued;
        }
        if state.starts_with("verifying")
            || state.starts_with("repairing")
            || state.starts_with("extracting")
            || state.starts_with("processing")
        {
            return Self::Processing;
        }
        Self::Unknown
    }

    /// TorBox is uploading the item to peers.
    pub(crate) fn is_seeding(self) -> bool {
        self == Self::Seeding
    }
}

#[cfg(test)]
mod tests {
    use super::TorboxRemoteState as S;

    #[test]
    fn torbox_states_parse_into_typed_states() {
        for (raw, expected) in [
            (Some("downloading"), S::Downloading),
            (Some("metaDL"), S::Queued),
            (Some("checkingResumeData"), S::Queued),
            (Some("queued"), S::Queued),
            (Some("stalled (no seeds)"), S::Stalled),
            (Some("stalledUP"), S::Seeding),
            (Some("uploading"), S::Seeding),
            (Some("seeding"), S::Seeding),
            (Some("paused"), S::Paused),
            (Some("completed"), S::Completed),
            (Some("cached"), S::Completed),
            (Some("verifying"), S::Processing),
            (Some("repairing"), S::Processing),
            (Some("extracting"), S::Processing),
            (Some("error"), S::Failed),
            (Some("failed (missing articles)"), S::Failed),
            (Some("  "), S::Unknown),
            (Some("somethingNew"), S::Unknown),
            (None, S::Unknown),
        ] {
            assert_eq!(S::parse(raw), expected, "{raw:?}");
        }
    }
}

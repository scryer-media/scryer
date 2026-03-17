use crate::ParsedReleaseMetadata;

/// Build a dedup key from parsed release metadata for cross-indexer deduplication.
///
/// Two results with the same key are considered the same release from different
/// indexers. Returns an empty string if there's not enough metadata to build a
/// reliable key (in which case the result should be kept).
pub fn build_release_dedup_key(parsed: &ParsedReleaseMetadata) -> String {
    let group = parsed
        .release_group
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    if group.is_empty() {
        return String::new();
    }

    let quality = parsed.quality.as_deref().unwrap_or("").to_ascii_lowercase();
    let codec = parsed
        .video_codec
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();

    let episode_key = if let Some(ref ep) = parsed.episode {
        if let Some(season) = ep.season {
            let eps = ep
                .episode_numbers
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!("s{season}e{eps}")
        } else if let Some(abs) = ep.absolute_episode {
            format!("abs{abs}")
        } else {
            return String::new();
        }
    } else {
        return String::new();
    };

    let proper = if parsed.is_repack {
        "repack"
    } else if parsed.is_proper_upload {
        "proper"
    } else {
        ""
    };

    let dual = if parsed.is_dual_audio { "dual" } else { "" };
    let edition = parsed.edition.as_deref().unwrap_or("").to_ascii_lowercase();

    format!("{group}|{episode_key}|{quality}|{codec}|{proper}|{dual}|{edition}")
}

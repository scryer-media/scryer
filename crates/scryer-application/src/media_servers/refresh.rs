//! FR-088: hand a set of changed folders to every connected media server.
//!
//! # Which servers get told
//!
//! Every **enabled** connection, not just the ones already holding a playback
//! mapping for the moved titles. `media_server_playback_items` maps a Scryer
//! entity to a *provider item id the server has already scanned*, which is
//! precisely the state a move invalidates: a title landing in a folder the
//! server has never indexed has no mapping, and that is the case most in need
//! of the notification. Selecting connections from the mapping table would
//! therefore skip exactly the servers that most need to hear about the move.
//!
//! Fanning out to enabled connections is also what the playback reconciliation
//! loop already does (`reconcile_enabled_connections`), and each connection
//! filters for itself: a Plex server with no section covering the path refreshes
//! nothing, and an Emby/Jellyfin server outside the path simply finds nothing
//! there. The cost of telling a server about a folder it does not own is one
//! cheap request.
//!
//! # Path mappings
//!
//! `media_server_connections.path_mappings` describes where Scryer's paths
//! appear inside the media server's container or host. Until FR-088 nothing in
//! the backend consumed them — they were stored and shown, and the playback
//! mapping worked on provider ids rather than paths — so this is their first
//! load-bearing use. An unmapped path is sent through unchanged, which is
//! correct for the (common) deployment where Scryer and the server see the same
//! filesystem.
//!
//! # Failure
//!
//! Per connection, logged and dropped: one unreachable server must not stop the
//! others, and none of them can affect the operation that triggered this — it
//! settled before this ran.

use super::*;

use scryer_domain::MediaServerPathMapping;

impl AppUseCase {
    /// Ask every enabled media server to re-read `folders` (FR-088).
    ///
    /// `folders` are Scryer paths; each connection's own mappings translate them
    /// before the request goes out.
    pub async fn refresh_media_server_folders(
        &self,
        operation_id: &str,
        folders: &[String],
    ) -> AppResult<()> {
        if folders.is_empty() {
            return Ok(());
        }

        let connections = self
            .services
            .integrations
            .media_server_connections
            .list(None)
            .await?;

        for connection in connections
            .into_iter()
            .filter(|connection| connection.enabled)
        {
            let paths = folders
                .iter()
                .map(|folder| map_media_server_path(folder, &connection.path_mappings))
                .collect::<Vec<_>>();
            match self
                .services
                .integrations
                .external_identity_verifier
                .refresh_media_server_paths(&connection, &paths)
                .await
            {
                Ok(()) => tracing::info!(
                    operation_id,
                    connection_id = %connection.id,
                    provider = ?connection.provider,
                    folders = paths.len(),
                    "asked a media server to re-read the folders a location operation changed"
                ),
                Err(error) => tracing::warn!(
                    operation_id,
                    connection_id = %connection.id,
                    provider = ?connection.provider,
                    error = %error,
                    "could not ask a media server to re-read a moved folder; it re-reads on its own schedule"
                ),
            }
        }
        Ok(())
    }
}

/// Translate one Scryer path into a connection's namespace.
///
/// The most specific mapping wins, matching is on whole path components, and a
/// path no mapping covers is returned unchanged. Both separators are accepted on
/// the Scryer side because a Windows deployment stores Windows paths; the
/// mapped result keeps whichever separator the mapping's destination uses.
pub fn map_media_server_path(path: &str, mappings: &[MediaServerPathMapping]) -> String {
    let path = path.trim();
    let mut best: Option<(&MediaServerPathMapping, &str)> = None;
    for mapping in mappings {
        let source = mapping.source_path.trim().trim_end_matches(['/', '\\']);
        if source.is_empty() {
            continue;
        }
        let Some(remainder) = strip_path_prefix(path, source) else {
            continue;
        };
        if best.is_none_or(|(current, _)| {
            source.len()
                > current
                    .source_path
                    .trim()
                    .trim_end_matches(['/', '\\'])
                    .len()
        }) {
            best = Some((mapping, remainder));
        }
    }

    let Some((mapping, remainder)) = best else {
        return path.to_string();
    };
    let destination = mapping
        .destination_path
        .trim()
        .trim_end_matches(['/', '\\'])
        .to_string();
    if remainder.is_empty() {
        return destination;
    }
    let separator = if destination.contains('\\') && !destination.contains('/') {
        '\\'
    } else {
        '/'
    };
    let remainder = remainder.replace('\\', &separator.to_string());
    format!("{destination}{separator}{remainder}")
}

/// The part of `path` below `prefix`, when `prefix` is a whole-component prefix
/// of it. Case-sensitive: a mapping is operator-entered configuration, and
/// silently matching a differently-cased path would be a guess.
fn strip_path_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = path.strip_prefix(prefix)?;
    if rest.is_empty() {
        return Some("");
    }
    let trimmed = rest.trim_start_matches(['/', '\\']);
    (trimmed.len() < rest.len()).then_some(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(source: &str, destination: &str, sort_order: i64) -> MediaServerPathMapping {
        MediaServerPathMapping {
            source_path: source.into(),
            destination_path: destination.into(),
            sort_order,
        }
    }

    #[test]
    fn an_unmapped_path_is_sent_unchanged() {
        assert_eq!(
            map_media_server_path("/media/tv/Some Show", &[]),
            "/media/tv/Some Show"
        );
    }

    #[test]
    fn a_matching_mapping_rebases_the_path() {
        let mappings = vec![mapping("/media", "/data", 0)];
        assert_eq!(
            map_media_server_path("/media/tv/Some Show", &mappings),
            "/data/tv/Some Show"
        );
    }

    #[test]
    fn the_most_specific_mapping_wins() {
        let mappings = vec![
            mapping("/media", "/data", 0),
            mapping("/media/tv", "/series", 1),
        ];
        assert_eq!(
            map_media_server_path("/media/tv/Some Show", &mappings),
            "/series/Some Show"
        );
    }

    #[test]
    fn a_partial_component_is_not_a_prefix() {
        let mappings = vec![mapping("/media/tv", "/series", 0)];
        assert_eq!(
            map_media_server_path("/media/tv-archive/Some Show", &mappings),
            "/media/tv-archive/Some Show"
        );
    }

    #[test]
    fn the_mapped_root_itself_maps_to_the_destination_root() {
        let mappings = vec![mapping("/media/tv/", "/series/", 0)];
        assert_eq!(map_media_server_path("/media/tv", &mappings), "/series");
    }

    #[test]
    fn a_windows_source_maps_onto_a_posix_destination() {
        let mappings = vec![mapping("D:\\Media\\TV", "/series", 0)];
        assert_eq!(
            map_media_server_path("D:\\Media\\TV\\Some Show", &mappings),
            "/series/Some Show"
        );
    }

    #[test]
    fn a_windows_destination_keeps_backslashes() {
        let mappings = vec![mapping("/media/tv", "D:\\Series", 0)];
        assert_eq!(
            map_media_server_path("/media/tv/Some Show", &mappings),
            "D:\\Series\\Some Show"
        );
    }
}

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::Utc;
use scryer_domain::{
    DomainEventFilter, DomainEventPayload, DomainEventType, ExternalId, MediaFacet,
    MediaServerConnection, MediaServerPlaybackEntityKind, MediaServerPlaybackItem, Title,
};
use tokio::time::Instant;

use super::*;
use crate::{MediaServerCatalogItem, MediaServerCatalogItemKind};

const PLAYBACK_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const PLAYBACK_EVENT_SUBSCRIBER: &str = "media-server-playback-refresh";
const PLAYBACK_EVENT_BATCH_LIMIT: usize = 100;
const PLAYBACK_INCREMENTAL_DELAYS: [Duration; 3] = [
    Duration::from_secs(2 * 60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(20 * 60),
];

type PlaybackEntityKey = (MediaServerPlaybackEntityKind, String);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PlaybackRefreshKey {
    connection_id: String,
    entity_kind: MediaServerPlaybackEntityKind,
    entity_id: String,
}

#[derive(Clone, Debug)]
struct PendingPlaybackRefresh {
    first_seen_at: Instant,
    attempt: usize,
}

type PlaybackRefreshQueue = HashMap<PlaybackRefreshKey, PendingPlaybackRefresh>;

/// Reconcile all mappings on startup and daily, and poll recent provider items for
/// imports after the provider has had time to notice the new file.
pub async fn start_background_media_server_playback_reconciliation_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    let repo = app.services.events.domain_events.clone();
    let mut event_rx = app.runtime.events.domain_event_broadcast.subscribe();
    let mut full_reconciliation = tokio::time::interval(PLAYBACK_RECONCILIATION_INTERVAL);
    let mut queue = PlaybackRefreshQueue::new();
    let mut should_poll_events = true;
    let started_at = Utc::now();
    let mut last_event_sequence = match repo.get_subscriber_offset(PLAYBACK_EVENT_SUBSCRIBER).await
    {
        Ok(sequence) => sequence,
        Err(error) => {
            tracing::warn!(error = %error, "failed to load media-server playback event offset; starting at 0");
            0
        }
    };

    loop {
        if should_poll_events {
            match enqueue_import_completed_events(&app, &mut queue, last_event_sequence, started_at)
                .await
            {
                Ok(sequence) => {
                    last_event_sequence = sequence;
                    should_poll_events = false;
                }
                Err(error) => {
                    tracing::warn!(error = %error, "failed to queue imported media for playback refresh");
                }
            }
        }

        let next_refresh_at = next_incremental_refresh_at(&queue);
        tokio::select! {
            _ = token.cancelled() => return,
            _ = full_reconciliation.tick() => reconcile_enabled_connections(&app).await,
            _ = async {
                match next_refresh_at {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            } => process_due_incremental_refreshes(&app, &mut queue).await,
            result = event_rx.recv() => match result {
                Ok(high_water_sequence) => should_poll_events = high_water_sequence > last_event_sequence,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "media-server playback event receiver lagged; replaying persisted imports");
                    should_poll_events = true;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            },
            _ = tokio::time::sleep(Duration::from_secs(30)), if should_poll_events => {},
        }
    }
}

async fn reconcile_enabled_connections(app: &AppUseCase) {
    let connections = match app
        .services
        .integrations
        .media_server_connections
        .list(None)
        .await
    {
        Ok(connections) => connections,
        Err(error) => {
            tracing::warn!(error = %error, "failed to list media servers for playback reconciliation");
            return;
        }
    };
    for connection in connections
        .into_iter()
        .filter(|connection| connection.enabled)
    {
        if let Err(error) = app
            .refresh_media_server_playback_mappings_for_connection(&connection)
            .await
        {
            tracing::warn!(connection_id = connection.id.as_str(), error = %error, "media server playback reconciliation failed");
        }
    }
}

async fn enqueue_import_completed_events(
    app: &AppUseCase,
    queue: &mut PlaybackRefreshQueue,
    mut after_sequence: i64,
    started_at: chrono::DateTime<Utc>,
) -> AppResult<i64> {
    let repo = app.services.events.domain_events.clone();
    let connections = app
        .services
        .integrations
        .media_server_connections
        .list(None)
        .await?
        .into_iter()
        .filter(|connection| connection.enabled)
        .collect::<Vec<_>>();

    loop {
        let events = repo
            .list(&DomainEventFilter {
                event_types: Some(vec![DomainEventType::ImportCompleted]),
                after_sequence: Some(after_sequence),
                limit: PLAYBACK_EVENT_BATCH_LIMIT,
                ..DomainEventFilter::default()
            })
            .await?;
        if events.is_empty() {
            return Ok(after_sequence);
        }
        for event in events {
            if event.occurred_at >= started_at
                && let (Some(title_id), DomainEventPayload::ImportCompleted(data)) =
                    (event.title_id.as_deref(), &event.payload)
            {
                enqueue_imported_entities(
                    queue,
                    &connections,
                    title_id,
                    &data.episode_ids,
                    Instant::now(),
                );
            }
            after_sequence = event.sequence;
            repo.set_subscriber_offset(PLAYBACK_EVENT_SUBSCRIBER, after_sequence)
                .await?;
        }
    }
}

fn enqueue_imported_entities(
    queue: &mut PlaybackRefreshQueue,
    connections: &[MediaServerConnection],
    title_id: &str,
    episode_ids: &[String],
    now: Instant,
) {
    let title_id = title_id.trim();
    if title_id.is_empty() {
        return;
    }
    for connection in connections {
        enqueue_playback_refresh(
            queue,
            PlaybackRefreshKey {
                connection_id: connection.id.clone(),
                entity_kind: MediaServerPlaybackEntityKind::Title,
                entity_id: title_id.to_string(),
            },
            now,
        );
        for episode_id in episode_ids {
            let episode_id = episode_id.trim();
            if episode_id.is_empty() {
                continue;
            }
            enqueue_playback_refresh(
                queue,
                PlaybackRefreshKey {
                    connection_id: connection.id.clone(),
                    entity_kind: MediaServerPlaybackEntityKind::Episode,
                    entity_id: episode_id.to_string(),
                },
                now,
            );
        }
    }
}

fn enqueue_playback_refresh(
    queue: &mut PlaybackRefreshQueue,
    key: PlaybackRefreshKey,
    now: Instant,
) {
    queue.entry(key).or_insert(PendingPlaybackRefresh {
        first_seen_at: now,
        attempt: 0,
    });
}

fn next_incremental_refresh_at(queue: &PlaybackRefreshQueue) -> Option<Instant> {
    queue
        .values()
        .filter_map(|pending| {
            PLAYBACK_INCREMENTAL_DELAYS
                .get(pending.attempt)
                .map(|delay| pending.first_seen_at + *delay)
        })
        .min()
}

fn due_incremental_refreshes(
    queue: &PlaybackRefreshQueue,
    now: Instant,
) -> HashMap<String, Vec<PlaybackEntityKey>> {
    let mut due = HashMap::<String, Vec<PlaybackEntityKey>>::new();
    for (key, pending) in queue {
        let Some(delay) = PLAYBACK_INCREMENTAL_DELAYS.get(pending.attempt) else {
            continue;
        };
        if now >= pending.first_seen_at + *delay {
            due.entry(key.connection_id.clone())
                .or_default()
                .push((key.entity_kind, key.entity_id.clone()));
        }
    }
    due
}

async fn process_due_incremental_refreshes(app: &AppUseCase, queue: &mut PlaybackRefreshQueue) {
    let due = due_incremental_refreshes(queue, Instant::now());
    if due.is_empty() {
        return;
    }
    let connections = match app
        .services
        .integrations
        .media_server_connections
        .list(None)
        .await
    {
        Ok(connections) => connections
            .into_iter()
            .map(|connection| (connection.id.clone(), connection))
            .collect::<HashMap<_, _>>(),
        Err(error) => {
            tracing::warn!(error = %error, "failed to list media servers for incremental playback refresh");
            advance_incremental_refreshes(queue, &due, &HashSet::new());
            return;
        }
    };

    for (connection_id, entities) in due {
        let Some(connection) = connections
            .get(&connection_id)
            .filter(|connection| connection.enabled)
        else {
            remove_incremental_refreshes(queue, &connection_id, &entities);
            continue;
        };
        let refreshed = match app
            .refresh_media_server_playback_mappings_incremental_for_connection(
                connection, &entities,
            )
            .await
        {
            Ok(refreshed) => refreshed,
            Err(error) => {
                tracing::warn!(connection_id, error = %error, "incremental media-server playback refresh failed");
                HashSet::new()
            }
        };
        advance_incremental_refreshes(
            queue,
            &HashMap::from([(connection_id, entities)]),
            &refreshed,
        );
    }
}

fn advance_incremental_refreshes(
    queue: &mut PlaybackRefreshQueue,
    due: &HashMap<String, Vec<PlaybackEntityKey>>,
    refreshed: &HashSet<PlaybackEntityKey>,
) {
    for (connection_id, entities) in due {
        for (entity_kind, entity_id) in entities {
            let key = PlaybackRefreshKey {
                connection_id: connection_id.clone(),
                entity_kind: *entity_kind,
                entity_id: entity_id.clone(),
            };
            if refreshed.contains(&(*entity_kind, entity_id.clone())) {
                queue.remove(&key);
                continue;
            }
            let remove = queue.get_mut(&key).is_some_and(|pending| {
                pending.attempt += 1;
                pending.attempt >= PLAYBACK_INCREMENTAL_DELAYS.len()
            });
            if remove {
                queue.remove(&key);
            }
        }
    }
}

fn remove_incremental_refreshes(
    queue: &mut PlaybackRefreshQueue,
    connection_id: &str,
    entities: &[PlaybackEntityKey],
) {
    for (entity_kind, entity_id) in entities {
        queue.remove(&PlaybackRefreshKey {
            connection_id: connection_id.to_string(),
            entity_kind: *entity_kind,
            entity_id: entity_id.clone(),
        });
    }
}

impl AppUseCase {
    /// Full reconciliation for one connection. A successful run atomically removes stale mappings.
    pub async fn refresh_media_server_playback_mappings(
        &self,
        actor: &User,
        connection_id: &str,
    ) -> AppResult<usize> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(connection_id.trim())
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("media server connection {connection_id}"))
            })?;
        self.refresh_media_server_playback_mappings_for_connection(&connection)
            .await
    }

    pub(crate) async fn refresh_media_server_playback_mappings_for_connection(
        &self,
        connection: &MediaServerConnection,
    ) -> AppResult<usize> {
        let catalog = self
            .services
            .integrations
            .external_identity_verifier
            .scan_media_server_catalog(connection)
            .await?;
        let mappings = self
            .playback_mappings_for_catalog(connection, &catalog)
            .await?;
        let count = mappings.len();
        self.services
            .integrations
            .media_server_connections
            .replace_playback_items_for_connection(&connection.id, mappings)
            .await?;
        Ok(count)
    }

    async fn refresh_media_server_playback_mappings_incremental_for_connection(
        &self,
        connection: &MediaServerConnection,
        entities: &[PlaybackEntityKey],
    ) -> AppResult<HashSet<PlaybackEntityKey>> {
        let catalog = self
            .services
            .integrations
            .external_identity_verifier
            .scan_media_server_catalog_incremental(connection)
            .await?;
        if catalog.is_empty() {
            return Ok(HashSet::new());
        }
        let mappings = self
            .playback_mappings_for_catalog_entities(connection, &catalog, entities)
            .await?;
        let refreshed = mappings
            .iter()
            .map(|item| (item.entity_kind, item.entity_id.clone()))
            .collect::<HashSet<_>>();
        self.services
            .integrations
            .media_server_connections
            .upsert_playback_items_for_connection(&connection.id, mappings)
            .await?;
        Ok(refreshed)
    }

    async fn playback_mappings_for_catalog(
        &self,
        connection: &MediaServerConnection,
        catalog: &[MediaServerCatalogItem],
    ) -> AppResult<Vec<MediaServerPlaybackItem>> {
        let titles = self.services.catalog.titles.list(None, None).await?;
        let now = Utc::now();
        let mut mappings = Vec::new();

        for title in titles {
            let kind = catalog_kind_for_title(&title);
            let matched_item = unique_title_match(&title, kind, catalog);
            if let Some(item) = matched_item {
                mappings.push(playback_item(
                    connection,
                    MediaServerPlaybackEntityKind::Title,
                    title.id.clone(),
                    item.provider_item_id.clone(),
                    now,
                ));
            }
            if kind != MediaServerCatalogItemKind::Series {
                continue;
            }
            let Some(series_provider_item_id) =
                matched_item.map(|item| item.provider_item_id.as_str())
            else {
                continue;
            };
            for episode in self
                .services
                .catalog
                .shows
                .list_episodes_for_title(&title.id)
                .await?
            {
                let Some(provider_episode) =
                    unique_episode_match(&episode, series_provider_item_id, catalog)
                else {
                    continue;
                };
                mappings.push(playback_item(
                    connection,
                    MediaServerPlaybackEntityKind::Episode,
                    episode.id,
                    provider_episode.provider_item_id.clone(),
                    now,
                ));
            }
        }
        Ok(mappings)
    }

    async fn playback_mappings_for_catalog_entities(
        &self,
        connection: &MediaServerConnection,
        catalog: &[MediaServerCatalogItem],
        entities: &[PlaybackEntityKey],
    ) -> AppResult<Vec<MediaServerPlaybackItem>> {
        let requested_titles = entities
            .iter()
            .filter(|(kind, _)| *kind == MediaServerPlaybackEntityKind::Title)
            .map(|(_, id)| id.clone())
            .collect::<HashSet<_>>();
        let requested_episode_ids = entities
            .iter()
            .filter(|(kind, _)| *kind == MediaServerPlaybackEntityKind::Episode)
            .map(|(_, id)| id.clone())
            .collect::<HashSet<_>>();
        let episodes = self
            .services
            .catalog
            .shows
            .get_episodes_by_ids(&requested_episode_ids.iter().cloned().collect::<Vec<_>>())
            .await?;
        let mut episode_by_title = HashMap::<String, Vec<scryer_domain::Episode>>::new();
        for episode in episodes {
            episode_by_title
                .entry(episode.title_id.clone())
                .or_default()
                .push(episode);
        }
        let title_ids = requested_titles
            .iter()
            .cloned()
            .chain(episode_by_title.keys().cloned())
            .collect::<HashSet<_>>();
        let mut mappings = Vec::new();
        let now = Utc::now();

        for title_id in title_ids {
            let Some(title) = self.services.catalog.titles.get_by_id(&title_id).await? else {
                continue;
            };
            let kind = catalog_kind_for_title(&title);
            let matched_title = unique_title_match(&title, kind, catalog);
            if requested_titles.contains(&title.id)
                && let Some(item) = matched_title
            {
                mappings.push(playback_item(
                    connection,
                    MediaServerPlaybackEntityKind::Title,
                    title.id.clone(),
                    item.provider_item_id.clone(),
                    now,
                ));
            }
            if kind != MediaServerCatalogItemKind::Series {
                continue;
            }
            let series_provider_item_id = match matched_title {
                Some(item) => Some(item.provider_item_id.clone()),
                None => self
                    .services
                    .integrations
                    .media_server_connections
                    .list_playback_items_for_entity(MediaServerPlaybackEntityKind::Title, &title.id)
                    .await?
                    .into_iter()
                    .find(|item| item.connection_id == connection.id)
                    .map(|item| item.provider_item_id),
            };
            let Some(series_provider_item_id) = series_provider_item_id else {
                continue;
            };
            for episode in episode_by_title.remove(&title.id).unwrap_or_default() {
                let Some(provider_episode) =
                    unique_episode_match(&episode, &series_provider_item_id, catalog)
                else {
                    continue;
                };
                mappings.push(playback_item(
                    connection,
                    MediaServerPlaybackEntityKind::Episode,
                    episode.id,
                    provider_episode.provider_item_id.clone(),
                    now,
                ));
            }
        }
        Ok(mappings)
    }
}

fn playback_item(
    connection: &MediaServerConnection,
    entity_kind: MediaServerPlaybackEntityKind,
    entity_id: String,
    provider_item_id: String,
    last_seen_at: chrono::DateTime<Utc>,
) -> MediaServerPlaybackItem {
    MediaServerPlaybackItem {
        connection_id: connection.id.clone(),
        entity_kind,
        entity_id,
        provider_item_id,
        last_seen_at,
    }
}

fn catalog_kind_for_title(title: &Title) -> MediaServerCatalogItemKind {
    match title.facet {
        MediaFacet::Movie => MediaServerCatalogItemKind::Movie,
        MediaFacet::Series | MediaFacet::Anime => MediaServerCatalogItemKind::Series,
    }
}

fn unique_title_match<'a>(
    title: &Title,
    kind: MediaServerCatalogItemKind,
    catalog: &'a [MediaServerCatalogItem],
) -> Option<&'a MediaServerCatalogItem> {
    let mut ids = title.external_ids.clone();
    if let Some(imdb_id) = title
        .imdb_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        ids.push(ExternalId {
            source: "imdb".into(),
            value: imdb_id.into(),
        });
    }
    let candidates = catalog
        .iter()
        .filter(|item| item.kind == kind)
        .collect::<Vec<_>>();
    let tmdb = matching_items(&ids, &candidates, true);
    if !tmdb.is_empty() {
        return unique_item(tmdb);
    }
    unique_item(matching_items(&ids, &candidates, false))
}

fn unique_episode_match<'a>(
    episode: &scryer_domain::Episode,
    series_id: &str,
    catalog: &'a [MediaServerCatalogItem],
) -> Option<&'a MediaServerCatalogItem> {
    let season = episode
        .season_number
        .as_deref()?
        .trim()
        .parse::<i32>()
        .ok()?;
    let number = episode
        .episode_number
        .as_deref()?
        .trim()
        .parse::<i32>()
        .ok()?;
    unique_episode_number_match(season, number, series_id, catalog)
}

fn unique_episode_number_match<'a>(
    season: i32,
    number: i32,
    series_id: &str,
    catalog: &'a [MediaServerCatalogItem],
) -> Option<&'a MediaServerCatalogItem> {
    unique_item(
        catalog
            .iter()
            .filter(|item| {
                item.kind == MediaServerCatalogItemKind::Episode
                    && item.series_provider_item_id.as_deref() == Some(series_id)
                    && item.season_number == Some(season)
                    && item.episode_number.is_some_and(|first| {
                        (first..=item.episode_number_end.unwrap_or(first)).contains(&number)
                    })
            })
            .collect(),
    )
}

fn matching_items<'a>(
    expected: &[ExternalId],
    candidates: &[&'a MediaServerCatalogItem],
    tmdb_only: bool,
) -> Vec<&'a MediaServerCatalogItem> {
    let expected = expected
        .iter()
        .filter_map(|id| {
            let source = normalize_source(&id.source);
            (!tmdb_only || source == "tmdb")
                .then(|| (source, normalize_value(&id.value)))
                .filter(|(_, value)| !value.is_empty())
        })
        .collect::<HashSet<_>>();
    candidates
        .iter()
        .copied()
        .filter(|item| {
            item.external_ids.iter().any(|id| {
                let source = normalize_source(&id.source);
                (!tmdb_only || source == "tmdb")
                    && expected.contains(&(source, normalize_value(&id.value)))
            })
        })
        .collect()
}

fn unique_item(items: Vec<&MediaServerCatalogItem>) -> Option<&MediaServerCatalogItem> {
    let provider_ids = items
        .iter()
        .map(|item| item.provider_item_id.as_str())
        .collect::<HashSet<_>>();
    (provider_ids.len() == 1).then(|| items[0])
}

fn normalize_source(source: &str) -> String {
    match source.trim().to_ascii_lowercase().as_str() {
        "themoviedb" | "tmdb" => "tmdb".into(),
        "thetvdb" | "tvdb" => "tvdb".into(),
        "imdb" => "imdb".into(),
        other => other.into(),
    }
}

fn normalize_value(value: &str) -> String {
    let value = value.trim();
    value
        .strip_prefix("tmdb://")
        .or_else(|| value.strip_prefix("tvdb://"))
        .or_else(|| value.strip_prefix("imdb://"))
        .unwrap_or(value)
        .trim()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_item(
        kind: MediaServerCatalogItemKind,
        provider_item_id: &str,
        external_ids: &[(&str, &str)],
    ) -> MediaServerCatalogItem {
        MediaServerCatalogItem {
            kind,
            provider_item_id: provider_item_id.into(),
            external_ids: external_ids
                .iter()
                .map(|(source, value)| ExternalId {
                    source: (*source).into(),
                    value: (*value).into(),
                })
                .collect(),
            series_provider_item_id: None,
            season_number: None,
            episode_number: None,
            episode_number_end: None,
        }
    }

    #[test]
    fn duplicate_imports_preserve_the_first_refresh_deadline() {
        let mut queue = PlaybackRefreshQueue::new();
        let first_seen_at = Instant::now();
        let key = PlaybackRefreshKey {
            connection_id: "connection-1".into(),
            entity_kind: MediaServerPlaybackEntityKind::Title,
            entity_id: "title-1".into(),
        };
        enqueue_playback_refresh(&mut queue, key.clone(), first_seen_at);
        enqueue_playback_refresh(&mut queue, key, first_seen_at + Duration::from_secs(30));

        assert_eq!(
            next_incremental_refresh_at(&queue),
            Some(first_seen_at + PLAYBACK_INCREMENTAL_DELAYS[0])
        );
    }

    #[test]
    fn queue_advances_missing_items_through_each_delayed_attempt() {
        let mut queue = PlaybackRefreshQueue::new();
        let first_seen_at = Instant::now();
        let key = PlaybackRefreshKey {
            connection_id: "connection-1".into(),
            entity_kind: MediaServerPlaybackEntityKind::Episode,
            entity_id: "episode-1".into(),
        };
        enqueue_playback_refresh(&mut queue, key, first_seen_at);
        let due = due_incremental_refreshes(&queue, first_seen_at + PLAYBACK_INCREMENTAL_DELAYS[0]);
        advance_incremental_refreshes(&mut queue, &due, &HashSet::new());
        assert_eq!(
            next_incremental_refresh_at(&queue),
            Some(first_seen_at + PLAYBACK_INCREMENTAL_DELAYS[1])
        );
        let due = due_incremental_refreshes(&queue, first_seen_at + PLAYBACK_INCREMENTAL_DELAYS[1]);
        advance_incremental_refreshes(&mut queue, &due, &HashSet::new());
        let due = due_incremental_refreshes(&queue, first_seen_at + PLAYBACK_INCREMENTAL_DELAYS[2]);
        advance_incremental_refreshes(&mut queue, &due, &HashSet::new());
        assert!(queue.is_empty());
    }

    #[test]
    fn exact_tmdb_matches_are_preferred_over_compatible_ids() {
        let expected = vec![
            ExternalId {
                source: "tmdb".into(),
                value: "tmdb://123".into(),
            },
            ExternalId {
                source: "tvdb".into(),
                value: "456".into(),
            },
        ];
        let tmdb = catalog_item(
            MediaServerCatalogItemKind::Series,
            "provider-tmdb",
            &[("themoviedb", "123")],
        );
        let tvdb = catalog_item(
            MediaServerCatalogItemKind::Series,
            "provider-tvdb",
            &[("thetvdb", "456")],
        );
        let candidates = vec![&tmdb, &tvdb];

        assert_eq!(
            unique_item(matching_items(&expected, &candidates, true))
                .map(|item| item.provider_item_id.as_str()),
            Some("provider-tmdb")
        );
    }

    #[test]
    fn ambiguous_exact_matches_are_skipped() {
        let expected = vec![ExternalId {
            source: "tmdb".into(),
            value: "123".into(),
        }];
        let first = catalog_item(
            MediaServerCatalogItemKind::Movie,
            "provider-one",
            &[("tmdb", "123")],
        );
        let second = catalog_item(
            MediaServerCatalogItemKind::Movie,
            "provider-two",
            &[("tmdb", "123")],
        );
        let candidates = vec![&first, &second];

        assert!(unique_item(matching_items(&expected, &candidates, true)).is_none());
    }

    #[test]
    fn combined_episode_ranges_map_each_included_episode() {
        let mut combined =
            catalog_item(MediaServerCatalogItemKind::Episode, "combined-episode", &[]);
        combined.series_provider_item_id = Some("series-1".into());
        combined.season_number = Some(2);
        combined.episode_number = Some(4);
        combined.episode_number_end = Some(5);
        let catalog = vec![combined];

        assert_eq!(
            unique_episode_number_match(2, 5, "series-1", &catalog)
                .map(|item| item.provider_item_id.as_str()),
            Some("combined-episode")
        );
        assert!(unique_episode_number_match(2, 6, "series-1", &catalog).is_none());
        assert!(unique_episode_number_match(2, 5, "series-2", &catalog).is_none());
    }
}

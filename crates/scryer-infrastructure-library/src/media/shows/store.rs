use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, CollectionUpdate, EpisodeImageUrlUpdate, EpisodeUpdate,
    PrimaryCollectionSummary, ScopedExternalId, SeriesMovieExternalIdLookupMatch, ShowRepository,
    TitleExternalIdLookup,
};
use scryer_domain::{
    CalendarEpisode, Collection, CollectionType, Episode, EpisodeType, Id, MovieEntity,
    SeriesMovieLink,
};
use std::collections::{BTreeSet, HashMap, HashSet};

use crate::media::canonical_tags::{
    load_movie_entity_metadata_ratings, replace_movie_entity_metadata_ratings_tx,
};
use crate::media::title_credits::{load_movie_entity_credits, replace_movie_entity_credits_tx};
// Series-movie tags are the same bag with the same rules as a title's, so the
// patch, the per-owner cap, and the reserved-namespace guard are the title
// store's function rather than a second copy that could drift from it.
use crate::media::titles::store::apply_user_tag_patch;
use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTarget, SqlTx, StoreDatastore,
};

const COLLECTION_COLUMNS: &str = "id, title_id, collection_type, collection_index, label, ordered_path, \
    narrative_order, first_episode_number, last_episode_number, monitored, created_at";

const SERIES_MOVIE_LINK_COLUMNS: &str = "sml.id AS link_id, sml.series_title_id, sml.placement, \
    sml.narrative_order, sml.after_season, sml.before_season, sml.linked_episode_id, \
    sml.association_confidence, sml.continuity_status, sml.movie_form, sml.confidence, \
    sml.signal_summary, sml.source, sml.monitoring_override, sml.metadata_active, \
    sml.monitored AS link_monitored, sml.legacy_collection_id, sml.tags AS link_tags, \
    sml.created_at AS link_created_at, sml.updated_at AS link_updated_at, \
    me.id AS movie_id, me.title AS movie_title, me.sort_title AS movie_sort_title, \
    me.slug AS movie_slug, me.year AS movie_year, me.overview AS movie_overview, \
    me.poster_url AS movie_poster_url, me.background_url AS movie_background_url, \
    me.language AS movie_language, me.runtime_minutes AS movie_runtime_minutes, \
    me.content_status AS movie_content_status, me.studio AS movie_studio, \
    me.digital_release_date AS movie_digital_release_date, \
    me.imdb_id AS movie_imdb_id, me.tvdb_id AS movie_tvdb_id, me.tmdb_id AS movie_tmdb_id, \
    me.mal_id AS movie_mal_id, me.anidb_id AS movie_anidb_id, me.created_at AS movie_created_at, \
    me.updated_at AS movie_updated_at";

const EPISODE_COLUMNS: &str = "id, title_id, collection_id, episode_type, episode_number, season_number, \
    episode_label, title, air_date, duration_seconds, has_multi_audio, has_subtitle, is_filler, is_recap, \
    absolute_number, overview, tvdb_id, image_url, monitored, created_at";

const COLLECTION_INSERT_SQL: &str = "INSERT INTO collections (
    id, title_id, collection_type, collection_index, label, ordered_path, narrative_order,
    first_episode_number, last_episode_number, monitored, created_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

const COLLECTION_UPDATE_SQL: &str = "UPDATE collections SET
    title_id = {},
    collection_type = {},
    collection_index = {},
    label = {},
    ordered_path = {},
    narrative_order = {},
    first_episode_number = {},
    last_episode_number = {},
    monitored = {}
WHERE id = {}";

const EPISODE_INSERT_SQL: &str = "INSERT INTO episodes (
    id, title_id, collection_id, episode_type, episode_number, season_number,
    episode_label, title, air_date, duration_seconds, has_multi_audio,
    has_subtitle, is_filler, is_recap, absolute_number, overview, tvdb_id,
    image_url, monitored, created_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

const EPISODE_UPDATE_SQL: &str = "UPDATE episodes SET
    title_id = {},
    collection_id = {},
    episode_type = {},
    episode_number = {},
    season_number = {},
    episode_label = {},
    title = {},
    air_date = {},
    duration_seconds = {},
    has_multi_audio = {},
    has_subtitle = {},
    is_filler = {},
    is_recap = {},
    absolute_number = {},
    overview = {},
    tvdb_id = {},
    image_url = {},
    monitored = {}
WHERE id = {}";

#[derive(Clone)]
pub struct ShowStore {
    datastore: StoreDatastore,
}

impl ShowStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    fn read_target(&self) -> SqlTarget<'_> {
        match &self.datastore {
            StoreDatastore::Sqlite { pool, .. } => SqlTarget::Sqlite(pool),
            StoreDatastore::Postgres { pool } => SqlTarget::Postgres(pool),
        }
    }
}

#[async_trait]
impl ShowRepository for ShowStore {
    async fn list_series_movie_links_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<SeriesMovieLink>> {
        list_series_movie_links_for_title_query(self.read_target(), title_id).await
    }

    async fn list_series_movie_links_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<SeriesMovieLink>> {
        list_series_movie_links_for_titles_query(self.read_target(), title_ids).await
    }

    async fn list_series_movie_external_id_lookup_matches(
        &self,
        library_ids: &[String],
        lookups: &[TitleExternalIdLookup],
    ) -> AppResult<Vec<SeriesMovieExternalIdLookupMatch>> {
        list_series_movie_external_id_lookup_matches_query(self.read_target(), library_ids, lookups)
            .await
    }

    async fn get_series_movie_link_by_id(
        &self,
        link_id: &str,
    ) -> AppResult<Option<SeriesMovieLink>> {
        get_series_movie_link_by_id_query(self.read_target(), link_id).await
    }

    async fn list_movie_entity_credits(
        &self,
        movie_entity_id: &str,
    ) -> AppResult<Vec<scryer_application::TitleCredit>> {
        let mut credits = load_movie_entity_credits(
            SqlExec::Target(self.read_target()),
            &[movie_entity_id.to_string()],
        )
        .await?;
        Ok(credits.remove(movie_entity_id).unwrap_or_default())
    }

    async fn find_series_movie_link_by_legacy_collection_id(
        &self,
        collection_id: &str,
    ) -> AppResult<Option<SeriesMovieLink>> {
        find_series_movie_link_by_legacy_collection_id_query(self.read_target(), collection_id)
            .await
    }

    async fn upsert_series_movie_link(&self, link: SeriesMovieLink) -> AppResult<SeriesMovieLink> {
        SqlRuntime::run_in_transaction(&self.datastore, "upsert_series_movie_link", move |tx| {
            let link = link.clone();
            Box::pin(async move { upsert_series_movie_link_tx(tx, link).await })
        })
        .await
    }

    async fn delete_stale_series_movie_links(
        &self,
        title_id: &str,
        retained_link_ids: &[String],
    ) -> AppResult<()> {
        let title_id = title_id.to_string();
        let retained_link_ids = retained_link_ids.to_vec();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_stale_series_movie_links",
            move |tx| {
                let title_id = title_id.clone();
                let retained_link_ids = retained_link_ids.clone();
                Box::pin(async move {
                    delete_stale_series_movie_links_tx(tx, &title_id, &retained_link_ids).await
                })
            },
        )
        .await
    }

    async fn update_series_movie_link_user_tags(
        &self,
        link_id: &str,
        add: &[String],
        remove: &[String],
    ) -> AppResult<SeriesMovieLink> {
        let link_id = link_id.to_string();
        let add = add.to_vec();
        let remove = remove.to_vec();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_series_movie_link_user_tags",
            move |tx| {
                let link_id = link_id.clone();
                let add = add.clone();
                let remove = remove.clone();
                Box::pin(async move {
                    // Read-modify-write inside the transaction rather than a
                    // whole-bag replace from a value the caller read earlier,
                    // for the same reason the title patch does it: two
                    // concurrent tag saves on one link must not lose each
                    // other's half.
                    let existing =
                        load_series_movie_link_tx(tx, &link_id)
                            .await?
                            .ok_or_else(|| {
                                AppError::NotFound(format!("series movie link {link_id}"))
                            })?;
                    let mut tags = existing.tags.clone();
                    apply_user_tag_patch(&mut tags, &add, &remove)?;
                    tx.execute(
                        "UPDATE series_movie_links SET tags = {}, updated_at = {} WHERE id = {}",
                        &[
                            SqlArg::Json(
                                serde_json::to_value(&tags)
                                    .unwrap_or_else(|_| serde_json::Value::Array(Vec::new())),
                            ),
                            SqlArg::Timestamp(Utc::now()),
                            SqlArg::Text(link_id.clone()),
                        ],
                    )
                    .await?;
                    load_series_movie_link_tx(tx, &link_id)
                        .await?
                        .ok_or_else(|| {
                            AppError::Repository("series movie tag patch returned no row".into())
                        })
                })
            },
        )
        .await
    }

    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        list_collections_for_title_query(self.read_target(), title_id).await
    }

    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        list_collection_external_ids_query(self.read_target(), collection_id).await
    }

    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>> {
        list_collections_for_titles_query(self.read_target(), title_ids).await
    }

    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        get_collection_by_id_query(self.read_target(), collection_id).await
    }

    async fn get_collections_by_ids(&self, ids: &[String]) -> AppResult<Vec<Collection>> {
        get_collections_by_ids_query(self.read_target(), ids).await
    }

    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        get_collection_by_ordered_path_query(self.read_target(), ordered_path).await
    }

    async fn list_collections_by_ordered_paths(
        &self,
        ordered_paths: &[String],
    ) -> AppResult<Vec<Collection>> {
        list_collections_by_ordered_paths_query(self.read_target(), ordered_paths).await
    }

    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_collection", move |tx| {
            let collection = collection.clone();
            Box::pin(async move {
                insert_collection_tx(tx, &collection).await?;
                Ok(collection)
            })
        })
        .await
    }

    async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection> {
        let collection_id = collection_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_collection", move |tx| {
            let collection_id = collection_id.clone();
            let update = update.clone();
            Box::pin(async move {
                if collection_update_is_empty(&update) {
                    return Err(AppError::Validation(
                        "at least one collection field must be provided".into(),
                    ));
                }

                let mut collection = load_collection_tx(tx, &collection_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("collection {collection_id}")))?;

                apply_collection_update(&mut collection, update);
                persist_collection_tx(tx, &collection).await?;
                Ok(collection)
            })
        })
        .await
    }

    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        let collection_id = collection_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "set_collection_episodes_monitored",
            move |tx| {
                let collection_id = collection_id.clone();
                Box::pin(async move {
                    set_collection_episodes_monitored_tx(tx, &collection_id, monitored).await
                })
            },
        )
        .await
    }

    async fn set_collections_monitored(
        &self,
        collection_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        let collection_ids = collection_ids.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "set_collections_monitored", move |tx| {
            let collection_ids = collection_ids.clone();
            Box::pin(
                async move { set_collections_monitored_tx(tx, &collection_ids, monitored).await },
            )
        })
        .await
    }

    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        let collection_id = collection_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_collection", move |tx| {
            let collection_id = collection_id.clone();
            Box::pin(async move { delete_collection_tx(tx, &collection_id).await })
        })
        .await
    }

    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_collections_for_title", move |tx| {
            let title_id = title_id.clone();
            Box::pin(async move { delete_collections_for_title_tx(tx, &title_id).await })
        })
        .await
    }

    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        list_episodes_for_collection_query(self.read_target(), collection_id).await
    }

    async fn list_episodes_for_collections(
        &self,
        collection_ids: &[String],
    ) -> AppResult<Vec<Episode>> {
        list_episodes_for_collections_query(self.read_target(), collection_ids).await
    }

    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        list_episodes_for_title_query(self.read_target(), title_id).await
    }

    async fn list_episode_external_ids(
        &self,
        episode_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        list_episode_external_ids_query(self.read_target(), episode_id).await
    }

    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        get_episode_by_id_query(self.read_target(), episode_id).await
    }

    async fn get_episodes_by_ids(&self, ids: &[String]) -> AppResult<Vec<Episode>> {
        get_episodes_by_ids_query(self.read_target(), ids).await
    }

    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_episode", move |tx| {
            let episode = episode.clone();
            Box::pin(async move {
                insert_episode_tx(tx, &episode).await?;
                Ok(episode)
            })
        })
        .await
    }

    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode> {
        let episode_id = episode_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_episode", move |tx| {
            let episode_id = episode_id.clone();
            let update = update.clone();
            Box::pin(async move {
                if episode_update_is_empty(&update) {
                    return Err(AppError::Validation(
                        "at least one episode field must be provided".into(),
                    ));
                }

                let mut episode = load_episode_tx(tx, &episode_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("episode {episode_id}")))?;

                apply_episode_update(&mut episode, update);
                persist_episode_tx(tx, &episode).await?;
                Ok(episode)
            })
        })
        .await
    }

    async fn set_episodes_monitored(
        &self,
        episode_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        let episode_ids = episode_ids.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "set_episodes_monitored", move |tx| {
            let episode_ids = episode_ids.clone();
            Box::pin(async move { set_episodes_monitored_tx(tx, &episode_ids, monitored).await })
        })
        .await
    }

    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        let episode_id = episode_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_episode", move |tx| {
            let episode_id = episode_id.clone();
            Box::pin(async move { delete_episode_tx(tx, &episode_id).await })
        })
        .await
    }

    async fn update_episode_image_urls(&self, updates: &[EpisodeImageUrlUpdate]) -> AppResult<u64> {
        if updates.is_empty() {
            return Ok(0);
        }
        let updates = updates.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "update_episode_image_urls", move |tx| {
            let updates = updates.clone();
            Box::pin(async move {
                let mut changed = 0_u64;
                for update in updates {
                    let rows = tx
                        .execute(
                            "UPDATE episodes
                                SET image_url = {}
                              WHERE id = {}
                                AND COALESCE(image_url, '') <> COALESCE({}, '')",
                            &[
                                SqlArg::OptText(update.image_url.clone()),
                                SqlArg::Text(update.episode_id.clone()),
                                SqlArg::OptText(update.image_url),
                            ],
                        )
                        .await?;
                    changed += rows;
                }
                Ok(changed)
            })
        })
        .await
    }

    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_episodes_for_title", move |tx| {
            let title_id = title_id.clone();
            Box::pin(async move { delete_episodes_for_title_tx(tx, &title_id).await })
        })
        .await
    }

    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        find_episode_by_title_and_numbers_query(
            self.read_target(),
            title_id,
            season_number,
            episode_number,
        )
        .await
    }

    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>> {
        find_episode_by_title_and_absolute_number_query(
            self.read_target(),
            title_id,
            absolute_number,
        )
        .await
    }

    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        list_primary_collection_summaries_query(self.read_target(), title_ids).await
    }

    async fn list_episodes_in_date_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>> {
        list_episodes_in_date_range_query(self.read_target(), start_date, end_date).await
    }

    async fn replace_anibridge_scoped_external_ids_for_title(
        &self,
        title_id: &str,
        collection_ids: Vec<ScopedExternalId>,
        episode_ids: Vec<ScopedExternalId>,
    ) -> AppResult<()> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "replace_anibridge_scoped_external_ids_for_title",
            move |tx| {
                let title_id = title_id.clone();
                let collection_ids = collection_ids.clone();
                let episode_ids = episode_ids.clone();
                Box::pin(async move {
                    replace_anibridge_scoped_external_ids_for_title_tx(
                        tx,
                        &title_id,
                        &collection_ids,
                        &episode_ids,
                    )
                    .await
                })
            },
        )
        .await
    }
}

async fn list_series_movie_links_for_title_query(
    target: SqlTarget<'_>,
    title_id: &str,
) -> AppResult<Vec<SeriesMovieLink>> {
    let sql = format!(
        "SELECT {SERIES_MOVIE_LINK_COLUMNS}
         FROM series_movie_links sml
         INNER JOIN movie_entities me ON me.id = sml.movie_entity_id
         WHERE sml.series_title_id = {{}}
         ORDER BY COALESCE(sml.narrative_order, ''), sml.created_at ASC, sml.id ASC"
    );
    let query_target = match &target {
        SqlTarget::Sqlite(pool) => SqlTarget::Sqlite(pool),
        SqlTarget::Postgres(pool) => SqlTarget::Postgres(pool),
    };
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(query_target),
        &sql,
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    let mut links = rows
        .iter()
        .map(row_to_series_movie_link)
        .collect::<AppResult<Vec<_>>>()?;
    attach_movie_entity_ratings(SqlExec::Target(target), &mut links).await?;
    Ok(links)
}

async fn list_series_movie_links_for_titles_query(
    target: SqlTarget<'_>,
    title_ids: &[String],
) -> AppResult<Vec<SeriesMovieLink>> {
    if title_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = bind_placeholders(title_ids.len());
    let sql = format!(
        "SELECT {SERIES_MOVIE_LINK_COLUMNS}
         FROM series_movie_links sml
         INNER JOIN movie_entities me ON me.id = sml.movie_entity_id
         WHERE sml.series_title_id IN ({placeholders})
         ORDER BY sml.series_title_id ASC, COALESCE(sml.narrative_order, ''), sml.created_at ASC, sml.id ASC"
    );
    let args = title_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    let query_target = match &target {
        SqlTarget::Sqlite(pool) => SqlTarget::Sqlite(pool),
        SqlTarget::Postgres(pool) => SqlTarget::Postgres(pool),
    };
    let rows = SqlRuntime::fetch_all(SqlExec::Target(query_target), &sql, &args).await?;
    let mut links = rows
        .iter()
        .map(row_to_series_movie_link)
        .collect::<AppResult<Vec<_>>>()?;
    attach_movie_entity_ratings(SqlExec::Target(target), &mut links).await?;
    Ok(links)
}

fn movie_entity_external_id_column(source: &str) -> Option<&'static str> {
    match source.trim().to_ascii_lowercase().as_str() {
        "imdb" => Some("imdb_id"),
        "tvdb" | "tvdb_movie" => Some("tvdb_id"),
        "tmdb" | "tmdb_movie" => Some("tmdb_id"),
        "mal" | "myanimelist" => Some("mal_id"),
        "anidb" => Some("anidb_id"),
        _ => None,
    }
}

async fn list_series_movie_external_id_lookup_matches_query(
    target: SqlTarget<'_>,
    library_ids: &[String],
    lookups: &[TitleExternalIdLookup],
) -> AppResult<Vec<SeriesMovieExternalIdLookupMatch>> {
    if library_ids.is_empty() || lookups.is_empty() {
        return Ok(Vec::new());
    }

    // Keep each provider in its own query so the matching movie_entities ID index remains usable.
    let mut lookup_indexes_by_column_and_id =
        HashMap::<&'static str, HashMap<String, Vec<usize>>>::new();
    for lookup in lookups {
        let Some(column) = movie_entity_external_id_column(&lookup.source) else {
            continue;
        };
        let external_id = lookup.external_id.trim();
        if external_id.is_empty() {
            continue;
        }
        lookup_indexes_by_column_and_id
            .entry(column)
            .or_default()
            .entry(external_id.to_string())
            .or_default()
            .push(lookup.lookup_index);
    }

    let library_placeholders = bind_placeholders(library_ids.len());
    let mut matched_lookup_indexes = BTreeSet::new();
    for (column, lookup_indexes_by_id) in lookup_indexes_by_column_and_id {
        let external_ids = lookup_indexes_by_id.keys().cloned().collect::<Vec<_>>();
        let external_id_placeholders = bind_placeholders(external_ids.len());
        let sql = format!(
            "SELECT DISTINCT me.{column} AS external_id
             FROM movie_entities me
             INNER JOIN series_movie_links sml ON sml.movie_entity_id = me.id
             INNER JOIN titles parent ON parent.id = sml.series_title_id
             WHERE parent.library_id IN ({library_placeholders})
               AND me.{column} IS NOT NULL
               AND me.{column} <> ''
               AND me.{column} IN ({external_id_placeholders})"
        );
        let args = library_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .chain(external_ids.into_iter().map(SqlArg::Text))
            .collect::<Vec<_>>();
        let execution_target = match &target {
            SqlTarget::Sqlite(pool) => SqlTarget::Sqlite(pool),
            SqlTarget::Postgres(pool) => SqlTarget::Postgres(pool),
        };
        for row in SqlRuntime::fetch_all(SqlExec::Target(execution_target), &sql, &args).await? {
            let external_id = row.text("external_id")?;
            if let Some(lookup_indexes) = lookup_indexes_by_id.get(&external_id) {
                matched_lookup_indexes.extend(lookup_indexes.iter().copied());
            }
        }
    }

    Ok(matched_lookup_indexes
        .into_iter()
        .map(|lookup_index| SeriesMovieExternalIdLookupMatch { lookup_index })
        .collect())
}

async fn get_series_movie_link_by_id_query(
    target: SqlTarget<'_>,
    link_id: &str,
) -> AppResult<Option<SeriesMovieLink>> {
    let sql = format!(
        "SELECT {SERIES_MOVIE_LINK_COLUMNS}
         FROM series_movie_links sml
         INNER JOIN movie_entities me ON me.id = sml.movie_entity_id
         WHERE sml.id = {{}}
         LIMIT 1"
    );
    let query_target = match &target {
        SqlTarget::Sqlite(pool) => SqlTarget::Sqlite(pool),
        SqlTarget::Postgres(pool) => SqlTarget::Postgres(pool),
    };
    let row = SqlRuntime::fetch_optional(
        SqlExec::Target(query_target),
        &sql,
        &[SqlArg::Text(link_id.to_string())],
    )
    .await?;
    let mut link = row.as_ref().map(row_to_series_movie_link).transpose()?;
    if let Some(link) = &mut link {
        attach_movie_entity_ratings(SqlExec::Target(target), std::slice::from_mut(link)).await?;
    }
    Ok(link)
}

async fn find_series_movie_link_by_legacy_collection_id_query(
    target: SqlTarget<'_>,
    collection_id: &str,
) -> AppResult<Option<SeriesMovieLink>> {
    let sql = format!(
        "SELECT {SERIES_MOVIE_LINK_COLUMNS}
         FROM series_movie_links sml
         INNER JOIN movie_entities me ON me.id = sml.movie_entity_id
         WHERE sml.legacy_collection_id = {{}}
         LIMIT 1"
    );
    let query_target = match &target {
        SqlTarget::Sqlite(pool) => SqlTarget::Sqlite(pool),
        SqlTarget::Postgres(pool) => SqlTarget::Postgres(pool),
    };
    let row = SqlRuntime::fetch_optional(
        SqlExec::Target(query_target),
        &sql,
        &[SqlArg::Text(collection_id.to_string())],
    )
    .await?;
    let mut link = row.as_ref().map(row_to_series_movie_link).transpose()?;
    if let Some(link) = &mut link {
        attach_movie_entity_ratings(SqlExec::Target(target), std::slice::from_mut(link)).await?;
    }
    Ok(link)
}

async fn upsert_series_movie_link_tx(
    tx: &mut SqlTx<'_>,
    mut link: SeriesMovieLink,
) -> AppResult<SeriesMovieLink> {
    let movie_id = find_existing_movie_entity_id_tx(tx, &link.movie)
        .await?
        .unwrap_or_else(|| link.movie.id.clone());
    link.movie.id = movie_id.clone();
    upsert_movie_entity_tx(tx, &link.movie).await?;

    if let Some(existing_link_id) =
        find_existing_series_movie_link_id_tx(tx, &link.series_title_id, &movie_id).await?
    {
        link.id = existing_link_id;
        if let Some(existing) = load_series_movie_link_tx(tx, &link.id).await? {
            if link.monitoring_override.is_none() {
                link.monitoring_override = existing.monitoring_override;
            }
            if let Some(monitored) = link.monitoring_override {
                link.monitored = monitored;
            }
        }
    }

    tx.execute(
        "INSERT INTO series_movie_links (
             id, series_title_id, movie_entity_id, placement, narrative_order, after_season,
             before_season, linked_episode_id, association_confidence, continuity_status,
             movie_form, confidence, signal_summary, source, monitoring_override, metadata_active,
             monitored, legacy_collection_id, tags, created_at, updated_at
         ) VALUES (
             {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
         )
         ON CONFLICT(id) DO UPDATE SET
             series_title_id = excluded.series_title_id,
             movie_entity_id = excluded.movie_entity_id,
             placement = excluded.placement,
             narrative_order = excluded.narrative_order,
             after_season = excluded.after_season,
             before_season = excluded.before_season,
             linked_episode_id = excluded.linked_episode_id,
             association_confidence = excluded.association_confidence,
             continuity_status = excluded.continuity_status,
             movie_form = excluded.movie_form,
             confidence = excluded.confidence,
             signal_summary = excluded.signal_summary,
             source = excluded.source,
             monitoring_override = excluded.monitoring_override,
             metadata_active = excluded.metadata_active,
             monitored = excluded.monitored,
             legacy_collection_id = COALESCE(excluded.legacy_collection_id, series_movie_links.legacy_collection_id),
             updated_at = excluded.updated_at",
        // `tags` is deliberately absent from the DO UPDATE list. This upsert is
        // the metadata-sync write: it runs on every hydration with whatever the
        // provider reported, and operator tag membership is not something a
        // provider reports. It is set on insert (so a caller creating a link
        // with tags keeps them) and thereafter only
        // `update_series_movie_link_user_tags` writes the column.
        &[
            SqlArg::Text(link.id.clone()),
            SqlArg::Text(link.series_title_id.clone()),
            SqlArg::Text(movie_id),
            SqlArg::OptText(link.placement.clone()),
            SqlArg::OptText(link.narrative_order.clone()),
            SqlArg::OptI32(link.after_season),
            SqlArg::OptI32(link.before_season),
            SqlArg::OptText(link.linked_episode_id.clone()),
            SqlArg::OptText(link.association_confidence.clone()),
            SqlArg::OptText(link.continuity_status.clone()),
            SqlArg::OptText(link.movie_form.clone()),
            SqlArg::OptText(link.confidence.clone()),
            SqlArg::OptText(link.signal_summary.clone()),
            SqlArg::OptText(link.source.clone()),
            SqlArg::OptBool(link.monitoring_override),
            SqlArg::Bool(link.metadata_active),
            SqlArg::Bool(link.monitored),
            SqlArg::OptText(link.legacy_collection_id.clone()),
            SqlArg::Json(
                serde_json::to_value(&link.tags)
                    .unwrap_or_else(|_| serde_json::Value::Array(Vec::new())),
            ),
            SqlArg::Timestamp(link.created_at),
            SqlArg::Timestamp(link.updated_at),
        ],
    )
    .await?;

    load_series_movie_link_tx(tx, &link.id)
        .await?
        .ok_or_else(|| AppError::Repository("series movie link upsert returned no row".into()))
}

async fn upsert_movie_entity_tx(tx: &mut SqlTx<'_>, movie: &MovieEntity) -> AppResult<()> {
    tx.execute(
        "INSERT INTO movie_entities (
             id, title, sort_title, slug, year, overview, poster_url, background_url, language,
             runtime_minutes, content_status, studio, digital_release_date,
             imdb_id, tvdb_id, tmdb_id, mal_id, anidb_id, created_at, updated_at
         ) VALUES (
             {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
         )
         ON CONFLICT(id) DO UPDATE SET
             title = excluded.title,
             sort_title = excluded.sort_title,
             slug = excluded.slug,
             year = excluded.year,
             overview = excluded.overview,
             poster_url = excluded.poster_url,
             background_url = excluded.background_url,
             language = excluded.language,
             runtime_minutes = excluded.runtime_minutes,
             content_status = excluded.content_status,
             studio = excluded.studio,
             digital_release_date = excluded.digital_release_date,
             imdb_id = COALESCE(excluded.imdb_id, movie_entities.imdb_id),
             tvdb_id = COALESCE(excluded.tvdb_id, movie_entities.tvdb_id),
             tmdb_id = COALESCE(excluded.tmdb_id, movie_entities.tmdb_id),
             mal_id = COALESCE(excluded.mal_id, movie_entities.mal_id),
             anidb_id = COALESCE(excluded.anidb_id, movie_entities.anidb_id),
             updated_at = excluded.updated_at",
        &[
            SqlArg::Text(movie.id.clone()),
            SqlArg::Text(movie.title.clone()),
            SqlArg::OptText(movie.sort_title.clone()),
            SqlArg::OptText(movie.slug.clone()),
            SqlArg::OptI32(movie.year),
            SqlArg::OptText(movie.overview.clone()),
            SqlArg::OptText(movie.poster_url.clone()),
            SqlArg::OptText(movie.background_url.clone()),
            SqlArg::OptText(movie.language.clone()),
            SqlArg::OptI32(movie.runtime_minutes),
            SqlArg::OptText(movie.content_status.clone()),
            SqlArg::OptText(movie.studio.clone()),
            SqlArg::OptText(movie.digital_release_date.clone()),
            SqlArg::OptText(movie.imdb_id.clone()),
            SqlArg::OptText(movie.tvdb_id.clone()),
            SqlArg::OptText(movie.tmdb_id.clone()),
            SqlArg::OptText(movie.mal_id.clone()),
            SqlArg::OptText(movie.anidb_id.clone()),
            SqlArg::Timestamp(movie.created_at),
            SqlArg::Timestamp(movie.updated_at),
        ],
    )
    .await?;
    if let Some(ratings) = &movie.ratings {
        replace_movie_entity_metadata_ratings_tx(tx, &movie.id, ratings).await?;
    }
    if let Some(credits) = &movie.credits {
        replace_movie_entity_credits_tx(tx, &movie.id, credits).await?;
    }
    Ok(())
}

async fn find_existing_movie_entity_id_tx(
    tx: &mut SqlTx<'_>,
    movie: &MovieEntity,
) -> AppResult<Option<String>> {
    for (column, value) in [
        ("tvdb_id", movie.tvdb_id.as_deref()),
        ("tmdb_id", movie.tmdb_id.as_deref()),
        ("imdb_id", movie.imdb_id.as_deref()),
        ("mal_id", movie.mal_id.as_deref()),
        ("anidb_id", movie.anidb_id.as_deref()),
    ] {
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            continue;
        };
        let sql = format!("SELECT id FROM movie_entities WHERE {column} = {{}} LIMIT 1");
        if let Some(row) =
            SqlRuntime::fetch_optional(SqlExec::Tx(tx), &sql, &[SqlArg::Text(value.to_string())])
                .await?
        {
            return row.opt_text("id");
        }
    }

    let title = movie.title.trim();
    if title.is_empty() {
        return Ok(None);
    }
    let (sql, args) = if let Some(year) = movie.year {
        (
            "SELECT id FROM movie_entities WHERE lower(title) = lower({}) AND year = {} LIMIT 1",
            vec![SqlArg::Text(title.to_string()), SqlArg::I32(year)],
        )
    } else {
        (
            "SELECT id FROM movie_entities WHERE lower(title) = lower({}) AND year IS NULL LIMIT 1",
            vec![SqlArg::Text(title.to_string())],
        )
    };
    let row = SqlRuntime::fetch_optional(SqlExec::Tx(tx), sql, &args).await?;
    row.as_ref().map(|row| row.text("id")).transpose()
}

async fn find_existing_series_movie_link_id_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    movie_id: &str,
) -> AppResult<Option<String>> {
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id FROM series_movie_links WHERE series_title_id = {} AND movie_entity_id = {} LIMIT 1",
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(movie_id.to_string()),
        ],
    )
    .await?;
    row.as_ref().map(|row| row.text("id")).transpose()
}

async fn load_series_movie_link_tx(
    tx: &mut SqlTx<'_>,
    link_id: &str,
) -> AppResult<Option<SeriesMovieLink>> {
    let sql = format!(
        "SELECT {SERIES_MOVIE_LINK_COLUMNS}
         FROM series_movie_links sml
         INNER JOIN movie_entities me ON me.id = sml.movie_entity_id
         WHERE sml.id = {{}}
         LIMIT 1"
    );
    let row =
        SqlRuntime::fetch_optional(SqlExec::Tx(tx), &sql, &[SqlArg::Text(link_id.to_string())])
            .await?;
    let mut link = row.as_ref().map(row_to_series_movie_link).transpose()?;
    if let Some(link) = &mut link {
        attach_movie_entity_ratings(SqlExec::Tx(tx), std::slice::from_mut(link)).await?;
    }
    Ok(link)
}

async fn delete_stale_series_movie_links_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    retained_link_ids: &[String],
) -> AppResult<()> {
    let now = chrono::Utc::now();
    if retained_link_ids.is_empty() {
        tx.execute(
            "UPDATE series_movie_links
             SET metadata_active = {},
                 monitored = CASE WHEN monitoring_override IS NULL THEN {} ELSE monitored END,
                 updated_at = {}
             WHERE series_title_id = {} AND COALESCE(source, '') = 'anibridge'",
            &[
                SqlArg::Bool(false),
                SqlArg::Bool(false),
                SqlArg::Timestamp(now),
                SqlArg::Text(title_id.to_string()),
            ],
        )
        .await?;
        return Ok(());
    }

    let placeholders = bind_placeholders(retained_link_ids.len());
    let sql = format!(
        "UPDATE series_movie_links
         SET metadata_active = {{}},
             monitored = CASE WHEN monitoring_override IS NULL THEN {{}} ELSE monitored END,
             updated_at = {{}}
         WHERE series_title_id = {{}}
           AND COALESCE(source, '') = 'anibridge'
           AND id NOT IN ({placeholders})"
    );
    let mut args = Vec::with_capacity(retained_link_ids.len() + 4);
    args.push(SqlArg::Bool(false));
    args.push(SqlArg::Bool(false));
    args.push(SqlArg::Timestamp(now));
    args.push(SqlArg::Text(title_id.to_string()));
    args.extend(retained_link_ids.iter().cloned().map(SqlArg::Text));
    tx.execute(&sql, &args).await?;
    Ok(())
}

async fn list_collections_for_title_query(
    target: SqlTarget<'_>,
    title_id: &str,
) -> AppResult<Vec<Collection>> {
    let sql = format!(
        "SELECT {COLLECTION_COLUMNS} FROM collections WHERE title_id = {{}} ORDER BY collection_index ASC, id ASC"
    );
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(target),
        &sql,
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_collection).collect()
}

async fn list_collection_external_ids_query(
    target: SqlTarget<'_>,
    collection_id: &str,
) -> AppResult<Vec<ScopedExternalId>> {
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(target),
        "SELECT collection_id AS scope_id, source, external_id, provenance, source_scope \
         FROM collection_external_ids WHERE collection_id = {} \
         ORDER BY source ASC, external_id ASC, source_scope ASC",
        &[SqlArg::Text(collection_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_scoped_external_id).collect()
}

async fn list_collections_for_titles_query(
    target: SqlTarget<'_>,
    title_ids: &[String],
) -> AppResult<HashMap<String, Vec<Collection>>> {
    if title_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = bind_placeholders(title_ids.len());
    let sql = format!(
        "SELECT {COLLECTION_COLUMNS} FROM collections WHERE title_id IN ({placeholders}) ORDER BY title_id ASC, collection_index ASC, id ASC"
    );
    let args = title_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(SqlExec::Target(target), &sql, &args).await?;

    let mut grouped = HashMap::<String, Vec<Collection>>::new();
    for row in &rows {
        let collection = row_to_collection(row)?;
        grouped
            .entry(collection.title_id.clone())
            .or_default()
            .push(collection);
    }

    Ok(grouped)
}

async fn list_primary_collection_summaries_query(
    target: SqlTarget<'_>,
    title_ids: &[String],
) -> AppResult<Vec<PrimaryCollectionSummary>> {
    if title_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = bind_placeholders(title_ids.len());
    let sql = format!(
        "SELECT title_id, collection_type, collection_index, label, ordered_path FROM collections \
         WHERE title_id IN ({placeholders}) AND (collection_index = '0' OR collection_type = 'movie')"
    );
    let args = title_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(SqlExec::Target(target), &sql, &args).await?;

    let mut candidates = rows
        .iter()
        .map(summary_candidate_from_row)
        .collect::<AppResult<Vec<_>>>()?;
    candidates.sort_by_key(summary_candidate_sort_key);

    let mut seen = HashSet::new();
    let mut summaries = Vec::new();
    for candidate in candidates {
        if seen.contains(candidate.title_id.as_str()) {
            continue;
        }
        if !summary_candidate_should_include(&candidate) {
            continue;
        }
        seen.insert(candidate.title_id.clone());
        summaries.push(PrimaryCollectionSummary {
            title_id: candidate.title_id,
            label: candidate.label,
            ordered_path: candidate.ordered_path,
        });
    }

    Ok(summaries)
}

async fn get_collection_by_id_query(
    target: SqlTarget<'_>,
    collection_id: &str,
) -> AppResult<Option<Collection>> {
    let sql = format!("SELECT {COLLECTION_COLUMNS} FROM collections WHERE id = {{}}");
    let row = SqlRuntime::fetch_optional(
        SqlExec::Target(target),
        &sql,
        &[SqlArg::Text(collection_id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_collection).transpose()
}

async fn get_collections_by_ids_query(
    target: SqlTarget<'_>,
    ids: &[String],
) -> AppResult<Vec<Collection>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = bind_placeholders(ids.len());
    let sql = format!("SELECT {COLLECTION_COLUMNS} FROM collections WHERE id IN ({placeholders})");
    let args = ids.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(SqlExec::Target(target), &sql, &args).await?;
    rows.iter().map(row_to_collection).collect()
}

async fn get_collection_by_ordered_path_query(
    target: SqlTarget<'_>,
    ordered_path: &str,
) -> AppResult<Option<Collection>> {
    let sql = format!(
        "SELECT {COLLECTION_COLUMNS} FROM collections WHERE ordered_path = {{}} ORDER BY id ASC LIMIT 1"
    );
    let row = SqlRuntime::fetch_optional(
        SqlExec::Target(target),
        &sql,
        &[SqlArg::Text(ordered_path.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_collection).transpose()
}

async fn list_collections_by_ordered_paths_query(
    target: SqlTarget<'_>,
    ordered_paths: &[String],
) -> AppResult<Vec<Collection>> {
    if ordered_paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut collections = Vec::new();
    for chunk in ordered_paths.chunks(SQL_BIND_CHUNK) {
        let placeholders = bind_placeholders(chunk.len());
        let sql = format!(
            "SELECT {COLLECTION_COLUMNS} FROM collections WHERE ordered_path IN ({placeholders}) ORDER BY id ASC"
        );
        let args = chunk.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
        let rows = SqlRuntime::fetch_all(SqlExec::Target(target), &sql, &args).await?;
        for row in &rows {
            collections.push(row_to_collection(row)?);
        }
    }
    Ok(collections)
}

async fn list_episodes_for_collection_query(
    target: SqlTarget<'_>,
    collection_id: &str,
) -> AppResult<Vec<Episode>> {
    let sql = format!(
        "SELECT {EPISODE_COLUMNS} FROM episodes WHERE collection_id = {{}} ORDER BY episode_number ASC, id ASC"
    );
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(target),
        &sql,
        &[SqlArg::Text(collection_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_episode).collect()
}

async fn list_episodes_for_collections_query(
    target: SqlTarget<'_>,
    collection_ids: &[String],
) -> AppResult<Vec<Episode>> {
    if collection_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = bind_placeholders(collection_ids.len());
    let sql = format!(
        "SELECT {EPISODE_COLUMNS} FROM episodes WHERE collection_id IN ({placeholders}) ORDER BY collection_id ASC, episode_number ASC, id ASC"
    );
    let args = collection_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(SqlExec::Target(target), &sql, &args).await?;
    rows.iter().map(row_to_episode).collect()
}

async fn list_episodes_for_title_query(
    target: SqlTarget<'_>,
    title_id: &str,
) -> AppResult<Vec<Episode>> {
    let sql = format!(
        "SELECT {EPISODE_COLUMNS} FROM episodes WHERE title_id = {{}} ORDER BY season_number ASC, episode_number ASC, id ASC"
    );
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(target),
        &sql,
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_episode).collect()
}

async fn list_episode_external_ids_query(
    target: SqlTarget<'_>,
    episode_id: &str,
) -> AppResult<Vec<ScopedExternalId>> {
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(target),
        "SELECT episode_id AS scope_id, source, external_id, provenance, source_scope \
         FROM episode_external_ids WHERE episode_id = {} \
         ORDER BY source ASC, external_id ASC, source_scope ASC",
        &[SqlArg::Text(episode_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_scoped_external_id).collect()
}

async fn get_episode_by_id_query(
    target: SqlTarget<'_>,
    episode_id: &str,
) -> AppResult<Option<Episode>> {
    let sql = format!("SELECT {EPISODE_COLUMNS} FROM episodes WHERE id = {{}}");
    let row = SqlRuntime::fetch_optional(
        SqlExec::Target(target),
        &sql,
        &[SqlArg::Text(episode_id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_episode).transpose()
}

async fn get_episodes_by_ids_query(
    target: SqlTarget<'_>,
    ids: &[String],
) -> AppResult<Vec<Episode>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = bind_placeholders(ids.len());
    let sql = format!("SELECT {EPISODE_COLUMNS} FROM episodes WHERE id IN ({placeholders})");
    let args = ids.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(SqlExec::Target(target), &sql, &args).await?;
    rows.iter().map(row_to_episode).collect()
}

async fn find_episode_by_title_and_numbers_query(
    target: SqlTarget<'_>,
    title_id: &str,
    season_number: &str,
    episode_number: &str,
) -> AppResult<Option<Episode>> {
    let sql = "SELECT e.id, e.title_id, e.collection_id, e.episode_type, e.episode_number, \
               e.season_number, e.episode_label, e.title, e.air_date, e.duration_seconds, \
               e.has_multi_audio, e.has_subtitle, e.is_filler, e.is_recap, e.absolute_number, \
               e.overview, e.tvdb_id, e.image_url, e.monitored, e.created_at \
          FROM episodes e \
          INNER JOIN collections c ON c.id = e.collection_id \
         WHERE e.title_id = {} \
           AND c.collection_index = {} \
           AND e.episode_number = {} \
         LIMIT 1";
    let row = SqlRuntime::fetch_optional(
        SqlExec::Target(target),
        sql,
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(season_number.to_string()),
            SqlArg::Text(episode_number.to_string()),
        ],
    )
    .await?;
    row.as_ref().map(row_to_episode).transpose()
}

async fn find_episode_by_title_and_absolute_number_query(
    target: SqlTarget<'_>,
    title_id: &str,
    absolute_number: &str,
) -> AppResult<Option<Episode>> {
    let sql = format!(
        "SELECT {EPISODE_COLUMNS} FROM episodes WHERE title_id = {{}} AND absolute_number = {{}} LIMIT 1"
    );
    let row = SqlRuntime::fetch_optional(
        SqlExec::Target(target),
        &sql,
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(absolute_number.to_string()),
        ],
    )
    .await?;
    row.as_ref().map(row_to_episode).transpose()
}

async fn list_episodes_in_date_range_query(
    target: SqlTarget<'_>,
    start_date: &str,
    end_date: &str,
) -> AppResult<Vec<CalendarEpisode>> {
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(target),
        "SELECT e.id, e.title_id, t.library_id, l.name AS library_name, l.slug AS library_slug, \
                t.name AS title_name, t.slug AS title_slug, t.facet AS title_facet, \
                e.season_number, e.episode_number, e.title AS episode_title, \
                CASE WHEN t.facet = 'movie' THEN t.overview ELSE COALESCE(e.overview, t.overview) END AS overview, \
                CASE WHEN t.facet = 'movie' THEN t.poster_url ELSE e.image_url END AS image_url, \
                e.air_date, (e.monitored AND t.monitored) AS monitored \
           FROM episodes e \
           JOIN titles t ON e.title_id = t.id \
           LEFT JOIN libraries l ON l.id = t.library_id \
          WHERE e.air_date IS NOT NULL AND e.air_date != '' \
            AND e.air_date >= {} AND e.air_date <= {} \
          ORDER BY e.air_date ASC",
        &[
            SqlArg::Text(start_date.to_string()),
            SqlArg::Text(end_date.to_string()),
        ],
    )
    .await?;
    rows.iter().map(row_to_calendar_episode).collect()
}

async fn load_collection_tx(
    tx: &mut SqlTx<'_>,
    collection_id: &str,
) -> AppResult<Option<Collection>> {
    let sql = format!("SELECT {COLLECTION_COLUMNS} FROM collections WHERE id = {{}}");
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        &sql,
        &[SqlArg::Text(collection_id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_collection).transpose()
}

async fn insert_collection_tx(tx: &mut SqlTx<'_>, collection: &Collection) -> AppResult<()> {
    let args = vec![
        SqlArg::Text(collection.id.clone()),
        SqlArg::Text(collection.title_id.clone()),
        SqlArg::Text(collection.collection_type.as_str().to_string()),
        SqlArg::Text(collection.collection_index.clone()),
        SqlArg::OptText(collection.label.clone()),
        SqlArg::OptText(collection.ordered_path.clone()),
        SqlArg::OptText(collection.narrative_order.clone()),
        SqlArg::OptText(collection.first_episode_number.clone()),
        SqlArg::OptText(collection.last_episode_number.clone()),
        SqlArg::Bool(collection.monitored),
        SqlArg::Timestamp(collection.created_at),
    ];

    tx.execute(COLLECTION_INSERT_SQL, &args).await?;

    Ok(())
}

async fn persist_collection_tx(tx: &mut SqlTx<'_>, collection: &Collection) -> AppResult<()> {
    let args = vec![
        SqlArg::Text(collection.title_id.clone()),
        SqlArg::Text(collection.collection_type.as_str().to_string()),
        SqlArg::Text(collection.collection_index.clone()),
        SqlArg::OptText(collection.label.clone()),
        SqlArg::OptText(collection.ordered_path.clone()),
        SqlArg::OptText(collection.narrative_order.clone()),
        SqlArg::OptText(collection.first_episode_number.clone()),
        SqlArg::OptText(collection.last_episode_number.clone()),
        SqlArg::Bool(collection.monitored),
    ];

    let mut args = args;
    args.push(SqlArg::Text(collection.id.clone()));
    tx.execute(COLLECTION_UPDATE_SQL, &args).await?;

    Ok(())
}

async fn set_collection_episodes_monitored_tx(
    tx: &mut SqlTx<'_>,
    collection_id: &str,
    monitored: bool,
) -> AppResult<()> {
    tx.execute(
        "UPDATE episodes SET monitored = {} WHERE collection_id = {}",
        &[
            SqlArg::Bool(monitored),
            SqlArg::Text(collection_id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

async fn set_collections_monitored_tx(
    tx: &mut SqlTx<'_>,
    collection_ids: &[String],
    monitored: bool,
) -> AppResult<()> {
    if collection_ids.is_empty() {
        return Ok(());
    }

    let placeholders = bind_placeholders(collection_ids.len());
    let sql = format!("UPDATE collections SET monitored = {{}} WHERE id IN ({placeholders})");
    let mut args = vec![SqlArg::Bool(monitored)];
    args.extend(collection_ids.iter().cloned().map(SqlArg::Text));
    tx.execute(&sql, &args).await?;
    Ok(())
}

async fn delete_collection_tx(tx: &mut SqlTx<'_>, collection_id: &str) -> AppResult<()> {
    let rows = tx
        .execute(
            "DELETE FROM collections WHERE id = {}",
            &[SqlArg::Text(collection_id.to_string())],
        )
        .await?;
    if rows == 0 {
        return Err(AppError::NotFound(format!("collection {collection_id}")));
    }
    Ok(())
}

async fn delete_collections_for_title_tx(tx: &mut SqlTx<'_>, title_id: &str) -> AppResult<()> {
    tx.execute(
        "DELETE FROM collections WHERE title_id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    Ok(())
}

async fn load_episode_tx(tx: &mut SqlTx<'_>, episode_id: &str) -> AppResult<Option<Episode>> {
    let sql = format!("SELECT {EPISODE_COLUMNS} FROM episodes WHERE id = {{}}");
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        &sql,
        &[SqlArg::Text(episode_id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_episode).transpose()
}

async fn insert_episode_tx(tx: &mut SqlTx<'_>, episode: &Episode) -> AppResult<()> {
    let args = vec![
        SqlArg::Text(episode.id.clone()),
        SqlArg::Text(episode.title_id.clone()),
        SqlArg::OptText(episode.collection_id.clone()),
        SqlArg::Text(episode.episode_type.as_str().to_string()),
        SqlArg::OptText(episode.episode_number.clone()),
        SqlArg::OptText(episode.season_number.clone()),
        SqlArg::OptText(episode.episode_label.clone()),
        SqlArg::OptText(episode.title.clone()),
        SqlArg::OptText(episode.air_date.clone()),
        SqlArg::OptI64(episode.duration_seconds),
        SqlArg::Bool(episode.has_multi_audio),
        SqlArg::Bool(episode.has_subtitle),
        SqlArg::Bool(episode.is_filler),
        SqlArg::Bool(episode.is_recap),
        SqlArg::OptText(episode.absolute_number.clone()),
        SqlArg::OptText(episode.overview.clone()),
        SqlArg::OptText(episode.tvdb_id.clone()),
        SqlArg::OptText(episode.image_url.clone()),
        SqlArg::Bool(episode.monitored),
        SqlArg::Timestamp(episode.created_at),
    ];

    tx.execute(EPISODE_INSERT_SQL, &args).await?;

    Ok(())
}

async fn persist_episode_tx(tx: &mut SqlTx<'_>, episode: &Episode) -> AppResult<()> {
    let args = vec![
        SqlArg::Text(episode.title_id.clone()),
        SqlArg::OptText(episode.collection_id.clone()),
        SqlArg::Text(episode.episode_type.as_str().to_string()),
        SqlArg::OptText(episode.episode_number.clone()),
        SqlArg::OptText(episode.season_number.clone()),
        SqlArg::OptText(episode.episode_label.clone()),
        SqlArg::OptText(episode.title.clone()),
        SqlArg::OptText(episode.air_date.clone()),
        SqlArg::OptI64(episode.duration_seconds),
        SqlArg::Bool(episode.has_multi_audio),
        SqlArg::Bool(episode.has_subtitle),
        SqlArg::Bool(episode.is_filler),
        SqlArg::Bool(episode.is_recap),
        SqlArg::OptText(episode.absolute_number.clone()),
        SqlArg::OptText(episode.overview.clone()),
        SqlArg::OptText(episode.tvdb_id.clone()),
        SqlArg::OptText(episode.image_url.clone()),
        SqlArg::Bool(episode.monitored),
    ];

    let mut args = args;
    args.push(SqlArg::Text(episode.id.clone()));
    tx.execute(EPISODE_UPDATE_SQL, &args).await?;

    Ok(())
}

async fn set_episodes_monitored_tx(
    tx: &mut SqlTx<'_>,
    episode_ids: &[String],
    monitored: bool,
) -> AppResult<()> {
    if episode_ids.is_empty() {
        return Ok(());
    }

    let placeholders = bind_placeholders(episode_ids.len());
    let sql = format!("UPDATE episodes SET monitored = {{}} WHERE id IN ({placeholders})");
    let mut args = vec![SqlArg::Bool(monitored)];
    args.extend(episode_ids.iter().cloned().map(SqlArg::Text));
    tx.execute(&sql, &args).await?;
    Ok(())
}

async fn delete_episode_tx(tx: &mut SqlTx<'_>, episode_id: &str) -> AppResult<()> {
    let rows = tx
        .execute(
            "DELETE FROM episodes WHERE id = {}",
            &[SqlArg::Text(episode_id.to_string())],
        )
        .await?;
    if rows == 0 {
        return Err(AppError::NotFound(format!("episode {episode_id}")));
    }
    Ok(())
}

async fn delete_episodes_for_title_tx(tx: &mut SqlTx<'_>, title_id: &str) -> AppResult<()> {
    tx.execute(
        "DELETE FROM episodes WHERE title_id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    Ok(())
}

async fn replace_anibridge_scoped_external_ids_for_title_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    collection_ids: &[ScopedExternalId],
    episode_ids: &[ScopedExternalId],
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM collection_external_ids WHERE title_id = {} AND provenance = 'anibridge'",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    tx.execute(
        "DELETE FROM episode_external_ids WHERE title_id = {} AND provenance = 'anibridge'",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;

    let now = Utc::now();
    match tx {
        SqlTx::Sqlite(_) => {
            insert_scoped_collection_ids_sqlite(tx, title_id, collection_ids, now).await?;
            insert_scoped_episode_ids_sqlite(tx, title_id, episode_ids, now).await?;
        }
        SqlTx::Postgres(_) => {
            insert_scoped_collection_ids_postgres(tx, title_id, collection_ids, now).await?;
            insert_scoped_episode_ids_postgres(tx, title_id, episode_ids, now).await?;
        }
    }

    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SummaryCandidate {
    title_id: String,
    collection_type: CollectionType,
    collection_index: String,
    label: Option<String>,
    ordered_path: Option<String>,
}

fn collection_update_is_empty(update: &CollectionUpdate) -> bool {
    update.collection_type.is_none()
        && update.collection_index.is_none()
        && update.label.is_none()
        && update.ordered_path.is_none()
        && !update.clear_ordered_path
        && update.first_episode_number.is_none()
        && update.last_episode_number.is_none()
        && update.monitored.is_none()
}

fn apply_collection_update(collection: &mut Collection, update: CollectionUpdate) {
    if let Some(value) = update.collection_type {
        collection.collection_type = value;
    }
    if let Some(value) = update.collection_index {
        collection.collection_index = value;
    }
    if let Some(value) = update.label {
        collection.label = Some(value);
    }
    if update.clear_ordered_path {
        collection.ordered_path = None;
    } else if let Some(value) = update.ordered_path {
        collection.ordered_path = Some(value);
    }
    if let Some(value) = update.first_episode_number {
        collection.first_episode_number = Some(value);
    }
    if let Some(value) = update.last_episode_number {
        collection.last_episode_number = Some(value);
    }
    if let Some(value) = update.monitored {
        collection.monitored = value;
    }
}

fn episode_update_is_empty(update: &EpisodeUpdate) -> bool {
    update.episode_type.is_none()
        && update.episode_number.is_none()
        && update.season_number.is_none()
        && update.episode_label.is_none()
        && update.title.is_none()
        && update.air_date.is_none()
        && update.duration_seconds.is_none()
        && update.has_multi_audio.is_none()
        && update.has_subtitle.is_none()
        && update.monitored.is_none()
        && update.collection_id.is_none()
        && update.overview.is_none()
        && update.tvdb_id.is_none()
        && update.image_url.is_none()
        && !update.clear_image_url
}

fn apply_episode_update(episode: &mut Episode, update: EpisodeUpdate) {
    if let Some(value) = update.episode_type {
        episode.episode_type = value;
    }
    if let Some(value) = update.episode_number {
        episode.episode_number = Some(value);
    }
    if let Some(value) = update.season_number {
        episode.season_number = Some(value);
    }
    if let Some(value) = update.episode_label {
        episode.episode_label = Some(value);
    }
    if let Some(value) = update.title {
        episode.title = Some(value);
    }
    if let Some(value) = update.air_date {
        episode.air_date = Some(value);
    }
    if let Some(value) = update.duration_seconds {
        episode.duration_seconds = Some(value);
    }
    if let Some(value) = update.has_multi_audio {
        episode.has_multi_audio = value;
    }
    if let Some(value) = update.has_subtitle {
        episode.has_subtitle = value;
    }
    if let Some(value) = update.monitored {
        episode.monitored = value;
    }
    if let Some(value) = update.collection_id {
        episode.collection_id = Some(value);
    }
    if let Some(value) = update.overview {
        episode.overview = Some(value);
    }
    if let Some(value) = update.tvdb_id {
        episode.tvdb_id = Some(value);
    }
    if update.clear_image_url {
        episode.image_url = None;
    } else if let Some(value) = update.image_url {
        episode.image_url = Some(value);
    }
}

fn normalized_scoped_external_id(
    scoped_id: &ScopedExternalId,
) -> Option<(String, String, String, String)> {
    let scope_id = scoped_id.scope_id.trim();
    let source = scoped_id.source.trim().to_ascii_lowercase();
    let external_id = scoped_id.external_id.trim();
    if scope_id.is_empty() || source.is_empty() || external_id.is_empty() {
        return None;
    }

    let source_scope = scoped_id
        .source_scope
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();

    Some((
        scope_id.to_string(),
        source,
        external_id.to_string(),
        source_scope,
    ))
}

fn summary_candidate_from_row(row: &SqlRow) -> AppResult<SummaryCandidate> {
    Ok(SummaryCandidate {
        title_id: row.text("title_id")?,
        collection_type: CollectionType::parse(&row.text("collection_type")?).unwrap_or_default(),
        collection_index: row.text("collection_index")?,
        label: row.opt_text("label")?,
        ordered_path: row.opt_text("ordered_path")?,
    })
}

fn summary_candidate_should_include(candidate: &SummaryCandidate) -> bool {
    if candidate.collection_type == CollectionType::Movie {
        return true;
    }
    candidate.collection_index.trim() == "0"
}

fn summary_candidate_sort_key(candidate: &SummaryCandidate) -> (String, bool, bool, u32, String) {
    (
        candidate.title_id.clone(),
        candidate.collection_type != CollectionType::Movie,
        candidate
            .ordered_path
            .as_deref()
            .is_none_or(|path| path.trim().is_empty()),
        candidate
            .collection_index
            .parse::<u32>()
            .unwrap_or(u32::MAX),
        candidate.collection_index.clone(),
    )
}

fn bind_placeholders(count: usize) -> String {
    std::iter::repeat_n("{}", count)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Binds per `IN (...)` lookup, kept at or below sqlite's historical 999
/// variable ceiling and postgres' 65535-parameter wire limit.
const SQL_BIND_CHUNK: usize = 900;

fn row_to_scoped_external_id(row: &SqlRow) -> AppResult<ScopedExternalId> {
    let source_scope = row.opt_text("source_scope")?.unwrap_or_default();
    Ok(ScopedExternalId {
        scope_id: row.text("scope_id")?,
        source: row.text("source")?,
        external_id: row.text("external_id")?,
        provenance: row.text("provenance")?,
        source_scope: if source_scope.trim().is_empty() {
            None
        } else {
            Some(source_scope)
        },
    })
}

fn row_to_collection(row: &SqlRow) -> AppResult<Collection> {
    let collection_type = CollectionType::parse(&row.text("collection_type")?).unwrap_or_default();
    Ok(Collection {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        collection_type,
        collection_index: row.text("collection_index")?,
        label: row.opt_text("label")?,
        ordered_path: row.opt_text("ordered_path")?,
        narrative_order: row.opt_text("narrative_order")?,
        first_episode_number: row.opt_text("first_episode_number")?,
        last_episode_number: row.opt_text("last_episode_number")?,
        monitored: row.bool("monitored")?,
        created_at: row.timestamp("created_at")?,
    })
}

async fn attach_movie_entity_ratings(
    exec: SqlExec<'_, '_>,
    links: &mut [SeriesMovieLink],
) -> AppResult<()> {
    let movie_entity_ids = links
        .iter()
        .map(|link| link.movie.id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let ratings = load_movie_entity_metadata_ratings(exec, &movie_entity_ids).await?;
    for link in links {
        link.movie.ratings = ratings.get(&link.movie.id).cloned();
    }
    Ok(())
}

fn row_to_series_movie_link(row: &SqlRow) -> AppResult<SeriesMovieLink> {
    Ok(SeriesMovieLink {
        id: row.text("link_id")?,
        series_title_id: row.text("series_title_id")?,
        movie: MovieEntity {
            id: row.text("movie_id")?,
            title: row.text("movie_title")?,
            sort_title: row.opt_text("movie_sort_title")?,
            slug: row.opt_text("movie_slug")?,
            year: row.opt_i32("movie_year")?,
            overview: row.opt_text("movie_overview")?,
            poster_url: row.opt_text("movie_poster_url")?,
            background_url: row.opt_text("movie_background_url")?,
            language: row.opt_text("movie_language")?,
            runtime_minutes: row.opt_i32("movie_runtime_minutes")?,
            content_status: row.opt_text("movie_content_status")?,
            studio: row.opt_text("movie_studio")?,
            digital_release_date: row.opt_text("movie_digital_release_date")?,
            imdb_id: row.opt_text("movie_imdb_id")?,
            tvdb_id: row.opt_text("movie_tvdb_id")?,
            tmdb_id: row.opt_text("movie_tmdb_id")?,
            mal_id: row.opt_text("movie_mal_id")?,
            anidb_id: row.opt_text("movie_anidb_id")?,
            ratings: None,
            credits: None,
            created_at: row.timestamp("movie_created_at")?,
            updated_at: row.timestamp("movie_updated_at")?,
        },
        placement: row.opt_text("placement")?,
        narrative_order: row.opt_text("narrative_order")?,
        after_season: row.opt_i32("after_season")?,
        before_season: row.opt_i32("before_season")?,
        linked_episode_id: row.opt_text("linked_episode_id")?,
        association_confidence: row.opt_text("association_confidence")?,
        continuity_status: row.opt_text("continuity_status")?,
        movie_form: row.opt_text("movie_form")?,
        confidence: row.opt_text("confidence")?,
        signal_summary: row.opt_text("signal_summary")?,
        source: row.opt_text("source")?,
        monitoring_override: row.opt_bool("monitoring_override")?,
        metadata_active: row.bool("metadata_active")?,
        monitored: row.bool("link_monitored")?,
        legacy_collection_id: row.opt_text("legacy_collection_id")?,
        tags: decode_series_movie_link_tags(row)?,
        created_at: row.timestamp("link_created_at")?,
        updated_at: row.timestamp("link_updated_at")?,
    })
}

/// The link's tag bag, defaulting to empty.
///
/// A row written before migration 0218 reads as `[]` from the column default,
/// and a bag that is not an array of strings is a corrupt row rather than a
/// reason to fail the whole read: the link is still a real link, so it comes
/// back untagged and the next tag write rewrites the column.
fn decode_series_movie_link_tags(row: &SqlRow) -> AppResult<Vec<String>> {
    Ok(row
        .opt_json("link_tags")?
        .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
        .unwrap_or_default())
}

fn row_to_episode(row: &SqlRow) -> AppResult<Episode> {
    let episode_type = EpisodeType::parse(&row.text("episode_type")?).unwrap_or_default();
    Ok(Episode {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        collection_id: row.opt_text("collection_id")?,
        episode_type,
        episode_number: row.opt_text("episode_number")?,
        season_number: row.opt_text("season_number")?,
        episode_label: row.opt_text("episode_label")?,
        title: row.opt_text("title")?,
        air_date: row.opt_text("air_date")?,
        duration_seconds: row.opt_i64("duration_seconds")?,
        has_multi_audio: row.bool("has_multi_audio")?,
        has_subtitle: row.bool("has_subtitle")?,
        is_filler: row.opt_bool("is_filler")?.unwrap_or(false),
        is_recap: row.opt_bool("is_recap")?.unwrap_or(false),
        absolute_number: row.opt_text("absolute_number")?,
        overview: row.opt_text("overview")?,
        tvdb_id: row.opt_text("tvdb_id")?,
        image_url: row.opt_text("image_url")?,
        monitored: row.bool("monitored")?,
        created_at: row.timestamp("created_at")?,
    })
}

fn row_to_calendar_episode(row: &SqlRow) -> AppResult<CalendarEpisode> {
    Ok(CalendarEpisode {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        library_id: row.text("library_id")?,
        library_name: row.opt_text("library_name")?,
        library_slug: row.opt_text("library_slug")?,
        title_name: row.text("title_name")?,
        title_slug: row.opt_text("title_slug")?,
        title_facet: row.text("title_facet")?,
        season_number: row.opt_text("season_number")?,
        episode_number: row.opt_text("episode_number")?,
        episode_title: row.opt_text("episode_title")?,
        overview: row.opt_text("overview")?,
        image_url: row.opt_text("image_url")?,
        air_date: row.opt_text("air_date")?,
        monitored: row.bool("monitored")?,
    })
}

async fn insert_scoped_collection_ids_sqlite(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    collection_ids: &[ScopedExternalId],
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    for scoped_id in collection_ids {
        let Some((collection_id, source, external_id, source_scope)) =
            normalized_scoped_external_id(scoped_id)
        else {
            continue;
        };
        tx.execute(
            "INSERT OR IGNORE INTO collection_external_ids \
             (id, title_id, collection_id, source, external_id, provenance, source_scope, created_at, updated_at) \
             VALUES ({}, {}, {}, {}, {}, 'anibridge', {}, {}, {})",
            &[
                SqlArg::Text(Id::new().0),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(collection_id),
                SqlArg::Text(source),
                SqlArg::Text(external_id),
                SqlArg::Text(source_scope),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_scoped_episode_ids_sqlite(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    episode_ids: &[ScopedExternalId],
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    for scoped_id in episode_ids {
        let Some((episode_id, source, external_id, source_scope)) =
            normalized_scoped_external_id(scoped_id)
        else {
            continue;
        };
        tx.execute(
            "INSERT OR IGNORE INTO episode_external_ids \
             (id, title_id, episode_id, source, external_id, provenance, source_scope, created_at, updated_at) \
             VALUES ({}, {}, {}, {}, {}, 'anibridge', {}, {}, {})",
            &[
                SqlArg::Text(Id::new().0),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(episode_id),
                SqlArg::Text(source),
                SqlArg::Text(external_id),
                SqlArg::Text(source_scope),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_scoped_collection_ids_postgres(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    collection_ids: &[ScopedExternalId],
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    for scoped_id in collection_ids {
        let Some((collection_id, source, external_id, source_scope)) =
            normalized_scoped_external_id(scoped_id)
        else {
            continue;
        };
        tx.execute(
            "INSERT INTO collection_external_ids \
             (id, title_id, collection_id, source, external_id, provenance, source_scope, created_at, updated_at) \
             VALUES ({}, {}, {}, {}, {}, 'anibridge', {}, {}, {}) ON CONFLICT DO NOTHING",
            &[
                SqlArg::Text(Id::new().0),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(collection_id),
                SqlArg::Text(source),
                SqlArg::Text(external_id),
                SqlArg::Text(source_scope),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_scoped_episode_ids_postgres(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    episode_ids: &[ScopedExternalId],
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    for scoped_id in episode_ids {
        let Some((episode_id, source, external_id, source_scope)) =
            normalized_scoped_external_id(scoped_id)
        else {
            continue;
        };
        tx.execute(
            "INSERT INTO episode_external_ids \
             (id, title_id, episode_id, source, external_id, provenance, source_scope, created_at, updated_at) \
             VALUES ({}, {}, {}, {}, {}, 'anibridge', {}, {}, {}) ON CONFLICT DO NOTHING",
            &[
                SqlArg::Text(Id::new().0),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(episode_id),
                SqlArg::Text(source),
                SqlArg::Text(external_id),
                SqlArg::Text(source_scope),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        SqlTarget, SummaryCandidate, list_series_movie_external_id_lookup_matches_query,
        summary_candidate_sort_key,
    };
    use scryer_application::TitleExternalIdLookup;
    use scryer_domain::CollectionType;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn series_movie_external_id_lookup_respects_parent_library_access() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open SQLite pool");
        for sql in [
            "CREATE TABLE titles (id TEXT PRIMARY KEY, library_id TEXT NOT NULL)",
            "CREATE TABLE movie_entities (id TEXT PRIMARY KEY, tvdb_id TEXT)",
            "CREATE TABLE series_movie_links (series_title_id TEXT NOT NULL, movie_entity_id TEXT NOT NULL)",
        ] {
            sqlx::query(sql)
                .execute(&pool)
                .await
                .expect("create lookup table");
        }
        for (id, library_id) in [
            ("visible-series", "library-visible"),
            ("hidden-series", "library-hidden"),
        ] {
            sqlx::query("INSERT INTO titles (id, library_id) VALUES (?, ?)")
                .bind(id)
                .bind(library_id)
                .execute(&pool)
                .await
                .expect("insert series parent");
        }
        for (id, tvdb_id) in [("visible-movie", "604"), ("hidden-movie", "605")] {
            sqlx::query("INSERT INTO movie_entities (id, tvdb_id) VALUES (?, ?)")
                .bind(id)
                .bind(tvdb_id)
                .execute(&pool)
                .await
                .expect("insert movie entity");
        }
        for (series_title_id, movie_entity_id) in [
            ("visible-series", "visible-movie"),
            ("hidden-series", "hidden-movie"),
        ] {
            sqlx::query(
                "INSERT INTO series_movie_links (series_title_id, movie_entity_id) VALUES (?, ?)",
            )
            .bind(series_title_id)
            .bind(movie_entity_id)
            .execute(&pool)
            .await
            .expect("link series movie");
        }

        let matches = list_series_movie_external_id_lookup_matches_query(
            SqlTarget::Sqlite(&pool),
            &["library-visible".to_string()],
            &[
                TitleExternalIdLookup {
                    lookup_index: 3,
                    source: "tvdb".to_string(),
                    external_id: "604".to_string(),
                },
                TitleExternalIdLookup {
                    lookup_index: 4,
                    source: "tvdb_movie".to_string(),
                    external_id: "605".to_string(),
                },
            ],
        )
        .await
        .expect("lookup linked movie ownership");

        assert_eq!(
            matches
                .into_iter()
                .map(|matched| matched.lookup_index)
                .collect::<Vec<_>>(),
            vec![3]
        );
    }

    #[test]
    fn movie_collection_wins_over_index_zero_fallback() {
        let mut candidates = [
            SummaryCandidate {
                title_id: "title-1".to_string(),
                collection_type: CollectionType::Season,
                collection_index: "0".to_string(),
                label: Some("Specials".to_string()),
                ordered_path: None,
            },
            SummaryCandidate {
                title_id: "title-1".to_string(),
                collection_type: CollectionType::Movie,
                collection_index: "1".to_string(),
                label: Some("1080P".to_string()),
                ordered_path: Some("/data/movies/Movie/Movie.1080P.mkv".to_string()),
            },
        ];
        candidates.sort_by_key(summary_candidate_sort_key);

        assert_eq!(candidates[0].collection_type, CollectionType::Movie);
        assert_eq!(candidates[0].label.as_deref(), Some("1080P"));
    }

    #[test]
    fn movie_collection_with_path_wins_over_pathless_movie_collection() {
        let mut candidates = [
            SummaryCandidate {
                title_id: "title-1".to_string(),
                collection_type: CollectionType::Movie,
                collection_index: "2".to_string(),
                label: Some("2160P".to_string()),
                ordered_path: None,
            },
            SummaryCandidate {
                title_id: "title-1".to_string(),
                collection_type: CollectionType::Movie,
                collection_index: "1".to_string(),
                label: Some("1080P".to_string()),
                ordered_path: Some("/data/movies/Movie/Movie.1080P.mkv".to_string()),
            },
        ];
        candidates.sort_by_key(summary_candidate_sort_key);

        assert_eq!(candidates[0].collection_index, "1");
        assert_eq!(candidates[0].label.as_deref(), Some("1080P"));
    }
}

#[cfg(test)]
mod collection_ordered_path_tests {
    use super::{
        SqlTarget, get_collection_by_ordered_path_query, list_collections_by_ordered_paths_query,
    };
    use sqlx::sqlite::SqlitePoolOptions;

    /// `ordered_path` has no unique constraint, so the batched lookup has to
    /// resolve duplicates exactly like the single-path query it replaced:
    /// lowest id wins. Callers key the batch by first-seen, so the order this
    /// returns is the contract.
    #[tokio::test]
    async fn batched_ordered_path_lookup_matches_single_path_winner() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open SQLite pool");
        sqlx::query(
            "CREATE TABLE collections (
                id TEXT PRIMARY KEY,
                title_id TEXT NOT NULL,
                collection_type TEXT NOT NULL,
                collection_index TEXT NOT NULL,
                label TEXT,
                ordered_path TEXT,
                narrative_order TEXT,
                first_episode_number TEXT,
                last_episode_number TEXT,
                monitored INTEGER NOT NULL,
                created_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create collections table");

        // Inserted highest id first so a naive last-wins map would pick "b".
        for id in ["collection-b", "collection-a"] {
            sqlx::query(
                "INSERT INTO collections (
                    id, title_id, collection_type, collection_index, label, ordered_path,
                    narrative_order, first_episode_number, last_episode_number, monitored, created_at
                ) VALUES (?, 'title-1', 'season', '1', 'Season 1', '/media/Show/Season 1',
                    NULL, NULL, NULL, 1, '2026-01-01T00:00:00Z')",
            )
            .bind(id)
            .execute(&pool)
            .await
            .expect("insert collection");
        }

        let single =
            get_collection_by_ordered_path_query(SqlTarget::Sqlite(&pool), "/media/Show/Season 1")
                .await
                .expect("single lookup")
                .expect("collection present");
        assert_eq!(single.id, "collection-a");

        let batched = list_collections_by_ordered_paths_query(
            SqlTarget::Sqlite(&pool),
            &["/media/Show/Season 1".to_string()],
        )
        .await
        .expect("batched lookup");
        assert_eq!(batched.len(), 2, "both duplicates are returned");
        assert_eq!(
            batched.first().map(|collection| collection.id.as_str()),
            Some(single.id.as_str()),
            "the first batched row is the single-path winner"
        );
    }

    #[tokio::test]
    async fn batched_ordered_path_lookup_returns_nothing_for_empty_input() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open SQLite pool");
        let batched = list_collections_by_ordered_paths_query(SqlTarget::Sqlite(&pool), &[])
            .await
            .expect("batched lookup");
        assert!(batched.is_empty());
    }
}

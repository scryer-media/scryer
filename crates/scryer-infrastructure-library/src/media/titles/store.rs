use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, CatalogOwnedExternalIdRecord, CatalogOwnedTitleRecord, CreateTitleOutcome,
    MetadataFieldUpdate, PendingImportStatus, PendingTitleHydration, SortDirection,
    TitleArtworkUrlUpdate, TitleCatalogContentStatus, TitleCatalogFilter, TitleCatalogFilterCounts,
    TitleCatalogFilterOptions, TitleCatalogResult, TitleCatalogSort, TitleCatalogSortKey,
    TitleCatalogTagFilterOption, TitleCredit, TitleDeletePreviewInfo, TitleExternalIdLookup,
    TitleExternalIdLookupMatch, TitleMetadataUpdate, TitleOptionsPatch, TitleRatingSummary,
    TitleRepository,
    persisted_records::{
        PersistedTitleDecodeOptions, PersistedTitleReadMode, finalize_persisted_title,
    },
};
use scryer_domain::{ExternalId, MediaFacet, Title, title_catalog_sort_key};
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;
use sqlx::{QueryBuilder, Row, Sqlite, postgres::PgRow};
use std::collections::BTreeSet;
use unicode_normalization::UnicodeNormalization;

use crate::media::canonical_tags::{
    attach_metadata_tags_to_titles, load_title_metadata_ratings, load_title_metadata_tags,
    replace_title_metadata_ratings_tx, replace_title_metadata_tags_tx,
};
use crate::media::title_credits::{load_title_credits, replace_title_credits_tx};
use crate::queries::{
    common::parse_utc_datetime,
    sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err},
    title::TITLE_COLUMNS,
    title_search::{
        build_title_search_plan, delete_title_search_projection_tx, normalize_title_search_text,
        push_ranked_title_matches_cte, replace_title_search_projection_pg_tx,
        replace_title_search_projection_tx,
    },
};
use crate::title_images::normalized_base_path_from_env;

const TITLE_INSERT_SQL: &str = "INSERT INTO titles (
    id, library_id, name, facet, monitored, tags, external_ids, root_folder_id, created_by, created_at,
    year, overview, poster_url, background_url, sort_title, catalog_sort_key, slug, imdb_id,
    runtime_minutes, popularity, content_status, language, first_aired, network, studio,
    country, aliases, metadata_language, metadata_fetched_at, min_availability,
    digital_release_date, folder_path, tagged_aliases_json,
    metadata_hydration_next_attempt_at, metadata_hydration_attempt_count
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {},
    {}, {}, {}, {}
)";

const TITLE_UPSERT_SQL: &str = "INSERT INTO titles (
    id, library_id, name, facet, monitored, tags, external_ids, root_folder_id, created_by, created_at,
    year, overview, poster_url, background_url, sort_title, catalog_sort_key, slug, imdb_id,
    runtime_minutes, popularity, content_status, language, first_aired, network, studio,
    country, aliases, metadata_language, metadata_fetched_at, min_availability,
    digital_release_date, folder_path, tagged_aliases_json,
    metadata_hydration_next_attempt_at, metadata_hydration_attempt_count
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {},
    {}, {}, {}, {}
)
ON CONFLICT (id) DO UPDATE SET
    library_id = excluded.library_id,
    name = excluded.name,
    facet = excluded.facet,
    monitored = excluded.monitored,
    tags = excluded.tags,
    external_ids = excluded.external_ids,
    root_folder_id = excluded.root_folder_id,
    created_by = excluded.created_by,
    created_at = excluded.created_at,
    year = excluded.year,
    overview = excluded.overview,
    poster_local_path = CASE
        WHEN COALESCE(titles.poster_url, '') <> COALESCE(excluded.poster_url, '') THEN NULL
        ELSE titles.poster_local_path
    END,
    poster_url = excluded.poster_url,
    background_local_path = CASE
        WHEN COALESCE(titles.background_url, '') <> COALESCE(excluded.background_url, '') THEN NULL
        ELSE titles.background_local_path
    END,
    background_url = excluded.background_url,
    sort_title = excluded.sort_title,
    catalog_sort_key = excluded.catalog_sort_key,
    slug = excluded.slug,
    imdb_id = excluded.imdb_id,
    runtime_minutes = excluded.runtime_minutes,
    popularity = excluded.popularity,
    content_status = excluded.content_status,
    language = excluded.language,
    first_aired = excluded.first_aired,
    network = excluded.network,
    studio = excluded.studio,
    country = excluded.country,
    aliases = excluded.aliases,
    metadata_language = excluded.metadata_language,
    metadata_fetched_at = excluded.metadata_fetched_at,
    min_availability = excluded.min_availability,
    digital_release_date = excluded.digital_release_date,
    folder_path = excluded.folder_path,
    tagged_aliases_json = excluded.tagged_aliases_json,
    metadata_hydration_next_attempt_at = CASE
        WHEN {} THEN titles.metadata_hydration_next_attempt_at
        ELSE excluded.metadata_hydration_next_attempt_at
    END,
    metadata_hydration_attempt_count = CASE
        WHEN {} THEN titles.metadata_hydration_attempt_count
        ELSE excluded.metadata_hydration_attempt_count
    END,
    smg_identity_backfill_attempt_count = CASE
        WHEN titles.external_ids <> excluded.external_ids THEN 0
        ELSE titles.smg_identity_backfill_attempt_count
    END";
const RECYCLE_BIN_PATH_SEGMENT: &str = "/.scryer-recycle/";
const TITLE_QUALITY_PROFILE_TAG_PREFIX: &str = "scryer:quality-profile:";

#[derive(Clone, Copy)]
enum TitleCatalogSqlDialect {
    Sqlite,
    Postgres,
}

#[derive(Clone)]
pub struct TitleStore {
    datastore: StoreDatastore,
}

impl TitleStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    async fn find_existing_title_after_unique_conflict(
        &self,
        library_id: &str,
        external_ids: &[(String, String)],
    ) -> AppResult<Option<Title>> {
        if external_ids.is_empty() {
            return Ok(None);
        }

        let library_id = library_id.to_string();
        let external_ids = external_ids.to_vec();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "lookup_existing_title_after_unique_conflict",
            move |tx| {
                let library_id = library_id.clone();
                let external_ids = external_ids.clone();
                Box::pin(async move {
                    let title_ids =
                        list_existing_title_ids_for_external_ids_tx(tx, &library_id, &external_ids)
                            .await?;
                    match title_ids.as_slice() {
                        [] => Ok(None),
                        [existing_id] => load_title_tx(tx, existing_id, true).await,
                        _ => Err(AppError::Validation(
                            "external ids already map to multiple titles".to_string(),
                        )),
                    }
                })
            },
        )
        .await
    }

    async fn reconcile_existing_title_options_after_unique_conflict(
        &self,
        existing: Title,
        options_patch: TitleOptionsPatch,
        requested_root_folder_id: String,
    ) -> AppResult<CreateTitleOutcome> {
        let existing_id = existing.id;
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "reconcile_reused_title_options_after_unique_conflict",
            move |tx| {
                let existing_id = existing_id.clone();
                let options_patch = options_patch.clone();
                let requested_root_folder_id = requested_root_folder_id.clone();
                Box::pin(async move {
                    let mut title =
                        load_title_canonical_tx_or_not_found(tx, &existing_id, true).await?;
                    apply_reused_title_options_patch(
                        &mut title,
                        &options_patch,
                        &requested_root_folder_id,
                    );
                    persist_title_tx(tx, &title, HydrationStateWrite::Preserve).await?;
                    let title = load_title_tx_or_not_found(tx, &title.id, true).await?;
                    Ok(CreateTitleOutcome {
                        title,
                        reused_existing: true,
                    })
                })
            },
        )
        .await
    }

    async fn list_internal(
        &self,
        facet: Option<MediaFacet>,
        library_ids: Option<&[String]>,
        query: Option<String>,
        mode: PersistedTitleReadMode,
        include_external_ids: bool,
    ) -> AppResult<Vec<Title>> {
        if matches!(library_ids, Some(library_ids) if library_ids.is_empty()) {
            return Ok(Vec::new());
        }

        let rows = match query.as_deref() {
            Some(query)
                if matches!(mode, PersistedTitleReadMode::Presentation)
                    && library_ids.is_none() =>
            {
                if normalize_title_search_text(query).is_empty() {
                    return Ok(Vec::new());
                }
                match &self.datastore {
                    StoreDatastore::Sqlite { pool, .. } => {
                        let mut titles = list_titles_via_sqlite_title_search_query(
                            pool,
                            facet,
                            query,
                            include_external_ids,
                        )
                        .await?;
                        attach_metadata_tags_to_titles(self.datastore.read_exec(), &mut titles)
                            .await?;
                        return Ok(titles);
                    }
                    StoreDatastore::Postgres { .. } => {
                        let (sql, args) = build_ranked_title_list_sql(facet, None, query);
                        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?
                    }
                }
            }
            Some(query) => {
                let (sql, args) = build_name_filtered_title_list_sql(facet, library_ids, query);
                SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?
            }
            None => {
                let (sql, args) = build_plain_title_list_sql(facet, library_ids);
                SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?
            }
        };

        let mut titles = decode_runtime_title_rows(&rows, mode, include_external_ids)?;
        attach_metadata_tags_to_titles(self.datastore.read_exec(), &mut titles).await?;
        Ok(titles)
    }

    async fn get_by_id_internal(
        &self,
        id: &str,
        include_external_ids: bool,
    ) -> AppResult<Option<Title>> {
        let sql = format!("SELECT {TITLE_COLUMNS} FROM titles WHERE id = {{}}");
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?;

        let mut title = decode_optional_runtime_title_row(
            row.as_ref(),
            PersistedTitleReadMode::Presentation,
            include_external_ids,
        )?;
        if let Some(title) = title.as_mut() {
            attach_metadata_tags_to_titles(self.datastore.read_exec(), std::slice::from_mut(title))
                .await?;
        }
        Ok(title)
    }
}

#[async_trait]
impl TitleRepository for TitleStore {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_internal(
            facet,
            None,
            query,
            PersistedTitleReadMode::Presentation,
            true,
        )
        .await
    }

    async fn count_by_quality_profile_id(&self, profile_id: &str) -> AppResult<u64> {
        let profile_id = profile_id.trim();
        if profile_id.is_empty() {
            return Ok(0);
        }
        let sql = match &self.datastore {
            StoreDatastore::Sqlite { .. } => {
                "SELECT tags
                   FROM titles
                  WHERE EXISTS (
                      SELECT 1
                        FROM json_each(titles.tags) AS title_tag
                       WHERE SUBSTR(title_tag.value, 1, LENGTH('scryer:quality-profile:')) = 'scryer:quality-profile:'
                  )"
            }
            StoreDatastore::Postgres { .. } => {
                "SELECT tags
                   FROM titles
                  WHERE EXISTS (
                      SELECT 1
                        FROM jsonb_array_elements_text(titles.tags) AS title_tag(value)
                       WHERE SUBSTRING(title_tag.value FROM 1 FOR LENGTH('scryer:quality-profile:')) = 'scryer:quality-profile:'
                  )"
            }
        };
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), sql, &[]).await?;
        let mut count = 0_u64;
        for row in rows {
            let tags = row.opt_json("tags")?.unwrap_or_default();
            if tags.as_array().is_some_and(|tags| {
                tags.iter().any(|tag| {
                    tag.as_str()
                        .and_then(|tag| tag.strip_prefix("scryer:quality-profile:"))
                        .is_some_and(|value| value.trim().eq_ignore_ascii_case(profile_id))
                })
            }) {
                count += 1;
            }
        }
        Ok(count)
    }

    async fn list_without_external_ids(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_internal(
            facet,
            None,
            query,
            PersistedTitleReadMode::Presentation,
            false,
        )
        .await
    }

    async fn list_delete_preview_info(&self) -> AppResult<Vec<TitleDeletePreviewInfo>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, library_id, name, facet, folder_path FROM titles ORDER BY id",
            &[],
        )
        .await?;

        rows.into_iter()
            .map(|row| {
                let facet = parse_facet(&row.text("facet")?);
                Ok(TitleDeletePreviewInfo {
                    title_id: row.text("id")?,
                    library_id: row
                        .opt_text("library_id")?
                        .unwrap_or_else(|| scryer_domain::default_library_id_for_facet(&facet)),
                    title_name: row.text("name")?,
                    facet,
                    folder_path: row.opt_text("folder_path")?,
                })
            })
            .collect()
    }

    async fn list_page_after_id(
        &self,
        after_id: Option<String>,
        limit: usize,
    ) -> AppResult<Vec<Title>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let (sql, args) = build_title_page_after_id_sql(after_id.as_deref(), limit);
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        decode_runtime_title_rows(&rows, PersistedTitleReadMode::Canonical, true)
    }

    async fn update_title_artwork_urls(&self, updates: &[TitleArtworkUrlUpdate]) -> AppResult<u64> {
        if updates.is_empty() {
            return Ok(0);
        }
        let updates = updates.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "update_title_artwork_urls", move |tx| {
            let updates = updates.clone();
            Box::pin(async move {
                let mut changed = 0_u64;
                for update in updates {
                    let rows = tx
                        .execute(
                            "UPDATE titles
                                SET poster_local_path = CASE
                                        WHEN COALESCE(poster_url, '') <> COALESCE({}, '') THEN NULL
                                        ELSE poster_local_path
                                    END,
                                    poster_url = {},
                                    background_local_path = CASE
                                        WHEN COALESCE(background_url, '') <> COALESCE({}, '') THEN NULL
                                        ELSE background_local_path
                                    END,
                                    background_url = {}
                              WHERE id = {}
                                AND (
                                    COALESCE(poster_url, '') <> COALESCE({}, '')
                                    OR COALESCE(background_url, '') <> COALESCE({}, '')
                                )",
                            &[
                                SqlArg::OptText(update.poster_url.clone()),
                                SqlArg::OptText(update.poster_url.clone()),
                                SqlArg::OptText(update.background_url.clone()),
                                SqlArg::OptText(update.background_url.clone()),
                                SqlArg::Text(update.title_id.clone()),
                                SqlArg::OptText(update.poster_url),
                                SqlArg::OptText(update.background_url),
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

    async fn list_for_libraries(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_internal(
            facet,
            Some(library_ids),
            query,
            PersistedTitleReadMode::Presentation,
            true,
        )
        .await
    }

    async fn list_catalog_owned_title_records(
        &self,
        library_ids: &[String],
    ) -> AppResult<Vec<CatalogOwnedTitleRecord>> {
        if library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = std::iter::repeat_n("{}", library_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT titles.id AS title_id,
                    titles.facet AS facet,
                    titles.imdb_id AS imdb_id,
                    external_ids.source AS external_source,
                    external_ids.external_id AS external_id
               FROM titles
               LEFT JOIN title_external_ids external_ids
                 ON external_ids.title_id = titles.id
              WHERE titles.library_id IN ({placeholders})
              ORDER BY titles.id ASC, external_ids.source ASC, external_ids.external_id ASC"
        );
        let args = library_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        let mut titles = Vec::<CatalogOwnedTitleRecord>::new();
        for row in rows {
            let title_id = row.text("title_id")?;
            if titles.last().is_none_or(|title| title.id != title_id) {
                titles.push(CatalogOwnedTitleRecord {
                    id: title_id.clone(),
                    facet: parse_facet(&row.text("facet")?),
                    imdb_id: row.opt_text("imdb_id")?,
                    external_ids: Vec::new(),
                });
            }
            let (Some(source), Some(value)) = (
                row.opt_text("external_source")?,
                row.opt_text("external_id")?,
            ) else {
                continue;
            };
            if let Some(title) = titles.last_mut() {
                title
                    .external_ids
                    .push(CatalogOwnedExternalIdRecord { source, value });
            }
        }
        Ok(titles)
    }

    async fn list_for_libraries_without_external_ids(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_internal(
            facet,
            Some(library_ids),
            query,
            PersistedTitleReadMode::Presentation,
            false,
        )
        .await
    }

    async fn list_for_libraries_catalog(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
        filter: TitleCatalogFilter,
        sort: TitleCatalogSort,
        limit: usize,
        offset: usize,
        include_external_ids: bool,
        include_catalog_counts: bool,
    ) -> AppResult<TitleCatalogResult> {
        if library_ids.is_empty() {
            return Ok(TitleCatalogResult {
                items: Vec::new(),
                limit,
                offset,
                has_more: false,
                total_count: 0,
                filter_counts: TitleCatalogFilterCounts::default(),
                managed_bytes: 0,
            });
        }

        let query = query.as_deref();
        let filter_counts = if include_catalog_counts {
            fetch_title_catalog_filter_counts(
                &self.datastore,
                facet.clone(),
                library_ids,
                query,
                &filter,
            )
            .await?
        } else {
            TitleCatalogFilterCounts::default()
        };
        let total_count = if include_catalog_counts {
            fetch_title_catalog_count(&self.datastore, facet.clone(), library_ids, query, &filter)
                .await?
        } else {
            0
        };
        let managed_bytes = if include_catalog_counts {
            fetch_title_catalog_managed_bytes(&self.datastore, facet.clone(), library_ids).await?
        } else {
            0
        };

        if limit == 0 || (include_catalog_counts && total_count == 0) {
            return Ok(TitleCatalogResult {
                items: Vec::new(),
                limit,
                offset,
                has_more: false,
                total_count,
                filter_counts,
                managed_bytes,
            });
        }

        let (page_sql, page_args) = build_title_catalog_page_sql(
            facet,
            library_ids,
            query,
            &filter,
            sort,
            limit,
            offset,
            title_catalog_dialect_for_datastore(&self.datastore),
        );
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &page_sql, &page_args).await?;
        let mut items = decode_runtime_title_rows(
            &rows,
            PersistedTitleReadMode::Presentation,
            include_external_ids,
        )?;
        attach_metadata_tags_to_titles(self.datastore.read_exec(), &mut items).await?;
        let has_more = include_catalog_counts && offset.saturating_add(items.len()) < total_count;

        Ok(TitleCatalogResult {
            items,
            limit,
            offset,
            has_more,
            total_count,
            filter_counts,
            managed_bytes,
        })
    }

    async fn title_catalog_filter_options(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        root_folder_ids: &[String],
    ) -> AppResult<TitleCatalogFilterOptions> {
        fetch_title_catalog_filter_options(&self.datastore, facet, library_ids, root_folder_ids)
            .await
    }

    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>> {
        if values.is_empty() {
            return Ok(Vec::new());
        }

        let requested_values = std::iter::repeat_n("({}, {})", values.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "WITH requested(request_ordinal, external_id) AS (
                 VALUES {requested_values}
             ),
             matched_title_ids AS (
                 SELECT requested.request_ordinal,
                        title_external_ids.title_id
                   FROM requested
                   JOIN title_external_ids
                     ON LOWER(title_external_ids.source) = LOWER({{}})
                    AND title_external_ids.external_id = requested.external_id
             ),
             deduped AS (
                 SELECT MIN(request_ordinal) AS first_request_ordinal,
                        title_id
                   FROM matched_title_ids
                  GROUP BY title_id
             )
             SELECT {TITLE_COLUMNS}
               FROM titles
               JOIN deduped ON deduped.title_id = titles.id
              ORDER BY deduped.first_request_ordinal, id"
        );
        let mut args = Vec::with_capacity(values.len() * 2 + 1);
        for (ordinal, value) in values.iter().enumerate() {
            args.push(SqlArg::I64(ordinal as i64));
            args.push(SqlArg::Text(value.clone()));
        }
        args.push(SqlArg::Text(source.to_string()));

        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        let mut titles =
            decode_runtime_title_rows(&rows, PersistedTitleReadMode::Presentation, true)?;
        attach_metadata_tags_to_titles(self.datastore.read_exec(), &mut titles).await?;
        Ok(titles)
    }

    async fn list_by_external_id_lookups(
        &self,
        lookups: &[TitleExternalIdLookup],
    ) -> AppResult<Vec<TitleExternalIdLookupMatch>> {
        if lookups.is_empty() {
            return Ok(Vec::new());
        }

        let requested_values = std::iter::repeat_n("({}, {}, {})", lookups.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "WITH requested(lookup_index, source, external_id) AS (
                 VALUES {requested_values}
             ),
             matched_title_ids AS (
                 SELECT requested.lookup_index,
                        title_external_ids.title_id
                   FROM requested
                   JOIN title_external_ids
                     ON LOWER(title_external_ids.source) = LOWER(requested.source)
                    AND title_external_ids.external_id = requested.external_id
             )
             SELECT matched_title_ids.lookup_index AS lookup_index,
                    {TITLE_COLUMNS}
               FROM titles
               JOIN matched_title_ids ON matched_title_ids.title_id = titles.id
              ORDER BY matched_title_ids.lookup_index ASC,
                       titles.id ASC"
        );
        let mut args = Vec::with_capacity(lookups.len() * 3);
        for lookup in lookups {
            args.push(SqlArg::I64(lookup.lookup_index as i64));
            args.push(SqlArg::Text(lookup.source.clone()));
            args.push(SqlArg::Text(lookup.external_id.clone()));
        }

        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        let base_path = normalized_base_path_from_env();
        let mut matches = rows
            .iter()
            .map(|row| {
                Ok(TitleExternalIdLookupMatch {
                    lookup_index: row.i64("lookup_index")? as usize,
                    title: title_from_projection_row(
                        row,
                        PersistedTitleReadMode::Presentation,
                        true,
                        &base_path,
                    )?,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        let title_ids = matches
            .iter()
            .map(|matched| matched.title.id.clone())
            .collect::<Vec<_>>();
        let tags_by_title =
            load_title_metadata_tags(self.datastore.read_exec(), &title_ids).await?;
        for matched in &mut matches {
            if let Some(tags) = tags_by_title.get(&matched.title.id) {
                matched.title.canonical_tags = tags.clone();
            }
        }
        Ok(matches)
    }

    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_internal(facet, None, query, PersistedTitleReadMode::Matching, true)
            .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        self.get_by_id_internal(id, true).await
    }

    async fn get_by_id_without_external_ids(&self, id: &str) -> AppResult<Option<Title>> {
        self.get_by_id_internal(id, false).await
    }

    async fn get_by_ids(&self, ids: &[String]) -> AppResult<Vec<Title>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("{}", ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT {TITLE_COLUMNS} FROM titles WHERE id IN ({placeholders})");
        let args = ids.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        let mut titles =
            decode_runtime_title_rows(&rows, PersistedTitleReadMode::Presentation, true)?;
        attach_metadata_tags_to_titles(self.datastore.read_exec(), &mut titles).await?;
        Ok(titles)
    }

    async fn get_title_ratings(&self, title_id: &str) -> AppResult<TitleRatingSummary> {
        load_title_ratings(&self.datastore, title_id).await
    }

    async fn list_title_ratings(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<(String, TitleRatingSummary)>> {
        load_title_ratings_batch(&self.datastore, title_ids).await
    }

    async fn get_title_credits(&self, title_id: &str) -> AppResult<Vec<TitleCredit>> {
        let mut credits =
            load_title_credits(self.datastore.read_exec(), &[title_id.to_string()]).await?;
        Ok(credits.remove(title_id).unwrap_or_default())
    }

    async fn get_by_facet_and_slug(
        &self,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>> {
        let Some(normalized_slug) = normalize_lookup_slug(slug) else {
            return Ok(None);
        };
        let sql = format!(
            "SELECT {TITLE_COLUMNS}
               FROM titles
              WHERE facet = {{}}
                AND LOWER(TRIM(slug)) = LOWER({{}})
              ORDER BY id
              LIMIT 2"
        );
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(facet.as_str().to_string()),
                SqlArg::Text(normalized_slug.clone()),
            ],
        )
        .await?;
        let mut titles =
            decode_runtime_title_rows(&rows, PersistedTitleReadMode::Presentation, true)?;
        attach_metadata_tags_to_titles(self.datastore.read_exec(), &mut titles).await?;
        single_slug_match(titles, &normalized_slug)
    }

    async fn get_by_facet_libraries_and_slug(
        &self,
        facet: MediaFacet,
        library_ids: &[String],
        slug: &str,
    ) -> AppResult<Option<Title>> {
        if library_ids.is_empty() {
            return Ok(None);
        }

        let Some(normalized_slug) = normalize_lookup_slug(slug) else {
            return Ok(None);
        };

        let placeholders = std::iter::repeat_n("{}", library_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {TITLE_COLUMNS}
               FROM titles
              WHERE facet = {{}}
                AND LOWER(TRIM(slug)) = LOWER({{}})
                AND library_id IN ({placeholders})
              ORDER BY id
              LIMIT 2"
        );
        let mut args = vec![
            SqlArg::Text(facet.as_str().to_string()),
            SqlArg::Text(normalized_slug.clone()),
        ];
        args.extend(library_ids.iter().cloned().map(SqlArg::Text));

        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        let mut titles =
            decode_runtime_title_rows(&rows, PersistedTitleReadMode::Presentation, true)?;
        attach_metadata_tags_to_titles(self.datastore.read_exec(), &mut titles).await?;
        single_slug_match(titles, &normalized_slug)
    }

    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
        let sql = format!(
            "SELECT {TITLE_COLUMNS}
               FROM titles
              WHERE id IN (
                    SELECT title_id
                      FROM title_external_ids
                     WHERE LOWER(source) = LOWER({{}})
                       AND external_id = {{}}
              )
              ORDER BY id
              LIMIT 1"
        );
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(source.to_string()),
                SqlArg::Text(value.to_string()),
            ],
        )
        .await?;
        let mut titles = decode_optional_runtime_title_row(
            row.as_ref(),
            PersistedTitleReadMode::Presentation,
            true,
        )?
        .into_iter()
        .collect::<Vec<_>>();
        attach_metadata_tags_to_titles(self.datastore.read_exec(), &mut titles).await?;
        Ok(titles.pop())
    }

    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>> {
        let sql = format!(
            "SELECT {TITLE_COLUMNS}
               FROM titles
              WHERE id IN (
                    SELECT title_id
                      FROM title_external_ids
                     WHERE facet = {{}}
                       AND LOWER(source) = LOWER({{}})
                       AND external_id = {{}}
                )
              ORDER BY id
              LIMIT 1"
        );
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(facet.as_str().to_string()),
                SqlArg::Text(source.to_string()),
                SqlArg::Text(value.to_string()),
            ],
        )
        .await?;
        let mut titles = decode_optional_runtime_title_row(
            row.as_ref(),
            PersistedTitleReadMode::Presentation,
            true,
        )?
        .into_iter()
        .collect::<Vec<_>>();
        attach_metadata_tags_to_titles(self.datastore.read_exec(), &mut titles).await?;
        Ok(titles.pop())
    }

    async fn find_by_external_id_in_library_and_facet(
        &self,
        library_id: &str,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>> {
        let library_id = library_id.trim();
        let source = source.trim().to_ascii_lowercase();
        let value = value.trim();
        if library_id.is_empty() || source.is_empty() || value.is_empty() {
            return Ok(None);
        }

        let sql = format!(
            "SELECT {TITLE_COLUMNS}
               FROM titles
              WHERE library_id = {{}}
                AND id IN (
                    SELECT title_id
                      FROM title_external_ids
                     WHERE library_id = {{}}
                       AND facet = {{}}
                       AND source = {{}}
                       AND external_id = {{}}
                )
              ORDER BY id
              LIMIT 1"
        );
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(library_id.to_string()),
                SqlArg::Text(library_id.to_string()),
                SqlArg::Text(facet.as_str().to_string()),
                SqlArg::Text(source),
                SqlArg::Text(value.to_string()),
            ],
        )
        .await?;
        let mut titles = decode_optional_runtime_title_row(
            row.as_ref(),
            PersistedTitleReadMode::Presentation,
            true,
        )?
        .into_iter()
        .collect::<Vec<_>>();
        attach_metadata_tags_to_titles(self.datastore.read_exec(), &mut titles).await?;
        Ok(titles.pop())
    }

    async fn list_existing_external_ids_in_library_and_facet(
        &self,
        library_id: &str,
        facet: MediaFacet,
        source: &str,
        values: &[String],
    ) -> AppResult<BTreeSet<String>> {
        let library_id = library_id.trim();
        let source = source.trim().to_ascii_lowercase();
        if library_id.is_empty() || source.is_empty() || values.is_empty() {
            return Ok(BTreeSet::new());
        }

        let requested = values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if requested.is_empty() {
            return Ok(BTreeSet::new());
        }

        let requested_values = std::iter::repeat_n("({})", requested.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "WITH requested(external_id) AS (
                 VALUES {requested_values}
             )
             SELECT DISTINCT requested.external_id AS external_id
               FROM requested
               JOIN title_external_ids
                 ON title_external_ids.library_id = {{}}
                AND title_external_ids.facet = {{}}
                AND title_external_ids.source = {{}}
                AND title_external_ids.external_id = requested.external_id
              ORDER BY requested.external_id"
        );
        let mut args = Vec::with_capacity(requested.len() + 3);
        for value in requested {
            args.push(SqlArg::Text(value));
        }
        args.push(SqlArg::Text(library_id.to_string()));
        args.push(SqlArg::Text(facet.as_str().to_string()));
        args.push(SqlArg::Text(source));

        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        let mut existing = BTreeSet::new();
        for row in rows {
            existing.insert(row.text("external_id")?);
        }
        Ok(existing)
    }

    async fn create_or_get_existing(&self, title: Title) -> AppResult<CreateTitleOutcome> {
        self.create_or_get_existing_with_options_patch(title, TitleOptionsPatch::default())
            .await
    }

    async fn create_or_get_existing_with_options_patch(
        &self,
        title: Title,
        options_patch: TitleOptionsPatch,
    ) -> AppResult<CreateTitleOutcome> {
        let external_ids = normalized_external_ids(&title.external_ids);
        let library_id = title.library_id.clone();
        let fallback_options_patch = options_patch.clone();
        let fallback_root_folder_id = title.root_folder_id.clone();
        let result = SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_or_get_existing_title",
            move |tx| {
                let title = title.clone();
                let options_patch = options_patch.clone();
                Box::pin(async move {
                    if let Some(mut existing) =
                        find_existing_title_for_create_tx(tx, &title).await?
                    {
                        apply_reused_title_options_patch(
                            &mut existing,
                            &options_patch,
                            &title.root_folder_id,
                        );
                        persist_title_tx(tx, &existing, HydrationStateWrite::Preserve).await?;
                        let title = load_title_tx_or_not_found(tx, &existing.id, true).await?;
                        return Ok(CreateTitleOutcome {
                            title,
                            reused_existing: true,
                        });
                    }

                    create_title_tx(tx, &title).await?;
                    let title = load_title_tx_or_not_found(tx, &title.id, true).await?;
                    Ok(CreateTitleOutcome {
                        title,
                        reused_existing: false,
                    })
                })
            },
        )
        .await;

        match result {
            Ok(outcome) => Ok(outcome),
            Err(error) if is_title_external_id_conflict_error(&error) => {
                match self
                    .find_existing_title_after_unique_conflict(&library_id, external_ids.as_slice())
                    .await?
                {
                    Some(existing) => {
                        self.reconcile_existing_title_options_after_unique_conflict(
                            existing,
                            fallback_options_patch,
                            fallback_root_folder_id,
                        )
                        .await
                    }
                    None => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    async fn create_or_get_existing_and_bind_pending_import(
        &self,
        title: Title,
        pending_import_id: &str,
    ) -> AppResult<CreateTitleOutcome> {
        let external_ids = normalized_external_ids(&title.external_ids);
        let library_id = title.library_id.clone();
        let pending_import_id = pending_import_id.to_string();
        let result = SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_or_get_existing_title_and_bind_pending_import",
            move |tx| {
                let title = title.clone();
                let pending_import_id = pending_import_id.clone();
                Box::pin(async move {
                    if let Some(existing) = find_existing_title_for_create_tx(tx, &title).await? {
                        return Ok(CreateTitleOutcome {
                            title: existing,
                            reused_existing: true,
                        });
                    }

                    create_title_tx(tx, &title).await?;
                    let title = load_title_tx_or_not_found(tx, &title.id, true).await?;
                    resolve_pending_import_for_created_title_tx(tx, &pending_import_id, &title)
                        .await?;
                    Ok(CreateTitleOutcome {
                        title,
                        reused_existing: false,
                    })
                })
            },
        )
        .await;

        match result {
            Ok(outcome) => Ok(outcome),
            Err(error) if is_title_external_id_conflict_error(&error) => {
                match self
                    .find_existing_title_after_unique_conflict(&library_id, external_ids.as_slice())
                    .await?
                {
                    Some(existing) => Ok(CreateTitleOutcome {
                        title: existing,
                        reused_existing: true,
                    }),
                    None => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    async fn create(&self, title: Title) -> AppResult<Title> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_title", move |tx| {
            let title = title.clone();
            Box::pin(async move {
                create_title_tx(tx, &title).await?;
                let title = load_title_tx_or_not_found(tx, &title.id, true).await?;
                Ok(title)
            })
        })
        .await
    }

    async fn list_titles_due_for_hydration(
        &self,
        limit: usize,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<PendingTitleHydration>> {
        let mut sql = format!(
            "SELECT {TITLE_COLUMNS}, metadata_hydration_attempt_count
               FROM titles
              WHERE metadata_hydration_next_attempt_at IS NOT NULL
                AND metadata_hydration_next_attempt_at <= {{}}"
        );
        let mut args = vec![SqlArg::Timestamp(Utc::now())];
        if !excluded_facets.is_empty() {
            let placeholders = std::iter::repeat_n("{}", excluded_facets.len())
                .collect::<Vec<_>>()
                .join(", ");
            sql.push_str(&format!(" AND facet NOT IN ({placeholders})"));
            args.extend(
                excluded_facets
                    .iter()
                    .map(|facet| SqlArg::Text(facet.as_str().to_string())),
            );
        }
        sql.push_str(" ORDER BY metadata_hydration_next_attempt_at ASC, id ASC LIMIT {}");
        args.push(SqlArg::I64(limit as i64));

        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        rows.into_iter()
            .map(|row| {
                Ok(PendingTitleHydration {
                    title: decode_optional_runtime_title_row(
                        Some(&row),
                        PersistedTitleReadMode::Canonical,
                        true,
                    )?
                    .ok_or_else(|| {
                        AppError::Repository(
                            "title hydration query returned an empty row projection".to_string(),
                        )
                    })?,
                    attempt_count: row.i64("metadata_hydration_attempt_count")?,
                })
            })
            .collect()
    }

    async fn list_movie_titles_missing_smg_id_after_id(
        &self,
        after_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<Title>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let after_id = after_id.unwrap_or_default().trim().to_string();
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {TITLE_COLUMNS}
                   FROM titles
                  WHERE facet = {{}}
                    AND (id > {{}} OR smg_identity_backfill_attempt_count = 0)
                    AND smg_identity_backfill_attempt_count < 5
                    AND EXISTS (
                        SELECT 1
                          FROM title_external_ids
                         WHERE title_id = titles.id
                           AND LOWER(source) IN ('tvdb', 'tmdb', 'imdb')
                           AND TRIM(external_id) <> ''
                    )
                    AND NOT EXISTS (
                        SELECT 1
                          FROM title_external_ids
                         WHERE title_id = titles.id
                           AND LOWER(source) = 'smg'
                    )
                  ORDER BY id ASC
                  LIMIT {{}}"
            ),
            &[
                SqlArg::Text(MediaFacet::Movie.as_str().to_string()),
                SqlArg::Text(after_id),
                SqlArg::I64(limit as i64),
            ],
        )
        .await?;
        rows.iter()
            .map(|row| {
                decode_optional_runtime_title_row(
                    Some(row),
                    PersistedTitleReadMode::Canonical,
                    true,
                )?
                .ok_or_else(|| {
                    AppError::Repository(
                        "movie SMG identity backfill query returned an empty row projection"
                            .to_string(),
                    )
                })
            })
            .collect()
    }

    async fn record_movie_smg_identity_backfill_unresolved(&self, title_id: &str) -> AppResult<()> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_movie_smg_identity_backfill_unresolved",
            move |tx| {
                let title_id = title_id.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE titles
                            SET smg_identity_backfill_attempt_count = smg_identity_backfill_attempt_count + 1
                          WHERE id = {} AND facet = {}",
                        &[
                            SqlArg::Text(title_id),
                            SqlArg::Text(MediaFacet::Movie.as_str().to_string()),
                        ],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn list_title_ids_with_metadata_hydration_due(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
    ) -> AppResult<Vec<String>> {
        if library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut sql = String::from(
            "SELECT id
               FROM titles
              WHERE metadata_hydration_next_attempt_at IS NOT NULL
                AND metadata_hydration_next_attempt_at <= {}",
        );
        let mut args = vec![SqlArg::Timestamp(Utc::now())];
        if let Some(facet) = facet {
            sql.push_str(" AND facet = {}");
            args.push(SqlArg::Text(facet.as_str().to_string()));
        }
        let placeholders = std::iter::repeat_n("{}", library_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND library_id IN ({placeholders})"));
        args.extend(library_ids.iter().cloned().map(SqlArg::Text));
        sql.push_str(" ORDER BY metadata_hydration_next_attempt_at ASC, id ASC");

        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        rows.into_iter().map(|row| row.text("id")).collect()
    }

    async fn mark_title_metadata_hydration_due_now(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "mark_title_metadata_hydration_due_now",
            move |tx| {
                let id = id.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE titles
                            SET metadata_hydration_next_attempt_at = {},
                                metadata_hydration_attempt_count = 0,
                                smg_identity_backfill_attempt_count = 0
                          WHERE id = {}",
                        &[SqlArg::Timestamp(Utc::now()), SqlArg::Text(id)],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn mark_titles_metadata_hydration_due_now(&self, ids: &[String]) -> AppResult<()> {
        if ids.is_empty() {
            return Ok(());
        }

        let ids = ids.to_vec();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "mark_titles_metadata_hydration_due_now",
            move |tx| {
                let ids = ids.clone();
                Box::pin(async move {
                    let placeholders = std::iter::repeat_n("{}", ids.len())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let sql = format!(
                        "UPDATE titles
                            SET metadata_hydration_next_attempt_at = {{}},
                                metadata_hydration_attempt_count = 0,
                                smg_identity_backfill_attempt_count = 0
                          WHERE id IN ({placeholders})"
                    );
                    let mut args = vec![SqlArg::Timestamp(Utc::now())];
                    args.extend(ids.into_iter().map(SqlArg::Text));
                    SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn schedule_title_metadata_hydration_retry(
        &self,
        id: &str,
        next_attempt_at: &str,
        attempt_count: i64,
    ) -> AppResult<()> {
        let id = id.to_string();
        let next_attempt_at = parse_utc_datetime(next_attempt_at)?;
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "schedule_title_metadata_hydration_retry",
            move |tx| {
                let id = id.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE titles
                            SET metadata_hydration_next_attempt_at = {},
                                metadata_hydration_attempt_count = {}
                          WHERE id = {}",
                        &[
                            SqlArg::Timestamp(next_attempt_at),
                            SqlArg::I64(attempt_count),
                            SqlArg::Text(id),
                        ],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn clear_title_metadata_hydration_retry_state(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "clear_title_metadata_hydration_retry_state",
            move |tx| {
                let id = id.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE titles
                            SET metadata_hydration_next_attempt_at = NULL,
                                metadata_hydration_attempt_count = 0
                          WHERE id = {}",
                        &[SqlArg::Text(id)],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn persist_smg_id(
        &self,
        title_id: &str,
        smg_id: i64,
        redirected_from: Option<i64>,
    ) -> AppResult<()> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "persist_smg_title_id", move |tx| {
            let title_id = title_id.clone();
            Box::pin(async move {
                let mut title = load_title_canonical_tx_or_not_found(tx, &title_id, true).await?;
                if redirected_from.is_some() {
                    title
                        .external_ids
                        .retain(|external_id| !external_id.source.eq_ignore_ascii_case("smg"));
                }
                merge_title_external_ids(
                    &mut title.external_ids,
                    vec![ExternalId {
                        source: "smg".to_string(),
                        value: smg_id.to_string(),
                    }],
                );
                persist_title_tx(tx, &title, HydrationStateWrite::Preserve).await
            })
        })
        .await
    }

    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_title_monitored", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let mut title = load_title_canonical_tx_or_not_found(tx, &id, true).await?;
                title.monitored = monitored;
                persist_title_tx(tx, &title, HydrationStateWrite::Preserve).await?;
                load_title_tx_or_not_found(tx, &id, true).await
            })
        })
        .await
    }

    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
        root_folder_id: Option<String>,
    ) -> AppResult<Title> {
        if name.is_none() && facet.is_none() && tags.is_none() && root_folder_id.is_none() {
            return Err(AppError::Validation(
                "at least one title field must be provided".to_string(),
            ));
        }

        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_title_metadata", move |tx| {
            let id = id.clone();
            let name = name.clone();
            let facet = facet.clone();
            let tags = tags.clone();
            let root_folder_id = root_folder_id.clone();
            Box::pin(async move {
                let mut title = load_title_canonical_tx_or_not_found(tx, &id, true).await?;
                if let Some(name) = name {
                    let normalized = name.trim();
                    if normalized.is_empty() {
                        return Err(AppError::Validation(
                            "title name cannot be empty".to_string(),
                        ));
                    }
                    title.name = normalized.to_string();
                }
                if let Some(facet) = facet {
                    if facet != title.facet {
                        return Err(AppError::Validation(
                            "changing a title facet is not supported because titles cannot move between libraries"
                                .to_string(),
                        ));
                    }
                    title.facet = facet;
                }
                if let Some(tags) = tags {
                    title.tags = tags;
                }
                if let Some(root_folder_id) = root_folder_id {
                    title.root_folder_id = root_folder_id;
                }
                persist_title_tx(tx, &title, HydrationStateWrite::Preserve).await?;
                load_title_tx_or_not_found(tx, &id, true).await
            })
        })
        .await
    }

    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        let metadata_marks_fetched = metadata
            .metadata_fetched_at
            .as_deref()
            .is_some_and(|value| !value.is_empty());
        let id = id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_title_hydrated_metadata",
            move |tx| {
                let id = id.clone();
                let metadata = metadata.clone();
                Box::pin(async move {
                    let ratings = metadata.ratings.clone();
                    let credits = metadata.credits.clone();
                    let canonical_tags = metadata.canonical_tags.clone();
                    let mut title = load_title_canonical_tx_or_not_found(tx, &id, true).await?;
                    apply_title_metadata_update(&mut title, metadata)?;
                    let hydration_state = if metadata_marks_fetched {
                        HydrationStateWrite::Clear
                    } else {
                        HydrationStateWrite::Preserve
                    };
                    persist_title_tx(tx, &title, hydration_state).await?;
                    // A fetched metadata payload is authoritative even when the gateway
                    // returns no normalized tags. Partial/manual updates omit the fetched
                    // marker and preserve the existing title-owned snapshot.
                    if metadata_marks_fetched || !canonical_tags.is_empty() {
                        replace_title_metadata_tags_tx(tx, &title.id, &canonical_tags).await?;
                    }
                    // Full SMG metadata responses pass Some(...), including an empty
                    // summary when SMG reports no ratings; None is reserved for callers
                    // that are not updating ratings and must preserve existing rows.
                    if let Some(ratings) = ratings {
                        replace_title_metadata_ratings_tx(tx, &title.id, &ratings).await?;
                    }
                    // Same Some/None contract as ratings: SMG hydration replaces the
                    // whole credit set (including an empty one), other callers pass
                    // None and keep the cached rows.
                    if let Some(credits) = credits {
                        replace_title_credits_tx(tx, &title.id, &credits).await?;
                    }
                    load_title_tx_or_not_found(tx, &id, true).await
                })
            },
        )
        .await
    }

    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_title_match_state", move |tx| {
            let id = id.clone();
            let external_ids = external_ids.clone();
            let tags = tags.clone();
            Box::pin(async move {
                let mut title = load_title_canonical_tx_or_not_found(tx, &id, true).await?;
                title.external_ids = external_ids;
                title.tags = tags;
                title.year = None;
                title.overview = None;
                title.poster_url = None;
                title.poster_source_url = None;
                title.background_url = None;
                title.background_source_url = None;
                title.sort_title = None;
                title.slug = None;
                title.imdb_id = None;
                title.runtime_minutes = None;
                title.popularity = None;
                title.content_status = None;
                title.language = None;
                title.first_aired = None;
                title.network = None;
                title.studio = None;
                title.country = None;
                title.aliases.clear();
                title.tagged_aliases.clear();
                title.metadata_language = None;
                title.metadata_fetched_at = None;
                title.digital_release_date = None;
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM title_images WHERE title_id = {}",
                    &[SqlArg::Text(title.id.clone())],
                )
                .await?;
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE titles
                        SET poster_local_path = NULL,
                            background_local_path = NULL
                      WHERE id = {}",
                    &[SqlArg::Text(title.id.clone())],
                )
                .await?;
                persist_title_tx(tx, &title, HydrationStateWrite::Reschedule).await?;
                replace_title_metadata_tags_tx(tx, &title.id, &[]).await?;
                replace_title_metadata_ratings_tx(tx, &title.id, &TitleRatingSummary::default())
                    .await?;
                replace_title_credits_tx(tx, &title.id, &[]).await?;
                load_title_tx_or_not_found(tx, &id, true).await
            })
        })
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_title", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                delete_title_search_projection_sql_tx(tx, &id).await?;
                delete_indexer_search_learning_for_title_tx(tx, &id).await?;
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM titles WHERE id = {}",
                    &[SqlArg::Text(id.clone())],
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("title {id}")));
                }
                Ok(())
            })
        })
        .await
    }

    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()> {
        let id = id.to_string();
        let folder_path = folder_path.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "set_title_folder_path", move |tx| {
            let id = id.clone();
            let folder_path = folder_path.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE titles SET folder_path = {} WHERE id = {}",
                    &[SqlArg::Text(folder_path), SqlArg::Text(id)],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn clear_folder_path(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "clear_title_folder_path", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE titles SET folder_path = NULL WHERE id = {}",
                    &[SqlArg::Text(id)],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "clear_metadata_language_for_all",
            move |tx| {
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE titles
                            SET metadata_language = NULL
                          WHERE metadata_language IS NOT NULL",
                        &[],
                    )
                    .await
                })
            },
        )
        .await
    }
}

async fn list_titles_via_sqlite_title_search_query(
    pool: &sqlx::SqlitePool,
    facet: Option<MediaFacet>,
    query: &str,
    include_external_ids: bool,
) -> AppResult<Vec<Title>> {
    let Some(search_plan) = build_title_search_plan(facet, query) else {
        return Ok(Vec::new());
    };

    let mut builder = QueryBuilder::<Sqlite>::new("");
    push_ranked_title_matches_cte(&mut builder, &search_plan);
    builder.push(format!(
        "SELECT {TITLE_COLUMNS} FROM ranked_title_matches
         JOIN titles ON titles.id = ranked_title_matches.title_id
         ORDER BY ranked_title_matches.rank ASC, LOWER(titles.name) ASC, titles.id ASC"
    ));

    let rows = builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(repo_err)?
        .into_iter()
        .map(SqlRow::Sqlite)
        .collect::<Vec<_>>();

    decode_runtime_title_rows(
        &rows,
        PersistedTitleReadMode::Presentation,
        include_external_ids,
    )
}

fn normalize_lookup_slug(slug: &str) -> Option<String> {
    let trimmed = slug.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn single_slug_match(mut titles: Vec<Title>, slug: &str) -> AppResult<Option<Title>> {
    match titles.len() {
        0 => Ok(None),
        1 => Ok(titles.pop()),
        _ => Err(AppError::Validation(format!(
            "slug '{slug}' maps to multiple titles"
        ))),
    }
}

fn normalized_external_ids(external_ids: &[ExternalId]) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for external_id in external_ids {
        let source = external_id.source.trim().to_ascii_lowercase();
        let value = external_id.value.trim().to_string();
        if source.is_empty() || value.is_empty() {
            continue;
        }
        if seen.insert((source.clone(), value.clone())) {
            out.push((source, value));
        }
    }
    out
}

trait TitleProjectionRow {
    fn text(&self, column: &str) -> AppResult<String>;
    fn opt_text(&self, column: &str) -> AppResult<Option<String>>;
    fn bool(&self, column: &str) -> AppResult<bool>;
    fn timestamp(&self, column: &str) -> AppResult<DateTime<Utc>>;
    fn opt_timestamp(&self, column: &str) -> AppResult<Option<DateTime<Utc>>>;
    fn opt_i32(&self, column: &str) -> AppResult<Option<i32>>;
    fn opt_f64(&self, column: &str) -> AppResult<Option<f64>>;
    fn opt_json_value(&self, column: &str) -> AppResult<Option<serde_json::Value>>;
}

impl TitleProjectionRow for sqlx::sqlite::SqliteRow {
    fn text(&self, column: &str) -> AppResult<String> {
        self.try_get(column).map_err(repo_err)
    }

    fn opt_text(&self, column: &str) -> AppResult<Option<String>> {
        self.try_get(column).map_err(repo_err)
    }

    fn bool(&self, column: &str) -> AppResult<bool> {
        let value: i64 = self.try_get(column).map_err(repo_err)?;
        Ok(value != 0)
    }

    fn timestamp(&self, column: &str) -> AppResult<DateTime<Utc>> {
        let raw: String = self.try_get(column).map_err(repo_err)?;
        parse_utc_datetime(&raw)
    }

    fn opt_timestamp(&self, column: &str) -> AppResult<Option<DateTime<Utc>>> {
        let raw: Option<String> = self.try_get(column).map_err(repo_err)?;
        match raw {
            Some(value) => parse_utc_datetime(&value).map(Some),
            None => Ok(None),
        }
    }

    fn opt_i32(&self, column: &str) -> AppResult<Option<i32>> {
        self.try_get(column).map_err(repo_err)
    }

    fn opt_f64(&self, column: &str) -> AppResult<Option<f64>> {
        self.try_get(column).map_err(repo_err)
    }

    fn opt_json_value(&self, column: &str) -> AppResult<Option<serde_json::Value>> {
        let raw: Option<String> = self.try_get(column).map_err(repo_err)?;
        match raw {
            Some(raw) => serde_json::from_str(&raw).map(Some).map_err(repo_err),
            None => Ok(None),
        }
    }
}

impl TitleProjectionRow for PgRow {
    fn text(&self, column: &str) -> AppResult<String> {
        self.try_get(column).map_err(repo_err)
    }

    fn opt_text(&self, column: &str) -> AppResult<Option<String>> {
        self.try_get(column).map_err(repo_err)
    }

    fn bool(&self, column: &str) -> AppResult<bool> {
        self.try_get(column).map_err(repo_err)
    }

    fn timestamp(&self, column: &str) -> AppResult<DateTime<Utc>> {
        self.try_get(column).map_err(repo_err)
    }

    fn opt_timestamp(&self, column: &str) -> AppResult<Option<DateTime<Utc>>> {
        self.try_get(column).map_err(repo_err)
    }

    fn opt_i32(&self, column: &str) -> AppResult<Option<i32>> {
        self.try_get(column).map_err(repo_err)
    }

    fn opt_f64(&self, column: &str) -> AppResult<Option<f64>> {
        self.try_get(column).map_err(repo_err)
    }

    fn opt_json_value(&self, column: &str) -> AppResult<Option<serde_json::Value>> {
        self.try_get(column).map_err(repo_err)
    }
}

impl TitleProjectionRow for SqlRow {
    fn text(&self, column: &str) -> AppResult<String> {
        SqlRow::text(self, column)
    }

    fn opt_text(&self, column: &str) -> AppResult<Option<String>> {
        match self {
            SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }

    fn bool(&self, column: &str) -> AppResult<bool> {
        SqlRow::bool(self, column)
    }

    fn timestamp(&self, column: &str) -> AppResult<DateTime<Utc>> {
        SqlRow::timestamp(self, column)
    }

    fn opt_timestamp(&self, column: &str) -> AppResult<Option<DateTime<Utc>>> {
        match self {
            SqlRow::Sqlite(row) => {
                let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
                raw.map(|value| parse_utc_datetime(&value)).transpose()
            }
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }

    fn opt_i32(&self, column: &str) -> AppResult<Option<i32>> {
        self.opt_i64(column)?
            .map(|value| {
                i32::try_from(value).map_err(|_| {
                    AppError::Repository(format!(
                        "value out of range for i32 column {column}: {value}"
                    ))
                })
            })
            .transpose()
    }

    fn opt_f64(&self, column: &str) -> AppResult<Option<f64>> {
        SqlRow::opt_f64(self, column)
    }

    fn opt_json_value(&self, column: &str) -> AppResult<Option<serde_json::Value>> {
        match self {
            SqlRow::Sqlite(row) => {
                let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
                raw.map(|value| serde_json::from_str(&value).map_err(repo_err))
                    .transpose()
            }
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }
}

fn decode_title_json_or_default<T, R>(row: &R, column: &str) -> AppResult<T>
where
    T: DeserializeOwned + Default,
    R: TitleProjectionRow,
{
    match row.opt_json_value(column)? {
        Some(value) => serde_json::from_value(value).map_err(repo_err),
        None => Ok(T::default()),
    }
}

fn decode_runtime_title_rows(
    rows: &[SqlRow],
    mode: PersistedTitleReadMode,
    include_external_ids: bool,
) -> AppResult<Vec<Title>> {
    let base_path = normalized_base_path_from_env();
    rows.iter()
        .map(|row| title_from_projection_row(row, mode, include_external_ids, &base_path))
        .collect()
}

fn decode_optional_runtime_title_row(
    row: Option<&SqlRow>,
    mode: PersistedTitleReadMode,
    include_external_ids: bool,
) -> AppResult<Option<Title>> {
    let base_path = normalized_base_path_from_env();
    row.map(|row| title_from_projection_row(row, mode, include_external_ids, &base_path))
        .transpose()
}

fn title_from_projection_row<R>(
    row: &R,
    mode: PersistedTitleReadMode,
    include_external_ids: bool,
    base_path: &str,
) -> AppResult<Title>
where
    R: TitleProjectionRow,
{
    let facet = parse_facet(&row.text("facet")?);
    let title = Title {
        id: row.text("id")?,
        library_id: row
            .opt_text("library_id")?
            .unwrap_or_else(|| scryer_domain::default_library_id_for_facet(&facet)),
        name: row.text("name")?,
        facet,
        monitored: row.bool("monitored")?,
        tags: decode_title_json_or_default(row, "tags")?,
        external_ids: if include_external_ids {
            decode_title_json_or_default(row, "external_ids")?
        } else {
            Vec::new()
        },
        root_folder_id: row.text("root_folder_id")?,
        created_by: row.opt_text("created_by")?,
        created_at: row.timestamp("created_at")?,
        year: row.opt_i32("year")?,
        overview: row.opt_text("overview")?,
        poster_url: row.opt_text("poster_url")?,
        poster_source_url: None,
        background_url: row.opt_text("background_url")?,
        background_source_url: None,
        sort_title: row.opt_text("sort_title")?,
        catalog_sort_key: row.text("catalog_sort_key")?,
        slug: row.opt_text("slug")?,
        imdb_id: row.opt_text("imdb_id")?,
        runtime_minutes: row.opt_i32("runtime_minutes")?,
        popularity: row.opt_f64("popularity")?,
        canonical_tags: vec![],
        content_status: row.opt_text("content_status")?,
        language: row.opt_text("language")?,
        first_aired: row.opt_text("first_aired")?,
        network: row.opt_text("network")?,
        studio: row.opt_text("studio")?,
        country: row.opt_text("country")?,
        aliases: decode_title_json_or_default(row, "aliases")?,
        tagged_aliases: decode_title_json_or_default(row, "tagged_aliases_json")?,
        metadata_language: row.opt_text("metadata_language")?,
        metadata_fetched_at: row.opt_timestamp("metadata_fetched_at")?,
        min_availability: row.opt_text("min_availability")?,
        digital_release_date: row.opt_text("digital_release_date")?,
        folder_path: row.opt_text("folder_path")?,
    };

    let poster_local_path = row.opt_text("poster_local_path")?;
    let background_local_path = row.opt_text("background_local_path")?;

    Ok(finalize_persisted_title(
        title,
        PersistedTitleDecodeOptions {
            mode,
            include_external_ids,
            base_path,
            poster_local_path: poster_local_path.as_deref(),
            background_local_path: background_local_path.as_deref(),
        },
    ))
}

fn parse_facet(raw: &str) -> MediaFacet {
    MediaFacet::parse(raw).unwrap_or_default()
}

fn apply_title_metadata_update(title: &mut Title, metadata: TitleMetadataUpdate) -> AppResult<()> {
    if let Some(name) = metadata.name.filter(|value| !value.is_empty()) {
        title.name = name;
    }
    if metadata.year.is_some() {
        title.year = metadata.year;
    }
    merge_optional_title_text(&mut title.overview, metadata.overview);
    merge_optional_title_text(&mut title.poster_url, metadata.poster_url);
    merge_optional_title_text(&mut title.background_url, metadata.background_url);
    merge_optional_title_text(&mut title.sort_title, metadata.sort_title);
    merge_optional_title_text(&mut title.slug, metadata.slug);
    merge_optional_title_text(&mut title.imdb_id, metadata.imdb_id);
    if metadata.runtime_minutes.is_some() {
        title.runtime_minutes = metadata.runtime_minutes;
    }
    title.popularity = metadata.popularity;
    merge_optional_title_text(&mut title.content_status, metadata.content_status);
    match metadata.language {
        MetadataFieldUpdate::Unchanged => {}
        MetadataFieldUpdate::Set(language) => title.language = Some(language),
        MetadataFieldUpdate::Clear => title.language = None,
    }
    merge_optional_title_text(&mut title.first_aired, metadata.first_aired);
    merge_optional_title_text(&mut title.network, metadata.network);
    merge_optional_title_text(&mut title.studio, metadata.studio);
    merge_optional_title_text(&mut title.country, metadata.country);
    if !metadata.aliases.is_empty() {
        title.aliases = metadata.aliases;
    }
    if !metadata.tagged_aliases.is_empty() {
        title.tagged_aliases = metadata.tagged_aliases;
    }
    merge_optional_title_text(&mut title.metadata_language, metadata.metadata_language);
    if let Some(raw) = metadata.metadata_fetched_at
        && !raw.is_empty()
    {
        title.metadata_fetched_at = Some(parse_utc_datetime(&raw)?);
    }
    merge_optional_title_text(
        &mut title.digital_release_date,
        metadata.digital_release_date,
    );
    merge_title_external_ids(&mut title.external_ids, metadata.extra_external_ids);
    merge_title_tags(&mut title.tags, metadata.extra_tags);
    Ok(())
}

fn title_has_tvdb_external_id(title: &Title) -> bool {
    title.external_ids.iter().any(|external_id| {
        let source = external_id.source.trim().to_ascii_lowercase();
        source == "tvdb"
    })
}

fn merge_optional_title_text(target: &mut Option<String>, incoming: Option<String>) {
    if let Some(incoming) = incoming
        && !incoming.is_empty()
    {
        *target = Some(incoming);
    }
}

fn merge_title_external_ids(target: &mut Vec<ExternalId>, incoming: Vec<ExternalId>) {
    for external_id in incoming {
        target.retain(|candidate| candidate.source != external_id.source);
        target.push(external_id);
    }
}

fn merge_title_tags(target: &mut Vec<String>, incoming: Vec<String>) {
    for tag in incoming {
        if let Some(colon_pos) = tag.rfind(':') {
            let prefix = &tag[..=colon_pos];
            target.retain(|candidate| !candidate.starts_with(prefix));
        }
        target.push(tag);
    }
}

fn build_name_filtered_title_list_sql(
    facet: Option<MediaFacet>,
    library_ids: Option<&[String]>,
    query: &str,
) -> (String, Vec<SqlArg>) {
    let mut sql = format!("SELECT {TITLE_COLUMNS} FROM titles");
    let mut where_clauses = Vec::<String>::new();
    let mut args = Vec::new();

    if let Some(library_ids) = library_ids {
        if library_ids.is_empty() {
            where_clauses.push("1 = 0".to_string());
        } else {
            let placeholders = std::iter::repeat_n("{}", library_ids.len())
                .collect::<Vec<_>>()
                .join(", ");
            where_clauses.push(format!("library_id IN ({placeholders})"));
            args.extend(library_ids.iter().cloned().map(SqlArg::Text));
        }
    }

    if let Some(facet) = facet {
        where_clauses.push("facet = {}".to_string());
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }

    where_clauses.push("LOWER(name) LIKE {}".to_string());
    args.push(SqlArg::Text(format!("%{}%", query.to_lowercase())));

    sql.push_str(" WHERE ");
    sql.push_str(&where_clauses.join(" AND "));
    sql.push_str(" ORDER BY LOWER(name), id");
    (sql, args)
}

fn build_plain_title_list_sql(
    facet: Option<MediaFacet>,
    library_ids: Option<&[String]>,
) -> (String, Vec<SqlArg>) {
    let mut sql = format!("SELECT {TITLE_COLUMNS} FROM titles");
    let mut where_clauses = Vec::<String>::new();
    let mut args = Vec::new();

    if let Some(facet) = facet {
        where_clauses.push("facet = {}".to_string());
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }

    if let Some(library_ids) = library_ids {
        if library_ids.is_empty() {
            where_clauses.push("1 = 0".to_string());
        } else {
            let placeholders = std::iter::repeat_n("{}", library_ids.len())
                .collect::<Vec<_>>()
                .join(", ");
            where_clauses.push(format!("library_id IN ({placeholders})"));
            args.extend(library_ids.iter().cloned().map(SqlArg::Text));
        }
    }

    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY LOWER(name), id");
    (sql, args)
}

fn build_title_catalog_count_sql(
    facet: Option<MediaFacet>,
    library_ids: &[String],
    query: Option<&str>,
    filter: &TitleCatalogFilter,
) -> (String, Vec<SqlArg>) {
    let mut sql = "SELECT COUNT(*) AS count FROM titles".to_string();
    let (where_sql, args) = build_title_catalog_where_sql(facet, library_ids, query, filter);
    if !where_sql.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_sql);
    }
    (sql, args)
}

async fn fetch_title_catalog_count(
    datastore: &StoreDatastore,
    facet: Option<MediaFacet>,
    library_ids: &[String],
    query: Option<&str>,
    filter: &TitleCatalogFilter,
) -> AppResult<usize> {
    let (sql, args) = build_title_catalog_count_sql(facet, library_ids, query, filter);
    Ok(
        SqlRuntime::fetch_optional(datastore.read_exec(), &sql, &args)
            .await?
            .map(|row| row.i64("count"))
            .transpose()?
            .unwrap_or(0)
            .max(0) as usize,
    )
}

async fn fetch_title_catalog_managed_bytes(
    datastore: &StoreDatastore,
    facet: Option<MediaFacet>,
    library_ids: &[String],
) -> AppResult<i64> {
    if library_ids.is_empty() {
        return Ok(0);
    }

    let (scope_sql, args) = build_title_catalog_options_scope_sql(facet, library_ids, &[]);
    let media_size_sql =
        title_catalog_media_size_subquery(title_catalog_dialect_for_datastore(datastore));
    let sql = format!(
        "SELECT CAST(COALESCE(SUM(COALESCE(catalog_media_size.total_size_bytes, 0)), 0) AS BIGINT) AS managed_bytes \
           FROM titles \
      LEFT JOIN ({media_size_sql}) catalog_media_size ON catalog_media_size.title_id = titles.id \
          WHERE {scope_sql}"
    );

    Ok(
        SqlRuntime::fetch_optional(datastore.read_exec(), &sql, &args)
            .await?
            .map(|row| row.i64("managed_bytes"))
            .transpose()?
            .unwrap_or(0)
            .max(0),
    )
}

async fn fetch_title_catalog_filter_counts(
    datastore: &StoreDatastore,
    facet: Option<MediaFacet>,
    library_ids: &[String],
    query: Option<&str>,
    active_filter: &TitleCatalogFilter,
) -> AppResult<TitleCatalogFilterCounts> {
    let all_filter = TitleCatalogFilter {
        monitored: None,
        content_statuses: Vec::new(),
        ..active_filter.clone()
    };
    let monitored_filter = TitleCatalogFilter {
        monitored: Some(true),
        content_statuses: active_filter.content_statuses.clone(),
        ..active_filter.clone()
    };
    let unmonitored_filter = TitleCatalogFilter {
        monitored: Some(false),
        content_statuses: active_filter.content_statuses.clone(),
        ..active_filter.clone()
    };
    let continuing_filter = TitleCatalogFilter {
        monitored: active_filter.monitored,
        content_statuses: vec![TitleCatalogContentStatus::Continuing],
        ..active_filter.clone()
    };
    let ended_filter = TitleCatalogFilter {
        monitored: active_filter.monitored,
        content_statuses: vec![TitleCatalogContentStatus::Ended],
        ..active_filter.clone()
    };

    Ok(TitleCatalogFilterCounts {
        all: fetch_title_catalog_count(datastore, facet.clone(), library_ids, query, &all_filter)
            .await?,
        monitored: fetch_title_catalog_count(
            datastore,
            facet.clone(),
            library_ids,
            query,
            &monitored_filter,
        )
        .await?,
        unmonitored: fetch_title_catalog_count(
            datastore,
            facet.clone(),
            library_ids,
            query,
            &unmonitored_filter,
        )
        .await?,
        continuing: fetch_title_catalog_count(
            datastore,
            facet.clone(),
            library_ids,
            query,
            &continuing_filter,
        )
        .await?,
        ended: fetch_title_catalog_count(datastore, facet, library_ids, query, &ended_filter)
            .await?,
    })
}

async fn fetch_title_catalog_filter_options(
    datastore: &StoreDatastore,
    facet: Option<MediaFacet>,
    library_ids: &[String],
    root_folder_ids: &[String],
) -> AppResult<TitleCatalogFilterOptions> {
    if library_ids.is_empty() {
        return Ok(TitleCatalogFilterOptions::default());
    }
    let (scope_sql, scope_args) =
        build_title_catalog_options_scope_sql(facet, library_ids, root_folder_ids);
    let tags_sql = format!(
        "SELECT catalog_tag.tag_key AS tag_key,
                LOWER(TRIM(catalog_tag.category)) AS category,
                MIN(catalog_tag.name) AS name
           FROM titles
           JOIN title_metadata_tags catalog_tag
             ON catalog_tag.title_id = titles.id
          WHERE {scope_sql}
            AND LOWER(TRIM(catalog_tag.category)) IN ('genre', 'theme')
          GROUP BY catalog_tag.tag_key, LOWER(TRIM(catalog_tag.category))
          ORDER BY category, LOWER(MIN(catalog_tag.name)), catalog_tag.tag_key"
    );
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), &tags_sql, &scope_args).await?;
    let mut genres = Vec::new();
    let mut tags = Vec::new();
    for row in rows {
        let option = TitleCatalogTagFilterOption {
            key: row.text("tag_key")?,
            name: row.text("name")?,
        };
        match row.text("category")?.as_str() {
            "genre" => genres.push(option),
            "theme" => tags.push(option),
            _ => {}
        }
    }

    let years_sql = format!(
        "SELECT MIN(titles.year) AS minimum_year,
                MAX(titles.year) AS maximum_year
           FROM titles
          WHERE {scope_sql}"
    );
    let years = SqlRuntime::fetch_optional(datastore.read_exec(), &years_sql, &scope_args).await?;

    Ok(TitleCatalogFilterOptions {
        genres,
        tags,
        minimum_year: years
            .as_ref()
            .map(|row| row.opt_i32("minimum_year"))
            .transpose()?
            .flatten(),
        maximum_year: years
            .as_ref()
            .map(|row| row.opt_i32("maximum_year"))
            .transpose()?
            .flatten(),
    })
}

fn build_title_catalog_options_scope_sql(
    facet: Option<MediaFacet>,
    library_ids: &[String],
    root_folder_ids: &[String],
) -> (String, Vec<SqlArg>) {
    let mut clauses = Vec::new();
    let mut args = Vec::new();
    let library_placeholders = std::iter::repeat_n("{}", library_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    clauses.push(format!("titles.library_id IN ({library_placeholders})"));
    args.extend(library_ids.iter().cloned().map(SqlArg::Text));

    if let Some(facet) = facet {
        clauses.push("titles.facet = {}".to_string());
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }
    if !root_folder_ids.is_empty() {
        let root_placeholders = std::iter::repeat_n("{}", root_folder_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        clauses.push(format!("titles.root_folder_id IN ({root_placeholders})"));
        args.extend(root_folder_ids.iter().cloned().map(SqlArg::Text));
    }

    (clauses.join(" AND "), args)
}

#[expect(
    clippy::too_many_arguments,
    reason = "title catalog SQL assembly mirrors the catalog query filter, sort, pagination, and dialect inputs"
)]
fn build_title_catalog_page_sql(
    facet: Option<MediaFacet>,
    library_ids: &[String],
    query: Option<&str>,
    filter: &TitleCatalogFilter,
    sort: TitleCatalogSort,
    limit: usize,
    offset: usize,
    dialect: TitleCatalogSqlDialect,
) -> (String, Vec<SqlArg>) {
    let mut sql = format!("SELECT {TITLE_COLUMNS} FROM titles");
    sql.push_str(&build_title_catalog_sort_join_sql(sort.key, dialect));
    let (where_sql, mut args) = build_title_catalog_where_sql(facet, library_ids, query, filter);
    if !where_sql.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_sql);
    }
    sql.push(' ');
    sql.push_str(&build_title_catalog_order_sql(sort, dialect));
    sql.push_str(" LIMIT {} OFFSET {}");
    args.push(SqlArg::I64(limit as i64));
    args.push(SqlArg::I64(offset as i64));
    (sql, args)
}

fn build_title_catalog_where_sql(
    facet: Option<MediaFacet>,
    library_ids: &[String],
    query: Option<&str>,
    filter: &TitleCatalogFilter,
) -> (String, Vec<SqlArg>) {
    let mut clauses = Vec::<String>::new();
    let mut args = Vec::new();

    if library_ids.is_empty() {
        clauses.push("1 = 0".to_string());
    } else {
        let placeholders = std::iter::repeat_n("{}", library_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        clauses.push(format!("library_id IN ({placeholders})"));
        args.extend(library_ids.iter().cloned().map(SqlArg::Text));
    }

    if let Some(facet) = facet {
        clauses.push("facet = {}".to_string());
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }

    if let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) {
        clauses.push("LOWER(name) LIKE {}".to_string());
        args.push(SqlArg::Text(format!("%{}%", query.to_lowercase())));
    }

    if !filter.root_folder_ids.is_empty() {
        let placeholders = std::iter::repeat_n("{}", filter.root_folder_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        clauses.push(format!("titles.root_folder_id IN ({placeholders})"));
        args.extend(filter.root_folder_ids.iter().cloned().map(SqlArg::Text));
    }

    if let Some(minimum_year) = filter.minimum_year {
        clauses.push("titles.year >= {}".to_string());
        args.push(SqlArg::I32(minimum_year));
    }
    if let Some(maximum_year) = filter.maximum_year {
        clauses.push("titles.year <= {}".to_string());
        args.push(SqlArg::I32(maximum_year));
    }

    if let Some(clause) =
        title_catalog_tag_filter_clause("genre", &filter.genre_tag_keys, &mut args)
    {
        clauses.push(clause);
    }
    if let Some(clause) =
        title_catalog_tag_filter_clause("theme", &filter.theme_tag_keys, &mut args)
    {
        clauses.push(clause);
    }

    if let Some(minimum_rating) = filter.minimum_rating {
        clauses.push(
            "EXISTS (
                SELECT 1
                  FROM title_metadata_rating_summaries catalog_rating
                 WHERE catalog_rating.title_id = titles.id
                   AND catalog_rating.rating >= {}
            )"
            .to_string(),
        );
        args.push(SqlArg::F64(minimum_rating));
    }

    if let Some(monitored) = filter.monitored {
        clauses.push("monitored = {}".to_string());
        args.push(SqlArg::Bool(monitored));
    }

    let statuses = title_catalog_content_status_values(&filter.content_statuses);
    if !statuses.is_empty() {
        let placeholders = std::iter::repeat_n("{}", statuses.len())
            .collect::<Vec<_>>()
            .join(", ");
        clauses.push(format!(
            "LOWER(TRIM(COALESCE(content_status, ''))) IN ({placeholders})"
        ));
        args.extend(statuses.into_iter().map(SqlArg::Text));
    }

    (clauses.join(" AND "), args)
}

fn title_catalog_tag_filter_clause(
    category: &str,
    tag_keys: &[String],
    args: &mut Vec<SqlArg>,
) -> Option<String> {
    if tag_keys.is_empty() {
        return None;
    }
    let placeholders = std::iter::repeat_n("{}", tag_keys.len())
        .collect::<Vec<_>>()
        .join(", ");
    args.push(SqlArg::Text(category.to_string()));
    args.extend(tag_keys.iter().cloned().map(SqlArg::Text));
    Some(format!(
        "EXISTS (
            SELECT 1
              FROM title_metadata_tags catalog_tag
             WHERE catalog_tag.title_id = titles.id
               AND LOWER(TRIM(catalog_tag.category)) = {{}}
               AND catalog_tag.tag_key IN ({placeholders})
        )"
    ))
}

fn title_catalog_content_status_values(statuses: &[TitleCatalogContentStatus]) -> Vec<String> {
    let mut normalized: Vec<String> = Vec::new();
    for status in statuses {
        let values = match status {
            TitleCatalogContentStatus::Continuing => ["continuing", "returning"],
            TitleCatalogContentStatus::Ended => ["ended", "finished"],
        };
        for value in values {
            if !normalized
                .iter()
                .any(|candidate| candidate.as_str() == value)
            {
                normalized.push(value.to_string());
            }
        }
    }
    normalized
}

fn build_title_catalog_sort_join_sql(
    key: TitleCatalogSortKey,
    dialect: TitleCatalogSqlDialect,
) -> String {
    match key {
        TitleCatalogSortKey::Library => " LEFT JOIN (
                SELECT id AS sort_library_id, name AS sort_library_name
                  FROM libraries
            ) title_catalog_library ON title_catalog_library.sort_library_id = titles.library_id"
            .to_string(),
        TitleCatalogSortKey::Size => format!(
            " LEFT JOIN ({}) catalog_media_size ON catalog_media_size.title_id = titles.id",
            title_catalog_media_size_subquery(dialect)
        ),
        TitleCatalogSortKey::Episodes => format!(
            " LEFT JOIN ({}) catalog_episode_progress ON catalog_episode_progress.title_id = titles.id",
            title_catalog_episode_progress_subquery(dialect)
        ),
        TitleCatalogSortKey::Root => " LEFT JOIN library_roots title_catalog_root
               ON title_catalog_root.id = titles.root_folder_id
              AND title_catalog_root.library_id = titles.library_id"
            .to_string(),
        TitleCatalogSortKey::MediaResolution
        | TitleCatalogSortKey::MediaHdr
        | TitleCatalogSortKey::MediaAudioCodec => format!(
            " LEFT JOIN ({}) catalog_movie_media ON catalog_movie_media.title_id = titles.id",
            title_catalog_movie_media_subquery(dialect)
        ),
        TitleCatalogSortKey::RatingScryer => {
            format!(
                " LEFT JOIN ({}) catalog_rating_scryer ON catalog_rating_scryer.title_id = titles.id",
                title_catalog_scryer_rating_subquery()
            )
        }
        key @ (TitleCatalogSortKey::RatingImdb
        | TitleCatalogSortKey::RatingRottenTomatoes
        | TitleCatalogSortKey::RatingPopcornmeter
        | TitleCatalogSortKey::RatingMetacritic
        | TitleCatalogSortKey::RatingMetacriticUser
        | TitleCatalogSortKey::RatingLetterboxd
        | TitleCatalogSortKey::RatingTmdb
        | TitleCatalogSortKey::RatingTvdb
        | TitleCatalogSortKey::RatingTrakt
        | TitleCatalogSortKey::RatingMyanimelist
        | TitleCatalogSortKey::RatingAnilist
        | TitleCatalogSortKey::RatingAnidb
        | TitleCatalogSortKey::RatingMdblist) => {
            let sources = title_catalog_rating_sources_for_sort_key(key).unwrap_or(&[]);
            format!(
                " LEFT JOIN ({}) catalog_external_rating ON catalog_external_rating.title_id = titles.id",
                title_catalog_external_rating_subquery(sources)
            )
        }
        TitleCatalogSortKey::Title
        | TitleCatalogSortKey::Monitored
        | TitleCatalogSortKey::Quality
        | TitleCatalogSortKey::Status
        | TitleCatalogSortKey::Added
        | TitleCatalogSortKey::Year
        | TitleCatalogSortKey::Runtime
        | TitleCatalogSortKey::Popularity => String::new(),
    }
}

fn build_title_catalog_order_sql(
    sort: TitleCatalogSort,
    dialect: TitleCatalogSqlDialect,
) -> String {
    let direction = match sort.direction {
        SortDirection::Asc => "ASC",
        SortDirection::Desc => "DESC",
    };
    match sort.key {
        TitleCatalogSortKey::Title => {
            let title_tie_expression = title_catalog_normalized_name_tie_expression_sql();
            format!(
                "ORDER BY catalog_sort_key {direction}, {title_tie_expression} {direction}, \
                 CASE WHEN year IS NULL THEN 1 ELSE 0 END ASC, year {direction}, id {direction}"
            )
        }
        TitleCatalogSortKey::Library => format!(
            "ORDER BY LOWER(COALESCE(NULLIF(TRIM(title_catalog_library.sort_library_name), ''), titles.library_id)) {direction}, {}",
            title_catalog_ascending_tie_order_sql()
        ),
        TitleCatalogSortKey::Monitored => format!(
            "ORDER BY monitored {direction}, {}",
            title_catalog_ascending_tie_order_sql()
        ),
        TitleCatalogSortKey::Quality => {
            let missing_direction = nullable_text_missing_direction(sort.direction);
            let expression = title_catalog_quality_profile_expression(dialect);
            format!(
                "ORDER BY CASE WHEN {expression} = '' THEN 1 ELSE 0 END {missing_direction}, \
                 {expression} {direction}, {}",
                title_catalog_ascending_tie_order_sql()
            )
        }
        TitleCatalogSortKey::Episodes => format!(
            "ORDER BY CASE
                 WHEN COALESCE(catalog_episode_progress.total_episodes, 0) > 0
                 THEN (catalog_episode_progress.owned_episodes * 1.0) / catalog_episode_progress.total_episodes
                 ELSE -1.0
             END {direction},
             COALESCE(catalog_episode_progress.owned_episodes, 0) {direction},
             COALESCE(catalog_episode_progress.total_episodes, 0) {direction}, {}",
            title_catalog_ascending_tie_order_sql()
        ),
        TitleCatalogSortKey::Status => {
            let missing_direction = nullable_text_missing_direction(sort.direction);
            let raw_expression = "LOWER(TRIM(COALESCE(content_status, '')))";
            let expression = format!(
                "CASE {raw_expression} \
                 WHEN 'returning' THEN 'continuing' \
                 WHEN 'finished' THEN 'ended' \
                 ELSE {raw_expression} END"
            );
            format!(
                "ORDER BY CASE WHEN {expression} = '' THEN 1 ELSE 0 END {missing_direction}, \
                 {expression} {direction}, {}",
                title_catalog_ascending_tie_order_sql()
            )
        }
        TitleCatalogSortKey::Size => format!(
            "ORDER BY COALESCE(catalog_media_size.total_size_bytes, -1) {direction}, {}",
            title_catalog_ascending_tie_order_sql()
        ),
        TitleCatalogSortKey::Added => format!(
            "ORDER BY created_at {direction}, {}",
            title_catalog_ascending_tie_order_sql()
        ),
        TitleCatalogSortKey::Year => format!(
            "ORDER BY CASE WHEN year IS NULL THEN 1 ELSE 0 END ASC, year {direction}, {}",
            title_catalog_ascending_tie_order_sql()
        ),
        TitleCatalogSortKey::Runtime => format!(
            "ORDER BY CASE WHEN runtime_minutes IS NULL THEN 1 ELSE 0 END ASC, runtime_minutes {direction}, {}",
            title_catalog_ascending_tie_order_sql()
        ),
        TitleCatalogSortKey::Root => format!(
            "ORDER BY CASE WHEN NULLIF(TRIM(COALESCE(title_catalog_root.path, '')), '') IS NULL THEN 1 ELSE 0 END ASC,
             LOWER(TRIM(COALESCE(title_catalog_root.path, ''))) {direction}, {}",
            title_catalog_ascending_tie_order_sql()
        ),
        TitleCatalogSortKey::Popularity => format!(
            "ORDER BY CASE WHEN popularity IS NULL THEN 1 ELSE 0 END ASC, popularity {direction}, {}",
            title_catalog_ascending_tie_order_sql()
        ),
        TitleCatalogSortKey::MediaResolution => format!(
            "ORDER BY CASE WHEN catalog_movie_media.resolution_rank IS NULL THEN 1 ELSE 0 END ASC,
             catalog_movie_media.resolution_rank {direction}, {}",
            title_catalog_ascending_tie_order_sql()
        ),
        TitleCatalogSortKey::MediaHdr => format!(
            "ORDER BY CASE WHEN catalog_movie_media.hdr_rank IS NULL THEN 1 ELSE 0 END ASC,
             catalog_movie_media.hdr_rank {direction}, {}",
            title_catalog_ascending_tie_order_sql()
        ),
        TitleCatalogSortKey::MediaAudioCodec => format!(
            "ORDER BY CASE WHEN NULLIF(TRIM(COALESCE(catalog_movie_media.audio_codec, '')), '') IS NULL THEN 1 ELSE 0 END ASC,
             LOWER(TRIM(COALESCE(catalog_movie_media.audio_codec, ''))) {direction}, {}",
            title_catalog_ascending_tie_order_sql()
        ),
        TitleCatalogSortKey::RatingScryer => title_catalog_numeric_sort_order_sql(
            "catalog_rating_scryer.rating",
            direction,
        ),
        TitleCatalogSortKey::RatingImdb
        | TitleCatalogSortKey::RatingRottenTomatoes
        | TitleCatalogSortKey::RatingPopcornmeter
        | TitleCatalogSortKey::RatingMetacritic
        | TitleCatalogSortKey::RatingMetacriticUser
        | TitleCatalogSortKey::RatingLetterboxd
        | TitleCatalogSortKey::RatingTmdb
        | TitleCatalogSortKey::RatingTvdb
        | TitleCatalogSortKey::RatingTrakt
        | TitleCatalogSortKey::RatingMyanimelist
        | TitleCatalogSortKey::RatingAnilist
        | TitleCatalogSortKey::RatingAnidb
        | TitleCatalogSortKey::RatingMdblist => {
            title_catalog_numeric_sort_order_sql("catalog_external_rating.normalized", direction)
        }
    }
}

fn title_catalog_numeric_sort_order_sql(expression: &str, direction: &str) -> String {
    format!(
        "ORDER BY CASE WHEN {expression} IS NULL THEN 1 ELSE 0 END ASC, {expression} {direction}, {}",
        title_catalog_ascending_tie_order_sql()
    )
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn title_catalog_cjk_width_normalization_chars() -> impl Iterator<Item = char> {
    std::iter::once('\u{3000}').chain('\u{ff01}'..='\u{ff5e}')
}

fn title_catalog_normalized_name_expression_sql() -> String {
    let mut expression = "name".to_string();
    for value in title_catalog_cjk_width_normalization_chars() {
        let source = value.to_string();
        let replacement = source.nfkc().collect::<String>();
        if source == replacement {
            continue;
        }
        expression = format!(
            "REPLACE({expression}, {}, {})",
            sql_string_literal(&source),
            sql_string_literal(&replacement),
        );
    }
    expression
}

// Secondary tiebreak for titles that share a primary `catalog_sort_key` (primary strength is
// case/accent-insensitive). Mirrors the Rust `title_catalog_name_tie_key`, but the case-folding
// is engine-specific: SQLite `LOWER()` is ASCII-only while Postgres and the Rust comparator fold
// full Unicode, so the tie order of equal-primary titles that differ only in non-ASCII case can
// vary across engines. That is cosmetic — the trailing `id` ordering keeps every result set a
// deterministic total order. Making the secondary order identical across all three would require
// a precomputed tie-key column.
fn title_catalog_normalized_name_tie_expression_sql() -> String {
    format!(
        "LOWER(TRIM({}))",
        title_catalog_normalized_name_expression_sql()
    )
}

fn title_catalog_ascending_tie_order_sql() -> String {
    format!(
        "{} ASC, {} ASC, \
         CASE WHEN year IS NULL THEN 1 ELSE 0 END ASC, year ASC, id ASC",
        "catalog_sort_key",
        title_catalog_normalized_name_tie_expression_sql()
    )
}

fn nullable_text_missing_direction(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Asc => "ASC",
        SortDirection::Desc => "DESC",
    }
}

fn title_catalog_dialect_for_datastore(datastore: &StoreDatastore) -> TitleCatalogSqlDialect {
    match datastore {
        StoreDatastore::Sqlite { .. } => TitleCatalogSqlDialect::Sqlite,
        StoreDatastore::Postgres { .. } => TitleCatalogSqlDialect::Postgres,
    }
}

fn title_catalog_live_media_file_predicate(dialect: TitleCatalogSqlDialect, alias: &str) -> String {
    match dialect {
        TitleCatalogSqlDialect::Sqlite => {
            format!("instr({alias}.file_path, '{RECYCLE_BIN_PATH_SEGMENT}') = 0")
        }
        TitleCatalogSqlDialect::Postgres => {
            format!("POSITION('{RECYCLE_BIN_PATH_SEGMENT}' IN {alias}.file_path) = 0")
        }
    }
}

fn title_catalog_total_size_sum_expression(
    dialect: TitleCatalogSqlDialect,
    expression: &str,
) -> String {
    match dialect {
        TitleCatalogSqlDialect::Sqlite => format!("COALESCE(SUM({expression}), 0)"),
        TitleCatalogSqlDialect::Postgres => format!("COALESCE(SUM({expression}), 0)::BIGINT"),
    }
}

fn title_catalog_bool_column_is_true(dialect: TitleCatalogSqlDialect, column: &str) -> String {
    match dialect {
        TitleCatalogSqlDialect::Sqlite => format!("{column} = 1"),
        TitleCatalogSqlDialect::Postgres => column.to_string(),
    }
}

fn title_catalog_quality_profile_expression(dialect: TitleCatalogSqlDialect) -> String {
    match dialect {
        TitleCatalogSqlDialect::Sqlite => format!(
            "COALESCE((
                SELECT LOWER(TRIM(SUBSTR(tag.value, LENGTH('{TITLE_QUALITY_PROFILE_TAG_PREFIX}') + 1)))
                  FROM json_each(titles.tags) AS tag
                 WHERE tag.value LIKE '{TITLE_QUALITY_PROFILE_TAG_PREFIX}%'
                 LIMIT 1
            ), '')"
        ),
        TitleCatalogSqlDialect::Postgres => format!(
            "COALESCE((
                SELECT LOWER(TRIM(SUBSTR(tag.value, LENGTH('{TITLE_QUALITY_PROFILE_TAG_PREFIX}') + 1)))
                  FROM jsonb_array_elements_text(titles.tags) AS tag(value)
                 WHERE tag.value LIKE '{TITLE_QUALITY_PROFILE_TAG_PREFIX}%'
                 LIMIT 1
            ), '')"
        ),
    }
}

fn title_catalog_rating_sources_for_sort_key(
    key: TitleCatalogSortKey,
) -> Option<&'static [&'static str]> {
    match key {
        TitleCatalogSortKey::RatingImdb => Some(&["imdb"]),
        TitleCatalogSortKey::RatingRottenTomatoes => Some(&["rottentomatoes", "tomatoes"]),
        TitleCatalogSortKey::RatingPopcornmeter => Some(&["popcornmeter", "popcorn", "audience"]),
        TitleCatalogSortKey::RatingMetacritic => Some(&["metacritic"]),
        TitleCatalogSortKey::RatingMetacriticUser => Some(&["metacriticuser", "mcuser"]),
        TitleCatalogSortKey::RatingLetterboxd => Some(&["letterboxd"]),
        TitleCatalogSortKey::RatingTmdb => Some(&["tmdb"]),
        TitleCatalogSortKey::RatingTvdb => Some(&["tvdb", "thetvdb"]),
        TitleCatalogSortKey::RatingTrakt => Some(&["trakt"]),
        TitleCatalogSortKey::RatingMyanimelist => Some(&["mal", "myanimelist", "myanimelistnet"]),
        TitleCatalogSortKey::RatingAnilist => Some(&["anilist"]),
        TitleCatalogSortKey::RatingAnidb => Some(&["anidb"]),
        TitleCatalogSortKey::RatingMdblist => Some(&["mdblist"]),
        _ => None,
    }
}

fn title_catalog_normalized_rating_source_expression(alias: &str) -> String {
    format!("LOWER(REPLACE(REPLACE(REPLACE(TRIM({alias}.source), ' ', ''), '_', ''), '-', ''))")
}

fn title_catalog_scryer_rating_subquery() -> String {
    "SELECT rating.title_id,
            MAX(rating.rating) AS rating
       FROM title_metadata_rating_summaries rating
      GROUP BY rating.title_id"
        .to_string()
}

fn title_catalog_external_rating_subquery(sources: &[&str]) -> String {
    let source_list = sources
        .iter()
        .map(|source| sql_string_literal(source))
        .collect::<Vec<_>>()
        .join(", ");
    let source_expression = title_catalog_normalized_rating_source_expression("ter");
    format!(
        "SELECT ter.title_id,
                MAX(CASE
                    WHEN ter.normalized <= 1.0 THEN ter.normalized * 10.0
                    ELSE ter.normalized
                END) AS normalized
           FROM title_metadata_external_ratings ter
          WHERE {source_expression} IN ({source_list})
          GROUP BY ter.title_id"
    )
}

fn title_catalog_movie_media_resolution_expression(alias: &str) -> String {
    format!(
        "CASE
            WHEN {alias}.video_width >= 7680 OR {alias}.video_height >= 4200 THEN '4320P'
            WHEN {alias}.video_width >= 3840 OR {alias}.video_height >= 2100 THEN '2160P'
            WHEN {alias}.video_height >= 1300 THEN '1440P'
            WHEN {alias}.video_width >= 1920 OR {alias}.video_height >= 1000 THEN '1080P'
            WHEN {alias}.video_width >= 1280 OR {alias}.video_height >= 700 THEN '720P'
            WHEN {alias}.video_width >= 854 OR {alias}.video_height >= 480 THEN '480P'
            WHEN {alias}.video_height >= 300 THEN '360P'
            WHEN TRIM(COALESCE({alias}.quality_id, '')) = '' THEN NULL
            ELSE UPPER(TRIM({alias}.quality_id))
         END"
    )
}

fn title_catalog_movie_media_resolution_rank_expression(alias: &str) -> String {
    format!(
        "CASE
            WHEN {alias}.video_width >= 7680 OR {alias}.video_height >= 4200 THEN 4320
            WHEN {alias}.video_width >= 3840 OR {alias}.video_height >= 2100 THEN 2160
            WHEN {alias}.video_height >= 1300 THEN 1440
            WHEN {alias}.video_width >= 1920 OR {alias}.video_height >= 1000 THEN 1080
            WHEN {alias}.video_width >= 1280 OR {alias}.video_height >= 700 THEN 720
            WHEN {alias}.video_width >= 854 OR {alias}.video_height >= 480 THEN 480
            WHEN {alias}.video_height >= 300 THEN 360
            ELSE CASE UPPER(TRIM(COALESCE({alias}.quality_id, '')))
                WHEN '4320P' THEN 4320
                WHEN '2160P' THEN 2160
                WHEN '1440P' THEN 1440
                WHEN '1080P' THEN 1080
                WHEN '1080I' THEN 1080
                WHEN '720P' THEN 720
                WHEN '480P' THEN 480
                WHEN '360P' THEN 360
                ELSE NULL
            END
         END"
    )
}

fn title_catalog_movie_media_hdr_rank_expression(alias: &str) -> String {
    format!(
        "CASE
            WHEN UPPER(TRIM(COALESCE({alias}.video_hdr_format, ''))) LIKE '%DOLBY%'
              OR UPPER(TRIM(COALESCE({alias}.video_hdr_format, ''))) = 'DV' THEN 4
            WHEN UPPER(TRIM(COALESCE({alias}.video_hdr_format, ''))) LIKE '%HDR10+%' THEN 3
            WHEN UPPER(TRIM(COALESCE({alias}.video_hdr_format, ''))) LIKE '%HDR10%' THEN 2
            WHEN TRIM(COALESCE({alias}.video_hdr_format, '')) <> '' THEN 1
            ELSE 0
         END"
    )
}

fn title_catalog_movie_media_subquery(dialect: TitleCatalogSqlDialect) -> String {
    let resolution = title_catalog_movie_media_resolution_expression("mf");
    let resolution_rank = title_catalog_movie_media_resolution_rank_expression("mf");
    let hdr_rank = title_catalog_movie_media_hdr_rank_expression("mf");
    format!(
        "SELECT title_id, resolution, hdr_format, audio_codec, resolution_rank, hdr_rank
           FROM (
                SELECT mf.title_id,
                       {resolution} AS resolution,
                       NULLIF(TRIM(mf.video_hdr_format), '') AS hdr_format,
                       COALESCE(NULLIF(TRIM(mf.audio_codec_parsed), ''), NULLIF(TRIM(mf.audio_codec), '')) AS audio_codec,
                       {resolution_rank} AS resolution_rank,
                       {hdr_rank} AS hdr_rank,
                       ROW_NUMBER() OVER (
                          PARTITION BY mf.title_id
                          ORDER BY CASE WHEN mf.size_bytes > 0 THEN mf.size_bytes ELSE 0 END DESC,
                                   mf.created_at DESC,
                                   mf.id DESC
                       ) AS media_row
                  FROM media_files mf
                  JOIN titles t ON t.id = mf.title_id
                 WHERE t.facet = 'movie'
                   AND {}
                   AND mf.role = 'primary'
           ) ranked
          WHERE media_row = 1",
        title_catalog_live_media_file_predicate(dialect, "mf")
    )
}

fn title_catalog_media_size_subquery(dialect: TitleCatalogSqlDialect) -> String {
    let total_size_expression =
        title_catalog_total_size_sum_expression(dialect, "matched.size_bytes");
    format!(
        "SELECT matched.title_id,
                {total_size_expression} AS total_size_bytes
           FROM (
                SELECT DISTINCT mf.id,
                       mf.title_id,
                       CASE
                           WHEN mf.size_bytes > 0 THEN mf.size_bytes
                           ELSE 0
                       END AS size_bytes
                  FROM media_files mf
             LEFT JOIN file_episode_map fem
                    ON fem.file_id = mf.id
             LEFT JOIN collections c
                    ON c.title_id = mf.title_id
                   AND c.ordered_path = mf.file_path
             LEFT JOIN file_series_movie_link_map fsmlm
                    ON fsmlm.file_id = mf.id
                 WHERE {}
                   AND (
                       fem.file_id IS NOT NULL
                       OR c.id IS NOT NULL
                       OR fsmlm.file_id IS NOT NULL
                   )
           ) matched
          GROUP BY matched.title_id",
        title_catalog_live_media_file_predicate(dialect, "mf")
    )
}

fn title_catalog_episode_progress_subquery(dialect: TitleCatalogSqlDialect) -> String {
    format!(
        "SELECT e.title_id,
                COUNT(DISTINCT e.id) AS total_episodes,
                COUNT(DISTINCT CASE WHEN {} THEN e.id END) AS monitored_episodes,
                COUNT(DISTINCT CASE WHEN mf.id IS NOT NULL THEN e.id END) AS owned_episodes
           FROM episodes e
          INNER JOIN collections c ON c.id = e.collection_id
           LEFT JOIN file_episode_map fem ON fem.episode_id = e.id
           LEFT JOIN media_files mf ON mf.id = fem.file_id AND {} AND mf.role = 'primary'
          WHERE c.collection_type <> 'specials'
            AND c.collection_index <> '0'
            AND trim(COALESCE(e.title, '')) <> ''
            AND upper(trim(e.title)) NOT IN ('TBA', 'TBD')
            AND trim(COALESCE(e.air_date, '')) <> ''
          GROUP BY e.title_id",
        title_catalog_bool_column_is_true(dialect, "e.monitored"),
        title_catalog_live_media_file_predicate(dialect, "mf")
    )
}

fn build_title_page_after_id_sql(after_id: Option<&str>, limit: usize) -> (String, Vec<SqlArg>) {
    let mut sql = format!("SELECT {TITLE_COLUMNS} FROM titles");
    let mut args = Vec::new();
    if let Some(after_id) = after_id {
        sql.push_str(" WHERE id > {}");
        args.push(SqlArg::Text(after_id.to_string()));
    }
    sql.push_str(" ORDER BY id LIMIT {}");
    args.push(SqlArg::I64(limit as i64));
    (sql, args)
}

fn build_ranked_title_list_sql(
    facet: Option<MediaFacet>,
    library_ids: Option<&[String]>,
    query: &str,
) -> (String, Vec<SqlArg>) {
    let normalized = normalize_title_search_text(query);
    let mut sql = format!(
        "SELECT {TITLE_COLUMNS}
           FROM titles
           JOIN (
                SELECT title_id,
                       MIN(
                           CASE
                               WHEN normalized_term = {{}} THEN 0
                               WHEN normalized_term LIKE {{}} THEN 1000
                               WHEN normalized_term LIKE {{}} THEN 2000
                               ELSE 3000
                           END + weight
                       ) AS rank
                  FROM title_search_terms"
    );
    let mut where_clauses = Vec::new();
    let mut args = vec![
        SqlArg::Text(normalized.clone()),
        SqlArg::Text(format!("{normalized}%")),
        SqlArg::Text(format!("%{normalized}%")),
    ];

    if let Some(facet) = facet {
        where_clauses.push("facet = {}");
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }
    where_clauses.push("term_kind NOT LIKE '%_token'");
    where_clauses.push("normalized_term LIKE {}");
    args.push(SqlArg::Text(format!("%{normalized}%")));

    sql.push_str(" WHERE ");
    sql.push_str(&where_clauses.join(" AND "));
    sql.push_str(
        " GROUP BY title_id
           ) ranked_titles ON ranked_titles.title_id = titles.id",
    );

    if let Some(library_ids) = library_ids {
        if library_ids.is_empty() {
            sql.push_str(" WHERE 1 = 0");
        } else {
            let placeholders = std::iter::repeat_n("{}", library_ids.len())
                .collect::<Vec<_>>()
                .join(", ");
            sql.push_str(&format!(" WHERE library_id IN ({placeholders})"));
            args.extend(library_ids.iter().cloned().map(SqlArg::Text));
        }
    }

    sql.push_str(" ORDER BY ranked_titles.rank, LOWER(name), id");
    (sql, args)
}

async fn load_title_tx(
    tx: &mut SqlTx<'_>,
    id: &str,
    include_external_ids: bool,
) -> AppResult<Option<Title>> {
    load_title_tx_with_mode(
        tx,
        id,
        include_external_ids,
        PersistedTitleReadMode::Presentation,
    )
    .await
}

async fn load_title_tx_with_mode(
    tx: &mut SqlTx<'_>,
    id: &str,
    include_external_ids: bool,
    mode: PersistedTitleReadMode,
) -> AppResult<Option<Title>> {
    let sql = format!("SELECT {TITLE_COLUMNS} FROM titles WHERE id = {{}}");
    let row =
        SqlRuntime::fetch_optional(SqlExec::Tx(tx), &sql, &[SqlArg::Text(id.to_string())]).await?;
    let mut title = decode_optional_runtime_title_row(row.as_ref(), mode, include_external_ids)?;
    if let Some(title) = title.as_mut() {
        attach_metadata_tags_to_titles(SqlExec::Tx(tx), std::slice::from_mut(title)).await?;
    }
    Ok(title)
}

async fn load_title_tx_or_not_found(
    tx: &mut SqlTx<'_>,
    id: &str,
    include_external_ids: bool,
) -> AppResult<Title> {
    load_title_tx(tx, id, include_external_ids)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title {id}")))
}

async fn load_title_canonical_tx_or_not_found(
    tx: &mut SqlTx<'_>,
    id: &str,
    include_external_ids: bool,
) -> AppResult<Title> {
    load_title_tx_with_mode(
        tx,
        id,
        include_external_ids,
        PersistedTitleReadMode::Canonical,
    )
    .await?
    .ok_or_else(|| AppError::NotFound(format!("title {id}")))
}

async fn list_existing_title_ids_for_external_ids_tx(
    tx: &mut SqlTx<'_>,
    library_id: &str,
    external_ids: &[(String, String)],
) -> AppResult<Vec<String>> {
    if external_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut sql =
        "SELECT DISTINCT title_id FROM title_external_ids WHERE library_id = {}".to_string();
    let mut args = vec![SqlArg::Text(library_id.to_string())];
    sql.push_str(" AND (");
    for (index, (source, value)) in external_ids.iter().enumerate() {
        if index > 0 {
            sql.push_str(" OR ");
        }
        sql.push_str("(LOWER(source) = LOWER({}) AND external_id = {})");
        args.push(SqlArg::Text(source.clone()));
        args.push(SqlArg::Text(value.clone()));
    }
    sql.push(')');

    SqlRuntime::fetch_all(SqlExec::Tx(tx), &sql, &args)
        .await?
        .into_iter()
        .map(|row| row.text("title_id"))
        .collect()
}

async fn find_existing_title_for_create_tx(
    tx: &mut SqlTx<'_>,
    title: &Title,
) -> AppResult<Option<Title>> {
    let external_ids = normalized_external_ids(&title.external_ids);
    if !external_ids.is_empty() {
        let title_ids =
            list_existing_title_ids_for_external_ids_tx(tx, &title.library_id, &external_ids)
                .await?;
        match title_ids.as_slice() {
            [] => {}
            [existing_id] => return load_title_tx(tx, existing_id, true).await,
            _ => {
                return Err(AppError::Validation(
                    "external ids already map to multiple titles".to_string(),
                ));
            }
        }
    }

    Ok(None)
}

async fn create_title_tx(tx: &mut SqlTx<'_>, title: &Title) -> AppResult<()> {
    let library = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id FROM libraries WHERE id = {} AND facet = {}",
        &[
            SqlArg::Text(title.library_id.clone()),
            SqlArg::Text(title.facet.as_str().to_string()),
        ],
    )
    .await?;
    if library.is_none() {
        return Err(AppError::Validation(format!(
            "library {} does not exist for facet {}",
            title.library_id,
            title.facet.as_str()
        )));
    }

    let args = title_write_args(title, scheduled_hydration_attempt(title), 0);
    SqlRuntime::execute(SqlExec::Tx(tx), TITLE_INSERT_SQL, &args).await?;
    replace_title_search_projection_sql_tx(tx, title).await?;
    replace_title_external_ids_projection_sql_tx(tx, title).await?;
    Ok(())
}

async fn resolve_pending_import_for_created_title_tx(
    tx: &mut SqlTx<'_>,
    pending_import_id: &str,
    title: &Title,
) -> AppResult<()> {
    if title.facet == MediaFacet::Movie {
        let rows = SqlRuntime::execute(
            SqlExec::Tx(tx),
            "DELETE FROM library_scan_unmatched_items
              WHERE id = {}
                AND library_id = {}
                AND facet = {}
                AND title_id IS NULL",
            &[
                SqlArg::Text(pending_import_id.to_string()),
                SqlArg::Text(title.library_id.clone()),
                SqlArg::Text(title.facet.as_str().to_string()),
            ],
        )
        .await?;
        if rows != 1 {
            return Err(AppError::Validation(format!(
                "pending import {pending_import_id} could not be resolved for title {}",
                title.id
            )));
        }
        return Ok(());
    }

    let now = Utc::now();
    let updated_at = match &*tx {
        SqlTx::Sqlite(_) => SqlArg::Text(now.to_rfc3339()),
        SqlTx::Postgres(_) => SqlArg::Timestamp(now),
    };
    let rows = SqlRuntime::execute(
        SqlExec::Tx(tx),
        "UPDATE library_scan_unmatched_items
            SET title_id = {},
                status = {},
                updated_at = {}
          WHERE id = {}
            AND library_id = {}
            AND facet = {}
            AND title_id IS NULL",
        &[
            SqlArg::Text(title.id.clone()),
            SqlArg::Text(PendingImportStatus::Pending.as_str().to_string()),
            updated_at,
            SqlArg::Text(pending_import_id.to_string()),
            SqlArg::Text(title.library_id.clone()),
            SqlArg::Text(title.facet.as_str().to_string()),
        ],
    )
    .await?;
    if rows != 1 {
        return Err(AppError::Validation(format!(
            "pending import {pending_import_id} could not be resolved for title {}",
            title.id
        )));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum HydrationStateWrite {
    Preserve,
    Reschedule,
    Clear,
}

fn set_reused_title_option_tag(tags: &mut Vec<String>, prefix: &str, value: Option<String>) {
    tags.retain(|tag| !tag.starts_with(prefix));
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        tags.push(format!("{prefix}{}", value.trim()));
    }
}

fn apply_reused_title_options_patch(
    title: &mut Title,
    patch: &TitleOptionsPatch,
    requested_root_folder_id: &str,
) {
    if let Some(value) = &patch.quality_profile_id {
        set_reused_title_option_tag(&mut title.tags, "scryer:quality-profile:", value.clone());
    }
    if let Some(value) = &patch.monitor_type {
        set_reused_title_option_tag(&mut title.tags, "scryer:monitor-type:", value.clone());
    }
    if let Some(value) = &patch.filler_policy {
        set_reused_title_option_tag(&mut title.tags, "scryer:filler-policy:", value.clone());
    }
    if let Some(value) = &patch.recap_policy {
        set_reused_title_option_tag(&mut title.tags, "scryer:recap-policy:", value.clone());
    }
    if let Some(value) = patch.use_season_folders {
        set_reused_title_option_tag(
            &mut title.tags,
            "scryer:season-folder:",
            value.map(|enabled| if enabled { "enabled" } else { "disabled" }.to_string()),
        );
    }
    if let Some(value) = patch.monitor_specials {
        set_reused_title_option_tag(
            &mut title.tags,
            "scryer:monitor-specials:",
            value.map(|enabled| if enabled { "true" } else { "false" }.to_string()),
        );
    }
    if let Some(value) = patch.inter_season_movies {
        set_reused_title_option_tag(
            &mut title.tags,
            "scryer:inter-season-movies:",
            value.map(|enabled| if enabled { "true" } else { "false" }.to_string()),
        );
    }
    if patch.root_folder_id.is_some() {
        title.root_folder_id = requested_root_folder_id.to_string();
    }
}

async fn persist_title_tx(
    tx: &mut SqlTx<'_>,
    title: &Title,
    hydration_state: HydrationStateWrite,
) -> AppResult<()> {
    let preserve_hydration_state = matches!(hydration_state, HydrationStateWrite::Preserve);
    let (metadata_hydration_next_attempt_at, metadata_hydration_attempt_count) =
        match hydration_state {
            HydrationStateWrite::Preserve | HydrationStateWrite::Clear => (None, 0),
            HydrationStateWrite::Reschedule => (scheduled_hydration_attempt(title), 0),
        };
    let mut args = title_write_args(
        title,
        metadata_hydration_next_attempt_at,
        metadata_hydration_attempt_count,
    );
    args.push(SqlArg::Bool(preserve_hydration_state));
    args.push(SqlArg::Bool(preserve_hydration_state));
    SqlRuntime::execute(SqlExec::Tx(tx), TITLE_UPSERT_SQL, &args).await?;
    replace_title_search_projection_sql_tx(tx, title).await?;
    replace_title_external_ids_projection_sql_tx(tx, title).await?;
    Ok(())
}

fn scheduled_hydration_attempt(title: &Title) -> Option<chrono::DateTime<Utc>> {
    if title.metadata_fetched_at.is_none() && title_has_tvdb_external_id(title) {
        Some(Utc::now())
    } else {
        None
    }
}

fn title_write_args(
    title: &Title,
    metadata_hydration_next_attempt_at: Option<chrono::DateTime<Utc>>,
    metadata_hydration_attempt_count: i64,
) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(title.id.clone()),
        SqlArg::Text(title.library_id.clone()),
        SqlArg::Text(title.name.clone()),
        SqlArg::Text(title.facet.as_str().to_string()),
        SqlArg::Bool(title.monitored),
        SqlArg::Json(serde_json::to_value(&title.tags).unwrap_or(JsonValue::Array(Vec::new()))),
        SqlArg::Json(
            serde_json::to_value(&title.external_ids).unwrap_or(JsonValue::Array(Vec::new())),
        ),
        SqlArg::Text(title.root_folder_id.clone()),
        SqlArg::OptText(title.created_by.clone()),
        SqlArg::Timestamp(title.created_at),
        SqlArg::OptI64(title.year.map(i64::from)),
        SqlArg::OptText(title.overview.clone()),
        SqlArg::OptText(title.poster_url.clone()),
        SqlArg::OptText(title.background_url.clone()),
        SqlArg::OptText(title.sort_title.clone()),
        SqlArg::Text(title_catalog_sort_key(
            &title.name,
            title.metadata_language.as_deref(),
        )),
        SqlArg::OptText(title.slug.clone()),
        SqlArg::OptText(title.imdb_id.clone()),
        SqlArg::OptI64(title.runtime_minutes.map(i64::from)),
        SqlArg::OptF64(title.popularity),
        SqlArg::OptText(title.content_status.clone()),
        SqlArg::OptText(title.language.clone()),
        SqlArg::OptText(title.first_aired.clone()),
        SqlArg::OptText(title.network.clone()),
        SqlArg::OptText(title.studio.clone()),
        SqlArg::OptText(title.country.clone()),
        SqlArg::Json(serde_json::to_value(&title.aliases).unwrap_or(JsonValue::Array(Vec::new()))),
        SqlArg::OptText(title.metadata_language.clone()),
        SqlArg::OptTimestamp(title.metadata_fetched_at),
        SqlArg::OptText(title.min_availability.clone()),
        SqlArg::OptText(title.digital_release_date.clone()),
        SqlArg::OptText(title.folder_path.clone()),
        SqlArg::Json(
            serde_json::to_value(&title.tagged_aliases).unwrap_or(JsonValue::Array(Vec::new())),
        ),
        SqlArg::OptTimestamp(metadata_hydration_next_attempt_at),
        SqlArg::I64(metadata_hydration_attempt_count),
    ]
}

async fn load_title_ratings(
    datastore: &StoreDatastore,
    title_id: &str,
) -> AppResult<TitleRatingSummary> {
    let mut ratings =
        load_title_metadata_ratings(datastore.read_exec(), &[title_id.to_string()]).await?;
    Ok(ratings.remove(title_id).unwrap_or_default())
}

async fn load_title_ratings_batch(
    datastore: &StoreDatastore,
    title_ids: &[String],
) -> AppResult<Vec<(String, TitleRatingSummary)>> {
    if title_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut by_title_id = load_title_metadata_ratings(datastore.read_exec(), title_ids)
        .await?
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();

    Ok(title_ids
        .iter()
        .map(|title_id| {
            let summary = by_title_id.remove(title_id).unwrap_or_default();
            (title_id.clone(), summary)
        })
        .collect())
}

fn is_title_external_id_conflict_error(error: &AppError) -> bool {
    matches!(
        error,
        AppError::Repository(message)
            if message.contains("UNIQUE constraint failed")
                || message.contains("duplicate key value violates unique constraint")
    )
}

async fn replace_title_external_ids_projection_sql_tx(
    tx: &mut SqlTx<'_>,
    title: &Title,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM title_external_ids WHERE title_id = {}",
        &[SqlArg::Text(title.id.clone())],
    )
    .await?;

    let now = Utc::now();
    for (source, value) in normalized_external_ids(&title.external_ids) {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO title_external_ids
             (id, title_id, library_id, facet, source, external_id, created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(scryer_domain::Id::new().0),
                SqlArg::Text(title.id.clone()),
                SqlArg::Text(title.library_id.clone()),
                SqlArg::Text(title.facet.as_str().to_string()),
                SqlArg::Text(source),
                SqlArg::Text(value),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }

    Ok(())
}

async fn replace_title_search_projection_sql_tx(
    tx: &mut SqlTx<'_>,
    title: &Title,
) -> AppResult<()> {
    if let Some(sqlite_tx) = tx.sqlite() {
        replace_title_search_projection_tx(sqlite_tx, title).await
    } else if let Some(pg_tx) = tx.postgres() {
        replace_title_search_projection_pg_tx(pg_tx, title).await
    } else {
        Err(AppError::Repository(
            "unsupported transaction backend for title search projection".to_string(),
        ))
    }
}

async fn delete_title_search_projection_sql_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
) -> AppResult<()> {
    if let Some(sqlite_tx) = tx.sqlite() {
        delete_title_search_projection_tx(sqlite_tx, title_id).await
    } else {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "DELETE FROM title_search_terms WHERE title_id = {}",
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?;
        Ok(())
    }
}

async fn delete_indexer_search_learning_for_title_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM indexer_search_learning WHERE title_id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use sqlx::sqlite::SqlitePoolOptions;

    async fn delete_test_store() -> (TitleStore, sqlx::SqlitePool) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");

        sqlx::query("CREATE TABLE titles (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("titles table should be created");
        sqlx::query(
            "CREATE TABLE title_search_terms (
                term_id INTEGER PRIMARY KEY,
                title_id TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("title_search_terms table should be created");
        sqlx::query("CREATE TABLE title_search_spellfix (term TEXT)")
            .execute(&pool)
            .await
            .expect("title_search_spellfix table should be created");
        sqlx::query(
            "CREATE TABLE indexer_search_learning (
                indexer_id TEXT NOT NULL,
                title_id TEXT NOT NULL,
                facet TEXT NOT NULL,
                strategy_key TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                empty_successes INTEGER NOT NULL DEFAULT 0,
                usable_successes INTEGER NOT NULL DEFAULT 0,
                last_attempt_at TEXT,
                last_usable_at TEXT,
                suppressed INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                PRIMARY KEY (indexer_id, title_id, facet, strategy_key)
            )",
        )
        .execute(&pool)
        .await
        .expect("indexer_search_learning table should be created");

        let store = TitleStore::new(StoreDatastore::Sqlite {
            pool: pool.clone(),
            writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        });

        (store, pool)
    }

    async fn insert_learning_row(pool: &sqlx::SqlitePool, title_id: &str) {
        sqlx::query(
            "INSERT INTO indexer_search_learning
             (indexer_id, title_id, facet, strategy_key)
             VALUES (?, ?, 'anime', 'ids_abs')",
        )
        .bind(format!("idx-{title_id}"))
        .bind(title_id)
        .execute(pool)
        .await
        .expect("learning row should insert");
    }

    async fn learning_count(pool: &sqlx::SqlitePool, title_id: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)
             FROM indexer_search_learning
             WHERE title_id = ?",
        )
        .bind(title_id)
        .fetch_one(pool)
        .await
        .expect("learning count should load")
    }

    #[tokio::test]
    async fn delete_title_clears_only_that_titles_indexer_search_learning_rows() {
        let (store, pool) = delete_test_store().await;
        sqlx::query("INSERT INTO titles (id) VALUES ('title-1'), ('title-2')")
            .execute(&pool)
            .await
            .expect("titles should insert");
        insert_learning_row(&pool, "title-1").await;
        insert_learning_row(&pool, "title-2").await;
        insert_learning_row(&pool, "missing-title").await;

        TitleRepository::delete(&store, "title-1")
            .await
            .expect("title delete should succeed");

        assert_eq!(learning_count(&pool, "title-1").await, 0);
        assert_eq!(learning_count(&pool, "title-2").await, 1);

        let error = TitleRepository::delete(&store, "missing-title")
            .await
            .expect_err("missing title should report not found");
        assert!(matches!(error, AppError::NotFound(_)));
        assert_eq!(learning_count(&pool, "missing-title").await, 1);
    }

    #[tokio::test]
    async fn persist_smg_id_replaces_a_redirected_value_and_rebuilds_external_id_lookups() {
        scryer_infrastructure_datastore::register_spellfix_auto_extension()
            .expect("spellfix extension should register before migrations");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
            &pool, None, true,
        )
        .await
        .expect("fresh migrations should apply");
        let store = TitleStore::new(StoreDatastore::Sqlite {
            pool: pool.clone(),
            writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        });
        let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
        let root_folder_id: String = sqlx::query_scalar(
            "SELECT id FROM library_roots
              WHERE library_id = ?
              ORDER BY is_default DESC, path
              LIMIT 1",
        )
        .bind(&library_id)
        .fetch_one(&pool)
        .await
        .expect("default movie root should exist");
        let title = Title {
            id: "redirected-smg-title".to_string(),
            library_id: library_id.clone(),
            name: "Redirected SMG Title".to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![
                ExternalId {
                    source: "SMG".to_string(),
                    value: "11".to_string(),
                },
                ExternalId {
                    source: "tvdb".to_string(),
                    value: "901001".to_string(),
                },
            ],
            root_folder_id,
            created_by: None,
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            canonical_tags: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        };
        TitleRepository::create(&store, title)
            .await
            .expect("title should create");

        TitleRepository::persist_smg_id(&store, "redirected-smg-title", 22, Some(11))
            .await
            .expect("redirected SMG id should persist");

        let found = TitleRepository::find_by_external_id_in_library_and_facet(
            &store,
            &library_id,
            MediaFacet::Movie,
            "SMG",
            "22",
        )
        .await
        .expect("SMG external-id lookup should succeed")
        .expect("new SMG id should resolve");
        assert_eq!(found.id, "redirected-smg-title");
        assert_eq!(
            found
                .external_ids
                .iter()
                .filter(|external_id| external_id.source.eq_ignore_ascii_case("smg"))
                .map(|external_id| external_id.value.as_str())
                .collect::<Vec<_>>(),
            vec!["22"]
        );
        assert!(
            TitleRepository::find_by_external_id_in_library_and_facet(
                &store,
                &library_id,
                MediaFacet::Movie,
                "smg",
                "11",
            )
            .await
            .expect("stale SMG lookup should succeed")
            .is_none()
        );
        let matches = TitleRepository::list_by_external_id_lookups(
            &store,
            &[TitleExternalIdLookup {
                lookup_index: 0,
                source: "smg".to_string(),
                external_id: "22".to_string(),
            }],
        )
        .await
        .expect("batched SMG external-id lookup should succeed");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].title.id, "redirected-smg-title");
    }

    #[test]
    fn title_catalog_rating_sort_scales_fractional_normalized_scores() {
        let sql = title_catalog_external_rating_subquery(&["imdb"]);

        assert!(
            sql.contains("WHEN ter.normalized <= 1.0 THEN ter.normalized * 10.0"),
            "rating sort should compare 0-1 and 0-10 normalized scores on the same scale: {sql}"
        );
    }

    #[test]
    fn title_catalog_where_sql_combines_advanced_filter_groups() {
        let filter = TitleCatalogFilter {
            monitored: Some(true),
            content_statuses: vec![TitleCatalogContentStatus::Continuing],
            root_folder_ids: vec!["root-1".to_string(), "root-2".to_string()],
            genre_tag_keys: vec!["canonical:genre:alpha".to_string()],
            theme_tag_keys: vec!["canonical:theme:beta".to_string()],
            minimum_year: Some(2000),
            maximum_year: Some(2020),
            minimum_rating: Some(7.5),
        };

        let (sql, args) = build_title_catalog_where_sql(
            Some(MediaFacet::Movie),
            &["library-1".to_string()],
            Some("sample"),
            &filter,
        );

        assert!(sql.contains("titles.root_folder_id IN"));
        assert!(sql.contains("titles.year >= {}"));
        assert!(sql.contains("titles.year <= {}"));
        assert_eq!(
            sql.matches("FROM title_metadata_tags catalog_tag").count(),
            2
        );
        assert!(sql.contains("FROM title_metadata_rating_summaries catalog_rating"));
        assert!(sql.contains("monitored = {}"));
        assert!(sql.contains("LOWER(TRIM(COALESCE(content_status, ''))) IN"));
        assert_eq!(args.len(), 15);
    }

    #[test]
    fn title_catalog_filter_options_scope_uses_library_root_and_facet() {
        let (sql, args) = build_title_catalog_options_scope_sql(
            Some(MediaFacet::Series),
            &["library-1".to_string()],
            &["root-1".to_string()],
        );

        assert!(sql.contains("titles.library_id IN"));
        assert!(sql.contains("titles.facet = {}"));
        assert!(sql.contains("titles.root_folder_id IN"));
        assert_eq!(args.len(), 3);
    }
}

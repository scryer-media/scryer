use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, ClaimedMediaFile, CollectionEpisodeProgressSummary,
    CutoffUnmetQualitySummary, EpisodeMediaAvailability, EpisodeMediaAvailabilityState,
    EpisodeScopedMediaFile, InsertMediaFileInput, MediaFileAnalysis, MediaFileAssociations,
    MediaFileCatalogDisposition, MediaFileHashCandidate, MediaFileRepository,
    MissingEpisodeCandidate,
    MissingScopeCandidates, MissingSeriesMovieLinkCandidate, MissingTitleCandidate,
    TitleEpisodeProgressSummary, TitleMediaFile, TitleMediaSizeSummary, TitleMovieMediaSummary,
    TitleQualitySummary, derive_primary_quality_label,
};
use scryer_domain::Id;
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

use crate::queries::common::parse_utc_datetime;
use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};
use crate::storage::sql::json::{canonical_json_text, json_text_or};

const RECYCLE_BIN_PATH_SEGMENT: &str = "/.scryer-recycle/";

#[derive(Clone)]
pub struct MediaFileStore {
    datastore: StoreDatastore,
}

#[derive(Clone, Copy)]
enum SqlDialect {
    Sqlite,
    Postgres,
}

impl MediaFileStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

const INSERT_MEDIA_FILE_SQL: &str =
    "INSERT INTO media_files
     (id, title_id, file_path, size_bytes, announced_size_bytes, role, quality_id, scan_status, created_at,
      source_signature_scheme, source_signature_value,
      scene_name, release_group, source_type, resolution,
      video_codec_parsed, audio_codec_parsed, audio_channels_parsed,
      acquisition_score, scoring_log,
      indexer_source, grabbed_release_title, grabbed_at,
      edition, original_file_path, release_hash)
     VALUES ({}, {}, {}, {}, {}, {}, {}, 'imported', {},
             {}, {},
             {}, {}, {}, {},
             {}, {}, {},
             {}, {},
             {}, {}, {},
             {}, {}, {})
     ON CONFLICT(file_path) DO UPDATE SET
        title_id = excluded.title_id,
        size_bytes = excluded.size_bytes,
        announced_size_bytes = excluded.announced_size_bytes,
        role = excluded.role,
        quality_id = excluded.quality_id,
        scan_status = excluded.scan_status,
        source_signature_scheme = excluded.source_signature_scheme,
        source_signature_value = excluded.source_signature_value,
        scene_name = excluded.scene_name,
        release_group = excluded.release_group,
        source_type = excluded.source_type,
        resolution = excluded.resolution,
        video_codec_parsed = excluded.video_codec_parsed,
        audio_codec_parsed = excluded.audio_codec_parsed,
        audio_channels_parsed = excluded.audio_channels_parsed,
        acquisition_score = excluded.acquisition_score,
        scoring_log = excluded.scoring_log,
        indexer_source = excluded.indexer_source,
        grabbed_release_title = excluded.grabbed_release_title,
        grabbed_at = excluded.grabbed_at,
        edition = excluded.edition,
        original_file_path = excluded.original_file_path,
        release_hash = excluded.release_hash";

fn media_file_insert_args(
    datastore: &StoreDatastore,
    input: &InsertMediaFileInput,
    id: &str,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(id.to_string()),
        SqlArg::Text(input.title_id.clone()),
        SqlArg::Text(input.file_path.clone()),
        SqlArg::I64(input.size_bytes),
        SqlArg::OptI64(input.announced_size_bytes),
        SqlArg::Text(input.role.as_str().to_string()),
        SqlArg::OptText(input.quality_label.clone()),
        SqlArg::Timestamp(Utc::now()),
        SqlArg::OptText(input.source_signature_scheme.clone()),
        SqlArg::OptText(input.source_signature_value.clone()),
        SqlArg::OptText(input.scene_name.clone()),
        SqlArg::OptText(input.release_group.clone()),
        SqlArg::OptText(input.source_type.clone()),
        SqlArg::OptText(input.resolution.clone()),
        SqlArg::OptText(input.video_codec_parsed.as_ref().map(ToString::to_string)),
        SqlArg::OptText(input.audio_codec_parsed.clone()),
        SqlArg::OptText(input.audio_channels_parsed.clone()),
        SqlArg::OptI32(input.acquisition_score),
        SqlArg::OptText(input.scoring_log.clone()),
        SqlArg::OptText(input.indexer_source.clone()),
        SqlArg::OptText(input.grabbed_release_title.clone()),
        opt_timestamp_arg_for_datastore(datastore, input.grabbed_at.as_deref())?,
        SqlArg::OptText(input.edition.clone()),
        SqlArg::OptText(input.original_file_path.clone()),
        SqlArg::OptText(input.release_hash.clone()),
    ])
}

async fn insert_media_file_tx(
    tx: &mut SqlTx<'_>,
    file_path: &str,
    args: &[SqlArg],
) -> AppResult<String> {
    SqlRuntime::execute(SqlExec::Tx(tx), INSERT_MEDIA_FILE_SQL, args).await?;
    SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id FROM media_files WHERE file_path = {} LIMIT 1",
        &[SqlArg::Text(file_path.to_string())],
    )
    .await?
    .ok_or_else(|| AppError::Repository(format!("inserted media file {file_path} was not found")))?
    .text("id")
}

async fn reconcile_media_file_associations_tx(
    tx: &mut SqlTx<'_>,
    file_id: &str,
    associations: &MediaFileAssociations,
) -> AppResult<()> {
    for episode_id in &associations.episode_ids {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO file_episode_map (file_id, episode_id)
             VALUES ({}, {})
             ON CONFLICT(file_id, episode_id) DO NOTHING",
            &[
                SqlArg::Text(file_id.to_string()),
                SqlArg::Text(episode_id.clone()),
            ],
        )
        .await?;
    }
    for series_movie_link_id in &associations.series_movie_link_ids {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO file_series_movie_link_map (file_id, series_movie_link_id)
             VALUES ({}, {})
             ON CONFLICT(file_id, series_movie_link_id) DO NOTHING",
            &[
                SqlArg::Text(file_id.to_string()),
                SqlArg::Text(series_movie_link_id.clone()),
            ],
        )
        .await?;
    }
    Ok(())
}

struct StoredMediaFileOwnership {
    media_file_id: String,
    title_id: String,
    original_file_path: Option<String>,
    episode_ids: Vec<String>,
    series_movie_link_ids: Vec<String>,
}

async fn load_media_file_ownership_tx(
    tx: &mut SqlTx<'_>,
    file_path: &str,
) -> AppResult<Option<StoredMediaFileOwnership>> {
    let (where_clause, args) = media_file_path_predicate(file_path);
    let Some(row) = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        &format!(
            "SELECT mf.id, mf.title_id, mf.original_file_path
               FROM media_files mf
              WHERE {where_clause}
              LIMIT 1"
        ),
        &args,
    )
    .await?
    else {
        return Ok(None);
    };
    let media_file_id = row.text("id")?;
    let title_id = row.text("title_id")?;
    let original_file_path = row.opt_text("original_file_path")?;
    let episode_ids = SqlRuntime::fetch_all(
        SqlExec::Tx(tx),
        "SELECT episode_id FROM file_episode_map WHERE file_id = {} ORDER BY episode_id",
        &[SqlArg::Text(media_file_id.clone())],
    )
    .await?
    .into_iter()
    .map(|row| row.text("episode_id"))
    .collect::<AppResult<Vec<_>>>()?;
    let series_movie_link_ids = SqlRuntime::fetch_all(
        SqlExec::Tx(tx),
        "SELECT series_movie_link_id
           FROM file_series_movie_link_map
          WHERE file_id = {}
          ORDER BY series_movie_link_id",
        &[SqlArg::Text(media_file_id.clone())],
    )
    .await?
    .into_iter()
    .map(|row| row.text("series_movie_link_id"))
    .collect::<AppResult<Vec<_>>>()?;
    Ok(Some(StoredMediaFileOwnership {
        media_file_id,
        title_id,
        original_file_path,
        episode_ids,
        series_movie_link_ids,
    }))
}

fn import_destination_ownership_matches(
    input: &InsertMediaFileInput,
    expected: &MediaFileAssociations,
    actual: &StoredMediaFileOwnership,
) -> bool {
    fn associations_are_compatible(
        expected: &[String],
        actual: &[String],
        missing_is_recoverable: bool,
    ) -> bool {
        if expected.is_empty() {
            actual.is_empty()
        } else {
            (missing_is_recoverable || !actual.is_empty())
                && actual.iter().all(|id| expected.contains(id))
        }
    }

    let has_any_association =
        !actual.episode_ids.is_empty() || !actual.series_movie_link_ids.is_empty();
    let source_matches = actual
        .original_file_path
        .as_deref()
        .zip(input.original_file_path.as_deref())
        .is_some_and(|(actual, expected)| {
            media_file_path_match_key(actual) == media_file_path_match_key(expected)
        });
    let missing_is_recoverable = has_any_association || source_matches;
    actual.title_id == input.title_id
        && missing_is_recoverable
        && associations_are_compatible(
            &expected.episode_ids,
            &actual.episode_ids,
            missing_is_recoverable,
        )
        && associations_are_compatible(
            &expected.series_movie_link_ids,
            &actual.series_movie_link_ids,
            missing_is_recoverable,
        )
}

#[async_trait]
impl MediaFileRepository for MediaFileStore {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        let id = Id::new().0;
        let args = media_file_insert_args(&self.datastore, input, &id)?;
        let file_path = input.file_path.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "insert_media_file", move |tx| {
            let args = args.clone();
            let file_path = file_path.clone();
            Box::pin(async move { insert_media_file_tx(tx, &file_path, &args).await })
        })
        .await
    }

    async fn claim_import_destination(
        &self,
        input: &InsertMediaFileInput,
        associations: &MediaFileAssociations,
    ) -> AppResult<ClaimedMediaFile> {
        let id = Id::new().0;
        let args = media_file_insert_args(&self.datastore, input, &id)?;
        let file_path = input.file_path.clone();
        let input = input.clone();
        let associations = associations.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "claim_import_destination",
            move |tx| {
                let args = args.clone();
                let file_path = file_path.clone();
                let input = input.clone();
                let associations = associations.clone();
                Box::pin(async move {
                    if let Some(existing) = load_media_file_ownership_tx(tx, &file_path).await? {
                        if !import_destination_ownership_matches(
                            &input,
                            &associations,
                            &existing,
                        ) {
                            return Err(AppError::ManualReconciliationRequired(format!(
                                "import destination {file_path} is cataloged for a different logical import target"
                            )));
                        }
                        reconcile_media_file_associations_tx(
                            tx,
                            &existing.media_file_id,
                            &associations,
                        )
                        .await?;
                        return Ok(ClaimedMediaFile {
                            media_file_id: existing.media_file_id,
                            disposition: MediaFileCatalogDisposition::Reused,
                        });
                    }
                    let file_id = insert_media_file_tx(tx, &file_path, &args).await?;
                    reconcile_media_file_associations_tx(tx, &file_id, &associations).await?;
                    Ok(ClaimedMediaFile {
                        media_file_id: file_id,
                        disposition: MediaFileCatalogDisposition::Created,
                    })
                })
            },
        )
        .await
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "link_file_to_episode",
            "INSERT INTO file_episode_map (file_id, episode_id)
             VALUES ({}, {})
             ON CONFLICT(file_id, episode_id) DO NOTHING",
            vec![
                SqlArg::Text(file_id.to_string()),
                SqlArg::Text(episode_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn link_file_to_series_movie(
        &self,
        file_id: &str,
        series_movie_link_id: &str,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "link_file_to_series_movie",
            "INSERT INTO file_series_movie_link_map (file_id, series_movie_link_id)
             VALUES ({}, {})
             ON CONFLICT(file_id, series_movie_link_id) DO NOTHING",
            vec![
                SqlArg::Text(file_id.to_string()),
                SqlArg::Text(series_movie_link_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        let dialect = dialect_for_datastore(&self.datastore);
        let sql = format!(
            "SELECT {}
             FROM media_files mf
             LEFT JOIN file_episode_map fem ON fem.file_id = mf.id
             WHERE mf.title_id = {{}}
               AND {}
             ORDER BY mf.created_at DESC",
            media_file_select_columns(dialect, "fem.episode_id", "COALESCE(fem.role, mf.role)"),
            live_media_file_predicate(dialect, "mf")
        );
        fetch_media_files(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
        )
        .await
    }

    async fn list_media_files_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaFile>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(title_ids.len());
        let sql = format!(
            "SELECT {}
             FROM media_files mf
             LEFT JOIN file_episode_map fem ON fem.file_id = mf.id
             WHERE mf.title_id IN ({placeholders})
               AND {}
             ORDER BY mf.title_id, mf.created_at DESC",
            media_file_select_columns(dialect, "fem.episode_id", "COALESCE(fem.role, mf.role)"),
            live_media_file_predicate(dialect, "mf")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        fetch_media_files(self.datastore.read_exec(), &sql, &args).await
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        if episode_ids.is_empty() {
            return Ok(Vec::new());
        }

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(episode_ids.len());
        let sql = format!(
            "SELECT {},
                    mf.role AS title_role,
                    {} AS episode_ids_json,
                    {} AS primary_episode_ids_json
             FROM media_files mf
             INNER JOIN file_episode_map fem_target ON fem_target.file_id = mf.id
             LEFT JOIN file_episode_map fem_all ON fem_all.file_id = mf.id
             WHERE mf.title_id = {{}}
               AND {}
               AND fem_target.episode_id IN ({placeholders})
             GROUP BY mf.id
             ORDER BY mf.created_at DESC",
            media_file_select_columns(
                dialect,
                "NULL",
                "CASE WHEN MAX(CASE WHEN fem_target.role = 'primary' THEN 1 ELSE 0 END) = 1
                      THEN 'primary' ELSE 'additional' END"
            ),
            episode_ids_aggregate(dialect),
            primary_episode_ids_aggregate(dialect),
            live_media_file_predicate(dialect, "mf")
        );
        let mut args = vec![SqlArg::Text(title_id.to_string())];
        args.extend(episode_ids.iter().cloned().map(SqlArg::Text));
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(row_to_episode_scoped_media_file)
            .collect()
    }

    async fn list_series_movie_link_ids_with_files_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<String>> {
        let dialect = dialect_for_datastore(&self.datastore);
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT DISTINCT fsmlm.series_movie_link_id
                 FROM media_files mf
                 INNER JOIN file_series_movie_link_map fsmlm ON fsmlm.file_id = mf.id
                 WHERE mf.title_id = {{}}
                   AND {}",
                live_media_file_predicate(dialect, "mf")
            ),
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?;
        rows.iter()
            .map(|row| row.text("series_movie_link_id"))
            .collect()
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(title_ids.len());
        let total_size_expression = total_size_bytes_sum_expression(dialect, "matched.size_bytes");
        let sql = format!(
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
                     WHERE mf.title_id IN ({placeholders})
                       AND {}
                       AND (
                           fem.file_id IS NOT NULL
                           OR c.id IS NOT NULL
                           OR fsmlm.file_id IS NOT NULL
                       )
               ) matched
              GROUP BY matched.title_id",
            live_media_file_predicate(dialect, "mf")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok(TitleMediaSizeSummary {
                    title_id: row.text("title_id")?,
                    total_size_bytes: row.i64("total_size_bytes")?,
                })
            })
            .collect()
    }

    async fn collection_media_size_bytes(
        &self,
        title_id: &str,
        ordered_path: &str,
    ) -> AppResult<Option<i64>> {
        let dialect = dialect_for_datastore(&self.datastore);
        let total_size_expression = total_size_bytes_sum_expression(dialect, "mf.size_bytes");
        let sql = format!(
            "SELECT {total_size_expression} AS total_size_bytes
               FROM media_files mf
              WHERE mf.title_id = {{}}
                AND mf.file_path = {{}}
                AND mf.size_bytes > 0
                AND {}",
            live_media_file_predicate(dialect, "mf")
        );
        let total = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(ordered_path.to_string()),
            ],
        )
        .await?
        .map(|row| row.i64("total_size_bytes"))
        .transpose()?
        .unwrap_or_default();
        Ok((total > 0).then_some(total))
    }

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(title_ids.len());
        let normalized_quality = normalized_quality_expression("media_files");
        let quality_rank = quality_rank_expression("media_files");
        let sql = format!(
            "SELECT title_id, quality_tier
             FROM (
                SELECT media_files.title_id AS title_id,
                       {normalized_quality} AS quality_tier,
                       ROW_NUMBER() OVER (
                          PARTITION BY media_files.title_id
                          ORDER BY {quality_rank} DESC,
                                   media_files.created_at DESC,
                                   media_files.id DESC
                       ) AS quality_row
                  FROM media_files
                 WHERE media_files.title_id IN ({placeholders})
                   AND {}
                   AND media_files.role = 'primary'
                   AND {normalized_quality} IS NOT NULL
             ) ranked
             WHERE quality_row = 1
               AND quality_tier IS NOT NULL",
            live_media_file_predicate(dialect, "media_files")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok(TitleQualitySummary {
                    title_id: row.text("title_id")?,
                    quality_tier: row.text("quality_tier")?,
                })
            })
            .collect()
    }

    async fn list_title_movie_media_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMovieMediaSummary>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = placeholders(title_ids.len());
        let dialect = dialect_for_datastore(&self.datastore);
        let resolution_expression = normalized_quality_expression("media_files");
        let audio_codec_expression = "COALESCE(NULLIF(TRIM(media_files.audio_codec_parsed), ''), NULLIF(TRIM(media_files.audio_codec), ''))";
        let hdr_expression = "NULLIF(TRIM(media_files.video_hdr_format), '')";
        let sql = format!(
            "SELECT title_id, resolution, hdr_format, audio_codec
             FROM (
                SELECT media_files.title_id AS title_id,
                       {resolution_expression} AS resolution,
                       {hdr_expression} AS hdr_format,
                       {audio_codec_expression} AS audio_codec,
                       ROW_NUMBER() OVER (
                          PARTITION BY media_files.title_id
                          ORDER BY CASE WHEN media_files.size_bytes > 0 THEN media_files.size_bytes ELSE 0 END DESC,
                                   media_files.created_at DESC,
                                   media_files.id DESC
                       ) AS media_row
                  FROM media_files
                  JOIN titles ON titles.id = media_files.title_id
                 WHERE media_files.title_id IN ({placeholders})
                   AND titles.facet = 'movie'
                   AND {}
                   AND media_files.role = 'primary'
             ) ranked
             WHERE media_row = 1",
            live_media_file_predicate(dialect, "media_files")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok(TitleMovieMediaSummary {
                    title_id: row.text("title_id")?,
                    resolution: row.opt_text("resolution")?,
                    hdr_format: row
                        .opt_text("hdr_format")?
                        .filter(|value| !value.trim().is_empty()),
                    audio_codec: row.opt_text("audio_codec")?,
                })
            })
            .collect()
    }

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(title_ids.len());
        let normalized_quality = normalized_quality_expression("media_files");
        let quality_rank = quality_rank_expression("media_files");
        let sql = format!(
            "SELECT title_id, episode_id, season_number, episode_number, quality_tier
             FROM (
                SELECT media_files.title_id AS title_id,
                       fem.episode_id AS episode_id,
                       e.season_number AS season_number,
                       e.episode_number AS episode_number,
                       {normalized_quality} AS quality_tier,
                       ROW_NUMBER() OVER (
                          PARTITION BY CASE
                              WHEN fem.episode_id IS NOT NULL THEN fem.episode_id
                              ELSE {}
                          END
                          ORDER BY {quality_rank} DESC,
                                   media_files.created_at DESC,
                                   media_files.id DESC
                       ) AS quality_row
                  FROM media_files
                  LEFT JOIN file_episode_map fem ON fem.file_id = media_files.id
                  LEFT JOIN episodes e ON e.id = fem.episode_id
                 WHERE media_files.title_id IN ({placeholders})
                   AND {}
                   AND media_files.role = 'primary'
                   AND {normalized_quality} IS NOT NULL
                   AND (fem.episode_id IS NULL OR {})
             ) ranked
             WHERE quality_row = 1
               AND quality_tier IS NOT NULL",
            title_partition_fallback(dialect, "media_files.title_id"),
            live_media_file_predicate(dialect, "media_files"),
            bool_column_is_true(dialect, "e.monitored")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok(CutoffUnmetQualitySummary {
                    title_id: row.text("title_id")?,
                    episode_id: row.opt_text("episode_id")?,
                    season_number: row.opt_text("season_number")?,
                    episode_number: row.opt_text("episode_number")?,
                    quality_tier: row.text("quality_tier")?,
                })
            })
            .collect()
    }

    async fn list_missing_scope_candidates(&self) -> AppResult<MissingScopeCandidates> {
        let dialect = dialect_for_datastore(&self.datastore);
        let live_file = live_media_file_predicate(dialect, "mf");

        // Monitored episodes (inside monitored collections of monitored titles)
        // with no live primary file. Episodes outside a collection are not
        // acquisition units, matching collection-driven monitoring.
        let episode_sql = format!(
            "SELECT e.id AS episode_id, e.title_id, t.library_id, t.facet,
                    e.collection_id, e.season_number, e.episode_number, e.air_date,
                    t.created_at AS title_created_at
               FROM episodes e
              INNER JOIN titles t ON t.id = e.title_id
              INNER JOIN collections c ON c.id = e.collection_id
              WHERE {} AND {} AND {}
                AND t.deleted_at IS NULL
                AND NOT EXISTS (
                    SELECT 1 FROM file_episode_map fem
                     INNER JOIN media_files mf ON mf.id = fem.file_id
                     WHERE fem.episode_id = e.id
                       AND fem.role = 'primary'
                       AND {live_file}
                )
              ORDER BY e.id",
            bool_column_is_true(dialect, "t.monitored"),
            bool_column_is_true(dialect, "e.monitored"),
            bool_column_is_true(dialect, "c.monitored"),
        );
        let episodes = SqlRuntime::fetch_all(self.datastore.read_exec(), &episode_sql, &[])
            .await?
            .iter()
            .map(|row| {
                Ok(MissingEpisodeCandidate {
                    episode_id: row.text("episode_id")?,
                    title_id: row.text("title_id")?,
                    library_id: row.text("library_id")?,
                    title_facet: row.text("facet")?,
                    collection_id: row.opt_text("collection_id")?,
                    season_number: row.opt_text("season_number")?,
                    episode_number: row.opt_text("episode_number")?,
                    air_date: row.opt_text("air_date")?,
                    title_created_at: timestamp_text(row, "title_created_at")?,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;

        // Monitored titles with no live primary file at all. Includes episodic
        // titles — the application layer keeps only movie-shaped facets.
        let title_sql = format!(
            "SELECT t.id AS title_id, t.library_id, t.facet, t.min_availability,
                    t.first_aired, t.digital_release_date, t.created_at
               FROM titles t
              WHERE {}
                AND t.deleted_at IS NULL
                AND NOT EXISTS (
                    SELECT 1 FROM media_files mf
                     WHERE mf.title_id = t.id
                       AND mf.role = 'primary'
                       AND {live_file}
                )
              ORDER BY t.id",
            bool_column_is_true(dialect, "t.monitored"),
        );
        let titles = SqlRuntime::fetch_all(self.datastore.read_exec(), &title_sql, &[])
            .await?
            .iter()
            .map(|row| {
                Ok(MissingTitleCandidate {
                    title_id: row.text("title_id")?,
                    library_id: row.text("library_id")?,
                    title_facet: row.text("facet")?,
                    min_availability: row.opt_text("min_availability")?,
                    first_aired: row.opt_text("first_aired")?,
                    digital_release_date: row.opt_text("digital_release_date")?,
                    created_at: timestamp_text(row, "created_at")?,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;

        // Monitored series-movie links with no linked live file (any role, matching
        // list_series_movie_link_ids_with_files_for_title).
        let link_sql = format!(
            "SELECT sml.id AS series_movie_link_id, sml.series_title_id AS title_id,
                    t.library_id, t.facet, sml.continuity_status,
                    me.digital_release_date AS movie_digital_release_date,
                    sml.created_at AS link_created_at
               FROM series_movie_links sml
              INNER JOIN titles t ON t.id = sml.series_title_id
              INNER JOIN movie_entities me ON me.id = sml.movie_entity_id
              WHERE {} AND {}
                AND ({} OR {})
                AND t.deleted_at IS NULL
                AND NOT EXISTS (
                    SELECT 1 FROM file_series_movie_link_map fsmlm
                     INNER JOIN media_files mf ON mf.id = fsmlm.file_id
                     WHERE fsmlm.series_movie_link_id = sml.id
                       AND {live_file}
                )
              ORDER BY sml.id",
            bool_column_is_true(dialect, "sml.monitored"),
            bool_column_is_true(dialect, "t.monitored"),
            bool_column_is_true(dialect, "sml.metadata_active"),
            bool_column_is_true(dialect, "sml.monitoring_override"),
        );
        let series_movie_links = SqlRuntime::fetch_all(self.datastore.read_exec(), &link_sql, &[])
            .await?
            .iter()
            .map(|row| {
                Ok(MissingSeriesMovieLinkCandidate {
                    series_movie_link_id: row.text("series_movie_link_id")?,
                    title_id: row.text("title_id")?,
                    library_id: row.text("library_id")?,
                    title_facet: row.text("facet")?,
                    continuity_status: row.opt_text("continuity_status")?,
                    movie_digital_release_date: row.opt_text("movie_digital_release_date")?,
                    link_created_at: timestamp_text(row, "link_created_at")?,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;

        Ok(MissingScopeCandidates {
            episodes,
            titles,
            series_movie_links,
        })
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(title_ids.len());
        let sql = format!(
            "SELECT e.title_id,
                    COUNT(DISTINCT e.id) AS total_episodes,
                    COUNT(DISTINCT CASE WHEN {} THEN e.id END) AS monitored_episodes,
                    COUNT(DISTINCT CASE WHEN mf.id IS NOT NULL THEN e.id END) AS owned_episodes
             FROM episodes e
             INNER JOIN collections c ON c.id = e.collection_id
             LEFT JOIN file_episode_map fem ON fem.episode_id = e.id
             LEFT JOIN media_files mf ON mf.id = fem.file_id AND {} AND fem.role = 'primary'
             WHERE e.title_id IN ({placeholders})
               AND c.collection_type <> 'specials'
               AND c.collection_index <> '0'
               AND trim(COALESCE(e.title, '')) <> ''
               AND upper(trim(e.title)) NOT IN ('TBA', 'TBD')
               AND trim(COALESCE(e.air_date, '')) <> ''
             GROUP BY e.title_id",
            bool_column_is_true(dialect, "e.monitored"),
            live_media_file_predicate(dialect, "mf")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok(TitleEpisodeProgressSummary {
                    title_id: row.text("title_id")?,
                    owned_episodes: row.i64("owned_episodes")?,
                    monitored_episodes: row.i64("monitored_episodes")?,
                    total_episodes: row.i64("total_episodes")?,
                })
            })
            .collect()
    }

    async fn list_episode_media_availability(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<EpisodeMediaAvailability>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(title_ids.len());
        let sql = format!(
            "WITH requested_episodes AS (
                SELECT id, title_id, monitored
                FROM episodes
                WHERE title_id IN ({placeholders})
             ), ranked_availability_files AS (
                SELECT fem.episode_id,
                       mf.scan_status,
                       mf.video_width,
                       mf.video_height,
                       mf.quality_id,
                       mf.resolution,
                       ROW_NUMBER() OVER (
                           PARTITION BY fem.episode_id
                           ORDER BY CASE WHEN fem.role = 'primary' THEN 0 ELSE 1 END,
                                    CASE WHEN fem.role = 'primary' THEN 0 ELSE mf.size_bytes END DESC,
                                    mf.created_at DESC,
                                    mf.id DESC
                       ) AS row_number
                FROM file_episode_map fem
                INNER JOIN requested_episodes e ON e.id = fem.episode_id
                INNER JOIN media_files mf ON mf.id = fem.file_id
                WHERE {} AND fem.role IN ('primary', 'additional')
             )
             SELECT e.title_id,
                    e.id AS episode_id,
                    e.monitored,
                    availability_file.scan_status,
                    availability_file.video_width,
                    availability_file.video_height,
                    availability_file.quality_id,
                    availability_file.resolution
             FROM requested_episodes e
             LEFT JOIN ranked_availability_files availability_file
               ON availability_file.episode_id = e.id AND availability_file.row_number = 1",
            live_media_file_predicate(dialect, "mf"),
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                let scan_status = row.opt_text("scan_status")?;
                let state = match scan_status.as_deref() {
                    Some("imported") => EpisodeMediaAvailabilityState::PendingScan,
                    Some("scan_failed") => EpisodeMediaAvailabilityState::ScanFailed,
                    Some(_) => EpisodeMediaAvailabilityState::Available,
                    None if row.bool("monitored")? => EpisodeMediaAvailabilityState::Missing,
                    None => EpisodeMediaAvailabilityState::Unmonitored,
                };
                let primary_quality_label = if state == EpisodeMediaAvailabilityState::Available {
                    let quality_label = row.opt_text("quality_id")?;
                    let resolution = row.opt_text("resolution")?;
                    derive_primary_quality_label(
                        row.opt_i32("video_width")?,
                        row.opt_i32("video_height")?,
                        quality_label.as_deref(),
                        resolution.as_deref(),
                    )
                } else {
                    None
                };
                Ok(EpisodeMediaAvailability {
                    title_id: row.text("title_id")?,
                    episode_id: row.text("episode_id")?,
                    state,
                    primary_quality_label,
                })
            })
            .collect()
    }

    async fn list_collection_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CollectionEpisodeProgressSummary>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }

        let dialect = dialect_for_datastore(&self.datastore);
        let placeholders = placeholders(title_ids.len());
        // Countable episodes exclude unnamed/TBA/undated placeholder records; the
        // record total intentionally counts every episode row so callers can tell
        // "no episodes at all" apart from "only placeholder episodes".
        let countable = "(trim(COALESCE(e.title, '')) <> ''
               AND upper(trim(e.title)) NOT IN ('TBA', 'TBD')
               AND trim(COALESCE(e.air_date, '')) <> '')";
        let sql = format!(
            "SELECT e.collection_id,
                    COUNT(DISTINCT e.id) AS episode_records_total,
                    COUNT(DISTINCT CASE WHEN {countable} THEN e.id END) AS total_episodes,
                    COUNT(DISTINCT CASE WHEN {countable} AND {} THEN e.id END) AS monitored_episodes,
                    COUNT(DISTINCT CASE WHEN {countable} AND mf.id IS NOT NULL THEN e.id END) AS owned_episodes
             FROM episodes e
             LEFT JOIN file_episode_map fem ON fem.episode_id = e.id
             LEFT JOIN media_files mf ON mf.id = fem.file_id AND {} AND fem.role = 'primary'
             WHERE e.title_id IN ({placeholders})
               AND e.collection_id IS NOT NULL
             GROUP BY e.collection_id",
            bool_column_is_true(dialect, "e.monitored"),
            live_media_file_predicate(dialect, "mf")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok(CollectionEpisodeProgressSummary {
                    collection_id: row.text("collection_id")?,
                    owned_episodes: row.i64("owned_episodes")?,
                    monitored_episodes: row.i64("monitored_episodes")?,
                    total_episodes: row.i64("total_episodes")?,
                    episode_records_total: row.i64("episode_records_total")?,
                })
            })
            .collect()
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        let analysis_json = serialized_media_analysis(&analysis)?;
        execute_write(
            &self.datastore,
            "update_media_file_analysis",
            "UPDATE media_files SET
                video_codec = {},
                video_width = {},
                video_height = {},
                video_bitrate_kbps = {},
                video_bit_depth = {},
                video_hdr_format = {},
                video_frame_rate = {},
                video_profile = {},
                audio_codec = {},
                audio_profile = {},
                audio_channels = {},
                audio_bitrate_kbps = {},
                duration_seconds = {},
                num_chapters = {},
                container_format = {},
                analysis_json = {},
                has_multiaudio = {},
                scan_status = 'scanned'
             WHERE id = {}",
            vec![
                SqlArg::OptText(analysis.video_codec.as_ref().map(ToString::to_string)),
                SqlArg::OptI32(analysis.video_width),
                SqlArg::OptI32(analysis.video_height),
                SqlArg::OptI32(analysis.video_bitrate_kbps),
                SqlArg::OptI32(analysis.video_bit_depth),
                SqlArg::OptText(analysis.video_hdr_format),
                SqlArg::OptText(analysis.video_frame_rate),
                SqlArg::OptText(analysis.video_profile),
                SqlArg::OptText(analysis.audio_codec),
                SqlArg::OptText(analysis.audio_profile),
                SqlArg::OptI32(analysis.audio_channels),
                SqlArg::OptI32(analysis.audio_bitrate_kbps),
                SqlArg::OptI32(analysis.duration_seconds),
                SqlArg::OptI32(analysis.num_chapters),
                SqlArg::OptText(analysis.container_format),
                SqlArg::Text(analysis_json),
                SqlArg::Bool(analysis.has_multiaudio),
                SqlArg::Text(file_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "update_media_file_source_signature",
            "UPDATE media_files SET
                size_bytes = {},
                source_signature_scheme = {},
                source_signature_value = {}
             WHERE id = {}",
            vec![
                SqlArg::I64(size_bytes),
                SqlArg::OptText(source_signature_scheme),
                SqlArg::OptText(source_signature_value),
                SqlArg::Text(file_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "update_media_file_path",
            "UPDATE media_files SET file_path = {} WHERE id = {}",
            vec![
                SqlArg::Text(file_path.to_string()),
                SqlArg::Text(file_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn set_media_file_roles_for_title(
        &self,
        title_id: &str,
        primary_file_id: &str,
        additional_file_ids: &[String],
    ) -> AppResult<()> {
        let mut ids = Vec::with_capacity(additional_file_ids.len() + 1);
        ids.push(primary_file_id.to_string());
        for file_id in additional_file_ids {
            if file_id != primary_file_id && !ids.contains(file_id) {
                ids.push(file_id.clone());
            }
        }

        let sql = format!(
            "UPDATE media_files
                SET role = CASE WHEN id = {{}} THEN 'primary' ELSE 'additional' END
              WHERE title_id = {{}}
                AND id IN ({})",
            placeholders(ids.len())
        );
        let mut args = vec![
            SqlArg::Text(primary_file_id.to_string()),
            SqlArg::Text(title_id.to_string()),
        ];
        args.extend(ids.iter().cloned().map(SqlArg::Text));

        let updated =
            execute_write(&self.datastore, "set_media_file_roles_for_title", sql, args).await?;
        if updated != ids.len() as u64 {
            return Err(AppError::Repository(format!(
                "expected to update {} media file roles, updated {updated}",
                ids.len()
            )));
        }

        Ok(())
    }

    async fn set_media_file_roles_for_episode(
        &self,
        title_id: &str,
        episode_id: &str,
        primary_file_id: &str,
        additional_file_ids: &[String],
    ) -> AppResult<()> {
        let mut ids = Vec::with_capacity(additional_file_ids.len() + 1);
        ids.push(primary_file_id.to_string());
        for file_id in additional_file_ids {
            if file_id != primary_file_id && !ids.contains(file_id) {
                ids.push(file_id.clone());
            }
        }

        let title_id = title_id.to_string();
        let episode_id = episode_id.to_string();
        let primary_file_id = primary_file_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "set_media_file_roles_for_episode",
            move |tx| {
                let title_id = title_id.clone();
                let episode_id = episode_id.clone();
                let primary_file_id = primary_file_id.clone();
                let ids = ids.clone();
                Box::pin(async move {
                    let mut candidate_args = vec![SqlArg::Text(episode_id.clone())];
                    candidate_args.extend(ids.iter().cloned().map(SqlArg::Text));
                    candidate_args.push(SqlArg::Text(title_id.clone()));
                    let updated = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        &format!(
                            "UPDATE file_episode_map
                                SET role = 'additional'
                              WHERE episode_id = {{}}
                                AND file_id IN ({})
                                AND EXISTS (
                                    SELECT 1
                                    FROM media_files mf
                                    WHERE mf.id = file_episode_map.file_id
                                      AND mf.title_id = {{}}
                                )",
                            placeholders(ids.len())
                        ),
                        &candidate_args,
                    )
                    .await?;
                    if updated != ids.len() as u64 {
                        return Err(AppError::Repository(format!(
                            "expected to update {} episode media file roles, updated {updated}",
                            ids.len()
                        )));
                    }

                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE file_episode_map
                            SET role = 'additional'
                          WHERE episode_id = {}
                            AND role = 'primary'",
                        &[SqlArg::Text(episode_id.clone())],
                    )
                    .await?;

                    let promoted = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE file_episode_map
                            SET role = 'primary'
                          WHERE episode_id = {}
                            AND file_id = {}
                            AND EXISTS (
                                SELECT 1
                                FROM media_files mf
                                WHERE mf.id = file_episode_map.file_id
                                  AND mf.title_id = {}
                            )",
                        &[
                            SqlArg::Text(episode_id),
                            SqlArg::Text(primary_file_id.clone()),
                            SqlArg::Text(title_id),
                        ],
                    )
                    .await?;
                    if promoted != 1 {
                        return Err(AppError::Repository(format!(
                            "expected to promote one episode media file, promoted {promoted}: {primary_file_id}"
                        )));
                    }

                    Ok(())
                })
            },
        )
        .await
    }

    async fn replace_media_file_for_upgrade(
        &self,
        old_file_id: &str,
        replacement_file_id: &str,
        replacement_file_path: &str,
    ) -> AppResult<()> {
        let old_file_id = old_file_id.to_string();
        let replacement_file_id = replacement_file_id.to_string();
        let replacement_file_path = replacement_file_path.to_string();

        SqlRuntime::run_in_transaction(
            &self.datastore,
            "replace_media_file_for_upgrade",
            move |tx| {
                let old_file_id = old_file_id.clone();
                let replacement_file_id = replacement_file_id.clone();
                let replacement_file_path = replacement_file_path.clone();
                Box::pin(async move {
                    let primary_episode_ids = SqlRuntime::fetch_all(
                        SqlExec::Tx(tx),
                        "SELECT old_link.episode_id
                           FROM file_episode_map old_link
                           INNER JOIN file_episode_map replacement_link
                                   ON replacement_link.episode_id = old_link.episode_id
                                  AND replacement_link.file_id = {}
                          WHERE old_link.file_id = {}
                            AND old_link.role = 'primary'",
                        &[
                            SqlArg::Text(replacement_file_id.clone()),
                            SqlArg::Text(old_file_id.clone()),
                        ],
                    )
                    .await?
                    .iter()
                    .map(|row| row.text("episode_id"))
                    .collect::<AppResult<Vec<_>>>()?;

                    let deleted = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM media_files WHERE id = {}",
                        &[SqlArg::Text(old_file_id.clone())],
                    )
                    .await?;
                    if deleted != 1 {
                        return Err(AppError::Repository(format!(
                            "expected to delete one old media file during upgrade replacement, deleted {deleted}: {old_file_id}"
                        )));
                    }

                    for episode_id in primary_episode_ids {
                        let promoted = SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "UPDATE file_episode_map
                                SET role = 'primary'
                              WHERE file_id = {}
                                AND episode_id = {}",
                            &[
                                SqlArg::Text(replacement_file_id.clone()),
                                SqlArg::Text(episode_id.clone()),
                            ],
                        )
                        .await?;
                        if promoted != 1 {
                            return Err(AppError::Repository(format!(
                                "expected to transfer one episode primary role, transferred {promoted}: {episode_id}"
                            )));
                        }
                    }

                    let updated = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE media_files SET file_path = {} WHERE id = {}",
                        &[
                            SqlArg::Text(replacement_file_path),
                            SqlArg::Text(replacement_file_id.clone()),
                        ],
                    )
                    .await?;
                    if updated != 1 {
                        return Err(AppError::Repository(format!(
                            "expected to update one replacement media file during upgrade replacement, updated {updated}: {replacement_file_id}"
                        )));
                    }

                    Ok(())
                })
            },
        )
        .await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "mark_scan_failed",
            "UPDATE media_files SET scan_status = 'scan_failed', scan_error = {} WHERE id = {}",
            vec![
                SqlArg::Text(error.to_string()),
                SqlArg::Text(file_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        let dialect = dialect_for_datastore(&self.datastore);
        let sql = format!(
            "SELECT {}
             FROM media_files mf
             WHERE mf.id = {{}}",
            media_file_select_columns(dialect, "NULL", "mf.role")
        );
        fetch_optional_media_file(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(file_id.to_string())],
        )
        .await
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        let dialect = dialect_for_datastore(&self.datastore);
        let (where_clause, args) = media_file_path_predicate(file_path);
        let sql = format!(
            "SELECT {}
             FROM media_files mf
             WHERE {where_clause}
             LIMIT 1",
            media_file_select_columns(dialect, "NULL", "mf.role")
        );
        fetch_optional_media_file(self.datastore.read_exec(), &sql, &args).await
    }

    async fn list_media_files_by_paths(
        &self,
        file_paths: &[String],
    ) -> AppResult<Vec<(String, TitleMediaFile)>> {
        if file_paths.is_empty() {
            return Ok(Vec::new());
        }
        let dialect = dialect_for_datastore(&self.datastore);
        let mut media_files = Vec::new();
        // Each path keeps the same predicate the single-path lookup uses, so
        // Windows' stored-vs-plain path matching stays identical here.
        for chunk in file_paths.chunks(MEDIA_FILE_PATH_LOOKUP_CHUNK) {
            let mut predicates = Vec::with_capacity(chunk.len());
            let mut args = Vec::new();
            // Several requested paths can share a match key (Windows matches
            // case- and separator-insensitively), and the predicate matches a
            // row for each of them, so every one keeps its own result entry.
            let mut requested_by_match_key: HashMap<String, Vec<String>> =
                HashMap::with_capacity(chunk.len());
            for file_path in chunk {
                let (predicate, mut predicate_args) = media_file_path_predicate(file_path);
                predicates.push(predicate);
                args.append(&mut predicate_args);
                requested_by_match_key
                    .entry(media_file_path_match_key(file_path))
                    .or_default()
                    .push(file_path.clone());
            }
            let where_clause = predicates.join(" OR ");
            let sql = format!(
                "SELECT {}
                 FROM media_files mf
                 WHERE {where_clause}",
                media_file_select_columns(dialect, "NULL", "mf.role")
            );
            for media_file in fetch_media_files(self.datastore.read_exec(), &sql, &args).await? {
                // Attribute each row to the path the caller asked for, using the
                // same equivalence the predicate above matched on.
                if let Some(requested_paths) =
                    requested_by_match_key.get(&media_file_path_match_key(&media_file.file_path))
                {
                    for requested in requested_paths {
                        media_files.push((requested.clone(), media_file.clone()));
                    }
                }
            }
        }
        Ok(media_files)
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_media_file",
            "DELETE FROM media_files WHERE id = {}",
            vec![SqlArg::Text(file_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn list_media_files_missing_full_hash(
        &self,
        after_id: Option<&str>,
        limit: u32,
    ) -> AppResult<Vec<MediaFileHashCandidate>> {
        // `full_blake3 IS NULL` is exactly migration 0205's partial index
        // (`idx_media_files_full_hash_missing`), and ordering by id keeps the
        // cursor a single opaque value that survives a restart.
        let mut sql = String::from(
            "SELECT mf.id, mf.title_id, mf.file_path, mf.size_bytes
               FROM media_files mf
              WHERE mf.full_blake3 IS NULL",
        );
        let mut args = Vec::new();
        if let Some(after_id) = after_id {
            sql.push_str(" AND mf.id > {}");
            args.push(SqlArg::Text(after_id.to_string()));
        }
        sql.push_str(" ORDER BY mf.id LIMIT {}");
        args.push(SqlArg::I64(i64::from(limit)));

        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok(MediaFileHashCandidate {
                    id: row.text("id")?,
                    title_id: row.text("title_id")?,
                    file_path: row.text("file_path")?,
                    size_bytes: row.i64("size_bytes")?,
                })
            })
            .collect()
    }

    async fn update_media_file_content_hashes(
        &self,
        file_id: &str,
        hashes: &scryer_application::location::model::PersistedContentHashes,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "update_media_file_content_hashes",
            "UPDATE media_files SET
                full_blake3 = {},
                move_crc = {},
                move_crc_algorithm = {},
                hash_computed_at = {}
             WHERE id = {}",
            vec![
                SqlArg::Text(hashes.full_blake3.clone()),
                // Stored as text: a u64 does not fit either engine's signed
                // 64-bit integer column without wrapping.
                SqlArg::OptText(hashes.move_crc.map(|crc| crc.to_string())),
                SqlArg::OptText(
                    hashes
                        .crc_algorithm
                        .map(|algorithm| algorithm.as_str().to_string()),
                ),
                SqlArg::OptTimestamp(hashes.hash_computed_at),
                SqlArg::Text(file_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn clear_media_file_content_hashes(&self, file_id: &str) -> AppResult<bool> {
        // The `full_blake3 IS NOT NULL` guard makes this a no-op write for the
        // overwhelmingly common case (a scan re-seeing an already-unhashed
        // file) and makes the returned flag mean "this file just re-entered the
        // backfill queue".
        let affected = execute_write(
            &self.datastore,
            "clear_media_file_content_hashes",
            "UPDATE media_files SET
                full_blake3 = NULL,
                move_crc = NULL,
                move_crc_algorithm = NULL,
                hash_computed_at = NULL
             WHERE id = {} AND full_blake3 IS NOT NULL",
            vec![SqlArg::Text(file_id.to_string())],
        )
        .await?;
        Ok(affected > 0)
    }
}

/// Paths per batched lookup. Windows spends two binds per path, so the chunk
/// stays well under sqlite's historical 999-variable ceiling.
const MEDIA_FILE_PATH_LOOKUP_CHUNK: usize = 400;

/// The key two paths share when [`media_file_path_predicate`] treats them as
/// the same file, used to attribute batched rows back to a requested path.
fn media_file_path_match_key(file_path: &str) -> String {
    #[cfg(windows)]
    {
        if file_path.starts_with("scryer-path-v1:") {
            return file_path.to_string();
        }
        normalize_windows_plain_media_file_path_lookup(file_path)
    }
    #[cfg(not(windows))]
    {
        file_path.to_string()
    }
}

/// The `WHERE` fragment matching one stored media-file path, with its binds.
fn media_file_path_predicate(file_path: &str) -> (String, Vec<SqlArg>) {
    #[cfg(windows)]
    {
        if file_path.starts_with("scryer-path-v1:") {
            return (
                "mf.file_path = {}".to_string(),
                vec![SqlArg::Text(file_path.to_string())],
            );
        }
        let normalized_path = normalize_windows_plain_media_file_path_lookup(file_path);
        (
            "(
                mf.file_path = {}
                OR (
                    mf.file_path NOT LIKE 'scryer-path-v1:%'
                    AND lower(replace(mf.file_path, '/', '\\')) = {}
                )
            )"
            .to_string(),
            vec![
                SqlArg::Text(file_path.to_string()),
                SqlArg::Text(normalized_path),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        (
            "mf.file_path = {}".to_string(),
            vec![SqlArg::Text(file_path.to_string())],
        )
    }
}

#[cfg(windows)]
fn normalize_windows_plain_media_file_path_lookup(file_path: &str) -> String {
    file_path.replace('/', "\\").to_lowercase()
}

fn dialect_for_datastore(datastore: &StoreDatastore) -> SqlDialect {
    match datastore {
        StoreDatastore::Sqlite { .. } => SqlDialect::Sqlite,
        StoreDatastore::Postgres { .. } => SqlDialect::Postgres,
    }
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("{}", count)
        .collect::<Vec<_>>()
        .join(", ")
}

fn live_media_file_predicate(dialect: SqlDialect, alias: &str) -> String {
    match dialect {
        SqlDialect::Sqlite => format!("instr({alias}.file_path, '{RECYCLE_BIN_PATH_SEGMENT}') = 0"),
        SqlDialect::Postgres => {
            format!("POSITION('{RECYCLE_BIN_PATH_SEGMENT}' IN {alias}.file_path) = 0")
        }
    }
}

fn total_size_bytes_sum_expression(dialect: SqlDialect, expr: &str) -> String {
    match dialect {
        SqlDialect::Sqlite => format!("COALESCE(SUM({expr}), 0)"),
        SqlDialect::Postgres => format!("COALESCE(SUM({expr}), 0)::BIGINT"),
    }
}

fn normalized_quality_expression(alias: &str) -> String {
    format!(
        "CASE
            WHEN {alias}.video_width >= 7680 OR {alias}.video_height >= 4200 THEN '4320P'
            WHEN {alias}.video_width >= 3840 OR {alias}.video_height >= 2100 THEN '2160P'
            WHEN {alias}.video_height >= 1300 THEN '1440P'
            WHEN {alias}.video_width >= 1920 OR {alias}.video_height >= 1000 THEN '1080P'
            WHEN {alias}.video_width >= 1280 OR {alias}.video_height >= 700 THEN '720P'
            WHEN {alias}.video_width >= 854 OR {alias}.video_height >= 480 THEN '480P'
            WHEN {alias}.video_height >= 300 THEN '360P'
            WHEN trim(COALESCE({alias}.quality_id, '')) = '' THEN NULL
            ELSE upper(trim({alias}.quality_id))
         END"
    )
}

fn quality_rank_expression(alias: &str) -> String {
    format!(
        "CASE
            WHEN {alias}.video_width >= 7680 OR {alias}.video_height >= 4200 THEN 0
            WHEN {alias}.video_width >= 3840 OR {alias}.video_height >= 2100 THEN 1
            WHEN {alias}.video_height >= 1300 THEN 2
            WHEN {alias}.video_width >= 1920 OR {alias}.video_height >= 1000 THEN 3
            WHEN {alias}.video_width >= 1280 OR {alias}.video_height >= 700 THEN 5
            WHEN {alias}.video_width >= 854 OR {alias}.video_height >= 480 THEN 6
            WHEN {alias}.video_height >= 300 THEN 7
            ELSE CASE upper(trim(COALESCE({alias}.quality_id, '')))
                WHEN '4320P' THEN 0
                WHEN '2160P' THEN 1
                WHEN '1440P' THEN 2
                WHEN '1080P' THEN 3
                WHEN '1080I' THEN 4
                WHEN '720P' THEN 5
                WHEN '480P' THEN 6
                WHEN '360P' THEN 7
                ELSE 999
            END
         END"
    )
}

fn serialized_media_analysis(analysis: &MediaFileAnalysis) -> AppResult<String> {
    canonical_json_text(analysis)
}

fn media_file_select_columns(dialect: SqlDialect, episode_expr: &str, role_expr: &str) -> String {
    let series_movie_link_ids_json = series_movie_link_ids_aggregate(dialect);
    format!(
        "mf.id, mf.title_id, {episode_expr} AS episode_id,
            {series_movie_link_ids_json} AS series_movie_link_ids_json,
            mf.file_path,
            mf.size_bytes, mf.announced_size_bytes, {role_expr} AS role,
            mf.source_signature_scheme, mf.source_signature_value,
            mf.full_blake3, mf.move_crc, mf.move_crc_algorithm, mf.hash_computed_at,
            mf.quality_id, mf.scan_status, mf.created_at,
            mf.video_codec, mf.video_width, mf.video_height,
            mf.video_bitrate_kbps, mf.video_bit_depth,
            mf.video_hdr_format, mf.video_frame_rate, mf.video_profile,
            mf.audio_codec, mf.audio_profile, mf.audio_channels, mf.audio_bitrate_kbps,
            mf.duration_seconds, mf.num_chapters, mf.container_format, mf.analysis_json,
            mf.has_multiaudio,
            mf.scene_name, mf.release_group, mf.source_type, mf.resolution,
            mf.video_codec_parsed, mf.audio_codec_parsed, mf.audio_channels_parsed,
            mf.acquisition_score, mf.scoring_log,
            mf.indexer_source, mf.grabbed_release_title, mf.grabbed_at,
            mf.edition, mf.original_file_path, mf.release_hash",
    )
}

fn series_movie_link_ids_aggregate(dialect: SqlDialect) -> &'static str {
    match dialect {
        SqlDialect::Sqlite => {
            "COALESCE(
                (
                    SELECT json_group_array(series_movie_link_id)
                      FROM (
                           SELECT DISTINCT fsmlm.series_movie_link_id AS series_movie_link_id
                             FROM file_series_movie_link_map fsmlm
                            WHERE fsmlm.file_id = mf.id
                            ORDER BY fsmlm.series_movie_link_id
                      )
                ),
                '[]'
            )"
        }
        SqlDialect::Postgres => {
            "COALESCE(
                (
                    SELECT jsonb_agg(DISTINCT fsmlm.series_movie_link_id)
                      FROM file_series_movie_link_map fsmlm
                     WHERE fsmlm.file_id = mf.id
                ),
                '[]'::jsonb
            )::text"
        }
    }
}

fn episode_ids_aggregate(dialect: SqlDialect) -> &'static str {
    match dialect {
        SqlDialect::Sqlite => "COALESCE(json_group_array(DISTINCT fem_all.episode_id), '[]')",
        SqlDialect::Postgres => {
            "COALESCE(
                jsonb_agg(DISTINCT fem_all.episode_id)
                    FILTER (WHERE fem_all.episode_id IS NOT NULL),
                '[]'::jsonb
             )::text"
        }
    }
}

fn primary_episode_ids_aggregate(dialect: SqlDialect) -> &'static str {
    match dialect {
        SqlDialect::Sqlite => {
            "COALESCE(
                json_group_array(DISTINCT fem_all.episode_id)
                    FILTER (WHERE fem_all.role = 'primary'),
                '[]'
             )"
        }
        SqlDialect::Postgres => {
            "COALESCE(
                jsonb_agg(DISTINCT fem_all.episode_id)
                    FILTER (WHERE fem_all.role = 'primary'),
                '[]'::jsonb
             )::text"
        }
    }
}

fn title_partition_fallback(dialect: SqlDialect, title_id_expr: &str) -> String {
    match dialect {
        SqlDialect::Sqlite => format!("printf('title:%s', {title_id_expr})"),
        SqlDialect::Postgres => format!("('title:' || {title_id_expr})"),
    }
}

fn bool_column_is_true(dialect: SqlDialect, column: &str) -> String {
    match dialect {
        SqlDialect::Sqlite => format!("{column} = 1"),
        SqlDialect::Postgres => column.to_string(),
    }
}

async fn fetch_media_files(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<TitleMediaFile>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(row_to_title_media_file)
        .collect()
}

async fn fetch_optional_media_file(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Option<TitleMediaFile>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .as_ref()
        .map(row_to_title_media_file)
        .transpose()
}

fn row_to_title_media_file(row: &SqlRow) -> AppResult<TitleMediaFile> {
    let analysis = analysis_json_from_row(row);
    let mut audio_streams: Vec<scryer_application::AudioStreamDetail> =
        analysis_array_field(&analysis, "audio_streams");
    let mut subtitle_streams: Vec<scryer_application::SubtitleStreamDetail> =
        analysis_array_field(&analysis, "subtitle_streams");

    let audio_language_values: Vec<String> = analysis_array_field(&analysis, "audio_languages");
    let audio_languages = scryer_application::normalize_detected_audio_languages(
        audio_language_values.iter().map(String::as_str),
    );
    for stream in &mut audio_streams {
        stream.language = stream
            .language
            .as_deref()
            .and_then(scryer_application::normalize_detected_audio_language_code);
    }
    let subtitle_language_values: Vec<String> =
        analysis_array_field(&analysis, "subtitle_languages");
    let subtitle_languages = scryer_application::normalize_detected_subtitle_languages(
        subtitle_language_values.iter().map(String::as_str),
    );
    for stream in &mut subtitle_streams {
        stream.language = stream
            .language
            .as_deref()
            .and_then(scryer_application::normalize_detected_subtitle_language_code);
    }

    Ok(TitleMediaFile {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        episode_id: row.opt_text("episode_id")?,
        series_movie_link_ids: string_array_from_json_column(
            row,
            "series_movie_link_ids_json",
            "media_files.series_movie_link_ids_json",
        ),
        file_path: row.text("file_path")?,
        size_bytes: row.i64("size_bytes")?,
        announced_size_bytes: row.opt_i64("announced_size_bytes")?,
        role: row
            .opt_text("role")?
            .as_deref()
            .map(scryer_application::MediaFileRole::from_label)
            .unwrap_or_default(),
        source_signature_scheme: row.opt_text("source_signature_scheme")?,
        source_signature_value: row.opt_text("source_signature_value")?,
        content_hashes: row_to_persisted_content_hashes(row)?,
        quality_label: row.opt_text("quality_id")?,
        scan_status: row.text("scan_status")?,
        created_at: timestamp_text(row, "created_at")?,
        video_codec: parse_stored_video_codec(row.opt_text("video_codec")?)?,
        video_width: row.opt_i32("video_width")?,
        video_height: row.opt_i32("video_height")?,
        video_bitrate_kbps: row.opt_i32("video_bitrate_kbps")?,
        video_bit_depth: row.opt_i32("video_bit_depth")?,
        video_hdr_format: row.opt_text("video_hdr_format")?,
        // Dolby Vision has no dedicated column: it rides in the serialized
        // analysis, so rows written before it was captured read as `None`.
        dovi_profile: analysis_u8_field(&analysis, "dovi_profile"),
        dovi_bl_compat_id: analysis_u8_field(&analysis, "dovi_bl_compat_id"),
        video_frame_rate: row.opt_text("video_frame_rate")?,
        video_profile: row.opt_text("video_profile")?,
        audio_codec: row.opt_text("audio_codec")?,
        audio_profile: row.opt_text("audio_profile")?,
        audio_channels: row.opt_i32("audio_channels")?,
        audio_bitrate_kbps: row.opt_i32("audio_bitrate_kbps")?,
        audio_languages,
        audio_streams,
        subtitle_languages,
        subtitle_codecs: analysis_array_field(&analysis, "subtitle_codecs"),
        subtitle_streams,
        has_multiaudio: row.opt_bool("has_multiaudio")?.unwrap_or(false),
        duration_seconds: row.opt_i32("duration_seconds")?,
        num_chapters: row.opt_i32("num_chapters")?,
        container_format: row.opt_text("container_format")?,
        scene_name: row.opt_text("scene_name")?,
        release_group: row.opt_text("release_group")?,
        source_type: row.opt_text("source_type")?,
        resolution: row.opt_text("resolution")?,
        video_codec_parsed: parse_stored_video_codec(row.opt_text("video_codec_parsed")?)?,
        audio_codec_parsed: row.opt_text("audio_codec_parsed")?,
        audio_channels_parsed: row.opt_text("audio_channels_parsed")?,
        acquisition_score: row.opt_i32("acquisition_score")?,
        scoring_log: row.opt_text("scoring_log")?,
        indexer_source: row.opt_text("indexer_source")?,
        grabbed_release_title: row.opt_text("grabbed_release_title")?,
        grabbed_at: opt_timestamp_text(row, "grabbed_at")?,
        edition: row.opt_text("edition")?,
        original_file_path: row.opt_text("original_file_path")?,
        release_hash: row.opt_text("release_hash")?,
    })
}

/// Reads the migration-0205 columns off a media file row.
///
/// The CRC is stored as text because it is a `u64` and neither engine has an
/// unsigned 64-bit column; an unparseable or untagged CRC reads back as absent
/// rather than as a number nothing can compare against. A missing
/// `full_blake3` means the whole group is absent — that is exactly the state
/// FR-046 invalidation writes.
fn row_to_persisted_content_hashes(
    row: &SqlRow,
) -> AppResult<Option<scryer_application::location::model::PersistedContentHashes>> {
    use scryer_application::location::model::{MoveCrcAlgorithm, PersistedContentHashes};

    let Some(full_blake3) = row
        .opt_text("full_blake3")?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let crc_algorithm = row
        .opt_text("move_crc_algorithm")?
        .and_then(|value| MoveCrcAlgorithm::from_setting(&value).ok());
    let move_crc = row
        .opt_text("move_crc")?
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|_| crc_algorithm.is_some());

    Ok(Some(PersistedContentHashes {
        full_blake3,
        move_crc,
        crc_algorithm,
        hash_computed_at: row.opt_timestamp("hash_computed_at")?,
    }))
}

fn string_array_from_json_column(row: &SqlRow, column: &str, context: &str) -> Vec<String> {
    match row.opt_text(column) {
        Ok(Some(json)) => match serde_json::from_str::<Vec<String>>(&json) {
            Ok(mut values) => {
                values.sort();
                values.dedup();
                values
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    column,
                    context,
                    "failed to parse media file JSON string array column; treating row as empty"
                );
                Vec::new()
            }
        },
        Ok(None) | Err(_) => Vec::new(),
    }
}

fn parse_stored_video_codec(
    raw: Option<String>,
) -> AppResult<Option<scryer_application::VideoCodec>> {
    raw.map(|value| {
        scryer_application::VideoCodec::parse(value.as_str())
            .ok_or_else(|| repo_err(format!("invalid stored video codec {value:?}")))
    })
    .transpose()
}

fn row_to_episode_scoped_media_file(row: &SqlRow) -> AppResult<EpisodeScopedMediaFile> {
    let media_file = row_to_title_media_file(row)?;
    let title_role = row
        .opt_text("title_role")?
        .as_deref()
        .map(scryer_application::MediaFileRole::from_label)
        .unwrap_or_default();
    let mut episode_ids =
        string_array_from_json_column(row, "episode_ids_json", "media_files.episode_ids_json");
    episode_ids.sort();
    episode_ids.dedup();
    let mut primary_episode_ids = string_array_from_json_column(
        row,
        "primary_episode_ids_json",
        "media_files.primary_episode_ids_json",
    );
    primary_episode_ids.sort();
    primary_episode_ids.dedup();

    Ok(EpisodeScopedMediaFile {
        media_file,
        title_role,
        episode_ids,
        primary_episode_ids,
    })
}

fn analysis_json_from_row(row: &SqlRow) -> JsonValue {
    json_text_or(row, "analysis_json", "{}")
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or(JsonValue::Null)
}

fn analysis_u8_field(analysis: &JsonValue, field: &str) -> Option<u8> {
    analysis
        .get(field)
        .and_then(JsonValue::as_u64)
        .and_then(|value| u8::try_from(value).ok())
}

fn analysis_array_field<T: DeserializeOwned>(analysis: &JsonValue, field: &str) -> Vec<T> {
    analysis
        .get(field)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn opt_timestamp_arg_for_datastore(
    datastore: &StoreDatastore,
    value: Option<&str>,
) -> AppResult<SqlArg> {
    match datastore {
        StoreDatastore::Sqlite { .. } => Ok(SqlArg::OptText(value.map(str::to_string))),
        StoreDatastore::Postgres { .. } => value
            .map(parse_utc_datetime)
            .transpose()
            .map(SqlArg::OptTimestamp),
    }
}

fn timestamp_text(row: &SqlRow, column: &str) -> AppResult<String> {
    match row {
        SqlRow::Sqlite(_) => row.text(column),
        SqlRow::Postgres(_) => row.timestamp(column).map(|value| value.to_rfc3339()),
    }
}

fn opt_timestamp_text(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(_) => row.opt_text(column),
        SqlRow::Postgres(_) => row
            .opt_timestamp(column)
            .map(|value| value.map(|value| value.to_rfc3339())),
    }
}

async fn execute_write(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: impl Into<String>,
    args: Vec<SqlArg>,
) -> AppResult<u64> {
    let sql = sql.into();
    SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let sql = sql.clone();
        let args = args.clone();
        Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::shows::store::ShowStore;
    use crate::media::titles::store::TitleStore;
    use chrono::Utc;
    use scryer_application::{
        AudioStreamDetail, MediaFileAnalysis, MediaFileAssociations, MediaFileRepository,
        MediaFileRole, ShowRepository, TitleRepository,
    };
    use scryer_domain::{
        Collection, CollectionType, Episode, MediaFacet, MovieEntity, SeriesMovieLink, Title,
    };
    use scryer_infrastructure_datastore::SqliteServices;

    #[cfg(windows)]
    #[test]
    fn windows_plain_media_file_path_lookup_normalizes_case_and_separators() {
        assert_eq!(
            normalize_windows_plain_media_file_path_lookup("C:/Media/Show/Episode.mkv"),
            "c:\\media\\show\\episode.mkv"
        );
    }

    fn make_test_series_title(id: &str) -> Title {
        Title {
            id: id.to_string(),
            name: "Live Query Test".to_string(),
            facet: MediaFacet::Series,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/series"),
            created_by: None,
            created_at: Utc::now(),
            year: Some(2026),
            overview: Some("overview".to_string()),
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
        }
    }

    fn title_store(services: &SqliteServices) -> TitleStore {
        TitleStore::new(services.datastore())
    }

    fn show_store(services: &SqliteServices) -> ShowStore {
        ShowStore::new(crate::queries::sql_runtime::StoreDatastore::Sqlite {
            pool: services.pool().clone(),
            writer_gate: services.writer_gate(),
        })
    }

    fn media_file_store(services: &SqliteServices) -> MediaFileStore {
        MediaFileStore::new(services.datastore())
    }

    /// FR-047 / FR-046 round trip: the queue predicate, the write, and the
    /// invalidation, over the real SQL rather than a double.
    #[tokio::test]
    async fn full_hash_columns_round_trip_and_drive_the_backfill_queue() {
        use scryer_application::location::model::{MoveCrcAlgorithm, PersistedContentHashes};

        let db = std::env::temp_dir().join(format!(
            "scryer_media_file_full_hashes_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let titles = title_store(&services);
        let media_files = media_file_store(&services);

        let title = make_test_series_title("title-full-hashes");
        titles.create(title.clone()).await.expect("insert title");

        let mut ids = Vec::new();
        for index in 0..3 {
            ids.push(
                media_files
                    .insert_media_file(&InsertMediaFileInput {
                        title_id: title.id.clone(),
                        file_path: format!("/library/show/file-{index}.mkv"),
                        size_bytes: 100 + index,
                        role: MediaFileRole::Primary,
                        ..Default::default()
                    })
                    .await
                    .expect("insert media file"),
            );
        }
        ids.sort();

        // Everything starts on the queue: the 0205 columns are nullable exactly
        // so an existing catalog enters the backfill wholesale.
        let queued = media_files
            .list_media_files_missing_full_hash(None, 10)
            .await
            .expect("read queue");
        assert_eq!(
            queued.iter().map(|row| row.id.clone()).collect::<Vec<_>>(),
            ids
        );
        assert_eq!(queued[0].title_id, title.id);

        // The cursor is "everything after this id", in id order.
        let after_first = media_files
            .list_media_files_missing_full_hash(Some(&ids[0]), 10)
            .await
            .expect("read queue after cursor");
        assert_eq!(
            after_first.iter().map(|row| row.id.clone()).collect::<Vec<_>>(),
            ids[1..]
        );

        // The limit is honoured, or a bounded run is not bounded.
        assert_eq!(
            media_files
                .list_media_files_missing_full_hash(None, 2)
                .await
                .expect("read limited queue")
                .len(),
            2
        );

        // A CRC is a u64 and is stored as text; a round trip that loses it
        // would silently disarm the move-corruption check.
        let computed_at = Utc::now();
        let hashes = PersistedContentHashes {
            full_blake3: "ab".repeat(32),
            move_crc: Some(u64::MAX - 1),
            crc_algorithm: Some(MoveCrcAlgorithm::Crc64Nvme),
            hash_computed_at: Some(computed_at),
        };
        media_files
            .update_media_file_content_hashes(&ids[0], &hashes)
            .await
            .expect("persist hashes");

        let stored = media_files
            .get_media_file_by_id(&ids[0])
            .await
            .expect("load media file")
            .expect("media file exists")
            .content_hashes
            .expect("persisted hashes");
        assert_eq!(stored.full_blake3, hashes.full_blake3);
        assert_eq!(stored.move_crc, Some(u64::MAX - 1));
        assert_eq!(stored.crc_algorithm, Some(MoveCrcAlgorithm::Crc64Nvme));
        assert_eq!(
            stored.hash_computed_at.map(|value| value.timestamp()),
            Some(computed_at.timestamp())
        );

        // A hashed row leaves the queue; that is what makes the sweep converge.
        assert_eq!(
            media_files
                .list_media_files_missing_full_hash(None, 10)
                .await
                .expect("read queue after hashing")
                .iter()
                .map(|row| row.id.clone())
                .collect::<Vec<_>>(),
            ids[1..]
        );

        // FR-046: invalidation clears the whole group and re-queues the row.
        assert!(
            media_files
                .clear_media_file_content_hashes(&ids[0])
                .await
                .expect("clear hashes"),
            "clearing a hashed row reports that it re-entered the queue"
        );
        assert!(
            media_files
                .get_media_file_by_id(&ids[0])
                .await
                .expect("load media file")
                .expect("media file exists")
                .content_hashes
                .is_none()
        );
        assert_eq!(
            media_files
                .list_media_files_missing_full_hash(None, 10)
                .await
                .expect("read queue after invalidation")
                .len(),
            3
        );

        // Clearing an already-unhashed row is a no-op, so a scan that keeps
        // seeing an unhashed file does not report an invalidation every time.
        assert!(
            !media_files
                .clear_media_file_content_hashes(&ids[0])
                .await
                .expect("clear an unhashed row")
        );
    }

    #[tokio::test]
    async fn recycled_media_files_are_excluded_from_live_title_queries() {
        let db = std::env::temp_dir().join(format!(
            "scryer_media_file_live_query_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let titles = title_store(&services);
        let shows = show_store(&services);
        let media_files = media_file_store(&services);

        let title = make_test_series_title("title-live-query");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");

        let collection = Collection {
            id: "collection-live-query".to_string(),
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            monitored: true,
            created_at: Utc::now(),
        };
        ShowRepository::create_collection(&shows, collection.clone())
            .await
            .expect("collection should insert");

        let episode_one = Episode {
            id: "episode-live-query-1".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Episode 1".to_string()),
            air_date: Some("2026-04-01".to_string()),
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let episode_two = Episode {
            id: "episode-live-query-2".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("2".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E02".to_string()),
            title: Some("Episode 2".to_string()),
            air_date: Some("2026-04-08".to_string()),
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        ShowRepository::create_episode(&shows, episode_one.clone())
            .await
            .expect("episode one should insert");
        ShowRepository::create_episode(&shows, episode_two.clone())
            .await
            .expect("episode two should insert");

        let live_file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E01.mkv".to_string(),
                size_bytes: 1_000,
                ..Default::default()
            })
            .await
            .expect("live media file should insert");
        media_files
            .link_file_to_episode(&live_file_id, &episode_one.id)
            .await
            .expect("live file should link");
        media_files
            .set_media_file_roles_for_episode(&title.id, &episode_one.id, &live_file_id, &[])
            .await
            .expect("live file should promote for episode one");

        let recycled_file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path:
                    "/library/Show/.scryer-recycle/20260404_000000_deadbeef/Show - S01E02.mkv"
                        .to_string(),
                size_bytes: 9_999,
                ..Default::default()
            })
            .await
            .expect("recycled media file should insert");
        media_files
            .link_file_to_episode(&recycled_file_id, &episode_two.id)
            .await
            .expect("recycled file should link");

        let live_files = media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files should succeed");
        assert_eq!(live_files.len(), 1);
        assert_eq!(live_files[0].id, live_file_id);
        assert_eq!(
            live_files[0].file_path,
            "/library/Show/Season 01/Show - S01E01.mkv"
        );

        let size_summaries = media_files
            .list_title_media_size_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("size summaries should succeed");
        assert_eq!(size_summaries.len(), 1);
        assert_eq!(size_summaries[0].title_id, title.id);
        assert_eq!(size_summaries[0].total_size_bytes, 1_000);

        let episode_progress = media_files
            .list_title_episode_progress_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("episode progress summaries should succeed");
        assert_eq!(episode_progress.len(), 1);
        assert_eq!(episode_progress[0].title_id, title.id);
        assert_eq!(episode_progress[0].total_episodes, 2);
        assert_eq!(episode_progress[0].monitored_episodes, 2);
        assert_eq!(episode_progress[0].owned_episodes, 1);

        let collection_progress = media_files
            .list_collection_episode_progress_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("collection episode progress summaries should succeed");
        assert_eq!(collection_progress.len(), 1);
        assert_eq!(collection_progress[0].collection_id, collection.id);
        assert_eq!(collection_progress[0].total_episodes, 2);
        assert_eq!(collection_progress[0].monitored_episodes, 2);
        assert_eq!(collection_progress[0].owned_episodes, 1);

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn destination_claim_reuses_the_row_with_every_episode_association() {
        let db = std::env::temp_dir().join(format!(
            "scryer_media_file_path_ownership_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let titles = title_store(&services);
        let shows = show_store(&services);
        let media_files = media_file_store(&services);
        let title = make_test_series_title("title-path-ownership");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");
        let collection = Collection {
            id: "collection-path-ownership".to_string(),
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            monitored: true,
            created_at: Utc::now(),
        };
        ShowRepository::create_collection(&shows, collection.clone())
            .await
            .expect("collection should insert");
        for (id, number) in [
            ("episode-path-ownership-1", "1"),
            ("episode-path-ownership-2", "2"),
        ] {
            ShowRepository::create_episode(
                &shows,
                Episode {
                    id: id.to_string(),
                    title_id: title.id.clone(),
                    collection_id: Some(collection.id.clone()),
                    episode_type: scryer_domain::EpisodeType::Standard,
                    episode_number: Some(number.to_string()),
                    season_number: Some("1".to_string()),
                    episode_label: Some(format!("S01E0{number}")),
                    title: Some(format!("Episode {number}")),
                    air_date: None,
                    duration_seconds: None,
                    has_multi_audio: false,
                    has_subtitle: false,
                    is_filler: false,
                    is_recap: false,
                    absolute_number: None,
                    overview: None,
                    tvdb_id: None,
                    image_url: None,
                    monitored: true,
                    created_at: Utc::now(),
                },
            )
            .await
            .expect("episode should insert");
        }
        let path = "C:\\Media\\Show\\Season 01\\Show - S01E01-E02.mkv";
        let input = InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: path.to_string(),
            original_file_path: Some("C:\\Downloads\\Show.S01E01-E02.mkv".to_string()),
            size_bytes: 1_000,
            ..Default::default()
        };
        let associations = MediaFileAssociations {
            episode_ids: vec![
                "episode-path-ownership-1".to_string(),
                "episode-path-ownership-2".to_string(),
            ],
            series_movie_link_ids: Vec::new(),
        };
        let created = media_files
            .claim_import_destination(&input, &associations)
            .await
            .expect("media file and associations should insert atomically");
        assert_eq!(created.disposition, MediaFileCatalogDisposition::Created);

        #[cfg(windows)]
        let lookup_path = "c:/media/show/season 01/show - s01e01-e02.mkv";
        #[cfg(not(windows))]
        let lookup_path = path;
        let reused = media_files
            .claim_import_destination(
                &InsertMediaFileInput {
                    file_path: lookup_path.to_string(),
                    ..input.clone()
                },
                &associations,
            )
            .await
            .expect("matching ownership should be reusable");
        assert_eq!(reused.media_file_id, created.media_file_id);
        assert_eq!(reused.disposition, MediaFileCatalogDisposition::Reused);

        let link_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM file_episode_map WHERE file_id = ?")
                .bind(&created.media_file_id)
                .fetch_one(services.pool())
                .await
                .expect("episode associations should load");
        assert_eq!(link_count, 2);

        let partial_input = InsertMediaFileInput {
            file_path: "C:\\Media\\Show\\Season 01\\Show - S01E01-E02 partial.mkv".to_string(),
            original_file_path: Some("C:\\Downloads\\Show.partial.mkv".to_string()),
            ..input.clone()
        };
        let partial_id = media_files
            .insert_media_file(&partial_input)
            .await
            .expect("partial media row should insert");
        media_files
            .link_file_to_episode(&partial_id, "episode-path-ownership-1")
            .await
            .expect("partial association should insert");
        let recovered = media_files
            .claim_import_destination(&partial_input, &associations)
            .await
            .expect("a partial same-target pack should be recovered");
        assert_eq!(recovered.media_file_id, partial_id);
        assert_eq!(recovered.disposition, MediaFileCatalogDisposition::Reused);
        let recovered_link_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM file_episode_map WHERE file_id = ?")
                .bind(&partial_id)
                .fetch_one(services.pool())
                .await
                .expect("recovered episode associations should load");
        assert_eq!(recovered_link_count, 2);
        media_files
            .claim_import_destination(
                &partial_input,
                &MediaFileAssociations {
                    episode_ids: vec!["episode-path-ownership-1".to_string()],
                    series_movie_link_ids: Vec::new(),
                },
            )
            .await
            .expect_err("an additional existing association must remain a conflict");

        let unassociated_input = InsertMediaFileInput {
            file_path: "C:\\Media\\Show\\Movie.mkv".to_string(),
            original_file_path: Some("C:\\Downloads\\Movie.mkv".to_string()),
            ..input
        };
        let unassociated_id = media_files
            .insert_media_file(&unassociated_input)
            .await
            .expect("unassociated media row should insert");
        media_files
            .claim_import_destination(
                &InsertMediaFileInput {
                    original_file_path: Some("C:\\Downloads\\Other.mkv".to_string()),
                    ..unassociated_input.clone()
                },
                &MediaFileAssociations::default(),
            )
            .await
            .expect_err("unassociated rows require matching source provenance");
        let recovered = media_files
            .claim_import_destination(&unassociated_input, &MediaFileAssociations::default())
            .await
            .expect("matching source provenance should recover an unassociated row");
        assert_eq!(recovered.media_file_id, unassociated_id);
        assert_eq!(recovered.disposition, MediaFileCatalogDisposition::Reused);

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn atomic_media_file_association_failure_rolls_back_the_media_row() {
        let db = std::env::temp_dir().join(format!(
            "scryer_media_file_association_rollback_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let titles = title_store(&services);
        let media_files = media_file_store(&services);
        let title = make_test_series_title("title-association-rollback");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");
        let path = "/library/Show/Season 01/Show - S01E01.mkv";

        media_files
            .claim_import_destination(
                &InsertMediaFileInput {
                    title_id: title.id,
                    file_path: path.to_string(),
                    size_bytes: 1_000,
                    ..Default::default()
                },
                &MediaFileAssociations {
                    episode_ids: vec!["missing-episode".to_string()],
                    series_movie_link_ids: Vec::new(),
                },
            )
            .await
            .expect_err("a missing association must roll back the media row");

        assert!(
            media_files
                .get_media_file_by_path(path)
                .await
                .expect("media lookup should succeed")
                .is_none()
        );
        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn missing_scope_candidates_include_monitored_titles_episodes_and_series_movies() {
        let db = std::env::temp_dir().join(format!(
            "scryer_missing_scope_candidates_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let titles = title_store(&services);
        let shows = show_store(&services);
        let media_files = media_file_store(&services);

        let title = make_test_series_title("title-missing-scope");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");

        let collection = Collection {
            id: "collection-missing-scope".to_string(),
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        };
        ShowRepository::create_collection(&shows, collection.clone())
            .await
            .expect("collection should insert");

        let episode = Episode {
            id: "episode-missing-scope".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Episode 1".to_string()),
            air_date: Some("2026-04-01".to_string()),
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        ShowRepository::create_episode(&shows, episode.clone())
            .await
            .expect("episode should insert");

        let now = Utc::now();
        let link = SeriesMovieLink {
            id: "series-movie-link-missing-scope".to_string(),
            series_title_id: title.id.clone(),
            movie: MovieEntity {
                id: "movie-entity-missing-scope".to_string(),
                title: "Live Query Movie".to_string(),
                sort_title: None,
                slug: None,
                year: Some(2026),
                overview: None,
                poster_url: None,
                background_url: None,
                language: None,
                runtime_minutes: Some(90),
                content_status: None,
                studio: None,
                digital_release_date: Some("2026-05-01".to_string()),
                imdb_id: None,
                tvdb_id: Some("999001".to_string()),
                tmdb_id: None,
                mal_id: None,
                anidb_id: None,
                ratings: None,
                credits: None,
                created_at: now,
                updated_at: now,
            },
            placement: Some("ordered".to_string()),
            narrative_order: Some("1.5".to_string()),
            after_season: Some(1),
            before_season: None,
            linked_episode_id: None,
            association_confidence: Some("high".to_string()),
            continuity_status: Some("canon".to_string()),
            movie_form: Some("movie".to_string()),
            confidence: Some("high".to_string()),
            signal_summary: None,
            source: Some("anibridge".to_string()),
            monitoring_override: None,
            metadata_active: true,
            monitored: true,
            legacy_collection_id: None,
            created_at: now,
            updated_at: now,
        };
        ShowRepository::upsert_series_movie_link(&shows, link.clone())
            .await
            .expect("series movie link should insert");

        let missing = media_files
            .list_missing_scope_candidates()
            .await
            .expect("missing scope candidates should load");

        let episode_candidate = missing
            .episodes
            .iter()
            .find(|candidate| candidate.episode_id == episode.id)
            .expect("missing episode candidate should be present");
        assert_eq!(episode_candidate.title_id, title.id);
        assert_eq!(
            episode_candidate.collection_id.as_deref(),
            Some(collection.id.as_str())
        );
        assert!(
            !episode_candidate.title_created_at.is_empty(),
            "episode title timestamp should be serialized"
        );

        let title_candidate = missing
            .titles
            .iter()
            .find(|candidate| candidate.title_id == title.id)
            .expect("missing title candidate should be present");
        assert_eq!(title_candidate.library_id, title.library_id);
        assert!(
            !title_candidate.created_at.is_empty(),
            "title timestamp should be serialized"
        );

        let link_candidate = missing
            .series_movie_links
            .iter()
            .find(|candidate| candidate.series_movie_link_id == link.id)
            .expect("missing series movie candidate should be present");
        assert_eq!(link_candidate.title_id, title.id);
        assert_eq!(link_candidate.continuity_status.as_deref(), Some("canon"));
        assert!(
            !link_candidate.link_created_at.is_empty(),
            "series movie link timestamp should be serialized"
        );

        ShowRepository::delete_stale_series_movie_links(&shows, &title.id, &[])
            .await
            .expect("stale series movie link should become inactive");
        let mut inactive_link =
            ShowRepository::list_series_movie_links_for_title(&shows, &title.id)
                .await
                .expect("series movie links should list")
                .into_iter()
                .find(|candidate| candidate.id == link.id)
                .expect("stale series movie link should be retained");
        assert!(!inactive_link.metadata_active);
        assert!(!inactive_link.monitored);
        let inactive = media_files
            .list_missing_scope_candidates()
            .await
            .expect("missing scope candidates should load");
        assert!(
            inactive
                .series_movie_links
                .iter()
                .all(|candidate| candidate.series_movie_link_id != link.id),
            "policy-disabled inactive links must not be acquired"
        );

        inactive_link.monitoring_override = Some(true);
        inactive_link.monitored = true;
        ShowRepository::upsert_series_movie_link(&shows, inactive_link)
            .await
            .expect("explicitly enabled inactive series movie link should update");
        let explicitly_enabled = media_files
            .list_missing_scope_candidates()
            .await
            .expect("missing scope candidates should load");
        assert!(
            explicitly_enabled
                .series_movie_links
                .iter()
                .any(|candidate| candidate.series_movie_link_id == link.id),
            "an explicit operator choice remains eligible after metadata retires the link"
        );

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn title_quality_summaries_use_lowest_live_quality_and_ignore_recycled_files() {
        let db = std::env::temp_dir().join(format!(
            "scryer_title_quality_summary_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let titles = title_store(&services);
        let media_files = media_file_store(&services);

        let title = make_test_series_title("title-quality-summary");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");

        media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E01.mkv".to_string(),
                size_bytes: 1_000,
                quality_label: Some("2160p".to_string()),
                ..Default::default()
            })
            .await
            .expect("high quality file should insert");

        media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E02.mkv".to_string(),
                size_bytes: 1_000,
                quality_label: Some("720p".to_string()),
                ..Default::default()
            })
            .await
            .expect("lower quality file should insert");

        media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path:
                    "/library/Show/.scryer-recycle/20260404_000000_deadbeef/Show - S01E03.mkv"
                        .to_string(),
                size_bytes: 1_000,
                quality_label: Some("360p".to_string()),
                ..Default::default()
            })
            .await
            .expect("recycled file should insert");

        let quality_summaries = media_files
            .list_title_quality_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("quality summaries should succeed");
        assert_eq!(quality_summaries.len(), 1);
        assert_eq!(quality_summaries[0].title_id, title.id);
        assert_eq!(quality_summaries[0].quality_tier, "720P");

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn cutoff_unmet_quality_summaries_expand_season_pack_links() {
        let db = std::env::temp_dir().join(format!(
            "scryer_cutoff_unmet_quality_summary_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let titles = title_store(&services);
        let media_files = media_file_store(&services);
        let shows = show_store(&services);

        let title = make_test_series_title("title-cutoff-summary");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");

        let collection = Collection {
            id: "collection-cutoff-summary".to_string(),
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("3".to_string()),
            monitored: true,
            created_at: Utc::now(),
        };
        ShowRepository::create_collection(&shows, collection.clone())
            .await
            .expect("collection should insert");

        let monitored_episode_one = Episode {
            id: "episode-cutoff-summary-1".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Episode 1".to_string()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let monitored_episode_two = Episode {
            id: "episode-cutoff-summary-2".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("2".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E02".to_string()),
            title: Some("Episode 2".to_string()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let unmonitored_episode_three = Episode {
            id: "episode-cutoff-summary-3".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("3".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E03".to_string()),
            title: Some("Episode 3".to_string()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: false,
            created_at: Utc::now(),
        };
        for episode in [
            &monitored_episode_one,
            &monitored_episode_two,
            &unmonitored_episode_three,
        ] {
            ShowRepository::create_episode(&shows, episode.clone())
                .await
                .expect("episode should insert");
        }

        let pack_file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01 pack.mkv".to_string(),
                size_bytes: 1_000,
                quality_label: Some("720p".to_string()),
                ..Default::default()
            })
            .await
            .expect("season pack should insert");

        for episode_id in [
            &monitored_episode_one.id,
            &monitored_episode_two.id,
            &unmonitored_episode_three.id,
        ] {
            media_files
                .link_file_to_episode(&pack_file_id, episode_id)
                .await
                .expect("season pack should link");
        }
        media_files
            .update_media_file_analysis(
                &pack_file_id,
                MediaFileAnalysis {
                    video_codec: None,
                    video_width: Some(1920),
                    video_height: Some(800),
                    video_bitrate_kbps: None,
                    video_bit_depth: None,
                    video_hdr_format: None,
                    dovi_profile: None,
                    dovi_bl_compat_id: None,
                    video_frame_rate: None,
                    video_profile: None,
                    audio_codec: None,
                    audio_profile: None,
                    audio_channels: None,
                    audio_bitrate_kbps: None,
                    audio_languages: vec![],
                    audio_streams: vec![],
                    subtitle_languages: vec![],
                    subtitle_codecs: vec![],
                    subtitle_streams: vec![],
                    has_multiaudio: false,
                    duration_seconds: None,
                    num_chapters: None,
                    container_format: None,
                },
            )
            .await
            .expect("season pack analysis should update");

        let summaries = media_files
            .list_cutoff_unmet_quality_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("cutoff summaries should succeed");

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].title_id, title.id);
        assert_eq!(summaries[0].quality_tier, "1080P");
        assert_eq!(summaries[0].season_number.as_deref(), Some("1"));
        assert_eq!(summaries[0].episode_number.as_deref(), Some("1"));
        assert_eq!(summaries[1].quality_tier, "1080P");
        assert_eq!(summaries[1].season_number.as_deref(), Some("1"));
        assert_eq!(summaries[1].episode_number.as_deref(), Some("2"));

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn episode_media_availability_prefers_primary_then_largest_additional_file() {
        let db = std::env::temp_dir().join(format!(
            "scryer_episode_media_availability_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let titles = title_store(&services);
        let shows = show_store(&services);
        let media_files = media_file_store(&services);

        let title = make_test_series_title("title-episode-media-availability");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");
        let collection = Collection {
            id: "collection-episode-media-availability".to_string(),
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("7".to_string()),
            monitored: true,
            created_at: Utc::now(),
        };
        ShowRepository::create_collection(&shows, collection.clone())
            .await
            .expect("collection should insert");
        let episode = |id: &str, number: &str, monitored: bool| Episode {
            id: id.to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some(number.to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some(format!("S01E{number}")),
            title: Some(format!("Episode {number}")),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored,
            created_at: Utc::now(),
        };
        let available = episode("episode-availability-available", "01", true);
        let missing = episode("episode-availability-missing", "02", true);
        let unmonitored = episode("episode-availability-unmonitored", "03", false);
        let pending = episode("episode-availability-pending", "04", true);
        let failed = episode("episode-availability-failed", "05", true);
        let unmonitored_owned = episode("episode-availability-unmonitored-owned", "06", false);
        let additional_only = episode("episode-availability-additional-only", "07", true);
        let legacy_multiple_primary =
            episode("episode-availability-legacy-multiple-primary", "08", true);
        for episode in [
            &available,
            &missing,
            &unmonitored,
            &pending,
            &failed,
            &unmonitored_owned,
            &additional_only,
            &legacy_multiple_primary,
        ] {
            ShowRepository::create_episode(&shows, episode.clone())
                .await
                .expect("episode should insert");
        }

        let analysis = |width| MediaFileAnalysis {
            video_codec: None,
            video_width: Some(width),
            video_height: None,
            video_bitrate_kbps: None,
            video_bit_depth: None,
            video_hdr_format: None,
            dovi_profile: None,
            dovi_bl_compat_id: None,
            video_frame_rate: None,
            video_profile: None,
            audio_codec: None,
            audio_profile: None,
            audio_channels: None,
            audio_bitrate_kbps: None,
            audio_languages: vec![],
            audio_streams: vec![],
            subtitle_languages: vec![],
            subtitle_codecs: vec![],
            subtitle_streams: vec![],
            has_multiaudio: false,
            duration_seconds: None,
            num_chapters: None,
            container_format: None,
        };
        let insert_file = |path: &str, role| InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: format!("/library/Show/{path}.mkv"),
            size_bytes: 1_000,
            role,
            ..Default::default()
        };

        let available_file = media_files
            .insert_media_file(&insert_file("S01E01", MediaFileRole::Primary))
            .await
            .expect("available primary should insert");
        media_files
            .link_file_to_episode(&available_file, &available.id)
            .await
            .expect("available primary should link");
        media_files
            .update_media_file_analysis(&available_file, analysis(1920))
            .await
            .expect("available primary should scan");

        let mut larger_additional_for_primary_input =
            insert_file("S01E01-extra", MediaFileRole::Additional);
        larger_additional_for_primary_input.size_bytes = 2_000;
        let larger_additional_for_primary = media_files
            .insert_media_file(&larger_additional_for_primary_input)
            .await
            .expect("larger additional file should insert");
        media_files
            .link_file_to_episode(&larger_additional_for_primary, &available.id)
            .await
            .expect("larger additional file should link");
        media_files
            .update_media_file_analysis(&larger_additional_for_primary, analysis(3840))
            .await
            .expect("larger additional file should scan");
        media_files
            .set_media_file_roles_for_episode(
                &title.id,
                &available.id,
                &available_file,
                std::slice::from_ref(&larger_additional_for_primary),
            )
            .await
            .expect("available episode association should promote");

        let pending_file = media_files
            .insert_media_file(&insert_file("S01E04", MediaFileRole::Primary))
            .await
            .expect("pending primary should insert");
        media_files
            .link_file_to_episode(&pending_file, &pending.id)
            .await
            .expect("pending primary should link");
        media_files
            .set_media_file_roles_for_episode(&title.id, &pending.id, &pending_file, &[])
            .await
            .expect("pending episode association should promote");

        let failed_file = media_files
            .insert_media_file(&insert_file("S01E05", MediaFileRole::Primary))
            .await
            .expect("failed primary should insert");
        media_files
            .link_file_to_episode(&failed_file, &failed.id)
            .await
            .expect("failed primary should link");
        media_files
            .set_media_file_roles_for_episode(&title.id, &failed.id, &failed_file, &[])
            .await
            .expect("failed episode association should promote");
        media_files
            .mark_scan_failed(&failed_file, "test failure")
            .await
            .expect("failed primary should be marked");

        let unmonitored_owned_file = media_files
            .insert_media_file(&insert_file("S01E06", MediaFileRole::Primary))
            .await
            .expect("unmonitored primary should insert");
        media_files
            .link_file_to_episode(&unmonitored_owned_file, &unmonitored_owned.id)
            .await
            .expect("unmonitored primary should link");
        media_files
            .set_media_file_roles_for_episode(
                &title.id,
                &unmonitored_owned.id,
                &unmonitored_owned_file,
                &[],
            )
            .await
            .expect("unmonitored episode association should promote");
        media_files
            .update_media_file_analysis(&unmonitored_owned_file, analysis(1280))
            .await
            .expect("unmonitored primary should scan");

        let additional_file = media_files
            .insert_media_file(&insert_file("S01E07-extra", MediaFileRole::Additional))
            .await
            .expect("additional file should insert");
        media_files
            .link_file_to_episode(&additional_file, &additional_only.id)
            .await
            .expect("additional file should link");
        media_files
            .update_media_file_analysis(&additional_file, analysis(3840))
            .await
            .expect("additional file should scan");

        let mut smaller_additional_input =
            insert_file("S01E07-extra-small", MediaFileRole::Additional);
        smaller_additional_input.size_bytes = 500;
        let smaller_additional_file = media_files
            .insert_media_file(&smaller_additional_input)
            .await
            .expect("smaller additional file should insert");
        media_files
            .link_file_to_episode(&smaller_additional_file, &additional_only.id)
            .await
            .expect("smaller additional file should link");
        media_files
            .update_media_file_analysis(&smaller_additional_file, analysis(1280))
            .await
            .expect("smaller additional file should scan");

        let older_legacy_primary = media_files
            .insert_media_file(&insert_file("S01E08-legacy-old", MediaFileRole::Primary))
            .await
            .expect("older legacy primary should insert");
        media_files
            .link_file_to_episode(&older_legacy_primary, &legacy_multiple_primary.id)
            .await
            .expect("older legacy primary should link");
        media_files
            .update_media_file_analysis(&older_legacy_primary, analysis(1280))
            .await
            .expect("older legacy primary should scan");

        let newer_legacy_primary = media_files
            .insert_media_file(&insert_file("S01E08-legacy-new", MediaFileRole::Primary))
            .await
            .expect("newer legacy primary should insert");
        media_files
            .link_file_to_episode(&newer_legacy_primary, &legacy_multiple_primary.id)
            .await
            .expect("newer legacy primary should link");
        media_files
            .update_media_file_analysis(&newer_legacy_primary, analysis(1920))
            .await
            .expect("newer legacy primary should scan");
        media_files
            .set_media_file_roles_for_episode(
                &title.id,
                &legacy_multiple_primary.id,
                &newer_legacy_primary,
                std::slice::from_ref(&older_legacy_primary),
            )
            .await
            .expect("newer episode association should promote");

        let summaries = media_files
            .list_episode_media_availability(std::slice::from_ref(&title.id))
            .await
            .expect("availability summaries should load");
        let summary = |episode_id: &str| {
            summaries
                .iter()
                .find(|summary| summary.episode_id == episode_id)
                .expect("episode availability summary")
        };

        assert_eq!(
            summary(&available.id).state,
            EpisodeMediaAvailabilityState::Available
        );
        assert_eq!(
            summary(&available.id).primary_quality_label.as_deref(),
            Some("1080p")
        );
        assert_eq!(
            summary(&missing.id).state,
            EpisodeMediaAvailabilityState::Missing
        );
        assert_eq!(
            summary(&unmonitored.id).state,
            EpisodeMediaAvailabilityState::Unmonitored
        );
        assert_eq!(
            summary(&pending.id).state,
            EpisodeMediaAvailabilityState::PendingScan
        );
        assert_eq!(
            summary(&failed.id).state,
            EpisodeMediaAvailabilityState::ScanFailed
        );
        assert_eq!(
            summary(&unmonitored_owned.id).state,
            EpisodeMediaAvailabilityState::Available,
            "an unmonitored episode with a primary file remains available"
        );
        assert_eq!(
            summary(&unmonitored_owned.id)
                .primary_quality_label
                .as_deref(),
            Some("720p")
        );
        assert_eq!(
            summary(&additional_only.id).state,
            EpisodeMediaAvailabilityState::Available,
            "the largest additional file supplies availability when primary is absent"
        );
        assert_eq!(
            summary(&additional_only.id)
                .primary_quality_label
                .as_deref(),
            Some("4K"),
            "the largest additional file supplies the collapsed-row quality"
        );
        assert_eq!(
            summary(&legacy_multiple_primary.id)
                .primary_quality_label
                .as_deref(),
            Some("1080p"),
            "the episode-scoped primary supplies the collapsed-row quality"
        );

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn episode_scoped_media_file_query_dedupes_file_ids_and_returns_full_episode_set() {
        let db = std::env::temp_dir().join(format!(
            "scryer_episode_scoped_media_files_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let titles = title_store(&services);
        let shows = show_store(&services);
        let media_files = media_file_store(&services);

        let title = make_test_series_title("title-episode-scope");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");

        let collection = Collection {
            id: "collection-episode-scope".to_string(),
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("3".to_string()),
            monitored: true,
            created_at: Utc::now(),
        };
        ShowRepository::create_collection(&shows, collection.clone())
            .await
            .expect("collection should insert");

        let episode_one = Episode {
            id: "episode-episode-scope-1".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Episode 1".to_string()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let episode_two = Episode {
            id: "episode-episode-scope-2".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("2".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E02".to_string()),
            title: Some("Episode 2".to_string()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let episode_three = Episode {
            id: "episode-episode-scope-3".to_string(),
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("3".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E03".to_string()),
            title: Some("Episode 3".to_string()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        for episode in [&episode_one, &episode_two, &episode_three] {
            ShowRepository::create_episode(&shows, episode.clone())
                .await
                .expect("episode should insert");
        }

        let pack_file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E01-E02.mkv".to_string(),
                size_bytes: 2_000,
                ..Default::default()
            })
            .await
            .expect("pack file should insert");
        media_files
            .link_file_to_episode(&pack_file_id, &episode_one.id)
            .await
            .expect("pack should link episode one");
        media_files
            .link_file_to_episode(&pack_file_id, &episode_two.id)
            .await
            .expect("pack should link episode two");
        let linked_pack = media_files
            .list_live_media_files_for_episode_ids(
                &title.id,
                &[episode_one.id.clone(), episode_two.id.clone()],
            )
            .await
            .expect("neutral episode links should load")
            .into_iter()
            .next()
            .expect("linked pack should be returned");
        assert!(linked_pack.title_role.is_primary());
        assert!(linked_pack.primary_episode_ids.is_empty());

        for episode_id in [&episode_one.id, &episode_two.id] {
            media_files
                .set_media_file_roles_for_episode(&title.id, episode_id, &pack_file_id, &[])
                .await
                .expect("pack should promote for linked episode");
        }

        let single_file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E03.mkv".to_string(),
                size_bytes: 1_000,
                ..Default::default()
            })
            .await
            .expect("single file should insert");
        media_files
            .link_file_to_episode(&single_file_id, &episode_three.id)
            .await
            .expect("single should link episode three");

        let recycled_file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path:
                    "/library/Show/.scryer-recycle/20260404_000000_deadbeef/Show - S01E01.mkv"
                        .to_string(),
                size_bytes: 999,
                ..Default::default()
            })
            .await
            .expect("recycled file should insert");
        media_files
            .link_file_to_episode(&recycled_file_id, &episode_one.id)
            .await
            .expect("recycled should link episode one");

        let scoped = media_files
            .list_live_media_files_for_episode_ids(
                &title.id,
                &[episode_one.id.clone(), episode_two.id.clone()],
            )
            .await
            .expect("episode scoped query should succeed");

        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].media_file.id, pack_file_id);
        assert_eq!(
            scoped[0].episode_ids,
            vec![episode_one.id.clone(), episode_two.id.clone()]
        );
        assert_eq!(
            scoped[0].primary_episode_ids,
            vec![episode_one.id.clone(), episode_two.id.clone()]
        );

        let episode_two_replacement_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E02 alternate.mkv".to_string(),
                size_bytes: 2_500,
                ..Default::default()
            })
            .await
            .expect("episode two replacement should insert");
        media_files
            .link_file_to_episode(&episode_two_replacement_id, &episode_two.id)
            .await
            .expect("replacement should link episode two");
        media_files
            .set_media_file_roles_for_episode(
                &title.id,
                &episode_two.id,
                &episode_two_replacement_id,
                std::slice::from_ref(&pack_file_id),
            )
            .await
            .expect("episode two replacement should promote");

        let scoped = media_files
            .list_live_media_files_for_episode_ids(
                &title.id,
                &[episode_one.id.clone(), episode_two.id.clone()],
            )
            .await
            .expect("promoted episode scoped query should succeed");
        let pack = scoped
            .iter()
            .find(|file| file.media_file.id == pack_file_id)
            .expect("shared pack should remain linked");
        assert_eq!(pack.primary_episode_ids, vec![episode_one.id.clone()]);
        let replacement = scoped
            .iter()
            .find(|file| file.media_file.id == episode_two_replacement_id)
            .expect("replacement should be returned");
        assert_eq!(
            replacement.primary_episode_ids,
            vec![episode_two.id.clone()]
        );

        let episode_two_files = media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("title media files should load")
            .into_iter()
            .filter(|file| file.episode_id.as_deref() == Some(episode_two.id.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            episode_two_files
                .iter()
                .filter(|file| file.role.is_primary())
                .count(),
            1
        );

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn media_file_roundtrip_persists_audio_profile_and_parsed_channel_backup() {
        let db = std::env::temp_dir().join(format!(
            "scryer_media_file_audio_profile_{}.db",
            chrono::Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("db should initialize");
        let titles = title_store(&services);
        let media_files = media_file_store(&services);

        let title = make_test_series_title("title-audio-profile");
        titles
            .create(title.clone())
            .await
            .expect("title should insert");

        let file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: "/library/Show/Season 01/Show - S01E01.mkv".to_string(),
                size_bytes: 1_000,
                quality_label: Some("720p".to_string()),
                resolution: Some("720p".to_string()),
                source_type: Some("WEB-DL".to_string()),
                audio_channels_parsed: Some("7.1".to_string()),
                ..Default::default()
            })
            .await
            .expect("media file should insert");

        media_files
            .update_media_file_analysis(
                &file_id,
                MediaFileAnalysis {
                    video_codec: Some(
                        scryer_application::VideoCodec::parse("hevc").expect("parse codec"),
                    ),
                    video_width: Some(1920),
                    video_height: Some(800),
                    video_bitrate_kbps: None,
                    video_bit_depth: Some(10),
                    video_hdr_format: Some("HDR10".to_string()),
                    dovi_profile: None,
                    dovi_bl_compat_id: None,
                    video_frame_rate: Some("23.976".to_string()),
                    video_profile: Some("Main 10".to_string()),
                    audio_codec: Some("dts".to_string()),
                    audio_profile: Some("DTS-HD MA + DTS:X IMAX".to_string()),
                    audio_channels: Some(8),
                    audio_bitrate_kbps: Some(4_000),
                    audio_languages: vec!["eng".to_string()],
                    audio_streams: vec![AudioStreamDetail {
                        codec: Some("dts".to_string()),
                        profile: Some("DTS-HD MA + DTS:X IMAX".to_string()),
                        channels: Some(8),
                        language: Some("eng".to_string()),
                        name: None,
                        bitrate_kbps: Some(4_000),
                    }],
                    subtitle_languages: vec![],
                    subtitle_codecs: vec![],
                    subtitle_streams: vec![],
                    has_multiaudio: false,
                    duration_seconds: Some(1800),
                    num_chapters: Some(4),
                    container_format: Some("matroska".to_string()),
                },
            )
            .await
            .expect("analysis should update");

        let files = media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files should succeed");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].quality_label.as_deref(), Some("720p"));
        assert_eq!(files[0].resolution.as_deref(), Some("720p"));
        assert_eq!(files[0].source_type.as_deref(), Some("WEB-DL"));
        assert_eq!(
            files[0].audio_profile.as_deref(),
            Some("DTS-HD MA + DTS:X IMAX")
        );
        assert_eq!(files[0].audio_channels_parsed.as_deref(), Some("7.1"));
        assert_eq!(
            files[0].audio_streams[0].profile.as_deref(),
            Some("DTS-HD MA + DTS:X IMAX")
        );
        let quality_summaries = media_files
            .list_title_quality_summaries(std::slice::from_ref(&title.id))
            .await
            .expect("quality summaries should succeed");
        assert_eq!(quality_summaries.len(), 1);
        assert_eq!(quality_summaries[0].quality_tier, "1080P");

        let _ = std::fs::remove_file(db);
    }
}

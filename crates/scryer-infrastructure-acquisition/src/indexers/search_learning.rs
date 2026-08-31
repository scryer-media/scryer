use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use scryer_application::{
    AppError, AppResult, HashDomain, IndexerSearchCandidateWrite, IndexerSearchLearningKey,
    IndexerSearchLearningRecord, IndexerSearchLearningRepository, IndexerSearchRunWrite,
    NormalizedIndexerSearchCandidate, ReusableIndexerSearchCandidate,
    ReusableIndexerSearchStrategy, blake3_identity_hex,
};
use scryer_infrastructure_crypto::{
    EncryptionKey,
    config::{current_encryption_key, decrypt_optional_value, encrypt_optional_value},
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore, repo_err};

fn sqlite_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[derive(Clone)]
pub struct IndexerSearchLearningStore {
    datastore: StoreDatastore,
    encryption_key: std::sync::Arc<std::sync::RwLock<Option<EncryptionKey>>>,
}

impl IndexerSearchLearningStore {
    pub fn new(
        datastore: StoreDatastore,
        encryption_key: std::sync::Arc<std::sync::RwLock<Option<EncryptionKey>>>,
    ) -> Self {
        Self {
            datastore,
            encryption_key,
        }
    }

    async fn get_record(
        &self,
        key: &IndexerSearchLearningKey,
    ) -> AppResult<Option<IndexerSearchLearningRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT indexer_id, title_id, facet, strategy_key, attempts, empty_successes,
                    usable_successes, last_attempt_at, last_usable_at, suppressed, updated_at
             FROM indexer_search_learning
             WHERE indexer_id = {} AND title_id = {} AND facet = {} AND strategy_key = {}",
            &[
                SqlArg::Text(key.indexer_id.clone()),
                SqlArg::Text(key.title_id.clone()),
                SqlArg::Text(key.facet.clone()),
                SqlArg::Text(key.strategy_key.clone()),
            ],
        )
        .await?;

        row.as_ref().map(row_to_learning_record).transpose()
    }

    async fn hydrate_candidates(
        &self,
        rows: Vec<SqlRow>,
    ) -> AppResult<Vec<ReusableIndexerSearchCandidate>> {
        let key = current_encryption_key(&self.encryption_key)?;
        let mut order = Vec::with_capacity(rows.len());
        let mut candidates = HashMap::with_capacity(rows.len());
        for row in &rows {
            let id = row.text("id")?;
            order.push(id.clone());
            candidates.insert(
                id,
                NormalizedIndexerSearchCandidate {
                    provider_ref: row.opt_text("provider_ref")?,
                    source: row.text("source")?,
                    title: row.text("title")?,
                    download_url: decrypt_optional_value(
                        key.as_ref(),
                        row.opt_text("encrypted_download_url")?,
                        "indexer candidate grab URL",
                        true,
                    )?,
                    download_url_credential_keys: Vec::new(),
                    link_url: decrypt_optional_value(
                        key.as_ref(),
                        row.opt_text("encrypted_link_url")?,
                        "indexer candidate link URL",
                        true,
                    )?,
                    link_url_credential_keys: Vec::new(),
                    size_bytes: row.opt_i64("size_bytes")?,
                    published_at: row.opt_text("published_at")?,
                    source_kind: row.opt_text("source_kind")?,
                    thumbs_up: row.opt_i32("thumbs_up")?,
                    thumbs_down: row.opt_i32("thumbs_down")?,
                    grabs: row.opt_i64("grabs")?,
                    grab_current: row.opt_i64("grab_current")?,
                    grab_max: row.opt_i64("grab_max")?,
                    languages: Vec::new(),
                    subtitles: Vec::new(),
                    response_tvdb_id: row.opt_text("response_tvdb_id")?,
                    response_tmdb_id: row.opt_text("response_tmdb_id")?,
                    response_imdb_id: row.opt_text("response_imdb_id")?,
                    response_categories: Vec::new(),
                    extra_categories: Vec::new(),
                    season: row.opt_i64("season")?,
                    episode: row.opt_i64("episode")?,
                    absolute_episode: row.opt_i64("absolute_episode")?,
                    series_names: Vec::new(),
                    release_group: row.opt_text("release_group")?,
                    provider_source: row.opt_text("provider_source")?,
                    info_hash: row.opt_text("info_hash")?,
                    seeders: row.opt_i64("seeders")?,
                    peers: row.opt_i64("peers")?,
                    download_volume_factor: row.opt_f64("download_volume_factor")?,
                    upload_volume_factor: row.opt_f64("upload_volume_factor")?,
                    protected: row.opt_bool("protected")?,
                    tags: Vec::new(),
                    provider_categories: Vec::new(),
                },
            );
        }
        if order.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = std::iter::repeat_n("{}", order.len())
            .collect::<Vec<_>>()
            .join(", ");
        let candidate_args = || order.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
        let value_rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT source_id AS candidate_id, value_kind, value
                   FROM indexer_search_candidate_source_values
                  WHERE source_id IN ({placeholders})
                  ORDER BY source_id, value_kind, ordinal"
            ),
            &candidate_args(),
        )
        .await?;
        for row in &value_rows {
            let candidate_id = row.text("candidate_id")?;
            let Some(candidate) = candidates.get_mut(&candidate_id) else {
                continue;
            };
            let value = row.text("value")?;
            match row.text("value_kind")?.as_str() {
                "language" => candidate.languages.push(value),
                "subtitle" => candidate.subtitles.push(value),
                "response_category" => candidate.response_categories.push(value),
                "extra_category" => candidate.extra_categories.push(value),
                "series_name" => candidate.series_names.push(value),
                "tag" => candidate.tags.push(value),
                "provider_category" => candidate.provider_categories.push(value),
                _ => {}
            }
        }

        Ok(order
            .into_iter()
            .filter_map(|id| candidates.remove(&id))
            .map(|normalized| ReusableIndexerSearchCandidate { normalized })
            .collect())
    }
}

#[async_trait]
impl IndexerSearchLearningRepository for IndexerSearchLearningStore {
    async fn list_for_title(
        &self,
        indexer_id: &str,
        title_id: &str,
        facet: &str,
    ) -> AppResult<Vec<IndexerSearchLearningRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT indexer_id, title_id, facet, strategy_key, attempts, empty_successes,
                    usable_successes, last_attempt_at, last_usable_at, suppressed, updated_at
             FROM indexer_search_learning
             WHERE indexer_id = {} AND title_id = {} AND facet = {}",
            &[
                SqlArg::Text(indexer_id.to_string()),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(facet.to_string()),
            ],
        )
        .await?;

        rows.iter().map(row_to_learning_record).collect()
    }

    async fn record_outcome(
        &self,
        key: &IndexerSearchLearningKey,
        usable_hits: u32,
    ) -> AppResult<IndexerSearchLearningRecord> {
        match &self.datastore {
            StoreDatastore::Sqlite { .. } => {
                let key = key.clone();
                SqlRuntime::run_serialized_sqlite(
                    &self.datastore,
                    "record_indexer_search_learning_outcome",
                    move |pool| {
                        let key = key.clone();
                        async move {
                            sqlx::query(
                                "INSERT INTO indexer_search_learning (
                                    indexer_id, title_id, facet, strategy_key, attempts,
                                    empty_successes, usable_successes, last_attempt_at,
                                    last_usable_at, suppressed, updated_at
                                 )
                                 VALUES (
                                    ?, ?, ?, ?, 1,
                                    CASE WHEN ? = 0 THEN 1 ELSE 0 END,
                                    CASE WHEN ? > 0 THEN 1 ELSE 0 END,
                                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                                    CASE WHEN ? > 0 THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') ELSE NULL END,
                                    0,
                                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                                 )
                                 ON CONFLICT(indexer_id, title_id, facet, strategy_key)
                                 DO UPDATE SET
                                    attempts = indexer_search_learning.attempts + 1,
                                    empty_successes = indexer_search_learning.empty_successes
                                        + CASE WHEN excluded.usable_successes = 0 THEN 1 ELSE 0 END,
                                    usable_successes = indexer_search_learning.usable_successes
                                        + CASE WHEN excluded.usable_successes > 0 THEN 1 ELSE 0 END,
                                    last_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                                    last_usable_at = CASE
                                        WHEN excluded.usable_successes > 0 THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                                        ELSE indexer_search_learning.last_usable_at
                                    END,
                                    suppressed = CASE
                                        WHEN excluded.usable_successes > 0 THEN 0
                                        ELSE indexer_search_learning.suppressed
                                    END,
                                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                            )
                            .bind(&key.indexer_id)
                            .bind(&key.title_id)
                            .bind(&key.facet)
                            .bind(&key.strategy_key)
                            .bind(usable_hits as i64)
                            .bind(usable_hits as i64)
                            .bind(usable_hits as i64)
                            .execute(&pool)
                            .await
                            .map_err(repo_err)?;
                            Ok(())
                        }
                    },
                )
                .await?;
            }
            StoreDatastore::Postgres { pool } => {
                sqlx::query(
                    "INSERT INTO indexer_search_learning (
                        indexer_id, title_id, facet, strategy_key, attempts,
                        empty_successes, usable_successes, last_attempt_at,
                        last_usable_at, suppressed, updated_at
                     )
                     VALUES (
                        $1, $2, $3, $4, 1,
                        CASE WHEN $5 = 0 THEN 1 ELSE 0 END,
                        CASE WHEN $5 > 0 THEN 1 ELSE 0 END,
                        NOW(),
                        CASE WHEN $5 > 0 THEN NOW() ELSE NULL END,
                        FALSE,
                        NOW()
                     )
                     ON CONFLICT(indexer_id, title_id, facet, strategy_key)
                     DO UPDATE SET
                        attempts = indexer_search_learning.attempts + 1,
                        empty_successes = indexer_search_learning.empty_successes
                            + CASE WHEN EXCLUDED.usable_successes = 0 THEN 1 ELSE 0 END,
                        usable_successes = indexer_search_learning.usable_successes
                            + CASE WHEN EXCLUDED.usable_successes > 0 THEN 1 ELSE 0 END,
                        last_attempt_at = NOW(),
                        last_usable_at = CASE
                            WHEN EXCLUDED.usable_successes > 0 THEN NOW()
                            ELSE indexer_search_learning.last_usable_at
                        END,
                        suppressed = CASE
                            WHEN EXCLUDED.usable_successes > 0 THEN FALSE
                            ELSE indexer_search_learning.suppressed
                        END,
                        updated_at = NOW()",
                )
                .bind(&key.indexer_id)
                .bind(&key.title_id)
                .bind(&key.facet)
                .bind(&key.strategy_key)
                .bind(usable_hits as i64)
                .execute(pool)
                .await
                .map_err(repo_err)?;
            }
        }

        self.get_record(key).await?.ok_or_else(|| {
            AppError::Repository("indexer search learning outcome was not persisted".to_string())
        })
    }

    async fn record_search_diagnostics(
        &self,
        run: &IndexerSearchRunWrite,
        candidates: &[IndexerSearchCandidateWrite],
    ) -> AppResult<()> {
        let run = run.clone();
        let candidates = candidates.to_vec();
        let encryption_key = current_encryption_key(&self.encryption_key)?;
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_indexer_search_diagnostics",
            move |tx| {
                let run = run.clone();
                let candidates = candidates.clone();
                let encryption_key = encryption_key.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO indexer_search_runs (
                            id, indexer_id, provider_type, search_session_id, scope_key, query_signature,
                            branch, page, range_min_size, range_max_size, result_count,
                            completion_state, retry_at, error_summary, indexer_fingerprint,
                            created_at
                         ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                        &[
                            SqlArg::Text(run.id.clone()),
                            SqlArg::Text(run.indexer_id.clone()),
                            SqlArg::Text(run.provider_type.clone()),
                            SqlArg::Text(run.search_session_id.clone()),
                            SqlArg::Text(run.scope_key.clone()),
                            SqlArg::Text(run.query_signature.clone()),
                            SqlArg::Text(run.branch.clone()),
                            SqlArg::OptI64(run.page.map(i64::from)),
                            SqlArg::OptI64(run.range_min_size),
                            SqlArg::OptI64(run.range_max_size),
                            SqlArg::I64(i64::from(run.result_count)),
                            SqlArg::Text(run.completion_state.clone()),
                            SqlArg::OptTimestamp(run.retry_at),
                            SqlArg::OptText(run.error_summary.clone()),
                            SqlArg::Text(run.indexer_fingerprint.clone()),
                            SqlArg::Timestamp(run.created_at),
                        ],
                    )
                    .await?;

                    for candidate in candidates {
                        let IndexerSearchCandidateWrite {
                            id: _,
                            run_id,
                            search_session_id,
                            indexer_id,
                            scope_key: _,
                            query_signature: _,
                            session_identity_hash,
                            normalized,
                            created_at,
                            reusable_until,
                            expires_at,
                        } = candidate;
                        let candidate_id = session_identity_hash.clone();
                        let source_reference = normalized
                            .provider_ref
                            .as_deref()
                            .filter(|value| !value.trim().is_empty())
                            .or(normalized.download_url.as_deref())
                            .or(normalized.link_url.as_deref())
                            .unwrap_or(normalized.source.as_str())
                            .to_string();
                        let source_identity = blake3_identity_hex(
                            HashDomain::CandidateSessionIdentity,
                            format!("candidate-source-reference\0{source_reference}"),
                        );
                        let source_id = blake3_identity_hex(
                            HashDomain::CandidateSessionIdentity,
                            format!("source\0{candidate_id}\0{indexer_id}\0{source_identity}"),
                        );
                        let encrypted_download_url = encrypt_optional_value(
                            encryption_key.as_ref(),
                            normalized.download_url.as_ref(),
                            "indexer candidate grab URL",
                            true,
                        )?;
                        let encrypted_link_url = encrypt_optional_value(
                            encryption_key.as_ref(),
                            normalized.link_url.as_ref(),
                            "indexer candidate link URL",
                            true,
                        )?;
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "INSERT INTO indexer_search_candidates (
                                id, fingerprint, title, normalized_title, size_bytes, source_kind,
                                info_hash, created_at, updated_at, reusable_until, expires_at
                             ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                             ON CONFLICT(fingerprint) DO UPDATE SET
                                title = excluded.title,
                                normalized_title = excluded.normalized_title,
                                size_bytes = excluded.size_bytes,
                                source_kind = excluded.source_kind,
                                info_hash = excluded.info_hash,
                                updated_at = excluded.updated_at,
                                reusable_until = excluded.reusable_until,
                                expires_at = excluded.expires_at",
                            &[
                                SqlArg::Text(candidate_id.clone()),
                                SqlArg::Text(candidate_id.clone()),
                                SqlArg::Text(normalized.title.clone()),
                                SqlArg::Text(normalized.title.to_lowercase()),
                                SqlArg::OptI64(normalized.size_bytes),
                                SqlArg::OptText(normalized.source_kind.clone()),
                                SqlArg::OptText(normalized.info_hash.clone()),
                                SqlArg::Timestamp(created_at),
                                SqlArg::Timestamp(created_at),
                                SqlArg::Timestamp(reusable_until),
                                SqlArg::Timestamp(expires_at),
                            ],
                        )
                        .await?;

                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "INSERT INTO indexer_search_candidate_sources (
                                id, candidate_id, indexer_id, source_identity, provider_ref, source,
                                encrypted_download_url, encrypted_link_url, published_at, thumbs_up,
                                thumbs_down, grabs, grab_current, grab_max, response_tvdb_id,
                                response_tmdb_id, response_imdb_id, season, episode, absolute_episode,
                                release_group, provider_source, seeders, peers, download_volume_factor,
                                upload_volume_factor, protected, first_seen_at, last_seen_at,
                                reusable_until, expires_at
                             ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {},
                                       {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                             ON CONFLICT(candidate_id, indexer_id, source_identity) DO UPDATE SET
                                provider_ref = excluded.provider_ref,
                                source = excluded.source,
                                encrypted_download_url = excluded.encrypted_download_url,
                                encrypted_link_url = excluded.encrypted_link_url,
                                published_at = excluded.published_at,
                                thumbs_up = excluded.thumbs_up,
                                thumbs_down = excluded.thumbs_down,
                                grabs = excluded.grabs,
                                grab_current = excluded.grab_current,
                                grab_max = excluded.grab_max,
                                response_tvdb_id = excluded.response_tvdb_id,
                                response_tmdb_id = excluded.response_tmdb_id,
                                response_imdb_id = excluded.response_imdb_id,
                                season = excluded.season,
                                episode = excluded.episode,
                                absolute_episode = excluded.absolute_episode,
                                release_group = excluded.release_group,
                                provider_source = excluded.provider_source,
                                seeders = excluded.seeders,
                                peers = excluded.peers,
                                download_volume_factor = excluded.download_volume_factor,
                                upload_volume_factor = excluded.upload_volume_factor,
                                protected = excluded.protected,
                                last_seen_at = excluded.last_seen_at,
                                reusable_until = excluded.reusable_until,
                                expires_at = excluded.expires_at",
                            &[
                                SqlArg::Text(source_id.clone()), SqlArg::Text(candidate_id),
                                SqlArg::Text(indexer_id), SqlArg::Text(source_identity),
                                SqlArg::OptText(normalized.provider_ref.clone()), SqlArg::Text(normalized.source.clone()),
                                SqlArg::OptText(encrypted_download_url), SqlArg::OptText(encrypted_link_url),
                                SqlArg::OptText(normalized.published_at.clone()), SqlArg::OptI32(normalized.thumbs_up),
                                SqlArg::OptI32(normalized.thumbs_down), SqlArg::OptI64(normalized.grabs),
                                SqlArg::OptI64(normalized.grab_current), SqlArg::OptI64(normalized.grab_max),
                                SqlArg::OptText(normalized.response_tvdb_id.clone()), SqlArg::OptText(normalized.response_tmdb_id.clone()),
                                SqlArg::OptText(normalized.response_imdb_id.clone()), SqlArg::OptI64(normalized.season),
                                SqlArg::OptI64(normalized.episode), SqlArg::OptI64(normalized.absolute_episode),
                                SqlArg::OptText(normalized.release_group.clone()), SqlArg::OptText(normalized.provider_source.clone()),
                                SqlArg::OptI64(normalized.seeders), SqlArg::OptI64(normalized.peers),
                                SqlArg::OptF64(normalized.download_volume_factor), SqlArg::OptF64(normalized.upload_volume_factor),
                                SqlArg::OptBool(normalized.protected), SqlArg::Timestamp(created_at),
                                SqlArg::Timestamp(created_at), SqlArg::Timestamp(reusable_until), SqlArg::Timestamp(expires_at),
                            ],
                        ).await?;
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "INSERT INTO indexer_search_run_candidate_sources (run_id, source_id, search_session_id)
                             VALUES ({}, {}, {}) ON CONFLICT DO NOTHING",
                            &[SqlArg::Text(run_id), SqlArg::Text(source_id.clone()), SqlArg::Text(search_session_id)],
                        ).await?;
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "DELETE FROM indexer_search_candidate_source_values WHERE source_id = {}",
                            &[SqlArg::Text(source_id.clone())],
                        ).await?;

                        for (value_kind, values) in [
                            ("language", &normalized.languages),
                            ("subtitle", &normalized.subtitles),
                            ("response_category", &normalized.response_categories),
                            ("extra_category", &normalized.extra_categories),
                            ("series_name", &normalized.series_names),
                            ("tag", &normalized.tags),
                            ("provider_category", &normalized.provider_categories),
                        ] {
                            for (ordinal, value) in values.iter().enumerate() {
                                SqlRuntime::execute(
                                    SqlExec::Tx(tx),
                                    "INSERT INTO indexer_search_candidate_source_values (
                                        source_id, value_kind, ordinal, value
                                     ) VALUES ({}, {}, {}, {})",
                                    &[
                                        SqlArg::Text(source_id.clone()),
                                        SqlArg::Text(value_kind.to_string()),
                                        SqlArg::I64(ordinal as i64),
                                        SqlArg::Text(value.clone()),
                                    ],
                                )
                                .await?;
                            }
                        }

                    }
                    Ok(())
                })
            },
        )
        .await
    }

    async fn finalize_search_session(
        &self,
        search_session_id: &str,
        admissible_fingerprints: &[String],
    ) -> AppResult<()> {
        let search_session_id = search_session_id.to_string();
        let admissible_fingerprints = admissible_fingerprints.to_vec();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "finalize_indexer_search_session",
            move |tx| {
                let search_session_id = search_session_id.clone();
                let admissible_fingerprints = admissible_fingerprints.clone();
                Box::pin(async move {
                    let mut args = vec![SqlArg::Text(search_session_id.clone())];
                    let rejected_clause = if admissible_fingerprints.is_empty() {
                        String::new()
                    } else {
                        args.extend(admissible_fingerprints.into_iter().map(SqlArg::Text));
                        format!(
                            " AND c.fingerprint NOT IN ({})",
                            std::iter::repeat_n("{}", args.len() - 1)
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        &format!(
                            "DELETE FROM indexer_search_run_candidate_sources
                              WHERE search_session_id = {{}}
                                AND source_id IN (
                                    SELECT s.id
                                      FROM indexer_search_candidate_sources s
                                      JOIN indexer_search_candidates c ON c.id = s.candidate_id
                                     WHERE 1 = 1{rejected_clause}
                                )"
                        ),
                        &args,
                    )
                    .await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE indexer_search_runs
                            SET completion_state = CASE completion_state
                                WHEN 'received_complete' THEN 'complete'
                                WHEN 'received_partial' THEN 'partial'
                                ELSE completion_state
                            END
                          WHERE search_session_id = {}",
                        &[SqlArg::Text(search_session_id)],
                    )
                    .await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM indexer_search_candidate_sources
                          WHERE NOT EXISTS (
                              SELECT 1 FROM indexer_search_run_candidate_sources rc
                               WHERE rc.source_id = indexer_search_candidate_sources.id
                          )",
                        &[],
                    )
                    .await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM indexer_search_candidates
                          WHERE NOT EXISTS (
                              SELECT 1 FROM indexer_search_candidate_sources s
                               WHERE s.candidate_id = indexer_search_candidates.id
                          )",
                        &[],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn list_reusable_search_candidates(
        &self,
        indexer_id: &str,
        scope_key: &str,
        indexer_fingerprint: &str,
        now: DateTime<Utc>,
        limit: u32,
    ) -> AppResult<Vec<ReusableIndexerSearchCandidate>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT DISTINCT s.id, s.provider_ref, s.source, c.title,
                    s.encrypted_download_url, s.encrypted_link_url, c.size_bytes,
                    s.published_at, c.source_kind, s.thumbs_up, s.thumbs_down,
                    s.grabs, s.grab_current, s.grab_max, s.response_tvdb_id,
                    s.response_tmdb_id, s.response_imdb_id, s.season, s.episode,
                    s.absolute_episode, s.release_group, s.provider_source, c.info_hash,
                    s.seeders, s.peers, s.download_volume_factor,
                    s.upload_volume_factor, s.protected
             FROM indexer_search_candidate_sources s
             JOIN indexer_search_candidates c ON c.id = s.candidate_id
             JOIN indexer_search_run_candidate_sources rc ON rc.source_id = s.id
             JOIN indexer_search_runs r ON r.id = rc.run_id
             WHERE s.indexer_id = {}
               AND r.scope_key = {}
               AND r.indexer_fingerprint = {}
               AND s.reusable_until >= {}
               AND s.expires_at >= {}
               AND r.completion_state IN ('complete', 'partial')
             ORDER BY s.last_seen_at DESC
             LIMIT {}",
            &[
                SqlArg::Text(indexer_id.to_string()),
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(indexer_fingerprint.to_string()),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
                SqlArg::I64(i64::from(limit.max(1))),
            ],
        )
        .await?;

        let key = current_encryption_key(&self.encryption_key)?;
        let mut order = Vec::with_capacity(rows.len());
        let mut candidates = HashMap::with_capacity(rows.len());
        for row in &rows {
            let id = row.text("id")?;
            order.push(id.clone());
            candidates.insert(
                id,
                NormalizedIndexerSearchCandidate {
                    provider_ref: row.opt_text("provider_ref")?,
                    source: row.text("source")?,
                    title: row.text("title")?,
                    download_url: decrypt_optional_value(
                        key.as_ref(),
                        row.opt_text("encrypted_download_url")?,
                        "indexer candidate grab URL",
                        true,
                    )?,
                    download_url_credential_keys: Vec::new(),
                    link_url: decrypt_optional_value(
                        key.as_ref(),
                        row.opt_text("encrypted_link_url")?,
                        "indexer candidate link URL",
                        true,
                    )?,
                    link_url_credential_keys: Vec::new(),
                    size_bytes: row.opt_i64("size_bytes")?,
                    published_at: row.opt_text("published_at")?,
                    source_kind: row.opt_text("source_kind")?,
                    thumbs_up: row.opt_i32("thumbs_up")?,
                    thumbs_down: row.opt_i32("thumbs_down")?,
                    grabs: row.opt_i64("grabs")?,
                    grab_current: row.opt_i64("grab_current")?,
                    grab_max: row.opt_i64("grab_max")?,
                    languages: Vec::new(),
                    subtitles: Vec::new(),
                    response_tvdb_id: row.opt_text("response_tvdb_id")?,
                    response_tmdb_id: row.opt_text("response_tmdb_id")?,
                    response_imdb_id: row.opt_text("response_imdb_id")?,
                    response_categories: Vec::new(),
                    extra_categories: Vec::new(),
                    season: row.opt_i64("season")?,
                    episode: row.opt_i64("episode")?,
                    absolute_episode: row.opt_i64("absolute_episode")?,
                    series_names: Vec::new(),
                    release_group: row.opt_text("release_group")?,
                    provider_source: row.opt_text("provider_source")?,
                    info_hash: row.opt_text("info_hash")?,
                    seeders: row.opt_i64("seeders")?,
                    peers: row.opt_i64("peers")?,
                    download_volume_factor: row.opt_f64("download_volume_factor")?,
                    upload_volume_factor: row.opt_f64("upload_volume_factor")?,
                    protected: row.opt_bool("protected")?,
                    tags: Vec::new(),
                    provider_categories: Vec::new(),
                },
            );
        }

        if order.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = std::iter::repeat_n("{}", order.len())
            .collect::<Vec<_>>()
            .join(", ");
        let candidate_args = || order.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
        let values_sql = format!(
            "SELECT source_id AS candidate_id, value_kind, value
             FROM indexer_search_candidate_source_values
             WHERE source_id IN ({placeholders})
             ORDER BY source_id, value_kind, ordinal"
        );
        let value_rows =
            SqlRuntime::fetch_all(self.datastore.read_exec(), &values_sql, &candidate_args())
                .await?;
        for row in &value_rows {
            let candidate_id = row.text("candidate_id")?;
            let Some(candidate) = candidates.get_mut(&candidate_id) else {
                continue;
            };
            let value = row.text("value")?;
            match row.text("value_kind")?.as_str() {
                "language" => candidate.languages.push(value),
                "subtitle" => candidate.subtitles.push(value),
                "response_category" => candidate.response_categories.push(value),
                "extra_category" => candidate.extra_categories.push(value),
                "series_name" => candidate.series_names.push(value),
                "tag" => candidate.tags.push(value),
                "provider_category" => candidate.provider_categories.push(value),
                _ => {}
            }
        }

        Ok(order
            .into_iter()
            .filter_map(|id| candidates.remove(&id))
            .map(|normalized| ReusableIndexerSearchCandidate { normalized })
            .collect())
    }

    async fn list_search_run_candidates(
        &self,
        run_id: &str,
    ) -> AppResult<Vec<ReusableIndexerSearchCandidate>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT s.id, s.provider_ref, s.source, c.title,
                    s.encrypted_download_url, s.encrypted_link_url, c.size_bytes,
                    s.published_at, c.source_kind, s.thumbs_up, s.thumbs_down,
                    s.grabs, s.grab_current, s.grab_max, s.response_tvdb_id,
                    s.response_tmdb_id, s.response_imdb_id, s.season, s.episode,
                    s.absolute_episode, s.release_group, s.provider_source, c.info_hash,
                    s.seeders, s.peers, s.download_volume_factor,
                    s.upload_volume_factor, s.protected
             FROM indexer_search_run_candidate_sources rc
             JOIN indexer_search_candidate_sources s ON s.id = rc.source_id
             JOIN indexer_search_candidates c ON c.id = s.candidate_id
             WHERE rc.run_id = {}
             ORDER BY s.first_seen_at ASC, s.id ASC",
            &[SqlArg::Text(run_id.to_string())],
        )
        .await?;
        self.hydrate_candidates(rows).await
    }

    async fn list_reusable_search_strategies(
        &self,
        indexer_id: &str,
        scope_key: &str,
        indexer_fingerprint: &str,
        created_after: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<ReusableIndexerSearchStrategy>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT r.id, r.query_signature, r.branch, r.completion_state, r.retry_at,
                    EXISTS (
                        SELECT 1 FROM indexer_search_run_candidate_sources rc
                        JOIN indexer_search_candidate_sources s ON s.id = rc.source_id
                        WHERE rc.run_id = r.id
                          AND s.reusable_until >= {}
                          AND s.expires_at >= {}
                    ) AS has_reusable_candidates
             FROM indexer_search_runs r
             WHERE r.indexer_id = {}
               AND r.scope_key = {}
               AND r.indexer_fingerprint = {}
               AND r.created_at >= {}
             ORDER BY r.created_at DESC, r.id DESC",
            &[
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
                SqlArg::Text(indexer_id.to_string()),
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(indexer_fingerprint.to_string()),
                SqlArg::Timestamp(created_after),
            ],
        )
        .await?;

        let mut positions = std::collections::HashMap::new();
        let mut states = Vec::new();
        for row in rows {
            let query_signature = row.text("query_signature")?;
            let has_reusable_candidates = row.bool("has_reusable_candidates")?;
            if let Some(position) = positions.get(&query_signature).copied() {
                let state: &mut ReusableIndexerSearchStrategy = &mut states[position];
                if state.completion_state != "complete"
                    && state.candidate_run_id.is_none()
                    && has_reusable_candidates
                {
                    state.candidate_run_id = Some(row.text("id")?);
                }
                continue;
            }
            let run_id = row.text("id")?;
            positions.insert(query_signature.clone(), states.len());
            states.push(ReusableIndexerSearchStrategy {
                candidate_run_id: has_reusable_candidates.then(|| run_id.clone()),
                run_id,
                query_signature,
                branch: row.text("branch")?,
                completion_state: row.text("completion_state")?,
                retry_at: row.opt_timestamp("retry_at")?,
            });
        }
        Ok(states)
    }

    async fn cleanup_search_diagnostics(
        &self,
        candidate_cutoff: DateTime<Utc>,
        run_cutoff: DateTime<Utc>,
        limit: u32,
    ) -> AppResult<u32> {
        let limit = i64::from(limit.max(1));
        let deleted_candidates = SqlRuntime::execute_write(
            &self.datastore,
            "cleanup_indexer_search_candidates",
            "DELETE FROM indexer_search_candidate_sources WHERE id IN (
                SELECT id FROM indexer_search_candidate_sources
                WHERE expires_at < {} OR first_seen_at < {}
                ORDER BY first_seen_at ASC LIMIT {}
             )",
            vec![
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Timestamp(candidate_cutoff),
                SqlArg::I64(limit),
            ],
        )
        .await?;
        SqlRuntime::execute_write(
            &self.datastore,
            "cleanup_orphan_indexer_search_candidates",
            "DELETE FROM indexer_search_candidates
              WHERE NOT EXISTS (
                  SELECT 1 FROM indexer_search_candidate_sources s
                   WHERE s.candidate_id = indexer_search_candidates.id
              )",
            vec![],
        )
        .await?;
        let deleted_runs = SqlRuntime::execute_write(
            &self.datastore,
            "cleanup_indexer_search_runs",
            "DELETE FROM indexer_search_runs WHERE id IN (
                SELECT id FROM indexer_search_runs
                WHERE created_at < {}
                ORDER BY created_at ASC LIMIT {}
             )",
            vec![SqlArg::Timestamp(run_cutoff), SqlArg::I64(limit)],
        )
        .await?;
        Ok(deleted_candidates
            .saturating_add(deleted_runs)
            .min(u64::from(u32::MAX)) as u32)
    }

    async fn prune_indexer(&self, indexer_id: &str) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "prune_indexer_search_learning",
            "DELETE FROM indexer_search_learning WHERE indexer_id = {}",
            vec![SqlArg::Text(indexer_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn set_suppressed(
        &self,
        key: &IndexerSearchLearningKey,
        suppressed: bool,
    ) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "set_indexer_search_learning_suppressed",
            "UPDATE indexer_search_learning
             SET suppressed = {}, updated_at = {}
             WHERE indexer_id = {} AND title_id = {} AND facet = {} AND strategy_key = {}",
            vec![
                SqlArg::Bool(suppressed),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Text(key.indexer_id.clone()),
                SqlArg::Text(key.title_id.clone()),
                SqlArg::Text(key.facet.clone()),
                SqlArg::Text(key.strategy_key.clone()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn try_claim_suppressed_reprobe(
        &self,
        key: &IndexerSearchLearningKey,
        stale_before: DateTime<Utc>,
    ) -> AppResult<bool> {
        let now = Utc::now();
        let rows = match &self.datastore {
            StoreDatastore::Sqlite { .. } => {
                SqlRuntime::execute_write(
                    &self.datastore,
                    "claim_suppressed_indexer_search_reprobe",
                    "UPDATE indexer_search_learning
                     SET updated_at = {}
                     WHERE indexer_id = {}
                       AND title_id = {}
                       AND facet = {}
                       AND strategy_key = {}
                       AND suppressed = {}
                       AND (
                            updated_at IS NULL
                            OR strftime('%s', updated_at) IS NULL
                            OR updated_at < {}
                       )",
                    vec![
                        SqlArg::Text(sqlite_timestamp(now)),
                        SqlArg::Text(key.indexer_id.clone()),
                        SqlArg::Text(key.title_id.clone()),
                        SqlArg::Text(key.facet.clone()),
                        SqlArg::Text(key.strategy_key.clone()),
                        SqlArg::Bool(true),
                        SqlArg::Text(sqlite_timestamp(stale_before)),
                    ],
                )
                .await?
            }
            StoreDatastore::Postgres { .. } => {
                SqlRuntime::execute_write(
                    &self.datastore,
                    "claim_suppressed_indexer_search_reprobe",
                    "UPDATE indexer_search_learning
                     SET updated_at = {}
                     WHERE indexer_id = {}
                       AND title_id = {}
                       AND facet = {}
                       AND strategy_key = {}
                       AND suppressed = {}
                       AND (updated_at IS NULL OR updated_at < {})",
                    vec![
                        SqlArg::Timestamp(now),
                        SqlArg::Text(key.indexer_id.clone()),
                        SqlArg::Text(key.title_id.clone()),
                        SqlArg::Text(key.facet.clone()),
                        SqlArg::Text(key.strategy_key.clone()),
                        SqlArg::Bool(true),
                        SqlArg::Timestamp(stale_before),
                    ],
                )
                .await?
            }
        };

        Ok(rows > 0)
    }
}

fn row_to_learning_record(row: &SqlRow) -> AppResult<IndexerSearchLearningRecord> {
    Ok(IndexerSearchLearningRecord {
        key: IndexerSearchLearningKey {
            indexer_id: row.text("indexer_id")?,
            title_id: row.text("title_id")?,
            facet: row.text("facet")?,
            strategy_key: row.text("strategy_key")?,
        },
        attempts: i64_to_u32(row.i64("attempts")?, "attempts")?,
        empty_successes: i64_to_u32(row.i64("empty_successes")?, "empty_successes")?,
        usable_successes: i64_to_u32(row.i64("usable_successes")?, "usable_successes")?,
        last_attempt_at: row
            .opt_timestamp("last_attempt_at")?
            .map(|timestamp| timestamp.to_rfc3339()),
        last_usable_at: row
            .opt_timestamp("last_usable_at")?
            .map(|timestamp| timestamp.to_rfc3339()),
        suppressed: row.bool("suppressed")?,
        updated_at: row
            .opt_timestamp("updated_at")?
            .map(|timestamp| timestamp.to_rfc3339()),
    })
}

fn i64_to_u32(value: i64, column: &str) -> AppResult<u32> {
    u32::try_from(value).map_err(|_| {
        AppError::Repository(format!(
            "indexer_search_learning.{column} is outside u32 range"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use sqlx::sqlite::SqlitePoolOptions;

    async fn sqlite_store() -> (IndexerSearchLearningStore, sqlx::SqlitePool) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");

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
        .expect("learning table should be created");
        sqlx::query(
            "CREATE TABLE indexer_search_runs (
                id TEXT PRIMARY KEY,
                indexer_id TEXT NOT NULL,
                provider_type TEXT NOT NULL,
                search_session_id TEXT NOT NULL DEFAULT '',
                scope_key TEXT NOT NULL,
                query_signature TEXT NOT NULL,
                branch TEXT NOT NULL,
                page INTEGER,
                range_min_size INTEGER,
                range_max_size INTEGER,
                result_count INTEGER NOT NULL,
                completion_state TEXT NOT NULL,
                retry_at TEXT,
                error_summary TEXT,
                indexer_fingerprint TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("search run table should be created");
        sqlx::query(
            "CREATE TABLE indexer_search_candidates (
                id TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                normalized_title TEXT NOT NULL,
                size_bytes INTEGER,
                source_kind TEXT,
                info_hash TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                reusable_until TEXT NOT NULL,
                expires_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("search candidate table should be created");
        sqlx::query(
            "CREATE TABLE indexer_search_candidate_sources (
                id TEXT PRIMARY KEY,
                candidate_id TEXT NOT NULL,
                indexer_id TEXT NOT NULL,
                source_identity TEXT NOT NULL,
                provider_ref TEXT,
                source TEXT NOT NULL,
                encrypted_download_url TEXT,
                encrypted_link_url TEXT,
                published_at TEXT,
                thumbs_up INTEGER,
                thumbs_down INTEGER,
                grabs INTEGER,
                grab_current INTEGER,
                grab_max INTEGER,
                response_tvdb_id TEXT,
                response_tmdb_id TEXT,
                response_imdb_id TEXT,
                season INTEGER,
                episode INTEGER,
                absolute_episode INTEGER,
                release_group TEXT,
                provider_source TEXT,
                seeders INTEGER,
                peers INTEGER,
                download_volume_factor REAL,
                upload_volume_factor REAL,
                protected INTEGER,
                first_seen_at TEXT NOT NULL,
                last_seen_at TEXT NOT NULL,
                reusable_until TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                UNIQUE(candidate_id, indexer_id, source_identity)
            )",
        )
        .execute(&pool)
        .await
        .expect("search candidate source table should be created");
        sqlx::query(
            "CREATE TABLE indexer_search_run_candidate_sources (
                run_id TEXT NOT NULL,
                source_id TEXT NOT NULL,
                search_session_id TEXT NOT NULL,
                PRIMARY KEY (run_id, source_id)
            )",
        )
        .execute(&pool)
        .await
        .expect("search run candidate source table should be created");
        sqlx::query(
            "CREATE TABLE indexer_search_candidate_source_values (
                source_id TEXT NOT NULL,
                value_kind TEXT NOT NULL,
                ordinal INTEGER NOT NULL,
                value TEXT NOT NULL,
                PRIMARY KEY (source_id, value_kind, ordinal)
            )",
        )
        .execute(&pool)
        .await
        .expect("search candidate value table should be created");

        let store = IndexerSearchLearningStore::new(
            StoreDatastore::Sqlite {
                pool: pool.clone(),
                writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            },
            std::sync::Arc::new(std::sync::RwLock::new(Some(EncryptionKey::from_bytes(
                [7; 32],
            )))),
        );

        (store, pool)
    }

    #[tokio::test]
    async fn sqlite_store_records_lists_and_updates_learning_records() {
        let (store, pool) = sqlite_store().await;
        let key = IndexerSearchLearningKey {
            indexer_id: "idx-1".into(),
            title_id: "title-1".into(),
            facet: "anime".into(),
            strategy_key: "ids_abs".into(),
        };

        let empty_record = store
            .record_outcome(&key, 0)
            .await
            .expect("empty outcome should persist");
        assert_eq!(empty_record.attempts, 1);
        assert_eq!(empty_record.empty_successes, 1);
        assert_eq!(empty_record.usable_successes, 0);
        assert!(!empty_record.suppressed);

        store
            .set_suppressed(&key, true)
            .await
            .expect("suppression flag should update");
        let suppressed_records = store
            .list_for_title("idx-1", "title-1", "anime")
            .await
            .expect("records should list");
        assert_eq!(suppressed_records.len(), 1);
        assert!(suppressed_records[0].suppressed);
        assert!(
            !store
                .try_claim_suppressed_reprobe(&key, Utc::now() - chrono::Duration::days(7))
                .await
                .expect("recent suppression should not be claimed")
        );

        sqlx::query(
            "UPDATE indexer_search_learning
             SET updated_at = ?
             WHERE indexer_id = ? AND title_id = ? AND facet = ? AND strategy_key = ?",
        )
        .bind(sqlite_timestamp(Utc::now() - chrono::Duration::days(8)))
        .bind(&key.indexer_id)
        .bind(&key.title_id)
        .bind(&key.facet)
        .bind(&key.strategy_key)
        .execute(&pool)
        .await
        .expect("learning row should be aged");
        assert!(
            store
                .try_claim_suppressed_reprobe(&key, Utc::now() - chrono::Duration::days(7))
                .await
                .expect("stale suppression should be claimed")
        );
        assert!(
            !store
                .try_claim_suppressed_reprobe(&key, Utc::now() - chrono::Duration::days(7))
                .await
                .expect("claimed suppression should not be claimed twice")
        );

        sqlx::query(
            "UPDATE indexer_search_learning
             SET updated_at = ?
             WHERE indexer_id = ? AND title_id = ? AND facet = ? AND strategy_key = ?",
        )
        .bind("not-a-timestamp")
        .bind(&key.indexer_id)
        .bind(&key.title_id)
        .bind(&key.facet)
        .bind(&key.strategy_key)
        .execute(&pool)
        .await
        .expect("learning row should accept legacy malformed timestamp");
        assert!(
            store
                .try_claim_suppressed_reprobe(&key, Utc::now() - chrono::Duration::days(7))
                .await
                .expect("malformed timestamp should be claimed for self-healing")
        );

        let usable_record = store
            .record_outcome(&key, 2)
            .await
            .expect("usable outcome should persist and reload");
        assert_eq!(usable_record.attempts, 2);
        assert_eq!(usable_record.empty_successes, 1);
        assert_eq!(usable_record.usable_successes, 1);
        assert!(usable_record.last_attempt_at.is_some());
        assert!(usable_record.last_usable_at.is_some());
        assert!(!usable_record.suppressed);
    }

    #[tokio::test]
    async fn sqlite_store_prunes_learning_for_only_the_changed_indexer() {
        let (store, _) = sqlite_store().await;
        for indexer_id in ["idx-pruned", "idx-kept"] {
            store
                .record_outcome(
                    &IndexerSearchLearningKey {
                        indexer_id: indexer_id.into(),
                        title_id: "title-1".into(),
                        facet: "series".into(),
                        strategy_key: "freetext".into(),
                    },
                    0,
                )
                .await
                .expect("learning outcome should persist");
        }

        store
            .prune_indexer("idx-pruned")
            .await
            .expect("indexer learning should prune");

        assert!(
            store
                .list_for_title("idx-pruned", "title-1", "series")
                .await
                .expect("pruned records should list")
                .is_empty()
        );
        assert_eq!(
            store
                .list_for_title("idx-kept", "title-1", "series")
                .await
                .expect("unrelated records should list")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn sqlite_store_reloads_only_fresh_matching_search_candidates() {
        let (store, pool) = sqlite_store().await;
        let now = Utc::now();
        let run = IndexerSearchRunWrite {
            id: "run-1".into(),
            indexer_id: "idx-1".into(),
            provider_type: "newznab".into(),
            search_session_id: "session-1".into(),
            scope_key: "title-1:anime:season:1:-:-".into(),
            query_signature: "query-1".into(),
            branch: "tvdb-season".into(),
            page: Some(0),
            range_min_size: None,
            range_max_size: None,
            result_count: 2,
            completion_state: "received_complete".into(),
            retry_at: None,
            error_summary: None,
            indexer_fingerprint: "fingerprint-1".into(),
            created_at: now,
        };
        let candidate = IndexerSearchCandidateWrite {
            id: "candidate-1".into(),
            run_id: run.id.clone(),
            search_session_id: run.search_session_id.clone(),
            indexer_id: run.indexer_id.clone(),
            scope_key: run.scope_key.clone(),
            query_signature: run.query_signature.clone(),
            session_identity_hash: "candidate-hash-1".into(),
            normalized: NormalizedIndexerSearchCandidate {
                provider_ref: Some("release-1".into()),
                source: "newznab".into(),
                title: "Example.S01E01.1080p".into(),
                download_url: Some("https://example.invalid/get?id=release-1".into()),
                download_url_credential_keys: vec!["apikey".into()],
                link_url: None,
                link_url_credential_keys: Vec::new(),
                size_bytes: Some(123),
                published_at: None,
                source_kind: Some("nzb_url".into()),
                thumbs_up: None,
                thumbs_down: None,
                grabs: Some(7),
                grab_current: Some(10),
                grab_max: Some(100),
                languages: vec!["en".into(), "ja".into()],
                subtitles: vec!["en".into()],
                response_tvdb_id: Some("1234".into()),
                response_tmdb_id: None,
                response_imdb_id: None,
                response_categories: vec!["5070".into()],
                extra_categories: vec!["anime".into()],
                season: Some(1),
                episode: Some(1),
                absolute_episode: None,
                series_names: vec!["Example".into()],
                release_group: Some("Group".into()),
                provider_source: Some("release".into()),
                info_hash: None,
                seeders: None,
                peers: None,
                download_volume_factor: None,
                upload_volume_factor: None,
                protected: None,
                tags: vec!["anime".into()],
                provider_categories: vec!["Anime".into()],
            },
            created_at: now,
            reusable_until: now + chrono::Duration::hours(24),
            expires_at: now + chrono::Duration::days(7),
        };
        let mut rejected_candidate = candidate.clone();
        rejected_candidate.id = "candidate-rejected".into();
        rejected_candidate.session_identity_hash = "candidate-hash-rejected".into();
        rejected_candidate.normalized.provider_ref = Some("release-rejected".into());
        rejected_candidate.normalized.download_url =
            Some("https://example.invalid/get?id=release-rejected&apikey=secret".into());
        store
            .record_search_diagnostics(&run, &[candidate.clone(), rejected_candidate])
            .await
            .expect("diagnostics should persist");

        assert_eq!(
            store
                .list_search_run_candidates(&run.id)
                .await
                .expect("the persisted page should rehydrate")
                .len(),
            2
        );

        let mut duplicate_run = run.clone();
        duplicate_run.id = "run-2".into();
        let mut duplicate_candidate = candidate;
        duplicate_candidate.id = "candidate-2".into();
        duplicate_candidate.run_id = duplicate_run.id.clone();
        store
            .record_search_diagnostics(&duplicate_run, &[duplicate_candidate])
            .await
            .expect("duplicate page diagnostics should persist its run");
        store
            .finalize_search_session(&run.search_session_id, &["candidate-hash-1".into()])
            .await
            .expect("evaluated candidates should finalize");
        assert_eq!(
            store
                .list_search_run_candidates(&run.id)
                .await
                .expect("rejected candidates should be removed")
                .len(),
            1
        );
        assert_eq!(
            store
                .list_search_run_candidates(&duplicate_run.id)
                .await
                .expect("duplicate page candidates should rehydrate")
                .len(),
            1,
            "each run observes the shared canonical source"
        );
        let encrypted_url: String = sqlx::query_scalar(
            "SELECT encrypted_download_url FROM indexer_search_candidate_sources LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("encrypted URL should be stored");
        assert!(!encrypted_url.contains("example.invalid"));
        assert!(!encrypted_url.contains("secret"));

        let completed_state = store
            .list_reusable_search_strategies(
                &run.indexer_id,
                &run.scope_key,
                &run.indexer_fingerprint,
                now - chrono::Duration::seconds(1),
                now,
            )
            .await
            .expect("strategy state should reload");
        assert_eq!(completed_state.len(), 1);
        assert_eq!(completed_state[0].run_id, duplicate_run.id);
        assert_eq!(completed_state[0].completion_state, "complete");
        assert_eq!(
            completed_state[0].candidate_run_id.as_deref(),
            Some(duplicate_run.id.as_str())
        );

        let mut partial_run = run.clone();
        partial_run.id = "run-3".into();
        partial_run.completion_state = "partial".into();
        partial_run.result_count = 0;
        partial_run.created_at = now + chrono::Duration::seconds(1);
        store
            .record_search_diagnostics(&partial_run, &[])
            .await
            .expect("partial strategy diagnostics should persist");
        let partial_state = store
            .list_reusable_search_strategies(
                &run.indexer_id,
                &run.scope_key,
                &run.indexer_fingerprint,
                now - chrono::Duration::seconds(1),
                now,
            )
            .await
            .expect("partial strategy state should reload");
        assert_eq!(partial_state.len(), 1);
        assert_eq!(partial_state[0].run_id, partial_run.id);
        assert_eq!(partial_state[0].completion_state, "partial");
        assert_eq!(
            partial_state[0].candidate_run_id.as_deref(),
            Some(duplicate_run.id.as_str())
        );

        let reusable = store
            .list_reusable_search_candidates(
                "idx-1",
                "title-1:anime:season:1:-:-",
                "fingerprint-1",
                now,
                10,
            )
            .await
            .expect("matching candidates should reload");
        assert_eq!(reusable.len(), 1);
        assert_eq!(
            reusable[0].normalized.provider_ref.as_deref(),
            Some("release-1")
        );
        assert_eq!(reusable[0].normalized.languages, ["en", "ja"]);
        assert!(
            reusable[0]
                .normalized
                .download_url_credential_keys
                .is_empty()
        );
        assert!(
            store
                .list_reusable_search_candidates(
                    "idx-1",
                    "title-1:anime:season:1:-:-",
                    "different-fingerprint",
                    now,
                    10,
                )
                .await
                .expect("a fingerprint mismatch should be a cache miss")
                .is_empty()
        );

        let deleted = store
            .cleanup_search_diagnostics(
                now + chrono::Duration::seconds(1),
                now + chrono::Duration::seconds(1),
                10,
            )
            .await
            .expect("expired diagnostics should clean up");
        assert_eq!(deleted, 3, "the candidate and run deletions are reported");
    }

    #[tokio::test]
    async fn postgres_cleanup_counts_and_indexer_pruning_from_env() -> AppResult<()> {
        let Some(database_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL search-learning cleanup test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .map_err(repo_err)?;
        sqlx::query(
            "CREATE TEMP TABLE indexer_search_learning (
                indexer_id TEXT NOT NULL
            ) ON COMMIT PRESERVE ROWS",
        )
        .execute(&pool)
        .await
        .map_err(repo_err)?;
        sqlx::query(
            "CREATE TEMP TABLE indexer_search_candidates (
                id TEXT PRIMARY KEY,
                created_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL
            ) ON COMMIT PRESERVE ROWS",
        )
        .execute(&pool)
        .await
        .map_err(repo_err)?;
        sqlx::query(
            "CREATE TEMP TABLE indexer_search_runs (
                id TEXT PRIMARY KEY,
                created_at TIMESTAMPTZ NOT NULL
            ) ON COMMIT PRESERVE ROWS",
        )
        .execute(&pool)
        .await
        .map_err(repo_err)?;
        sqlx::query(
            "INSERT INTO indexer_search_learning (indexer_id)
             VALUES ('idx-pruned'), ('idx-pruned'), ('idx-kept')",
        )
        .execute(&pool)
        .await
        .map_err(repo_err)?;
        let old_candidate = Utc::now() - chrono::Duration::days(8);
        let old_run = Utc::now() - chrono::Duration::days(91);
        for ordinal in 0..2 {
            sqlx::query(
                "INSERT INTO indexer_search_candidates (id, created_at, expires_at)
                 VALUES ($1, $2, $3)",
            )
            .bind(format!("candidate-{ordinal}"))
            .bind(old_candidate)
            .bind(old_candidate)
            .execute(&pool)
            .await
            .map_err(repo_err)?;
            sqlx::query(
                "INSERT INTO indexer_search_runs (id, created_at)
                 VALUES ($1, $2)",
            )
            .bind(format!("run-{ordinal}"))
            .bind(old_run)
            .execute(&pool)
            .await
            .map_err(repo_err)?;
        }
        let store = IndexerSearchLearningStore::new(
            StoreDatastore::Postgres { pool: pool.clone() },
            std::sync::Arc::new(std::sync::RwLock::new(None)),
        );

        store.prune_indexer("idx-pruned").await?;
        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM indexer_search_learning")
            .fetch_one(&pool)
            .await
            .map_err(repo_err)?;
        assert_eq!(kept, 1);
        assert_eq!(
            store
                .cleanup_search_diagnostics(Utc::now(), Utc::now(), 1)
                .await?,
            2
        );
        assert_eq!(
            store
                .cleanup_search_diagnostics(Utc::now(), Utc::now(), 1)
                .await?,
            2
        );
        assert_eq!(
            store
                .cleanup_search_diagnostics(Utc::now(), Utc::now(), 1)
                .await?,
            0
        );

        Ok(())
    }
}

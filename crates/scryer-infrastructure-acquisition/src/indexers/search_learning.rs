use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use scryer_application::{
    AppError, AppResult, BackgroundIndexerSearchStrategyState, HashDomain,
    IndexerFacetLearningPolicy, IndexerFacetLearningState, IndexerFacetSearchOutcome,
    IndexerFacetTransition, IndexerSearchCandidateWrite, IndexerSearchLearningBatch,
    IndexerSearchLearningKey, IndexerSearchLearningRecord, IndexerSearchLearningRepository,
    IndexerSearchListing, IndexerSearchListingClaim, IndexerSearchListingReader,
    IndexerSearchListingScope, IndexerSearchListingStrategy, IndexerSearchRunWrite,
    NormalizedIndexerSearchCandidate, ReusableIndexerSearchCandidate,
    ReusableIndexerSearchCandidatePage, ReusableIndexerSearchStrategy, SEARCH_STRATEGY_CONTAINED,
    SEARCH_STRATEGY_CONVERGED, blake3_identity_hex,
};
use scryer_infrastructure_crypto::{
    EncryptionKey,
    config::{current_encryption_key, decrypt_optional_value, encrypt_optional_value},
};

use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};

/// Strategy-key prefix of title-text learning rows. One row exists per
/// distinct query text (`v3:freetext:<query digest>`), so a localized name and
/// the primary name never share evidence.
pub(crate) const FREETEXT_LEARNING_KEY_PREFIX: &str = "v3:freetext:";

/// Most title-text learning rows kept per (indexer, title, facet).
///
/// Text rows are keyed per query, so without a bound they grow with every
/// episode number, localized name and custom alias an indexer is ever asked
/// for. The rows are read back in two ways only, both through
/// `list_for_title` on every automatic search of that indexer and title: a
/// text row with a usable success is the "working alternative" that lets an
/// empty ID strategy be suppressed, and any usable row marks the indexer as
/// historically useful to the scheduler. Neither needs history beyond the
/// queries the title is currently searched with, so the cap only has to hold
/// one complete search pass: the primary name, up to 16 search languages
/// (the library and title validation limit) and the anime numbering and
/// alias forms. Twice the language limit covers that pass with headroom, so a
/// pass never evicts its own rows, while keeping the per-search read at a few
/// dozen rows instead of one per episode ever searched.
///
/// Eviction keeps the row just written, then rows with a usable success, then
/// the most recently attempted, so the evidence the readers depend on is the
/// last to go.
pub(crate) const FREETEXT_LEARNING_ROWS_PER_TITLE: i64 = 32;

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
        let mut reuse_deadlines = HashMap::with_capacity(rows.len());
        for row in &rows {
            let id = row.text("id")?;
            reuse_deadlines.insert(
                id.clone(),
                row.timestamp("reusable_until")?
                    .min(row.timestamp("expires_at")?),
            );
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
                    release_listing_json: None,
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
                "release_listing" => candidate.release_listing_json = Some(value),
                _ => {}
            }
        }

        Ok(order
            .into_iter()
            .filter_map(|id| {
                candidates
                    .remove(&id)
                    .map(|normalized| ReusableIndexerSearchCandidate {
                        normalized,
                        reusable_until: reuse_deadlines[&id],
                    })
            })
            .collect())
    }
}

/// The arguments naming one title's listings on one indexer, in column order
/// `indexer_id, indexer_fingerprint, title_id, facet`.
fn listing_scope_args(scope: &IndexerSearchListingScope) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(scope.indexer_id.clone()),
        SqlArg::Text(scope.indexer_fingerprint.clone()),
        SqlArg::Text(scope.title_id.clone()),
        SqlArg::Text(scope.facet.clone()),
    ]
}

/// Records a listing reader from arguments in column order: the listing
/// scope, then `scope_key, query_signature, strategy_family,
/// first_page_fingerprint, observed_at`.
async fn upsert_listing_reader(tx: &mut SqlTx<'_>, args: &[SqlArg]) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO indexer_search_listing_readers (
            indexer_id, indexer_fingerprint, title_id, facet, scope_key, query_signature,
            strategy_family, first_page_fingerprint, observed_at
         ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(indexer_id, indexer_fingerprint, title_id, facet, scope_key, query_signature)
         DO UPDATE SET
            strategy_family = excluded.strategy_family,
            first_page_fingerprint = excluded.first_page_fingerprint,
            observed_at = excluded.observed_at",
        args,
    )
    .await?;
    Ok(())
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

    async fn list_for_title_by_indexer(
        &self,
        indexer_ids: &[String],
        title_id: &str,
        facet: &str,
    ) -> AppResult<HashMap<String, Vec<IndexerSearchLearningRecord>>> {
        let mut records = indexer_ids
            .iter()
            .map(|indexer_id| (indexer_id.clone(), Vec::new()))
            .collect::<HashMap<_, _>>();
        if indexer_ids.is_empty() {
            return Ok(records);
        }
        let placeholders = std::iter::repeat_n("{}", indexer_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut args = vec![
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(facet.to_string()),
        ];
        args.extend(indexer_ids.iter().cloned().map(SqlArg::Text));
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT indexer_id, title_id, facet, strategy_key, attempts, empty_successes,
                        usable_successes, last_attempt_at, last_usable_at, suppressed, updated_at
                   FROM indexer_search_learning
                  WHERE title_id = {{}} AND facet = {{}} AND indexer_id IN ({placeholders})"
            ),
            &args,
        )
        .await?;
        for row in &rows {
            let record = row_to_learning_record(row)?;
            records
                .entry(record.key.indexer_id.clone())
                .or_default()
                .push(record);
        }
        Ok(records)
    }

    async fn record_learning_batch(
        &self,
        batch: &IndexerSearchLearningBatch,
    ) -> AppResult<Vec<IndexerFacetTransition>> {
        if batch.is_empty() {
            return Ok(Vec::new());
        }
        let batch = batch.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_indexer_search_learning_batch",
            move |tx| {
                let batch = batch.clone();
                Box::pin(async move {
                    for (strategy_key, usable_hits) in &batch.strategy_outcomes {
                        let key = IndexerSearchLearningKey {
                            indexer_id: batch.indexer_id.clone(),
                            title_id: batch.title_id.clone(),
                            facet: batch.facet.clone(),
                            strategy_key: strategy_key.clone(),
                        };
                        record_learning_outcome_in_tx(tx, &key, *usable_hits).await?;
                    }
                    for strategy_key in &batch.suppress {
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            SET_LEARNING_SUPPRESSED_SQL,
                            &[
                                SqlArg::Bool(true),
                                SqlArg::Timestamp(Utc::now()),
                                SqlArg::Text(batch.indexer_id.clone()),
                                SqlArg::Text(batch.title_id.clone()),
                                SqlArg::Text(batch.facet.clone()),
                                SqlArg::Text(strategy_key.clone()),
                            ],
                        )
                        .await?;
                    }
                    apply_facet_outcomes_in_tx(tx, &batch).await
                })
            },
        )
        .await
    }

    async fn record_outcome(
        &self,
        key: &IndexerSearchLearningKey,
        usable_hits: u32,
    ) -> AppResult<IndexerSearchLearningRecord> {
        let outcome_key = key.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_indexer_search_learning_outcome",
            move |tx| {
                let key = outcome_key.clone();
                Box::pin(async move { record_learning_outcome_in_tx(tx, &key, usable_hits).await })
            },
        )
        .await?;

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
                            created_at, strategy_state, page_cursor, cursor_chain_id
                         ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
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
                            SqlArg::OptText(run.strategy_state.clone()),
                            SqlArg::OptText(run.page_cursor.clone()),
                            SqlArg::OptText(run.cursor_chain_id.clone()),
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
                        if let Some(listing) = normalized.release_listing_json.as_ref() {
                            SqlRuntime::execute(
                                SqlExec::Tx(tx),
                                "INSERT INTO indexer_search_candidate_source_values (
                                    source_id, value_kind, ordinal, value
                                 ) VALUES ({}, {}, {}, {})",
                                &[
                                    SqlArg::Text(source_id.clone()),
                                    SqlArg::Text("release_listing".to_string()),
                                    SqlArg::I64(0),
                                    SqlArg::Text(listing.clone()),
                                ],
                            )
                            .await?;
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
                    // A run that reads a shared listing keeps every result:
                    // other scopes of the title are scored from it, and what
                    // this scope rejected may be theirs.
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
                                )
                                AND NOT EXISTS (
                                    SELECT 1 FROM indexer_search_listing_runs lr
                                     WHERE lr.run_id = indexer_search_run_candidate_sources.run_id
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
                    s.upload_volume_factor, s.protected, s.reusable_until, s.expires_at
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
        let mut reuse_deadlines = HashMap::with_capacity(rows.len());
        for row in &rows {
            let id = row.text("id")?;
            reuse_deadlines.insert(
                id.clone(),
                row.timestamp("reusable_until")?
                    .min(row.timestamp("expires_at")?),
            );
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
                    release_listing_json: None,
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
                "release_listing" => candidate.release_listing_json = Some(value),
                _ => {}
            }
        }

        Ok(order
            .into_iter()
            .filter_map(|id| {
                candidates
                    .remove(&id)
                    .map(|normalized| ReusableIndexerSearchCandidate {
                        normalized,
                        reusable_until: reuse_deadlines[&id],
                    })
            })
            .collect())
    }

    async fn list_search_run_candidates(
        &self,
        run_ids: &[String],
        after_source_id: Option<&str>,
        limit: u32,
    ) -> AppResult<ReusableIndexerSearchCandidatePage> {
        if limit == 0 || run_ids.is_empty() {
            return Ok(ReusableIndexerSearchCandidatePage::default());
        }
        let placeholders = std::iter::repeat_n("{}", run_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut args = run_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        args.push(SqlArg::Text(
            after_source_id.unwrap_or_default().to_string(),
        ));
        args.push(SqlArg::I64(i64::from(limit)));
        let query = "SELECT s.id, s.provider_ref, s.source, c.title,
                    s.encrypted_download_url, s.encrypted_link_url, c.size_bytes,
                    s.published_at, c.source_kind, s.thumbs_up, s.thumbs_down,
                    s.grabs, s.grab_current, s.grab_max, s.response_tvdb_id,
                    s.response_tmdb_id, s.response_imdb_id, s.season, s.episode,
                    s.absolute_episode, s.release_group, s.provider_source, c.info_hash,
                    s.seeders, s.peers, s.download_volume_factor,
                    s.upload_volume_factor, s.protected, s.reusable_until, s.expires_at
             FROM indexer_search_candidate_sources s
             JOIN indexer_search_candidates c ON c.id = s.candidate_id
             WHERE s.id IN (
                 SELECT rc.source_id FROM indexer_search_run_candidate_sources rc
                 WHERE rc.run_id IN (__RUN_IDS__)
             ) AND s.id > {}
             ORDER BY s.id ASC LIMIT {}"
            .replace("__RUN_IDS__", &placeholders);
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &query, &args).await?;
        let next_cursor = if rows.len() == limit as usize {
            rows.last().map(|row| row.text("id")).transpose()?
        } else {
            None
        };
        Ok(ReusableIndexerSearchCandidatePage {
            candidates: self.hydrate_candidates(rows).await?,
            next_cursor,
        })
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

    async fn list_background_strategy_states(
        &self,
        indexer_id: &str,
        scope_key: &str,
        indexer_fingerprint: &str,
        created_after: DateTime<Utc>,
    ) -> AppResult<Vec<BackgroundIndexerSearchStrategyState>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT r.id, r.query_signature, r.branch, r.strategy_state, r.retry_at,
                    r.created_at, r.page_cursor, r.cursor_chain_id
             FROM indexer_search_runs r
             WHERE r.indexer_id = {}
               AND r.scope_key = {}
               AND r.indexer_fingerprint = {}
               AND r.strategy_state IS NOT NULL
               AND r.created_at >= {}
             ORDER BY r.created_at DESC, r.id DESC",
            &[
                SqlArg::Text(indexer_id.to_string()),
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(indexer_fingerprint.to_string()),
                SqlArg::Timestamp(created_after),
            ],
        )
        .await?;
        let mut positions = std::collections::HashMap::new();
        let mut states: Vec<BackgroundIndexerSearchStrategyState> = Vec::new();
        for row in rows {
            let query_signature = row.text("query_signature")?;
            let cursor_chain_id = row.opt_text("cursor_chain_id")?;
            if let Some(position) = positions.get(&query_signature).copied() {
                // Older runs only add the pages of the latest run's chain.
                let state: &mut BackgroundIndexerSearchStrategyState = &mut states[position];
                if cursor_chain_id.is_some() && state.cursor_chain_id == cursor_chain_id {
                    state.chain_run_ids.push(row.text("id")?);
                }
                continue;
            }
            positions.insert(query_signature.clone(), states.len());
            states.push(BackgroundIndexerSearchStrategyState {
                query_signature,
                strategy_state: row.text("strategy_state")?,
                retry_at: row.opt_timestamp("retry_at")?,
                created_at: row.timestamp("created_at")?,
                page_cursor: row.opt_text("page_cursor")?,
                branch: row.text("branch")?,
                chain_run_ids: if cursor_chain_id.is_some() {
                    vec![row.text("id")?]
                } else {
                    Vec::new()
                },
                cursor_chain_id,
            });
        }
        Ok(states)
    }

    async fn link_search_session_coverage_scope(
        &self,
        search_session_id: &str,
        coverage_scope_key: &str,
    ) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "link_indexer_search_session_coverage_scope",
            "UPDATE indexer_search_runs
                SET coverage_scope_key = {}
              WHERE search_session_id = {}
                AND strategy_state IS NOT NULL",
            vec![
                SqlArg::Text(coverage_scope_key.to_string()),
                SqlArg::Text(search_session_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn contained_search_hold(
        &self,
        coverage_scope_key: &str,
        indexer_id: &str,
        indexer_fingerprint: &str,
        created_after: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<Option<DateTime<Utc>>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT r.search_session_id, r.strategy_state, r.retry_at
             FROM indexer_search_runs r
             WHERE r.coverage_scope_key = {}
               AND r.indexer_id = {}
               AND r.indexer_fingerprint = {}
               AND r.strategy_state IS NOT NULL
               AND r.created_at >= {}
             ORDER BY r.created_at DESC, r.id DESC",
            &[
                SqlArg::Text(coverage_scope_key.to_string()),
                SqlArg::Text(indexer_id.to_string()),
                SqlArg::Text(indexer_fingerprint.to_string()),
                SqlArg::Timestamp(created_after),
            ],
        )
        .await?;
        let mut latest_session: Option<String> = None;
        let mut hold_until: Option<DateTime<Utc>> = None;
        for row in rows {
            let session = row.text("search_session_id")?;
            match latest_session.as_deref() {
                None => latest_session = Some(session),
                Some(latest) if latest == session => {}
                // Rows are newest first, so the latest session's rows are all
                // read once another session appears.
                Some(_) => break,
            }
            let state = row.text("strategy_state")?;
            if state == SEARCH_STRATEGY_CONVERGED {
                continue;
            }
            if state != SEARCH_STRATEGY_CONTAINED {
                return Ok(None);
            }
            match row.opt_timestamp("retry_at")? {
                Some(retry_at) if retry_at > now => {
                    hold_until = Some(hold_until.map_or(retry_at, |held| held.min(retry_at)));
                }
                _ => return Ok(None),
            }
        }
        Ok(hold_until)
    }

    async fn list_search_listings(
        &self,
        scope: &IndexerSearchListingScope,
        fresh_after: DateTime<Utc>,
    ) -> AppResult<Vec<IndexerSearchListing>> {
        let args = || {
            let mut args = listing_scope_args(scope);
            args.push(SqlArg::Timestamp(fresh_after));
            args
        };
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT first_page_fingerprint, owner_scope_key, owner_query_signature,
                    owner_strategy_family, page_cursor, finished, chain_started_at, advanced_at
               FROM indexer_search_listings
              WHERE indexer_id = {} AND indexer_fingerprint = {} AND title_id = {} AND facet = {}
                AND advanced_at >= {}
              ORDER BY chain_started_at ASC, first_page_fingerprint ASC",
            &args(),
        )
        .await?;
        let mut listings = Vec::with_capacity(rows.len());
        for row in &rows {
            listings.push(IndexerSearchListing {
                first_page_fingerprint: row.text("first_page_fingerprint")?,
                owner: IndexerSearchListingStrategy {
                    scope_key: row.text("owner_scope_key")?,
                    query_signature: row.text("owner_query_signature")?,
                },
                owner_strategy_family: row.text("owner_strategy_family")?,
                page_cursor: row.opt_text("page_cursor")?,
                finished: row.bool("finished")?,
                chain_started_at: row.timestamp("chain_started_at")?,
                advanced_at: row.timestamp("advanced_at")?,
                readers: Vec::new(),
                run_ids: Vec::new(),
            });
        }
        if listings.is_empty() {
            return Ok(listings);
        }
        let readers = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT scope_key, query_signature, strategy_family, first_page_fingerprint, observed_at
               FROM indexer_search_listing_readers
              WHERE indexer_id = {} AND indexer_fingerprint = {} AND title_id = {} AND facet = {}
                AND observed_at >= {}
              ORDER BY observed_at ASC, scope_key ASC, query_signature ASC",
            &args(),
        )
        .await?;
        for row in &readers {
            let fingerprint = row.text("first_page_fingerprint")?;
            let Some(listing) = listings
                .iter_mut()
                .find(|listing| listing.first_page_fingerprint == fingerprint)
            else {
                continue;
            };
            listing.readers.push(IndexerSearchListingReader {
                strategy: IndexerSearchListingStrategy {
                    scope_key: row.text("scope_key")?,
                    query_signature: row.text("query_signature")?,
                },
                strategy_family: row.text("strategy_family")?,
                observed_at: row.timestamp("observed_at")?,
            });
        }
        let runs = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT lr.run_id, lr.first_page_fingerprint
               FROM indexer_search_listing_runs lr
               JOIN indexer_search_listings l
                 ON l.indexer_id = lr.indexer_id
                AND l.indexer_fingerprint = lr.indexer_fingerprint
                AND l.title_id = lr.title_id
                AND l.facet = lr.facet
                AND l.first_page_fingerprint = lr.first_page_fingerprint
              WHERE l.indexer_id = {} AND l.indexer_fingerprint = {} AND l.title_id = {}
                AND l.facet = {}
                AND l.advanced_at >= {}
                AND lr.created_at >= l.chain_started_at
              ORDER BY lr.created_at ASC, lr.run_id ASC",
            &args(),
        )
        .await?;
        for row in &runs {
            let fingerprint = row.text("first_page_fingerprint")?;
            if let Some(listing) = listings
                .iter_mut()
                .find(|listing| listing.first_page_fingerprint == fingerprint)
            {
                listing.run_ids.push(row.text("run_id")?);
            }
        }
        Ok(listings)
    }

    async fn claim_search_listing(
        &self,
        scope: &IndexerSearchListingScope,
        first_page_fingerprint: &str,
        claimant: &IndexerSearchListingStrategy,
        strategy_family: &str,
        fresh_after: DateTime<Utc>,
        idle_before: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<IndexerSearchListingClaim> {
        let scope = scope.clone();
        let fingerprint = first_page_fingerprint.to_string();
        let claimant = claimant.clone();
        let family = strategy_family.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "claim_indexer_search_listing", move |tx| {
            let scope = scope.clone();
            let fingerprint = fingerprint.clone();
            let claimant = claimant.clone();
            let family = family.clone();
            Box::pin(async move {
                let listing_args = || {
                    let mut args = listing_scope_args(&scope);
                    args.push(SqlArg::Text(fingerprint.clone()));
                    args
                };
                let existing = SqlRuntime::fetch_optional(
                    SqlExec::Tx(tx),
                    "SELECT owner_scope_key, owner_query_signature, page_cursor, finished,
                            advanced_at
                       FROM indexer_search_listings
                      WHERE indexer_id = {} AND indexer_fingerprint = {} AND title_id = {}
                        AND facet = {} AND first_page_fingerprint = {}",
                    &listing_args(),
                )
                .await?;
                let shared = match existing {
                    Some(row) => {
                        let owner = IndexerSearchListingStrategy {
                            scope_key: row.text("owner_scope_key")?,
                            query_signature: row.text("owner_query_signature")?,
                        };
                        let page_cursor = row.opt_text("page_cursor")?;
                        let finished = row.bool("finished")?;
                        let advanced_at = row.timestamp("advanced_at")?;
                        // A chain that claimed its first page and recorded
                        // nothing before going idle was abandoned.
                        let abandoned =
                            page_cursor.is_none() && !finished && advanced_at < idle_before;
                        (owner != claimant && advanced_at >= fresh_after && !abandoned)
                            .then_some((owner, !finished))
                    }
                    None => None,
                };
                let mut strategy_args = listing_scope_args(&scope);
                strategy_args.push(SqlArg::Text(claimant.scope_key.clone()));
                strategy_args.push(SqlArg::Text(claimant.query_signature.clone()));
                let claim = match shared {
                    Some((owner, listing_open)) => {
                        let mut args = listing_scope_args(&scope);
                        args.extend([
                            SqlArg::Text(claimant.scope_key.clone()),
                            SqlArg::Text(claimant.query_signature.clone()),
                            SqlArg::Text(family.clone()),
                            SqlArg::Text(fingerprint.clone()),
                            SqlArg::Timestamp(now),
                        ]);
                        upsert_listing_reader(tx, &args).await?;
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "DELETE FROM indexer_search_listings
                              WHERE indexer_id = {} AND indexer_fingerprint = {}
                                AND title_id = {} AND facet = {}
                                AND owner_scope_key = {} AND owner_query_signature = {}",
                            &strategy_args,
                        )
                        .await?;
                        IndexerSearchListingClaim::SameListing {
                            owner,
                            listing_open,
                        }
                    }
                    None => {
                        let mut args = listing_args();
                        args.extend([
                            SqlArg::Text(claimant.scope_key.clone()),
                            SqlArg::Text(claimant.query_signature.clone()),
                            SqlArg::Text(family.clone()),
                            SqlArg::Timestamp(now),
                            SqlArg::Timestamp(now),
                        ]);
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "INSERT INTO indexer_search_listings (
                                indexer_id, indexer_fingerprint, title_id, facet,
                                first_page_fingerprint, owner_scope_key, owner_query_signature,
                                owner_strategy_family, page_cursor, finished, chain_started_at,
                                advanced_at
                             ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, NULL, FALSE, {}, {})
                             ON CONFLICT(indexer_id, indexer_fingerprint, title_id, facet,
                                         first_page_fingerprint) DO UPDATE SET
                                owner_scope_key = excluded.owner_scope_key,
                                owner_query_signature = excluded.owner_query_signature,
                                owner_strategy_family = excluded.owner_strategy_family,
                                page_cursor = NULL,
                                finished = FALSE,
                                chain_started_at = excluded.chain_started_at,
                                advanced_at = excluded.advanced_at",
                            &args,
                        )
                        .await?;
                        // A listing the claimant owned under another first
                        // page is no longer what the indexer answers it.
                        let mut stale_args = strategy_args.clone();
                        stale_args.push(SqlArg::Text(fingerprint.clone()));
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "DELETE FROM indexer_search_listings
                              WHERE indexer_id = {} AND indexer_fingerprint = {}
                                AND title_id = {} AND facet = {}
                                AND owner_scope_key = {} AND owner_query_signature = {}
                                AND first_page_fingerprint <> {}",
                            &stale_args,
                        )
                        .await?;
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "DELETE FROM indexer_search_listing_readers
                              WHERE indexer_id = {} AND indexer_fingerprint = {}
                                AND title_id = {} AND facet = {}
                                AND scope_key = {} AND query_signature = {}",
                            &strategy_args,
                        )
                        .await?;
                        IndexerSearchListingClaim::Owner
                    }
                };
                Ok(claim)
            })
        })
        .await
    }

    async fn join_search_listing(
        &self,
        scope: &IndexerSearchListingScope,
        first_page_fingerprint: &str,
        reader: &IndexerSearchListingStrategy,
        strategy_family: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut args = listing_scope_args(scope);
        args.extend([
            SqlArg::Text(reader.scope_key.clone()),
            SqlArg::Text(reader.query_signature.clone()),
            SqlArg::Text(strategy_family.to_string()),
            SqlArg::Text(first_page_fingerprint.to_string()),
            SqlArg::Timestamp(now),
        ]);
        SqlRuntime::run_in_transaction(&self.datastore, "join_indexer_search_listing", move |tx| {
            let args = args.clone();
            Box::pin(async move { upsert_listing_reader(tx, &args).await })
        })
        .await
    }

    async fn link_search_listing_run(
        &self,
        scope: &IndexerSearchListingScope,
        first_page_fingerprint: &str,
        owner: &IndexerSearchListingStrategy,
        run_id: &str,
        next_cursor: Option<&str>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let scope = scope.clone();
        let fingerprint = first_page_fingerprint.to_string();
        let owner = owner.clone();
        let run_id = run_id.to_string();
        let next_cursor = next_cursor.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "link_indexer_search_listing_run",
            move |tx| {
                let scope = scope.clone();
                let fingerprint = fingerprint.clone();
                let owner = owner.clone();
                let run_id = run_id.clone();
                let next_cursor = next_cursor.clone();
                Box::pin(async move {
                    let mut args = vec![
                        SqlArg::OptText(next_cursor.clone()),
                        SqlArg::Bool(next_cursor.is_none()),
                        SqlArg::Timestamp(now),
                    ];
                    args.extend(listing_scope_args(&scope));
                    args.extend([
                        SqlArg::Text(fingerprint.clone()),
                        SqlArg::Text(owner.scope_key.clone()),
                        SqlArg::Text(owner.query_signature.clone()),
                    ]);
                    let advanced = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE indexer_search_listings
                        SET page_cursor = {}, finished = {}, advanced_at = {}
                      WHERE indexer_id = {} AND indexer_fingerprint = {} AND title_id = {}
                        AND facet = {} AND first_page_fingerprint = {}
                        AND owner_scope_key = {} AND owner_query_signature = {}",
                        &args,
                    )
                    .await?;
                    if advanced == 0 {
                        return Ok(false);
                    }
                    let mut args = vec![SqlArg::Text(run_id.clone())];
                    args.extend(listing_scope_args(&scope));
                    args.extend([SqlArg::Text(fingerprint.clone()), SqlArg::Timestamp(now)]);
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO indexer_search_listing_runs (
                        run_id, indexer_id, indexer_fingerprint, title_id, facet,
                        first_page_fingerprint, created_at
                     ) VALUES ({}, {}, {}, {}, {}, {}, {})
                     ON CONFLICT(run_id) DO NOTHING",
                        &args,
                    )
                    .await?;
                    Ok(true)
                })
            },
        )
        .await
    }

    async fn adopt_search_listing(
        &self,
        scope: &IndexerSearchListingScope,
        first_page_fingerprint: &str,
        previous_owner: &IndexerSearchListingStrategy,
        adopter: &IndexerSearchListingStrategy,
        expected_cursor: &str,
        idle_before: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let scope = scope.clone();
        let fingerprint = first_page_fingerprint.to_string();
        let previous_owner = previous_owner.clone();
        let adopter = adopter.clone();
        let expected_cursor = expected_cursor.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "adopt_indexer_search_listing", move |tx| {
            let scope = scope.clone();
            let fingerprint = fingerprint.clone();
            let previous_owner = previous_owner.clone();
            let adopter = adopter.clone();
            let expected_cursor = expected_cursor.clone();
            Box::pin(async move {
                let mut chain_args = listing_scope_args(&scope);
                chain_args.extend([
                    SqlArg::Text(fingerprint.clone()),
                    SqlArg::Text(previous_owner.scope_key.clone()),
                    SqlArg::Text(previous_owner.query_signature.clone()),
                    SqlArg::Text(expected_cursor.clone()),
                    SqlArg::Bool(false),
                    SqlArg::Timestamp(idle_before),
                ]);
                const IDLE_CHAIN: &str = "indexer_id = {} AND indexer_fingerprint = {}
                    AND title_id = {} AND facet = {} AND first_page_fingerprint = {}
                    AND owner_scope_key = {} AND owner_query_signature = {}
                    AND page_cursor = {} AND finished = {} AND advanced_at < {}";
                let Some(row) = SqlRuntime::fetch_optional(
                    SqlExec::Tx(tx),
                    &format!(
                        "SELECT owner_strategy_family FROM indexer_search_listings WHERE {IDLE_CHAIN}"
                    ),
                    &chain_args,
                )
                .await?
                else {
                    return Ok(false);
                };
                let previous_family = row.text("owner_strategy_family")?;
                let mut adopter_args = listing_scope_args(&scope);
                adopter_args.extend([
                    SqlArg::Text(adopter.scope_key.clone()),
                    SqlArg::Text(adopter.query_signature.clone()),
                ]);
                let adopter_family = SqlRuntime::fetch_optional(
                    SqlExec::Tx(tx),
                    "SELECT strategy_family FROM indexer_search_listing_readers
                      WHERE indexer_id = {} AND indexer_fingerprint = {} AND title_id = {}
                        AND facet = {} AND scope_key = {} AND query_signature = {}",
                    &adopter_args,
                )
                .await?
                .map(|row| row.text("strategy_family"))
                .transpose()?
                .unwrap_or_else(|| previous_family.clone());
                let mut args = vec![
                    SqlArg::Text(adopter.scope_key.clone()),
                    SqlArg::Text(adopter.query_signature.clone()),
                    SqlArg::Text(adopter_family),
                    SqlArg::Timestamp(now),
                ];
                args.extend(chain_args.iter().cloned());
                let adopted = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    &format!(
                        "UPDATE indexer_search_listings
                            SET owner_scope_key = {{}}, owner_query_signature = {{}},
                                owner_strategy_family = {{}}, advanced_at = {{}}
                          WHERE {IDLE_CHAIN}"
                    ),
                    &args,
                )
                .await?;
                if adopted == 0 {
                    return Ok(false);
                }
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM indexer_search_listing_readers
                      WHERE indexer_id = {} AND indexer_fingerprint = {} AND title_id = {}
                        AND facet = {} AND scope_key = {} AND query_signature = {}",
                    &adopter_args,
                )
                .await?;
                // The strategy that left the chain reads it from here on.
                let mut args = listing_scope_args(&scope);
                args.extend([
                    SqlArg::Text(previous_owner.scope_key.clone()),
                    SqlArg::Text(previous_owner.query_signature.clone()),
                    SqlArg::Text(previous_family),
                    SqlArg::Text(fingerprint.clone()),
                    SqlArg::Timestamp(now),
                ]);
                upsert_listing_reader(tx, &args).await?;
                Ok(true)
            })
        })
        .await
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
        // A listing whose pages can no longer be replayed is read afresh.
        for (op_name, sql) in [
            (
                "cleanup_indexer_search_listing_runs",
                "DELETE FROM indexer_search_listing_runs WHERE created_at < {}",
            ),
            (
                "cleanup_indexer_search_listing_readers",
                "DELETE FROM indexer_search_listing_readers WHERE observed_at < {}",
            ),
            (
                "cleanup_indexer_search_listings",
                "DELETE FROM indexer_search_listings WHERE advanced_at < {}",
            ),
        ] {
            SqlRuntime::execute_write(
                &self.datastore,
                op_name,
                sql,
                vec![SqlArg::Timestamp(candidate_cutoff)],
            )
            .await?;
        }
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

    async fn prune_orphaned_learning(&self, limit: u32) -> AppResult<u32> {
        let deleted = SqlRuntime::execute_write(
            &self.datastore,
            "prune_orphaned_indexer_search_learning",
            "DELETE FROM indexer_search_learning
              WHERE (indexer_id, title_id, facet, strategy_key) IN (
                  SELECT l.indexer_id, l.title_id, l.facet, l.strategy_key
                    FROM indexer_search_learning l
                   WHERE NOT EXISTS (SELECT 1 FROM titles t WHERE t.id = l.title_id)
                      OR NOT EXISTS (SELECT 1 FROM indexers i WHERE i.id = l.indexer_id)
                   LIMIT {}
              )",
            vec![SqlArg::I64(i64::from(limit.max(1)))],
        )
        .await?;
        let deleted_facet_rows = SqlRuntime::execute_write(
            &self.datastore,
            "prune_orphaned_indexer_facet_learning",
            "DELETE FROM indexer_facet_learning
              WHERE (indexer_id, facet) IN (
                  SELECT f.indexer_id, f.facet
                    FROM indexer_facet_learning f
                   WHERE NOT EXISTS (SELECT 1 FROM indexers i WHERE i.id = f.indexer_id)
                   LIMIT {}
              )",
            vec![SqlArg::I64(i64::from(limit.max(1)))],
        )
        .await?;
        Ok(deleted
            .saturating_add(deleted_facet_rows)
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
        SqlRuntime::execute_write(
            &self.datastore,
            "prune_indexer_facet_learning",
            "DELETE FROM indexer_facet_learning WHERE indexer_id = {}",
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
            SET_LEARNING_SUPPRESSED_SQL,
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

    async fn facet_learning_states(
        &self,
        facet: &str,
        indexer_ids: &[String],
    ) -> AppResult<HashMap<String, IndexerFacetLearningState>> {
        if indexer_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = std::iter::repeat_n("{}", indexer_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut args = vec![SqlArg::Text(facet.to_string())];
        args.extend(indexer_ids.iter().cloned().map(SqlArg::Text));
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT indexer_id, empty_title_ids, suppressed_until, reprobe_claimed_at, revision
                   FROM indexer_facet_learning
                  WHERE facet = {{}} AND indexer_id IN ({placeholders})"
            ),
            &args,
        )
        .await?;
        rows.iter()
            .map(|row| Ok((row.text("indexer_id")?, row_to_facet_learning_state(row)?)))
            .collect()
    }

    async fn record_facet_search_outcome(
        &self,
        indexer_id: &str,
        facet: &str,
        title_id: &str,
        outcome: IndexerFacetSearchOutcome,
        now: DateTime<Utc>,
        policy: IndexerFacetLearningPolicy,
    ) -> AppResult<IndexerFacetTransition> {
        for _ in 0..FACET_LEARNING_WRITE_ATTEMPTS {
            let current = self.facet_learning_state(indexer_id, facet).await?;
            let (next, transition) = current
                .clone()
                .unwrap_or_default()
                .apply_outcome(title_id, outcome, now, policy);
            let Some(next) = next else {
                return Ok(transition);
            };
            if self
                .compare_and_set_facet_learning(indexer_id, facet, current.as_ref(), &next, now)
                .await?
            {
                return Ok(transition);
            }
        }
        Err(AppError::Repository(
            "indexer facet learning kept changing under a concurrent writer".to_string(),
        ))
    }

    async fn try_claim_facet_reprobe(
        &self,
        indexer_id: &str,
        facet: &str,
        now: DateTime<Utc>,
        policy: IndexerFacetLearningPolicy,
    ) -> AppResult<bool> {
        let Some(current) = self.facet_learning_state(indexer_id, facet).await? else {
            return Ok(false);
        };
        let Some(next) = current.claim_reprobe(now, policy) else {
            return Ok(false);
        };
        // Losing the compare-and-set means another search changed the row
        // first, most likely by claiming this same re-probe.
        self.compare_and_set_facet_learning(indexer_id, facet, Some(&current), &next, now)
            .await
    }
}

/// Compare-and-set rounds a facet learning write gets before it gives up.
/// Writers are the searches of one indexer for one facet, so a conflict is
/// rare and a second round almost always lands.
const FACET_LEARNING_WRITE_ATTEMPTS: usize = 5;

impl IndexerSearchLearningStore {
    async fn facet_learning_state(
        &self,
        indexer_id: &str,
        facet: &str,
    ) -> AppResult<Option<IndexerFacetLearningState>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            FACET_LEARNING_STATE_SQL,
            &[
                SqlArg::Text(indexer_id.to_string()),
                SqlArg::Text(facet.to_string()),
            ],
        )
        .await?;
        row.as_ref().map(row_to_facet_learning_state).transpose()
    }

    /// Writes `next` only if the row still is `current` (`None`: still
    /// absent). Returns whether the write landed.
    async fn compare_and_set_facet_learning(
        &self,
        indexer_id: &str,
        facet: &str,
        current: Option<&IndexerFacetLearningState>,
        next: &IndexerFacetLearningState,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let (template, args) = facet_learning_write(indexer_id, facet, current, next, now)?;
        let op_name = if current.is_some() {
            "update_indexer_facet_learning"
        } else {
            "insert_indexer_facet_learning"
        };
        let rows = SqlRuntime::execute_write(&self.datastore, op_name, template, args).await?;
        Ok(rows > 0)
    }
}

fn row_to_facet_learning_state(row: &SqlRow) -> AppResult<IndexerFacetLearningState> {
    let empty_title_ids = serde_json::from_str::<Vec<String>>(&row.text("empty_title_ids")?)
        .map_err(|error| {
            AppError::Repository(format!(
                "indexer_facet_learning.empty_title_ids is not a title list: {error}"
            ))
        })?;
    Ok(IndexerFacetLearningState {
        empty_title_ids,
        suppressed_until: row.opt_timestamp("suppressed_until")?,
        reprobe_claimed_at: row.opt_timestamp("reprobe_claimed_at")?,
        revision: row.i64("revision")?,
    })
}

const SET_LEARNING_SUPPRESSED_SQL: &str = "UPDATE indexer_search_learning
             SET suppressed = {}, updated_at = {}
             WHERE indexer_id = {} AND title_id = {} AND facet = {} AND strategy_key = {}";

const SQLITE_RECORD_LEARNING_OUTCOME_SQL: &str = "INSERT INTO indexer_search_learning (
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
        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

const POSTGRES_RECORD_LEARNING_OUTCOME_SQL: &str = "INSERT INTO indexer_search_learning (
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
        updated_at = NOW()";

/// Counts one strategy outcome into its learning row and keeps the title's
/// text rows within [`FREETEXT_LEARNING_ROWS_PER_TITLE`].
async fn record_learning_outcome_in_tx(
    tx: &mut SqlTx<'_>,
    key: &IndexerSearchLearningKey,
    usable_hits: u32,
) -> AppResult<()> {
    let evicts_text = key.strategy_key.starts_with(FREETEXT_LEARNING_KEY_PREFIX);
    match tx {
        SqlTx::Sqlite(tx) => {
            sqlx::query(SQLITE_RECORD_LEARNING_OUTCOME_SQL)
                .bind(&key.indexer_id)
                .bind(&key.title_id)
                .bind(&key.facet)
                .bind(&key.strategy_key)
                .bind(usable_hits as i64)
                .bind(usable_hits as i64)
                .bind(usable_hits as i64)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
            if evicts_text {
                sqlx::query(SQLITE_EVICT_FREETEXT_LEARNING_SQL)
                    .bind(&key.indexer_id)
                    .bind(&key.title_id)
                    .bind(&key.facet)
                    .bind(&key.indexer_id)
                    .bind(&key.title_id)
                    .bind(&key.facet)
                    .bind(&key.strategy_key)
                    .bind(FREETEXT_LEARNING_ROWS_PER_TITLE)
                    .execute(&mut **tx)
                    .await
                    .map_err(repo_err)?;
            }
        }
        SqlTx::Postgres(tx) => {
            sqlx::query(POSTGRES_RECORD_LEARNING_OUTCOME_SQL)
                .bind(&key.indexer_id)
                .bind(&key.title_id)
                .bind(&key.facet)
                .bind(&key.strategy_key)
                .bind(usable_hits as i64)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
            if evicts_text {
                sqlx::query(POSTGRES_EVICT_FREETEXT_LEARNING_SQL)
                    .bind(&key.indexer_id)
                    .bind(&key.title_id)
                    .bind(&key.facet)
                    .bind(&key.strategy_key)
                    .bind(FREETEXT_LEARNING_ROWS_PER_TITLE)
                    .execute(&mut **tx)
                    .await
                    .map_err(repo_err)?;
            }
        }
    }
    Ok(())
}

/// Applies a batch's facet outcomes in order to the indexer's facet state in
/// one compare-and-set, as one-by-one writes with no other writer between
/// them would leave it.
async fn apply_facet_outcomes_in_tx(
    tx: &mut SqlTx<'_>,
    batch: &IndexerSearchLearningBatch,
) -> AppResult<Vec<IndexerFacetTransition>> {
    if batch.facet_outcomes.is_empty() {
        return Ok(Vec::new());
    }
    for _ in 0..FACET_LEARNING_WRITE_ATTEMPTS {
        let current = SqlRuntime::fetch_optional(
            SqlExec::Tx(tx),
            FACET_LEARNING_STATE_SQL,
            &[
                SqlArg::Text(batch.indexer_id.clone()),
                SqlArg::Text(batch.facet.clone()),
            ],
        )
        .await?
        .as_ref()
        .map(row_to_facet_learning_state)
        .transpose()?;
        let mut state = current.clone().unwrap_or_default();
        let mut changed = false;
        let mut transitions = Vec::with_capacity(batch.facet_outcomes.len());
        for outcome in &batch.facet_outcomes {
            let (next, transition) =
                state.apply_outcome(&batch.title_id, *outcome, batch.now, batch.facet_policy);
            transitions.push(transition);
            if let Some(next) = next {
                state = next;
                changed = true;
            }
        }
        if !changed {
            return Ok(transitions);
        }
        let (template, args) = facet_learning_write(
            &batch.indexer_id,
            &batch.facet,
            current.as_ref(),
            &state,
            batch.now,
        )?;
        if SqlRuntime::execute(SqlExec::Tx(tx), template, &args).await? > 0 {
            return Ok(transitions);
        }
    }
    Err(AppError::Repository(
        "indexer facet learning kept changing under a concurrent writer".to_string(),
    ))
}

const FACET_LEARNING_STATE_SQL: &str =
    "SELECT empty_title_ids, suppressed_until, reprobe_claimed_at, revision
       FROM indexer_facet_learning
      WHERE indexer_id = {} AND facet = {}";

/// The compare-and-set write of `next` over `current` (`None`: the row is
/// still absent).
fn facet_learning_write(
    indexer_id: &str,
    facet: &str,
    current: Option<&IndexerFacetLearningState>,
    next: &IndexerFacetLearningState,
    now: DateTime<Utc>,
) -> AppResult<(&'static str, Vec<SqlArg>)> {
    let empty_title_ids = serde_json::to_string(&next.empty_title_ids).map_err(|error| {
        AppError::Repository(format!("failed to encode facet learning titles: {error}"))
    })?;
    Ok(match current {
        None => (
            "INSERT INTO indexer_facet_learning (
                indexer_id, facet, empty_title_ids, suppressed_until,
                reprobe_claimed_at, revision, updated_at
             )
             VALUES ({}, {}, {}, {}, {}, 1, {})
             ON CONFLICT (indexer_id, facet) DO NOTHING",
            vec![
                SqlArg::Text(indexer_id.to_string()),
                SqlArg::Text(facet.to_string()),
                SqlArg::Text(empty_title_ids),
                SqlArg::OptTimestamp(next.suppressed_until),
                SqlArg::OptTimestamp(next.reprobe_claimed_at),
                SqlArg::Timestamp(now),
            ],
        ),
        Some(current) => (
            "UPDATE indexer_facet_learning
                SET empty_title_ids = {}, suppressed_until = {},
                    reprobe_claimed_at = {}, revision = revision + 1,
                    updated_at = {}
              WHERE indexer_id = {} AND facet = {} AND revision = {}",
            vec![
                SqlArg::Text(empty_title_ids),
                SqlArg::OptTimestamp(next.suppressed_until),
                SqlArg::OptTimestamp(next.reprobe_claimed_at),
                SqlArg::Timestamp(now),
                SqlArg::Text(indexer_id.to_string()),
                SqlArg::Text(facet.to_string()),
                SqlArg::I64(current.revision),
            ],
        ),
    })
}

/// Drops the title-text rows of one (indexer, title, facet) beyond
/// [`FREETEXT_LEARNING_ROWS_PER_TITLE`]; see that constant for the order.
const SQLITE_EVICT_FREETEXT_LEARNING_SQL: &str = "DELETE FROM indexer_search_learning
  WHERE indexer_id = ? AND title_id = ? AND facet = ?
    AND strategy_key LIKE 'v3:freetext:%'
    AND strategy_key NOT IN (
        SELECT strategy_key FROM indexer_search_learning
         WHERE indexer_id = ? AND title_id = ? AND facet = ?
           AND strategy_key LIKE 'v3:freetext:%'
         ORDER BY CASE WHEN strategy_key = ? THEN 0 ELSE 1 END,
                  CASE WHEN usable_successes > 0 THEN 0 ELSE 1 END,
                  COALESCE(last_attempt_at, updated_at) DESC,
                  strategy_key
         LIMIT ?
    )";

const POSTGRES_EVICT_FREETEXT_LEARNING_SQL: &str = "DELETE FROM indexer_search_learning
  WHERE indexer_id = $1 AND title_id = $2 AND facet = $3
    AND strategy_key LIKE 'v3:freetext:%'
    AND strategy_key NOT IN (
        SELECT strategy_key FROM indexer_search_learning
         WHERE indexer_id = $1 AND title_id = $2 AND facet = $3
           AND strategy_key LIKE 'v3:freetext:%'
         ORDER BY CASE WHEN strategy_key = $4 THEN 0 ELSE 1 END,
                  CASE WHEN usable_successes > 0 THEN 0 ELSE 1 END,
                  COALESCE(last_attempt_at, updated_at) DESC,
                  strategy_key
         LIMIT $5
    )";

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
pub(crate) mod tests {
    use super::*;
    use scryer_application::SEARCH_STRATEGY_OPEN;
    use std::collections::HashSet;

    use sqlx::sqlite::SqlitePoolOptions;

    const POSTGRES_TEST_FACET_LEARNING_TABLE: &str = "CREATE TEMP TABLE indexer_facet_learning (
            indexer_id TEXT NOT NULL,
            facet TEXT NOT NULL,
            empty_title_ids TEXT NOT NULL DEFAULT '[]',
            suppressed_until TIMESTAMPTZ,
            reprobe_claimed_at TIMESTAMPTZ,
            revision BIGINT NOT NULL DEFAULT 0,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (indexer_id, facet)
        ) ON COMMIT PRESERVE ROWS";

    const FACET_POLICY: IndexerFacetLearningPolicy = IndexerFacetLearningPolicy {
        empty_title_threshold: 5,
        suppression: chrono::Duration::hours(24),
        reprobe_claim: chrono::Duration::hours(1),
    };

    #[tokio::test]
    async fn sqlite_store_suppresses_a_facet_after_distinct_empty_titles_and_clears_on_a_hit() {
        let (store, _pool) = sqlite_store().await;
        let now = DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z")
            .expect("fixed instant")
            .with_timezone(&Utc);
        let record = |title: &'static str, outcome, at| {
            let store = store.clone();
            async move {
                store
                    .record_facet_search_outcome(
                        "idx-general",
                        "anime",
                        title,
                        outcome,
                        at,
                        FACET_POLICY,
                    )
                    .await
                    .expect("facet outcome should persist")
            }
        };

        for (index, title) in ["title-a", "title-b", "title-c", "title-d"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                record(title, IndexerFacetSearchOutcome::Empty, now).await,
                IndexerFacetTransition::StreakAdvanced { streak: index + 1 }
            );
        }
        assert_eq!(
            record("title-b", IndexerFacetSearchOutcome::Empty, now).await,
            IndexerFacetTransition::Unchanged,
            "a repeated title is not a new one"
        );
        let until = now + FACET_POLICY.suppression;
        assert_eq!(
            record("title-e", IndexerFacetSearchOutcome::Empty, now).await,
            IndexerFacetTransition::Suppressed { streak: 5, until }
        );

        let ids = vec!["idx-general".to_string(), "idx-other".to_string()];
        let states = store
            .facet_learning_states("anime", &ids)
            .await
            .expect("facet states should load");
        assert_eq!(states.len(), 1);
        let state = &states["idx-general"];
        assert_eq!(state.streak(), 5);
        assert_eq!(state.held_until(now, FACET_POLICY), Some(until));
        assert!(
            store
                .facet_learning_states("series", &ids)
                .await
                .expect("facet states should load")
                .is_empty(),
            "another facet is untouched"
        );
        assert!(
            !store
                .try_claim_facet_reprobe("idx-general", "anime", now, FACET_POLICY)
                .await
                .expect("claim"),
            "no re-probe inside the window"
        );

        // After the window exactly one re-probe is claimed.
        let after = until + chrono::Duration::minutes(1);
        assert!(
            store
                .try_claim_facet_reprobe("idx-general", "anime", after, FACET_POLICY)
                .await
                .expect("claim")
        );
        assert!(
            !store
                .try_claim_facet_reprobe("idx-general", "anime", after, FACET_POLICY)
                .await
                .expect("claim")
        );
        let claimed = &store
            .facet_learning_states("anime", &ids)
            .await
            .expect("facet states should load")["idx-general"];
        assert_eq!(
            claimed.held_until(after, FACET_POLICY),
            Some(after + FACET_POLICY.reprobe_claim),
            "the claimed re-probe holds the walk off"
        );

        // An empty re-probe suppresses for another full window.
        assert_eq!(
            record("title-f", IndexerFacetSearchOutcome::Empty, after).await,
            IndexerFacetTransition::Suppressed {
                streak: 5,
                until: after + FACET_POLICY.suppression,
            }
        );

        // Any usable hit clears the streak and the suppression.
        assert_eq!(
            record("title-g", IndexerFacetSearchOutcome::Usable, after).await,
            IndexerFacetTransition::Cleared {
                streak: 5,
                was_suppressed: true,
            }
        );
        let cleared = &store
            .facet_learning_states("anime", &ids)
            .await
            .expect("facet states should load")["idx-general"];
        assert_eq!(cleared.streak(), 0);
        assert_eq!(cleared.held_until(after, FACET_POLICY), None);
        assert_eq!(
            record("title-g", IndexerFacetSearchOutcome::Usable, after).await,
            IndexerFacetTransition::Unchanged
        );

        store
            .prune_indexer("idx-general")
            .await
            .expect("prune should succeed");
        assert!(
            store
                .facet_learning_states("anime", &ids)
                .await
                .expect("facet states should load")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn sqlite_store_lapsed_reprobe_claim_can_be_claimed_again() {
        let (store, _pool) = sqlite_store().await;
        let now = DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z")
            .expect("fixed instant")
            .with_timezone(&Utc);
        for title in ["t1", "t2", "t3", "t4", "t5"] {
            store
                .record_facet_search_outcome(
                    "idx-general",
                    "anime",
                    title,
                    IndexerFacetSearchOutcome::Empty,
                    now,
                    FACET_POLICY,
                )
                .await
                .expect("facet outcome should persist");
        }
        let after = now + FACET_POLICY.suppression;
        assert!(
            store
                .try_claim_facet_reprobe("idx-general", "anime", after, FACET_POLICY)
                .await
                .expect("claim")
        );
        // The re-probe errored and landed nothing; once its claim lapses
        // another background search may try.
        let lapsed = after + FACET_POLICY.reprobe_claim;
        assert!(
            store
                .try_claim_facet_reprobe("idx-general", "anime", lapsed, FACET_POLICY)
                .await
                .expect("claim")
        );
    }

    pub(crate) async fn sqlite_store() -> (IndexerSearchLearningStore, sqlx::SqlitePool) {
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
                created_at TEXT NOT NULL,
                strategy_state TEXT,
                coverage_scope_key TEXT,
                page_cursor TEXT,
                cursor_chain_id TEXT
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
        sqlx::query(include_str!(
            "../../../scryer/src/db/migrations/0313_indexer_facet_learning.sql"
        ))
        .execute(&pool)
        .await
        .expect("facet learning table should be created");
        sqlx::query(include_str!(
            "../../../scryer/src/db/migrations/0314_indexer_search_listings.sql"
        ))
        .execute(&pool)
        .await
        .expect("search listing table should be created");
        for table in ["titles", "indexers"] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "CREATE TABLE {table} (id TEXT PRIMARY KEY)"
            )))
            .execute(&pool)
            .await
            .expect("owner table should be created");
        }

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
    async fn sqlite_store_learning_batch_lands_as_the_same_outcomes_one_by_one() {
        let (store, _pool) = sqlite_store().await;
        let now = Utc::now();
        let text_key = format!("{FREETEXT_LEARNING_KEY_PREFIX}digest-1");
        let strategy_outcomes = vec![
            ("v2:ids_sxex".to_string(), 2),
            ("v2:ids_abs".to_string(), 0),
            (text_key.clone(), 0),
            ("v2:ids_abs".to_string(), 0),
            ("v2:ids_sxex".to_string(), 0),
        ];
        let facet_outcomes = vec![
            IndexerFacetSearchOutcome::Empty,
            IndexerFacetSearchOutcome::Empty,
            IndexerFacetSearchOutcome::Usable,
            IndexerFacetSearchOutcome::Empty,
        ];
        let suppress = vec!["v2:ids_abs".to_string()];

        let one_by_one = "idx-sequential";
        let mut sequential_transitions = Vec::new();
        for (strategy_key, usable_hits) in &strategy_outcomes {
            store
                .record_outcome(
                    &IndexerSearchLearningKey {
                        indexer_id: one_by_one.into(),
                        title_id: "title-1".into(),
                        facet: "anime".into(),
                        strategy_key: strategy_key.clone(),
                    },
                    *usable_hits,
                )
                .await
                .expect("outcome");
        }
        for strategy_key in &suppress {
            store
                .set_suppressed(
                    &IndexerSearchLearningKey {
                        indexer_id: one_by_one.into(),
                        title_id: "title-1".into(),
                        facet: "anime".into(),
                        strategy_key: strategy_key.clone(),
                    },
                    true,
                )
                .await
                .expect("suppression");
        }
        for outcome in &facet_outcomes {
            sequential_transitions.push(
                store
                    .record_facet_search_outcome(
                        one_by_one,
                        "anime",
                        "title-1",
                        *outcome,
                        now,
                        FACET_POLICY,
                    )
                    .await
                    .expect("facet outcome"),
            );
        }

        let batched = "idx-batched";
        let batch_transitions = store
            .record_learning_batch(&IndexerSearchLearningBatch {
                indexer_id: batched.into(),
                title_id: "title-1".into(),
                facet: "anime".into(),
                strategy_outcomes: strategy_outcomes.clone(),
                suppress: suppress.clone(),
                facet_outcomes: facet_outcomes.clone(),
                now,
                facet_policy: FACET_POLICY,
            })
            .await
            .expect("batch should land");
        assert_eq!(batch_transitions, sequential_transitions);

        let by_indexer = store
            .list_for_title_by_indexer(
                &[
                    one_by_one.to_string(),
                    batched.to_string(),
                    "idx-unseen".to_string(),
                ],
                "title-1",
                "anime",
            )
            .await
            .expect("one read for every indexer");
        assert_eq!(by_indexer.get("idx-unseen"), Some(&Vec::new()));
        let counts = |indexer_id: &str| {
            let mut rows = by_indexer[indexer_id]
                .iter()
                .map(|record| {
                    (
                        record.key.strategy_key.clone(),
                        record.attempts,
                        record.empty_successes,
                        record.usable_successes,
                        record.suppressed,
                        record.last_usable_at.is_some(),
                    )
                })
                .collect::<Vec<_>>();
            rows.sort();
            rows
        };
        assert_eq!(counts(batched), counts(one_by_one));
        assert_eq!(counts(batched).len(), 3);

        let facets = store
            .facet_learning_states("anime", &[one_by_one.to_string(), batched.to_string()])
            .await
            .expect("facet states");
        let shape = |state: &IndexerFacetLearningState| {
            (
                state.empty_title_ids.clone(),
                state.suppressed_until,
                state.reprobe_claimed_at,
            )
        };
        assert_eq!(shape(&facets[batched]), shape(&facets[one_by_one]));
        assert_eq!(facets[batched].empty_title_ids, vec!["title-1".to_string()]);
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

    fn text_learning_key(
        indexer_id: &str,
        title_id: &str,
        query: &str,
    ) -> IndexerSearchLearningKey {
        IndexerSearchLearningKey {
            indexer_id: indexer_id.into(),
            title_id: title_id.into(),
            facet: "series".into(),
            strategy_key: format!("{FREETEXT_LEARNING_KEY_PREFIX}{query}"),
        }
    }

    async fn seed_learning_row(
        store: &IndexerSearchLearningStore,
        key: &IndexerSearchLearningKey,
        usable: i64,
        attempted_at: DateTime<Utc>,
    ) {
        SqlRuntime::execute_write(
            &store.datastore,
            "seed_indexer_search_learning",
            "INSERT INTO indexer_search_learning (
                indexer_id, title_id, facet, strategy_key, attempts, empty_successes,
                usable_successes, last_attempt_at, suppressed, updated_at
             ) VALUES ({}, {}, {}, {}, 1, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(key.indexer_id.clone()),
                SqlArg::Text(key.title_id.clone()),
                SqlArg::Text(key.facet.clone()),
                SqlArg::Text(key.strategy_key.clone()),
                SqlArg::I64(1 - usable.min(1)),
                SqlArg::I64(usable),
                SqlArg::Timestamp(attempted_at),
                SqlArg::Bool(false),
                SqlArg::Timestamp(attempted_at),
            ],
        )
        .await
        .expect("learning row should seed");
    }

    /// Forty-one text rows of one title arrive at an indexer: the oldest
    /// with the only usable success, forty empty ones after it, then a new
    /// empty query. The write keeps the cap, never evicts the row it wrote,
    /// keeps the usable row and evicts the least recently attempted empties.
    /// ID rows and other titles are outside the cap.
    async fn assert_text_learning_cap(store: &IndexerSearchLearningStore) {
        let base = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .expect("fixed timestamp")
            .with_timezone(&Utc);
        seed_learning_row(
            store,
            &text_learning_key("idx-1", "title-1", "usable"),
            1,
            base,
        )
        .await;
        for ordinal in 0..40 {
            seed_learning_row(
                store,
                &text_learning_key("idx-1", "title-1", &format!("e{ordinal:02}")),
                0,
                base + chrono::Duration::minutes(ordinal + 1),
            )
            .await;
        }
        seed_learning_row(
            store,
            &text_learning_key("idx-1", "title-2", "other"),
            0,
            base,
        )
        .await;
        let ids = IndexerSearchLearningKey {
            strategy_key: "v2:ids".into(),
            ..text_learning_key("idx-1", "title-1", "")
        };
        seed_learning_row(store, &ids, 0, base).await;

        let written = store
            .record_outcome(&text_learning_key("idx-1", "title-1", "new"), 0)
            .await
            .expect("the newest query must survive its own write");
        assert_eq!(written.attempts, 1);

        let records = store
            .list_for_title("idx-1", "title-1", "series")
            .await
            .expect("records should list");
        let text: Vec<String> = records
            .iter()
            .filter_map(|record| {
                record
                    .key
                    .strategy_key
                    .strip_prefix(FREETEXT_LEARNING_KEY_PREFIX)
                    .map(str::to_owned)
            })
            .collect();
        assert_eq!(
            text.len() as i64,
            FREETEXT_LEARNING_ROWS_PER_TITLE,
            "{text:?}"
        );
        assert!(text.contains(&"new".to_string()));
        assert!(text.contains(&"usable".to_string()), "{text:?}");
        for ordinal in 0..10 {
            assert!(!text.contains(&format!("e{ordinal:02}")), "{text:?}");
        }
        for ordinal in 10..40 {
            assert!(text.contains(&format!("e{ordinal:02}")), "{text:?}");
        }
        assert!(
            records
                .iter()
                .any(|record| record.key.strategy_key == "v2:ids")
        );
        assert_eq!(
            store
                .list_for_title("idx-1", "title-2", "series")
                .await
                .expect("other title should list")
                .len(),
            1
        );
    }

    /// Rows whose indexer or title is gone go, a bounded batch at a time.
    async fn assert_orphan_learning_pruning(store: &IndexerSearchLearningStore) {
        for (table, id) in [("titles", "title-live"), ("indexers", "idx-live")] {
            SqlRuntime::execute_write(
                &store.datastore,
                "seed_owner",
                &format!("INSERT INTO {table} (id) VALUES ({{}})"),
                vec![SqlArg::Text(id.into())],
            )
            .await
            .expect("owner row should seed");
        }
        let now = Utc::now();
        for (indexer_id, title_id) in [
            ("idx-live", "title-live"),
            ("idx-gone", "title-live"),
            ("idx-live", "title-gone"),
        ] {
            seed_learning_row(store, &text_learning_key(indexer_id, title_id, "q"), 1, now).await;
        }
        assert_eq!(store.prune_orphaned_learning(1).await.expect("prune"), 1);
        assert_eq!(store.prune_orphaned_learning(5).await.expect("prune"), 1);
        assert_eq!(store.prune_orphaned_learning(5).await.expect("prune"), 0);
        assert_eq!(
            store
                .list_for_title("idx-live", "title-live", "series")
                .await
                .expect("live records should list")
                .len(),
            1
        );
    }

    fn background_run(
        id: &str,
        session: &str,
        query_signature: &str,
        state: Option<&str>,
        retry_at: Option<DateTime<Utc>>,
        created_at: DateTime<Utc>,
    ) -> IndexerSearchRunWrite {
        IndexerSearchRunWrite {
            id: id.into(),
            indexer_id: "idx-1".into(),
            provider_type: "newznab".into(),
            search_session_id: session.into(),
            scope_key: "title-1:series:episode:1:2:-".into(),
            query_signature: query_signature.into(),
            branch: "ids".into(),
            page: None,
            range_min_size: None,
            range_max_size: None,
            result_count: 0,
            completion_state: "received_partial".into(),
            retry_at,
            error_summary: None,
            indexer_fingerprint: "fingerprint-1".into(),
            created_at,
            strategy_state: state.map(str::to_string),
            page_cursor: None,
            cursor_chain_id: None,
        }
    }

    #[tokio::test]
    async fn sqlite_store_reads_a_paged_strategy_chain_and_its_resume_cursor() {
        let (store, _pool) = sqlite_store().await;
        let now = Utc::now();
        let minutes = chrono::Duration::minutes;
        let paged = |id: &str, chain: &str, cursor: Option<&str>, created_at| {
            let mut run = background_run(
                id,
                &format!("session-{id}"),
                "ids",
                Some(SEARCH_STRATEGY_OPEN),
                None,
                created_at,
            );
            run.cursor_chain_id = Some(chain.to_string());
            run.page_cursor = cursor.map(str::to_string);
            run
        };
        for run in [
            // An earlier chain that ran to its end.
            paged("run-1", "chain-old", None, now - minutes(40)),
            paged("run-2", "chain-new", Some("offset:500"), now - minutes(15)),
            paged("run-3", "chain-new", Some("offset:1000"), now - minutes(10)),
            // An operator search in between moves nothing.
            background_run("run-4", "operator", "ids", None, None, now - minutes(5)),
        ] {
            store
                .record_search_diagnostics(&run, &[])
                .await
                .expect("run should persist");
        }

        let states = store
            .list_background_strategy_states(
                "idx-1",
                "title-1:series:episode:1:2:-",
                "fingerprint-1",
                now - chrono::Duration::hours(72),
            )
            .await
            .expect("background states should load");
        assert_eq!(states.len(), 1);
        let ids = &states[0];
        assert_eq!(ids.page_cursor.as_deref(), Some("offset:1000"));
        assert_eq!(ids.cursor_chain_id.as_deref(), Some("chain-new"));
        assert_eq!(ids.chain_run_ids, vec!["run-3", "run-2"]);
        assert_eq!(ids.branch, "ids");
    }

    #[tokio::test]
    async fn sqlite_store_reads_background_strategy_state_and_contained_holds() {
        let (store, _pool) = sqlite_store().await;
        let now = Utc::now();
        let hour = chrono::Duration::hours(1);
        let lookback = now - chrono::Duration::hours(72);
        let scope = "episode:synthetic-episode-1";

        // Session one: the id strategy was contained, the text one converged.
        for run in [
            background_run(
                "run-1",
                "session-1",
                "ids",
                Some(SEARCH_STRATEGY_CONTAINED),
                Some(now + hour),
                now - chrono::Duration::minutes(10),
            ),
            background_run(
                "run-2",
                "session-1",
                "text",
                Some(SEARCH_STRATEGY_CONVERGED),
                None,
                now - chrono::Duration::minutes(10),
            ),
            // An operator run afterwards writes no background state.
            background_run(
                "run-3",
                "operator-session",
                "ids",
                None,
                None,
                now - chrono::Duration::minutes(5),
            ),
        ] {
            store
                .record_search_diagnostics(&run, &[])
                .await
                .expect("run should persist");
        }

        let states = store
            .list_background_strategy_states(
                "idx-1",
                "title-1:series:episode:1:2:-",
                "fingerprint-1",
                lookback,
            )
            .await
            .expect("background states should load");
        let ids = states
            .iter()
            .find(|state| state.query_signature == "ids")
            .expect("the id strategy keeps its background state");
        assert_eq!(ids.strategy_state, SEARCH_STRATEGY_CONTAINED);
        assert_eq!(
            ids.retry_at.map(|at| at.timestamp()),
            Some((now + hour).timestamp())
        );
        assert_eq!(states.len(), 2);

        // Until the session is linked to its convergence scope, nothing holds.
        assert_eq!(
            store
                .contained_search_hold(scope, "idx-1", "fingerprint-1", lookback, now)
                .await
                .expect("hold should read"),
            None
        );
        store
            .link_search_session_coverage_scope("session-1", scope)
            .await
            .expect("link should write");
        store
            .link_search_session_coverage_scope("operator-session", scope)
            .await
            .expect("link should write");
        let held = store
            .contained_search_hold(scope, "idx-1", "fingerprint-1", lookback, now)
            .await
            .expect("hold should read")
            .expect("a contained, converged session holds the indexer");
        assert_eq!(held.timestamp(), (now + hour).timestamp());

        // A changed indexer identity is not held, and neither is a due one.
        assert_eq!(
            store
                .contained_search_hold(scope, "idx-1", "fingerprint-2", lookback, now)
                .await
                .expect("hold should read"),
            None
        );
        assert_eq!(
            store
                .contained_search_hold(scope, "idx-1", "fingerprint-1", lookback, now + hour * 2)
                .await
                .expect("hold should read"),
            None
        );

        // A later session with open work releases the hold.
        store
            .record_search_diagnostics(
                &background_run(
                    "run-4",
                    "session-2",
                    "ids",
                    Some(SEARCH_STRATEGY_OPEN),
                    None,
                    now - chrono::Duration::minutes(1),
                ),
                &[],
            )
            .await
            .expect("run should persist");
        store
            .link_search_session_coverage_scope("session-2", scope)
            .await
            .expect("link should write");
        assert_eq!(
            store
                .contained_search_hold(scope, "idx-1", "fingerprint-1", lookback, now)
                .await
                .expect("hold should read"),
            None
        );
    }

    #[tokio::test]
    async fn sqlite_store_caps_text_learning_rows_per_title() {
        let (store, _) = sqlite_store().await;
        assert_text_learning_cap(&store).await;
    }

    #[tokio::test]
    async fn sqlite_store_prunes_orphaned_learning_rows() {
        let (store, _) = sqlite_store().await;
        assert_orphan_learning_pruning(&store).await;
    }

    #[tokio::test]
    async fn postgres_store_caps_text_learning_and_prunes_orphans_from_env() -> AppResult<()> {
        let Some(database_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL search-learning cap test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .map_err(repo_err)?;
        for statement in [
            "CREATE TEMP TABLE indexer_search_learning (
                indexer_id TEXT NOT NULL,
                title_id TEXT NOT NULL,
                facet TEXT NOT NULL,
                strategy_key TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                empty_successes INTEGER NOT NULL DEFAULT 0,
                usable_successes INTEGER NOT NULL DEFAULT 0,
                last_attempt_at TIMESTAMPTZ,
                last_usable_at TIMESTAMPTZ,
                suppressed BOOLEAN NOT NULL DEFAULT FALSE,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (indexer_id, title_id, facet, strategy_key)
            ) ON COMMIT PRESERVE ROWS",
            "CREATE TEMP TABLE titles (id TEXT PRIMARY KEY) ON COMMIT PRESERVE ROWS",
            "CREATE TEMP TABLE indexers (id TEXT PRIMARY KEY) ON COMMIT PRESERVE ROWS",
            POSTGRES_TEST_FACET_LEARNING_TABLE,
        ] {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(repo_err)?;
        }
        let store = IndexerSearchLearningStore::new(
            StoreDatastore::Postgres { pool: pool.clone() },
            std::sync::Arc::new(std::sync::RwLock::new(None)),
        );
        assert_text_learning_cap(&store).await;
        sqlx::query("DELETE FROM indexer_search_learning")
            .execute(&pool)
            .await
            .map_err(repo_err)?;
        assert_orphan_learning_pruning(&store).await;
        Ok(())
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
            strategy_state: None,
            page_cursor: None,
            cursor_chain_id: None,
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
                release_listing_json: Some(r#"{"v":1,"extra":{"protocol":"usenet"}}"#.into()),
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
                .list_search_run_candidates(std::slice::from_ref(&run.id), None, 100)
                .await
                .expect("the persisted page should rehydrate")
                .candidates
                .len(),
            2
        );

        let first = store
            .list_search_run_candidates(std::slice::from_ref(&run.id), None, 1)
            .await
            .unwrap();
        assert_eq!(first.candidates.len(), 1);
        assert_eq!(
            first.candidates[0].reusable_until.timestamp(),
            candidate.reusable_until.timestamp()
        );
        let second = store
            .list_search_run_candidates(
                std::slice::from_ref(&run.id),
                first.next_cursor.as_deref(),
                1,
            )
            .await
            .unwrap();
        assert_eq!(second.candidates.len(), 1);
        assert_ne!(
            first.candidates[0].normalized.provider_ref,
            second.candidates[0].normalized.provider_ref
        );
        let last = store
            .list_search_run_candidates(
                std::slice::from_ref(&run.id),
                second.next_cursor.as_deref(),
                1,
            )
            .await
            .unwrap();
        assert!(last.candidates.is_empty());
        assert!(last.next_cursor.is_none());
        assert!(
            store
                .list_search_run_candidates(std::slice::from_ref(&run.id), None, 0)
                .await
                .unwrap()
                .candidates
                .is_empty()
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
                .list_search_run_candidates(std::slice::from_ref(&run.id), None, 100)
                .await
                .expect("rejected candidates should be removed")
                .candidates
                .len(),
            1
        );
        let rehydrated = store
            .list_search_run_candidates(std::slice::from_ref(&duplicate_run.id), None, 100)
            .await
            .expect("duplicate page candidates should rehydrate")
            .candidates;
        assert_eq!(
            rehydrated.len(),
            1,
            "each run observes the shared canonical source"
        );
        let chain = store
            .list_search_run_candidates(&[run.id.clone(), duplicate_run.id.clone()], None, 1)
            .await
            .unwrap();
        assert_eq!(chain.candidates.len(), 1);
        let end = store
            .list_search_run_candidates(
                &[run.id.clone(), duplicate_run.id.clone()],
                chain.next_cursor.as_deref(),
                1,
            )
            .await
            .unwrap();
        assert!(
            end.candidates.is_empty(),
            "a source shared by two runs is replayed once"
        );
        assert!(
            store
                .list_search_run_candidates(&[], None, 100)
                .await
                .unwrap()
                .candidates
                .is_empty()
        );
        assert!(
            store
                .list_search_run_candidates(&["missing-run".into()], None, 100)
                .await
                .unwrap()
                .candidates
                .is_empty()
        );

        assert_eq!(
            rehydrated[0].normalized.release_listing_json.as_deref(),
            Some(r#"{"v":1,"extra":{"protocol":"usenet"}}"#),
            "a replayed result keeps its listing facts"
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
        sqlx::query(POSTGRES_TEST_FACET_LEARNING_TABLE)
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
        // `cleanup_search_diagnostics` deletes candidate *sources* and lets the
        // orphan sweep take the candidates, so the sources table has to exist
        // here even though only its cutoff columns are exercised.
        sqlx::query(
            "CREATE TEMP TABLE indexer_search_candidate_sources (
                id TEXT PRIMARY KEY,
                candidate_id TEXT NOT NULL,
                first_seen_at TIMESTAMPTZ NOT NULL,
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
        // Cleanup also retires listing state; keep its production schema local
        // to this fixture's single connection.
        let listing_schema = include_str!(
            "../../../scryer/src/db/postgres/migrations/0314_indexer_search_listings.sql"
        )
        .replace("CREATE TABLE", "CREATE TEMP TABLE");
        sqlx::raw_sql(sqlx::AssertSqlSafe(listing_schema.as_str()))
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
            // One source per candidate, so deleting a source orphans exactly
            // one candidate and the orphan sweep has something to do.
            sqlx::query(
                "INSERT INTO indexer_search_candidate_sources
                    (id, candidate_id, first_seen_at, expires_at)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(format!("source-{ordinal}"))
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

        // Each sweep is limited to one source and one run, and reports those
        // two deletions. The candidate the source belonged to goes with it via
        // the orphan sweep, which is deliberately not counted.
        for remaining_candidates in [1_i64, 0] {
            assert_eq!(
                store
                    .cleanup_search_diagnostics(Utc::now(), Utc::now(), 1)
                    .await?,
                2,
                "one expired source plus one expired run are reported"
            );
            let candidates: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM indexer_search_candidates")
                    .fetch_one(&pool)
                    .await
                    .map_err(repo_err)?;
            assert_eq!(
                candidates, remaining_candidates,
                "a candidate whose last source was deleted must not survive as an orphan"
            );
        }
        assert_eq!(
            store
                .cleanup_search_diagnostics(Utc::now(), Utc::now(), 1)
                .await?,
            0
        );
        let sources: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM indexer_search_candidate_sources")
                .fetch_one(&pool)
                .await
                .map_err(repo_err)?;
        assert_eq!(sources, 0, "every expired source is eventually swept");

        Ok(())
    }

    pub(crate) fn listing_scope() -> IndexerSearchListingScope {
        IndexerSearchListingScope {
            indexer_id: "idx-1".into(),
            indexer_fingerprint: "fingerprint-1".into(),
            title_id: "title-1".into(),
            facet: "series".into(),
        }
    }

    fn listing_strategy(scope_key: &str, query_signature: &str) -> IndexerSearchListingStrategy {
        IndexerSearchListingStrategy {
            scope_key: scope_key.into(),
            query_signature: query_signature.into(),
        }
    }

    const LISTING_SCOPE_S1E1: &str = "title-1:series:episode:1:1:-";
    const LISTING_SCOPE_S1E2: &str = "title-1:series:episode:1:2:-";
    const LISTING_SCOPE_S2E1: &str = "title-1:series:episode:2:1:-";
    const LISTING_SCOPE_S2E2: &str = "title-1:series:episode:2:2:-";

    async fn claim(
        store: &IndexerSearchLearningStore,
        fingerprint: &str,
        claimant: &IndexerSearchListingStrategy,
        now: DateTime<Utc>,
    ) -> IndexerSearchListingClaim {
        store
            .claim_search_listing(
                &listing_scope(),
                fingerprint,
                claimant,
                "family-sxex",
                now - chrono::Duration::hours(24),
                now - chrono::Duration::hours(1),
                now,
            )
            .await
            .expect("claim should record")
    }

    async fn listings(store: &IndexerSearchLearningStore) -> Vec<IndexerSearchListing> {
        store
            .list_search_listings(&listing_scope(), Utc::now() - chrono::Duration::hours(24))
            .await
            .expect("listings should read")
    }

    #[tokio::test]
    async fn sqlite_store_records_readers_of_a_listing_another_strategy_owns() {
        let (store, _pool) = sqlite_store().await;
        let now = Utc::now();
        let owner = listing_strategy(LISTING_SCOPE_S1E1, "sig-e1");
        let reader = listing_strategy(LISTING_SCOPE_S1E2, "sig-e2");

        assert_eq!(
            claim(&store, "page-a", &owner, now).await,
            IndexerSearchListingClaim::Owner
        );
        assert_eq!(
            claim(&store, "page-a", &owner, now).await,
            IndexerSearchListingClaim::Owner,
            "the owner reading its own first page again keeps the listing"
        );
        assert_eq!(
            claim(&store, "page-a", &reader, now).await,
            IndexerSearchListingClaim::SameListing {
                owner: owner.clone(),
                listing_open: true,
            }
        );
        let found = listings(&store).await;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].owner, owner);
        assert_eq!(
            found[0]
                .readers
                .iter()
                .map(|reader| reader.strategy.clone())
                .collect::<Vec<_>>(),
            vec![reader.clone()]
        );

        // Only the owner advances the chain.
        let scope = listing_scope();
        assert!(
            !store
                .link_search_listing_run(&scope, "page-a", &reader, "run-x", Some("p2"), now)
                .await
                .unwrap()
        );
        assert!(
            store
                .link_search_listing_run(&scope, "page-a", &owner, "run-1", Some("p2"), now)
                .await
                .unwrap()
        );
        let found = listings(&store).await;
        assert_eq!(found[0].page_cursor.as_deref(), Some("p2"));
        assert!(!found[0].finished);
        assert_eq!(found[0].run_ids, vec!["run-1".to_string()]);
        assert!(
            store
                .link_search_listing_run(&scope, "page-a", &owner, "run-2", None, now)
                .await
                .unwrap()
        );
        let found = listings(&store).await;
        assert!(found[0].finished);
        assert_eq!(found[0].run_ids.len(), 2);
        assert_eq!(
            claim(
                &store,
                "page-a",
                &listing_strategy(LISTING_SCOPE_S2E1, "sig-s2"),
                now
            )
            .await,
            IndexerSearchListingClaim::SameListing {
                owner: owner.clone(),
                listing_open: false,
            },
            "a reader of a finished chain is answered in full"
        );

        // Another indexer identity never sees the listing.
        let mut other = listing_scope();
        other.indexer_fingerprint = "fingerprint-2".into();
        assert!(
            store
                .list_search_listings(&other, now - chrono::Duration::hours(24))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn sqlite_store_keeps_one_listing_per_first_page() {
        let (store, _pool) = sqlite_store().await;
        let now = Utc::now();
        let season_one = listing_strategy(LISTING_SCOPE_S1E1, "sig-s1e1");
        let season_two = listing_strategy(LISTING_SCOPE_S2E1, "sig-s2e1");
        claim(&store, "page-s1", &season_one, now).await;
        claim(&store, "page-s2", &season_two, now).await;
        claim(
            &store,
            "page-s1",
            &listing_strategy(LISTING_SCOPE_S1E2, "sig-s1e2"),
            now,
        )
        .await;
        claim(
            &store,
            "page-s2",
            &listing_strategy(LISTING_SCOPE_S2E2, "sig-s2e2"),
            now,
        )
        .await;

        let found = listings(&store).await;
        let owners = found
            .iter()
            .map(|listing| {
                (
                    listing.first_page_fingerprint.clone(),
                    listing.owner.clone(),
                    listing.readers.len(),
                )
            })
            .collect::<HashSet<_>>();
        assert_eq!(
            owners,
            HashSet::from([
                ("page-s1".to_string(), season_one.clone(), 1),
                ("page-s2".to_string(), season_two.clone(), 1),
            ]),
            "a season-sensitive indexer keeps a listing per season"
        );

        // An owner whose first page changed no longer owns the old listing.
        claim(&store, "page-s1-new", &season_one, now).await;
        let pages = listings(&store)
            .await
            .into_iter()
            .map(|listing| listing.first_page_fingerprint)
            .collect::<HashSet<_>>();
        assert_eq!(
            pages,
            HashSet::from(["page-s1-new".to_string(), "page-s2".to_string()])
        );
    }

    #[tokio::test]
    async fn sqlite_store_listing_expires_with_the_reuse_window() {
        let (store, _pool) = sqlite_store().await;
        let now = Utc::now();
        let stale = now - chrono::Duration::hours(25);
        let owner = listing_strategy(LISTING_SCOPE_S1E1, "sig-e1");
        claim(&store, "page-a", &owner, stale).await;
        assert!(listings(&store).await.is_empty());
        assert_eq!(
            claim(
                &store,
                "page-a",
                &listing_strategy(LISTING_SCOPE_S1E2, "sig-e2"),
                now
            )
            .await,
            IndexerSearchListingClaim::Owner,
            "an expired listing is read afresh by whoever reads it next"
        );
    }

    #[tokio::test]
    async fn sqlite_store_hands_an_idle_chain_to_a_reader() {
        let (store, _pool) = sqlite_store().await;
        let now = Utc::now();
        let earlier = now - chrono::Duration::hours(2);
        let idle_before = now - chrono::Duration::hours(1);
        let scope = listing_scope();
        let owner = listing_strategy(LISTING_SCOPE_S1E1, "sig-e1");
        let reader = listing_strategy(LISTING_SCOPE_S1E2, "sig-e2");
        claim(&store, "page-a", &owner, earlier).await;
        store
            .link_search_listing_run(&scope, "page-a", &owner, "run-1", Some("p3"), earlier)
            .await
            .unwrap();
        claim(&store, "page-a", &reader, earlier).await;

        assert!(
            !store
                .adopt_search_listing(&scope, "page-a", &owner, &reader, "p2", idle_before, now)
                .await
                .unwrap(),
            "a chain that moved since it was read is not taken over"
        );
        assert!(
            !store
                .adopt_search_listing(&scope, "page-a", &owner, &reader, "p3", earlier, now)
                .await
                .unwrap(),
            "a chain still moving is not taken over"
        );
        assert!(
            store
                .adopt_search_listing(&scope, "page-a", &owner, &reader, "p3", idle_before, now)
                .await
                .unwrap()
        );
        let found = listings(&store).await;
        assert_eq!(found[0].owner, reader);
        assert_eq!(found[0].page_cursor.as_deref(), Some("p3"));
        assert_eq!(
            found[0]
                .readers
                .iter()
                .map(|reader| reader.strategy.clone())
                .collect::<Vec<_>>(),
            vec![owner.clone()],
            "the strategy that left the chain reads it from here on"
        );
        assert!(
            !store
                .link_search_listing_run(&scope, "page-a", &owner, "run-x", None, now)
                .await
                .unwrap()
        );
        assert!(
            store
                .link_search_listing_run(&scope, "page-a", &reader, "run-2", Some("p4"), now)
                .await
                .unwrap()
        );
        assert_eq!(
            listings(&store).await[0].run_ids,
            vec!["run-1".to_string(), "run-2".to_string()],
            "the chain keeps the pages its first owner read"
        );

        // A chain that claimed its first page and recorded nothing is read
        // again from page one by the next strategy that reaches it.
        let abandoned_owner = listing_strategy(LISTING_SCOPE_S2E1, "sig-s2e1");
        claim(&store, "page-b", &abandoned_owner, earlier).await;
        assert_eq!(
            claim(
                &store,
                "page-b",
                &listing_strategy(LISTING_SCOPE_S2E2, "sig-s2e2"),
                now
            )
            .await,
            IndexerSearchListingClaim::Owner
        );
    }

    pub(crate) fn listing_candidate(
        run: &IndexerSearchRunWrite,
        reference: &str,
        now: DateTime<Utc>,
    ) -> IndexerSearchCandidateWrite {
        IndexerSearchCandidateWrite {
            id: format!("candidate-{reference}"),
            run_id: run.id.clone(),
            search_session_id: run.search_session_id.clone(),
            indexer_id: run.indexer_id.clone(),
            scope_key: run.scope_key.clone(),
            query_signature: run.query_signature.clone(),
            session_identity_hash: format!("hash-{reference}"),
            normalized: NormalizedIndexerSearchCandidate {
                provider_ref: Some(reference.into()),
                source: "newznab".into(),
                title: format!("Synthetic.Show.{reference}.1080p"),
                download_url: Some(format!("https://example.invalid/get?id={reference}")),
                download_url_credential_keys: Vec::new(),
                link_url: None,
                link_url_credential_keys: Vec::new(),
                size_bytes: Some(123),
                published_at: None,
                source_kind: Some("nzb_url".into()),
                thumbs_up: None,
                thumbs_down: None,
                grabs: None,
                grab_current: None,
                grab_max: None,
                languages: Vec::new(),
                subtitles: Vec::new(),
                response_tvdb_id: None,
                response_tmdb_id: None,
                response_imdb_id: None,
                response_categories: Vec::new(),
                extra_categories: Vec::new(),
                season: None,
                episode: None,
                absolute_episode: None,
                series_names: Vec::new(),
                release_group: None,
                provider_source: None,
                info_hash: None,
                seeders: None,
                peers: None,
                download_volume_factor: None,
                upload_volume_factor: None,
                protected: None,
                tags: Vec::new(),
                provider_categories: Vec::new(),
                release_listing_json: None,
            },
            created_at: now,
            reusable_until: now + chrono::Duration::hours(24),
            expires_at: now + chrono::Duration::days(1),
        }
    }

    #[tokio::test]
    async fn sqlite_store_finalize_keeps_every_result_of_a_listing_run() {
        let (store, pool) = sqlite_store().await;
        let now = Utc::now();
        let mut owner_run = background_run(
            "run-owner",
            "session-1",
            "sig-owner",
            Some(SEARCH_STRATEGY_OPEN),
            None,
            now,
        );
        owner_run.scope_key = LISTING_SCOPE_S1E1.into();
        let mut other_run = background_run(
            "run-other",
            "session-1",
            "sig-other",
            Some(SEARCH_STRATEGY_OPEN),
            None,
            now,
        );
        other_run.scope_key = LISTING_SCOPE_S1E1.into();
        store
            .record_search_diagnostics(
                &owner_run,
                &[
                    listing_candidate(&owner_run, "E01", now),
                    listing_candidate(&owner_run, "E02", now),
                ],
            )
            .await
            .unwrap();
        store
            .record_search_diagnostics(
                &other_run,
                &[
                    listing_candidate(&other_run, "E03", now),
                    listing_candidate(&other_run, "E04", now),
                ],
            )
            .await
            .unwrap();
        let owner = listing_strategy(LISTING_SCOPE_S1E1, "sig-owner");
        claim(&store, "page-a", &owner, now).await;
        store
            .link_search_listing_run(&listing_scope(), "page-a", &owner, "run-owner", None, now)
            .await
            .unwrap();
        let admissible = vec!["hash-E01".to_string(), "hash-E03".to_string()];
        store
            .finalize_search_session("session-1", &admissible)
            .await
            .unwrap();

        let kept = |run_id: &str| {
            let store = &store;
            let run_id = run_id.to_string();
            async move {
                store
                    .list_search_run_candidates(std::slice::from_ref(&run_id), None, 100)
                    .await
                    .unwrap()
                    .candidates
                    .len()
            }
        };
        assert_eq!(
            kept("run-owner").await,
            2,
            "a listing run keeps the whole listing"
        );
        assert_eq!(
            kept("run-other").await,
            1,
            "other runs keep what their scope admitted"
        );

        // The listing exemption is a primary-key probe per link, not a scan.
        let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
            "EXPLAIN QUERY PLAN
             DELETE FROM indexer_search_run_candidate_sources
              WHERE search_session_id = ?
                AND source_id IN (
                    SELECT s.id
                      FROM indexer_search_candidate_sources s
                      JOIN indexer_search_candidates c ON c.id = s.candidate_id
                     WHERE 1 = 1 AND c.fingerprint NOT IN (?)
                )
                AND NOT EXISTS (
                    SELECT 1 FROM indexer_search_listing_runs lr
                     WHERE lr.run_id = indexer_search_run_candidate_sources.run_id
                )",
        )
        .bind("session-1")
        .bind("hash-E01")
        .fetch_all(&pool)
        .await
        .expect("query plan should explain");
        let listing_steps = plan
            .iter()
            .map(|(_, _, _, detail)| detail.as_str())
            .filter(|detail| detail.contains("lr"))
            .collect::<Vec<_>>();
        assert!(!listing_steps.is_empty(), "plan: {plan:?}");
        assert!(
            listing_steps.iter().all(|detail| detail.contains("INDEX")),
            "plan: {plan:?}"
        );
    }
}

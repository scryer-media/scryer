use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use scryer_application::{
    AcquisitionScopeState, AcquisitionScopeStateRepository, AcquisitionScopeStatesQuery,
    AcquisitionScopeStatus, AppResult, BlocklistRepository, DownloadSourceKind,
    HousekeepingMediaFileRootRow, HousekeepingRepository, LibraryProbeRepository,
    LibraryProbeSignature, NewBlocklistEntry, PendingRelease, PendingReleaseObservation,
    PendingReleasePageSort, PendingReleaseRepository, PendingReleaseRole, PendingReleaseStatus,
    PendingReleasesPageQuery, ReleaseDecision, ReleaseSeedMinimums, SubtitleDownloadRepository,
    normalize_release_name,
    subtitles::{ExternalSubtitleDetectionSource, ExternalSubtitleProbeCacheEntry},
};
use scryer_domain::{
    BlocklistEntry, DomainEventType, ExternalSubtitleSourceKind, Id, SubtitleBlocklistEntry,
    SubtitleDownload,
};
use scryer_plugin_sdk::torrent::normalize_info_hash;

use crate::config_store::{current_encryption_key, decrypt_optional_value, encrypt_optional_value};
use crate::encryption::{EncryptionKey, is_encrypted};
use crate::queries::common::parse_utc_datetime;
use crate::queries::sql_runtime::repo_err;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

use super::{decode_release_decision_explanation, encode_release_decision_explanation};

// Throttle the SQLite full VACUUM so it does not run on every
// daily maintenance tick. `auto_vacuum` is not enabled on the connection, so
// `PRAGMA incremental_vacuum` would be a no-op; instead gate a full VACUUM on
// the free-page ratio (bloat left behind by discovery prune + raw-page removal).
// VACUUM only when free pages are a meaningful fraction of the file AND exceed a
// small absolute floor (skip trivially small databases).
const SQLITE_VACUUM_MIN_FREELIST_FRACTION: f64 = 0.10;
const SQLITE_VACUUM_MIN_FREELIST_PAGES: i64 = 2_000;

fn validate_sqlite_checkpoint_result(
    busy: i64,
    log_frames: i64,
    checkpointed_frames: i64,
) -> AppResult<()> {
    if busy == 0 && (log_frames < 0 || checkpointed_frames >= log_frames) {
        return Ok(());
    }

    Err(scryer_application::AppError::Repository(format!(
        "sqlite WAL checkpoint incomplete: busy={busy}, log_frames={log_frames}, checkpointed_frames={checkpointed_frames}"
    )))
}

#[cfg(test)]
mod sqlite_checkpoint_tests {
    use super::validate_sqlite_checkpoint_result;

    #[test]
    fn checkpoint_result_requires_no_busy_or_uncheckpointed_frames() {
        assert!(validate_sqlite_checkpoint_result(0, 0, 0).is_ok());
        assert!(validate_sqlite_checkpoint_result(0, -1, -1).is_ok());
        assert!(validate_sqlite_checkpoint_result(1, 10, 10).is_err());
        assert!(validate_sqlite_checkpoint_result(0, 10, 9).is_err());
    }
}

const LIBRARY_PROBE_COLUMNS: &str = "title_id, path, probe_signature_scheme, probe_signature_value, last_probed_at, last_changed_at";

const UPSERT_LIBRARY_PROBE_SIGNATURE_SQL: &str = "INSERT INTO library_probe_signatures (
    title_id, path, probe_signature_scheme, probe_signature_value, last_probed_at, last_changed_at,
    created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}
)
ON CONFLICT(title_id) DO UPDATE SET
    path = excluded.path,
    probe_signature_scheme = excluded.probe_signature_scheme,
    probe_signature_value = excluded.probe_signature_value,
    last_probed_at = excluded.last_probed_at,
    last_changed_at = excluded.last_changed_at,
    updated_at = excluded.updated_at";

fn library_probe_signature_from_row(row: &SqlRow) -> AppResult<LibraryProbeSignature> {
    Ok(LibraryProbeSignature {
        title_id: row.text("title_id")?,
        path: row.text("path")?,
        probe_signature_scheme: row.opt_text("probe_signature_scheme")?,
        probe_signature_value: row.opt_text("probe_signature_value")?,
        last_probed_at: row.opt_timestamp("last_probed_at")?,
        last_changed_at: row.opt_timestamp("last_changed_at")?,
    })
}

#[derive(Clone)]
pub struct LibraryProbeStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct WantedStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct PendingReleaseStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

#[derive(Clone)]
pub struct BlocklistStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct SubtitleDownloadStore {
    datastore: StoreDatastore,
}

#[derive(Clone)]
pub struct HousekeepingStore {
    datastore: StoreDatastore,
}

macro_rules! impl_store_new {
    ($store:ident) => {
        impl $store {
            pub fn new(datastore: StoreDatastore) -> Self {
                Self { datastore }
            }
        }
    };
}

impl_store_new!(LibraryProbeStore);
impl_store_new!(WantedStore);
impl_store_new!(BlocklistStore);
impl_store_new!(SubtitleDownloadStore);
impl_store_new!(HousekeepingStore);

impl PendingReleaseStore {
    pub fn new(
        datastore: StoreDatastore,
        encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
    ) -> Self {
        Self {
            datastore,
            encryption_key,
        }
    }

    pub async fn backfill_source_passwords(&self) -> AppResult<u64> {
        let encryption_key = self.encryption_key()?;
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, source_password FROM pending_releases WHERE source_password IS NOT NULL",
            &[],
        )
        .await?;

        let mut updates = Vec::new();
        for row in rows {
            let stored = row.text("source_password")?;
            if is_encrypted(&stored) {
                continue;
            }

            let encrypted =
                encrypt_pending_release_source_password(encryption_key.as_ref(), Some(&stored))?
                    .expect("non-null source_password should encrypt to non-null value");
            updates.push((row.text("id")?, encrypted));
        }

        if updates.is_empty() {
            return Ok(0);
        }

        let update_count = updates.len() as u64;
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "backfill_pending_release_source_passwords",
            move |tx| {
                let updates = updates.clone();
                Box::pin(async move {
                    for (id, encrypted) in updates {
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "UPDATE pending_releases
                             SET source_password = {}
                             WHERE id = {}",
                            &[SqlArg::Text(encrypted), SqlArg::Text(id)],
                        )
                        .await?;
                    }
                    Ok(update_count)
                })
            },
        )
        .await
    }

    fn encryption_key(&self) -> AppResult<Option<EncryptionKey>> {
        current_encryption_key(&self.encryption_key)
    }
}

#[async_trait]
impl LibraryProbeRepository for LibraryProbeStore {
    async fn get_probe_signature(
        &self,
        title_id: &str,
    ) -> AppResult<Option<LibraryProbeSignature>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {LIBRARY_PROBE_COLUMNS} FROM library_probe_signatures WHERE title_id = {{}}"
            ),
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?;

        row.as_ref()
            .map(library_probe_signature_from_row)
            .transpose()
    }

    async fn upsert_probe_signature(&self, probe: &LibraryProbeSignature) -> AppResult<()> {
        let now = Utc::now();
        let args = vec![
            SqlArg::Text(probe.title_id.clone()),
            SqlArg::Text(probe.path.clone()),
            SqlArg::OptText(probe.probe_signature_scheme.clone()),
            SqlArg::OptText(probe.probe_signature_value.clone()),
            SqlArg::OptTimestamp(probe.last_probed_at),
            SqlArg::OptTimestamp(probe.last_changed_at),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ];

        SqlRuntime::run_in_transaction(
            &self.datastore,
            "upsert_library_probe_signature",
            move |tx| {
                let args = args.clone();
                Box::pin(async move {
                    SqlRuntime::execute(SqlExec::Tx(tx), UPSERT_LIBRARY_PROBE_SIGNATURE_SQL, &args)
                        .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn delete_probe_signatures_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        if title_ids.is_empty() {
            return Ok(0);
        }

        let sql = format!(
            "DELETE FROM library_probe_signatures WHERE title_id IN ({})",
            vec!["{}"; title_ids.len()].join(", ")
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();

        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_library_probe_signatures_for_title_ids",
            move |tx| {
                let sql = sql.clone();
                let args = args.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await?;
                    Ok(rows as u32)
                })
            },
        )
        .await
    }
}

fn timestamp_arg_for_datastore(datastore: &StoreDatastore, value: &str) -> AppResult<SqlArg> {
    match datastore {
        StoreDatastore::Sqlite { .. } => Ok(SqlArg::Text(value.to_string())),
        StoreDatastore::Postgres { .. } => parse_utc_datetime(value).map(SqlArg::Timestamp),
    }
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

fn opt_json_arg_for_datastore(
    datastore: &StoreDatastore,
    value: Option<&str>,
) -> AppResult<SqlArg> {
    match datastore {
        StoreDatastore::Sqlite { .. } => Ok(SqlArg::OptText(value.map(str::to_string))),
        StoreDatastore::Postgres { .. } => value
            .map(serde_json::from_str)
            .transpose()
            .map(SqlArg::OptJson)
            .map_err(repo_err),
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

fn required_timestamp_text(row: &SqlRow, column: &str) -> AppResult<String> {
    match row {
        SqlRow::Sqlite(_) => row.text(column),
        SqlRow::Postgres(_) => row.timestamp(column).map(|value| value.to_rfc3339()),
    }
}

fn wanted_seed_row_to_item(row: &SqlRow) -> AppResult<AcquisitionScopeState> {
    let status = row.text("status")?;
    Ok(AcquisitionScopeState {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        title_name: None,
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: row.opt_text("episode_id")?,
        collection_id: row.opt_text("collection_id")?,
        series_movie_link_id: row.opt_text("series_movie_link_id")?,
        season_number: None,
        episode_number: None,
        media_type: row.text("media_type")?,
        last_search_at: opt_timestamp_text(row, "last_search_at")?,
        status: AcquisitionScopeStatus::parse(&status).unwrap_or_default(),
        grabbed_release: row.opt_text("grabbed_release")?,
        // Resolved from the library by the caller, never stored.
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: required_timestamp_text(row, "created_at")?,
        updated_at: required_timestamp_text(row, "updated_at")?,
    })
}

fn release_decision_row_to_item(row: &SqlRow) -> AppResult<ReleaseDecision> {
    let id = row.text("id")?;
    Ok(ReleaseDecision {
        explanation_json: release_decision_explanation_from_row(row, "explanation_json", &id)?,
        id,
        wanted_item_id: row.text("wanted_item_id")?,
        title_id: row.text("title_id")?,
        release_title: row.text("release_title")?,
        release_url: row.opt_text("release_url")?,
        release_size_bytes: row.opt_i64("release_size_bytes")?,
        decision_code: row.text("decision_code")?,
        candidate_score: row.i32("candidate_score")?,
        current_score: row.opt_i32("current_score")?,
        score_delta: row.opt_i32("score_delta")?,
        created_at: required_timestamp_text(row, "created_at")?,
    })
}

fn release_decision_explanation_from_row(
    row: &SqlRow,
    column: &str,
    decision_id: &str,
) -> AppResult<Option<String>> {
    let encoded = row.opt_bytes(column)?;
    match decode_release_decision_explanation(encoded.as_deref()) {
        Ok(explanation) => Ok(explanation),
        Err(error) => {
            tracing::warn!(
                decision_id,
                error = %error,
                "release decision explanation could not be decoded"
            );
            Ok(None)
        }
    }
}

fn json_text_from_row(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(_) => row.opt_text(column),
        SqlRow::Postgres(_) => row
            .opt_json(column)
            .map(|value| value.map(|json| json.to_string())),
    }
}

fn wanted_row_to_item(row: &SqlRow) -> AppResult<AcquisitionScopeState> {
    let latest_release_decision = match row.opt_text("latest_decision_id")? {
        Some(id) => Some(ReleaseDecision {
            explanation_json: release_decision_explanation_from_row(
                row,
                "latest_decision_explanation_json",
                &id,
            )?,
            id,
            wanted_item_id: row.text("latest_decision_wanted_item_id")?,
            title_id: row.text("latest_decision_title_id")?,
            release_title: row.text("latest_decision_release_title")?,
            release_url: row.opt_text("latest_decision_release_url")?,
            release_size_bytes: row.opt_i64("latest_decision_release_size_bytes")?,
            decision_code: row.text("latest_decision_decision_code")?,
            candidate_score: row
                .opt_i32("latest_decision_candidate_score")?
                .unwrap_or_default(),
            current_score: row.opt_i32("latest_decision_current_score")?,
            score_delta: row.opt_i32("latest_decision_score_delta")?,
            created_at: required_timestamp_text(row, "latest_decision_created_at")?,
        }),
        None => None,
    };

    let status = row.text("status")?;
    Ok(AcquisitionScopeState {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        title_name: row.opt_text("title_name")?,
        title_slug: row.opt_text("title_slug")?,
        title_facet: row.opt_text("title_facet")?,
        library_id: row.opt_text("library_id")?,
        library_name: row.opt_text("library_name")?,
        library_slug: row.opt_text("library_slug")?,
        episode_id: row.opt_text("episode_id")?,
        collection_id: row.opt_text("collection_id")?,
        series_movie_link_id: row.opt_text("series_movie_link_id")?,
        season_number: row.opt_text("season_number")?,
        episode_number: row.opt_text("episode_number")?,
        media_type: row.text("media_type")?,
        last_search_at: opt_timestamp_text(row, "last_search_at")?,
        status: AcquisitionScopeStatus::parse(&status).unwrap_or_default(),
        grabbed_release: row.opt_text("grabbed_release")?,
        // Resolved from the library by the caller, never stored.
        landed_bar: None,
        latest_release_decision,
        mismatch_recovery_eligible: row.bool("mismatch_recovery_eligible")?,
        created_at: required_timestamp_text(row, "created_at")?,
        updated_at: required_timestamp_text(row, "updated_at")?,
    })
}

fn wanted_item_select_sql() -> &'static str {
    "SELECT w.id, w.title_id, t.name AS title_name, t.slug AS title_slug,
            t.facet AS title_facet, t.library_id AS library_id,
            libraries.name AS library_name, libraries.slug AS library_slug,
            w.episode_id, w.collection_id, w.series_movie_link_id,
            e.season_number, e.episode_number, w.media_type,
            w.last_search_at, w.status, w.grabbed_release,
            latest_decision.id AS latest_decision_id,
            latest_decision.wanted_item_id AS latest_decision_wanted_item_id,
            latest_decision.title_id AS latest_decision_title_id,
            latest_decision.release_title AS latest_decision_release_title,
            latest_decision.release_url AS latest_decision_release_url,
            latest_decision.release_size_bytes AS latest_decision_release_size_bytes,
            latest_decision.decision_code AS latest_decision_decision_code,
            latest_decision.candidate_score AS latest_decision_candidate_score,
            latest_decision.current_score AS latest_decision_current_score,
            latest_decision.score_delta AS latest_decision_score_delta,
            latest_decision.explanation_json AS latest_decision_explanation_json,
            latest_decision.created_at AS latest_decision_created_at,
            CASE
                WHEN w.status = 'wanted'
                 AND EXISTS (
                     SELECT 1
                       FROM release_decisions mismatch_any
                      WHERE mismatch_any.wanted_item_id = w.id
                 )
                 AND NOT EXISTS (
                     SELECT 1
                       FROM release_decisions mismatch_other
                      WHERE mismatch_other.wanted_item_id = w.id
                        AND mismatch_other.decision_code <> 'title_mismatch'
                 )
                THEN TRUE
                ELSE FALSE
            END AS mismatch_recovery_eligible,
            w.created_at, w.updated_at
       FROM wanted_items w
       LEFT JOIN titles t ON t.id = w.title_id
       LEFT JOIN libraries ON libraries.id = t.library_id
       LEFT JOIN episodes e ON e.id = w.episode_id
       LEFT JOIN release_decisions latest_decision ON latest_decision.id = (
           SELECT rd.id
             FROM release_decisions rd
            WHERE rd.wanted_item_id = w.id
            ORDER BY rd.created_at DESC
            LIMIT 1
       )"
}

fn append_in_filter(sql: &mut String, args: &mut Vec<SqlArg>, column: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }

    sql.push_str(" AND ");
    sql.push_str(column);
    sql.push_str(" IN (");
    sql.push_str(&placeholders(values.len()));
    sql.push(')');
    args.extend(values.iter().cloned().map(SqlArg::Text));
}

fn append_wanted_query_filters(
    sql: &mut String,
    args: &mut Vec<SqlArg>,
    query: &AcquisitionScopeStatesQuery,
    include_title_search: bool,
) {
    append_in_filter(sql, args, "w.status", &query.statuses);
    append_in_filter(sql, args, "w.media_type", &query.media_types);
    if let Some(title_id) = query.title_id.as_deref() {
        sql.push_str(" AND w.title_id = {}");
        args.push(SqlArg::Text(title_id.to_string()));
    }
    append_in_filter(sql, args, "t.library_id", &query.library_ids);
    if include_title_search
        && let Some(normalized) = query
            .title_search
            .as_deref()
            .map(crate::queries::title_search::normalize_title_search_text)
            .filter(|value| !value.is_empty())
    {
        sql.push_str(
            " AND EXISTS (
                SELECT 1
                  FROM title_search_terms wanted_title_search
                 WHERE wanted_title_search.title_id = w.title_id
                   AND wanted_title_search.term_kind NOT LIKE '%_token'
                   AND (
                        wanted_title_search.normalized_term = {}
                        OR wanted_title_search.normalized_term LIKE {}
                        OR wanted_title_search.normalized_term LIKE {}
                   )
            )",
        );
        args.push(SqlArg::Text(normalized.clone()));
        args.push(SqlArg::Text(format!("{normalized}%")));
        args.push(SqlArg::Text(format!("%{normalized}%")));
    }
    append_in_filter(
        sql,
        args,
        "latest_decision.decision_code",
        &query.latest_decision_codes,
    );
}

fn sqlite_title_search_requires_spellfix(query: &AcquisitionScopeStatesQuery) -> bool {
    query
        .title_search
        .as_deref()
        .map(crate::queries::title_search::normalize_title_search_text)
        .is_some_and(|value| !value.is_empty())
}

fn wanted_upsert_sql(datastore: &StoreDatastore, item: &AcquisitionScopeState) -> String {
    let conflict_target = if item.series_movie_link_id.is_some() {
        "(series_movie_link_id) WHERE series_movie_link_id IS NOT NULL"
    } else if item.collection_id.is_some() {
        "(collection_id) WHERE collection_id IS NOT NULL"
    } else if item.episode_id.is_some() {
        match datastore {
            StoreDatastore::Sqlite { .. } => "(title_id, episode_id)",
            StoreDatastore::Postgres { .. } => {
                "(title_id, episode_id) WHERE episode_id IS NOT NULL"
            }
        }
    } else {
        "(title_id) WHERE episode_id IS NULL AND collection_id IS NULL AND series_movie_link_id IS NULL"
    };

    format!(
        "INSERT INTO wanted_items
         (id, title_id, episode_id, collection_id, series_movie_link_id, media_type,
          last_search_at, status, grabbed_release,
          created_at, updated_at)
         VALUES ({{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}})
         ON CONFLICT{conflict_target} DO UPDATE SET
            status = CASE
                WHEN wanted_items.status IN ('completed', 'paused') AND excluded.status = 'wanted'
                THEN wanted_items.status
                ELSE excluded.status
            END,
            updated_at = excluded.updated_at"
    )
}

fn wanted_upsert_args(
    datastore: &StoreDatastore,
    item: &AcquisitionScopeState,
) -> AppResult<Vec<SqlArg>> {
    let now = Utc::now().to_rfc3339();
    Ok(vec![
        SqlArg::Text(item.id.clone()),
        SqlArg::Text(item.title_id.clone()),
        SqlArg::OptText(item.episode_id.clone()),
        SqlArg::OptText(item.collection_id.clone()),
        SqlArg::OptText(item.series_movie_link_id.clone()),
        SqlArg::Text(item.media_type.clone()),
        opt_timestamp_arg_for_datastore(datastore, item.last_search_at.as_deref())?,
        SqlArg::Text(item.status.as_str().to_string()),
        SqlArg::OptText(item.grabbed_release.clone()),
        timestamp_arg_for_datastore(datastore, &now)?,
        timestamp_arg_for_datastore(datastore, &now)?,
    ])
}

async fn execute_wanted_upsert_tx(
    tx: &mut crate::queries::sql_runtime::SqlTx<'_>,
    datastore: &StoreDatastore,
    item: &AcquisitionScopeState,
) -> AppResult<String> {
    let sql = wanted_upsert_sql(datastore, item);
    let args = wanted_upsert_args(datastore, item)?;
    SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await?;
    Ok(item.id.clone())
}

async fn execute_datastore_write(
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

async fn fetch_seed_target_tx(
    tx: &mut crate::queries::sql_runtime::SqlTx<'_>,
    item: &AcquisitionScopeState,
) -> AppResult<Option<AcquisitionScopeState>> {
    let columns =
        "SELECT id, title_id, episode_id, collection_id, series_movie_link_id, media_type,
                          last_search_at, status,
                          grabbed_release, created_at, updated_at
                     FROM wanted_items";
    let (sql, args) = if let Some(collection_id) = item.collection_id.as_deref() {
        (
            format!("{columns} WHERE title_id = {{}} AND collection_id = {{}}"),
            vec![
                SqlArg::Text(item.title_id.clone()),
                SqlArg::Text(collection_id.to_string()),
            ],
        )
    } else if let Some(episode_id) = item.episode_id.as_deref() {
        (
            format!("{columns} WHERE title_id = {{}} AND episode_id = {{}}"),
            vec![
                SqlArg::Text(item.title_id.clone()),
                SqlArg::Text(episode_id.to_string()),
            ],
        )
    } else if let Some(series_movie_link_id) = item.series_movie_link_id.as_deref() {
        (
            format!("{columns} WHERE title_id = {{}} AND series_movie_link_id = {{}}"),
            vec![
                SqlArg::Text(item.title_id.clone()),
                SqlArg::Text(series_movie_link_id.to_string()),
            ],
        )
    } else {
        (
            format!(
                "{columns} WHERE title_id = {{}} AND episode_id IS NULL AND collection_id IS NULL AND series_movie_link_id IS NULL"
            ),
            vec![SqlArg::Text(item.title_id.clone())],
        )
    };

    SqlRuntime::fetch_optional(SqlExec::Tx(tx), &sql, &args)
        .await?
        .as_ref()
        .map(wanted_seed_row_to_item)
        .transpose()
}

#[async_trait]
impl AcquisitionScopeStateRepository for WantedStore {
    async fn upsert_acquisition_scope_state(
        &self,
        item: &AcquisitionScopeState,
    ) -> AppResult<String> {
        let item = item.clone();
        let datastore = self.datastore.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "upsert_acquisition_scope_state",
            move |tx| {
                let datastore = datastore.clone();
                let item = item.clone();
                Box::pin(async move { execute_wanted_upsert_tx(tx, &datastore, &item).await })
            },
        )
        .await
    }

    async fn ensure_acquisition_scope_state(
        &self,
        item: &AcquisitionScopeState,
    ) -> AppResult<String> {
        let item = item.clone();
        let datastore = self.datastore.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "ensure_acquisition_scope_state",
            move |tx| {
                let datastore = datastore.clone();
                let item = item.clone();
                Box::pin(async move {
                    if let Some(existing) = fetch_seed_target_tx(tx, &item).await? {
                        return Ok(existing.id);
                    }
                    execute_wanted_upsert_tx(tx, &datastore, &item).await?;
                    Ok(item.id.clone())
                })
            },
        )
        .await
    }

    async fn update_acquisition_scope_status(
        &self,
        id: &str,
        status: &str,
        last_search_at: Option<&str>,
        grabbed_release: Option<&str>,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        execute_datastore_write(
            &self.datastore,
            "update_acquisition_scope_status",
            "UPDATE wanted_items
                SET status = {},
                    last_search_at = {},
                    grabbed_release = {},
                    updated_at = {}
              WHERE id = {}",
            vec![
                SqlArg::Text(status.to_string()),
                opt_timestamp_arg_for_datastore(&self.datastore, last_search_at)?,
                SqlArg::OptText(grabbed_release.map(str::to_string)),
                timestamp_arg_for_datastore(&self.datastore, &now)?,
                SqlArg::Text(id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn record_acquisition_scope_search_attempt(
        &self,
        id: &str,
        last_search_at: &str,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        execute_datastore_write(
            &self.datastore,
            "record_acquisition_scope_search_attempt",
            "UPDATE wanted_items
                SET last_search_at = {},
                    updated_at = {}
              WHERE id = {}",
            vec![
                timestamp_arg_for_datastore(&self.datastore, last_search_at)?,
                timestamp_arg_for_datastore(&self.datastore, &now)?,
                SqlArg::Text(id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn get_acquisition_scope_state_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        let (sql, args) = if let Some(episode_id) = episode_id {
            (
                format!(
                    "{} WHERE w.title_id = {{}} AND w.episode_id = {{}}",
                    wanted_item_select_sql()
                ),
                vec![
                    SqlArg::Text(title_id.to_string()),
                    SqlArg::Text(episode_id.to_string()),
                ],
            )
        } else {
            (
                format!(
                    "{} WHERE w.title_id = {{}} AND w.episode_id IS NULL AND w.collection_id IS NULL AND w.series_movie_link_id IS NULL",
                    wanted_item_select_sql()
                ),
                vec![SqlArg::Text(title_id.to_string())],
            )
        };
        SqlRuntime::fetch_optional(self.datastore.read_exec(), &sql, &args)
            .await?
            .as_ref()
            .map(wanted_row_to_item)
            .transpose()
    }

    async fn list_acquisition_scope_states_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = placeholders(title_ids.len());
        let sql = format!(
            "{} WHERE w.title_id IN ({placeholders})",
            wanted_item_select_sql()
        );
        let args = title_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(wanted_row_to_item)
            .collect()
    }

    async fn complete_acquisition_scope_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
        last_search_at: Option<&str>,
        landed_import: bool,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        let (sql, args) = if let Some(episode_id) = episode_id {
            (
                "UPDATE wanted_items
                    SET status = {},
                        last_search_at = {},
                        grabbed_release = CASE WHEN {} THEN NULL ELSE grabbed_release END,
                        updated_at = {}
                  WHERE title_id = {} AND episode_id = {}"
                    .to_string(),
                vec![
                    SqlArg::Text(AcquisitionScopeStatus::Completed.as_str().to_string()),
                    opt_timestamp_arg_for_datastore(&self.datastore, last_search_at)?,
                    SqlArg::Bool(landed_import),
                    timestamp_arg_for_datastore(&self.datastore, &now)?,
                    SqlArg::Text(title_id.to_string()),
                    SqlArg::Text(episode_id.to_string()),
                ],
            )
        } else {
            (
                "UPDATE wanted_items
                    SET status = {},
                        last_search_at = {},
                        grabbed_release = CASE WHEN {} THEN NULL ELSE grabbed_release END,
                        updated_at = {}
                  WHERE title_id = {}
                    AND episode_id IS NULL
                    AND collection_id IS NULL
                    AND series_movie_link_id IS NULL"
                    .to_string(),
                vec![
                    SqlArg::Text(AcquisitionScopeStatus::Completed.as_str().to_string()),
                    opt_timestamp_arg_for_datastore(&self.datastore, last_search_at)?,
                    SqlArg::Bool(landed_import),
                    timestamp_arg_for_datastore(&self.datastore, &now)?,
                    SqlArg::Text(title_id.to_string()),
                ],
            )
        };
        let rows = execute_datastore_write(
            &self.datastore,
            "complete_acquisition_scope_for_title",
            sql,
            args,
        )
        .await?;
        Ok(rows > 0)
    }

    async fn delete_acquisition_scope_states_for_title(&self, title_id: &str) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_acquisition_scope_states_for_title",
            "DELETE FROM wanted_items WHERE title_id = {}",
            vec![SqlArg::Text(title_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn delete_acquisition_scope_states_for_collection(
        &self,
        collection_id: &str,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_acquisition_scope_states_for_collection",
            "DELETE FROM wanted_items
              WHERE collection_id = {}
                 OR episode_id IN (
                    SELECT id
                      FROM episodes
                     WHERE collection_id = {}
                 )",
            vec![
                SqlArg::Text(collection_id.to_string()),
                SqlArg::Text(collection_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn delete_acquisition_scope_states_for_series_movie_link(
        &self,
        series_movie_link_id: &str,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_acquisition_scope_states_for_series_movie_link",
            "DELETE FROM wanted_items WHERE series_movie_link_id = {}",
            vec![SqlArg::Text(series_movie_link_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn delete_acquisition_scope_states_for_episode(&self, episode_id: &str) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_acquisition_scope_states_for_episode",
            "DELETE FROM wanted_items WHERE episode_id = {}",
            vec![SqlArg::Text(episode_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String> {
        let explanation_json =
            match encode_release_decision_explanation(decision.explanation_json.as_deref()) {
                Ok(encoded) => encoded,
                Err(error) => {
                    tracing::warn!(
                        decision_id = %decision.id,
                        error = %error,
                        "release decision explanation could not be encoded"
                    );
                    None
                }
            };
        execute_datastore_write(
            &self.datastore,
            "insert_release_decision",
            "INSERT INTO release_decisions
             (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
              decision_code, candidate_score, current_score, score_delta, explanation_json, created_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(decision.id.clone()),
                SqlArg::Text(decision.wanted_item_id.clone()),
                SqlArg::Text(decision.title_id.clone()),
                SqlArg::Text(decision.release_title.clone()),
                SqlArg::OptText(decision.release_url.clone()),
                SqlArg::OptI64(decision.release_size_bytes),
                SqlArg::Text(decision.decision_code.clone()),
                SqlArg::I32(decision.candidate_score),
                SqlArg::OptI32(decision.current_score),
                SqlArg::OptI32(decision.score_delta),
                SqlArg::OptBytes(explanation_json),
                timestamp_arg_for_datastore(&self.datastore, &decision.created_at)?,
            ],
        )
        .await?;
        Ok(decision.id.clone())
    }

    async fn get_acquisition_scope_state_by_id(
        &self,
        id: &str,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        let sql = format!("{} WHERE w.id = {{}}", wanted_item_select_sql());
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .as_ref()
        .map(wanted_row_to_item)
        .transpose()
    }

    async fn list_acquisition_scope_states_by_ids(
        &self,
        ids: &[String],
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = placeholders(ids.len());
        let sql = format!(
            "{} WHERE w.id IN ({placeholders})",
            wanted_item_select_sql()
        );
        let args = ids.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(wanted_row_to_item)
            .collect()
    }

    async fn list_acquisition_scope_states(
        &self,
        query: AcquisitionScopeStatesQuery,
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        if let StoreDatastore::Sqlite { pool, .. } = &self.datastore
            && sqlite_title_search_requires_spellfix(&query)
        {
            return crate::queries::wanted::list_wanted_items_query(pool, &query).await;
        }

        let mut sql = wanted_item_select_sql().to_string();
        sql.push_str(" WHERE 1=1");
        let mut args = Vec::new();
        append_wanted_query_filters(&mut sql, &mut args, &query, true);
        sql.push_str(" ORDER BY w.updated_at DESC LIMIT {} OFFSET {}");
        args.push(SqlArg::I64(query.limit));
        args.push(SqlArg::I64(query.offset));

        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(wanted_row_to_item)
            .collect()
    }

    async fn count_acquisition_scope_states(
        &self,
        query: AcquisitionScopeStatesQuery,
    ) -> AppResult<i64> {
        if let StoreDatastore::Sqlite { pool, .. } = &self.datastore
            && sqlite_title_search_requires_spellfix(&query)
        {
            return crate::queries::wanted::count_wanted_items_query(pool, &query).await;
        }

        let mut sql = String::from(
            "SELECT COUNT(*) AS cnt
               FROM wanted_items w
               LEFT JOIN titles t ON t.id = w.title_id
               LEFT JOIN release_decisions latest_decision ON latest_decision.id = (
                   SELECT rd.id
                     FROM release_decisions rd
                    WHERE rd.wanted_item_id = w.id
                    ORDER BY rd.created_at DESC
                    LIMIT 1
               )
              WHERE 1=1",
        );
        let mut args = Vec::new();
        append_wanted_query_filters(&mut sql, &mut args, &query, true);
        SqlRuntime::fetch_optional(self.datastore.read_exec(), &sql, &args)
            .await?
            .map(|row| row.i64("cnt"))
            .transpose()
            .map(|value| value.unwrap_or_default())
    }

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
                    decision_code, candidate_score, current_score, score_delta, explanation_json, created_at
               FROM release_decisions
              WHERE title_id = {}
              ORDER BY created_at DESC
              LIMIT {} OFFSET {}",
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::I64(limit),
                SqlArg::I64(offset.max(0)),
            ],
        )
        .await?;
        rows.iter().map(release_decision_row_to_item).collect()
    }

    async fn list_release_decisions_for_acquisition_scope_state(
        &self,
        wanted_item_id: &str,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
                    decision_code, candidate_score, current_score, score_delta, explanation_json, created_at
               FROM release_decisions
              WHERE wanted_item_id = {}
              ORDER BY created_at DESC
              LIMIT {} OFFSET {}",
            &[
                SqlArg::Text(wanted_item_id.to_string()),
                SqlArg::I64(limit),
                SqlArg::I64(offset.max(0)),
            ],
        )
        .await?;
        rows.iter().map(release_decision_row_to_item).collect()
    }

    async fn count_release_decisions_for_title(&self, title_id: &str) -> AppResult<i64> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS cnt FROM release_decisions WHERE title_id = {}",
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?
        .map(|row| row.i64("cnt"))
        .transpose()
        .map(|value| value.unwrap_or_default())
    }

    async fn count_release_decisions_for_acquisition_scope_state(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<i64> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS cnt FROM release_decisions WHERE wanted_item_id = {}",
            &[SqlArg::Text(wanted_item_id.to_string())],
        )
        .await?
        .map(|row| row.i64("cnt"))
        .transpose()
        .map(|value| value.unwrap_or_default())
    }
}

fn housekeeping_cutoff_arg(days: i64) -> SqlArg {
    SqlArg::Timestamp(Utc::now() - Duration::days(days))
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("{}", count)
        .collect::<Vec<_>>()
        .join(", ")
}

async fn execute_housekeeping_delete(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: impl Into<String>,
    args: Vec<SqlArg>,
) -> AppResult<u32> {
    let sql = sql.into();
    let rows_affected = SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let sql = sql.clone();
        let args = args.clone();
        Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await })
    })
    .await?;
    Ok(rows_affected as u32)
}

async fn delete_for_title_ids_shared(
    datastore: &StoreDatastore,
    op_name: &'static str,
    table: &'static str,
    title_ids: &[String],
) -> AppResult<u32> {
    if title_ids.is_empty() {
        return Ok(0);
    }

    let sql = format!(
        "DELETE FROM {table} WHERE title_id IN ({})",
        placeholders(title_ids.len())
    );
    let args = title_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    execute_housekeeping_delete(datastore, op_name, sql, args).await
}

async fn delete_media_files_by_ids_shared(
    datastore: &StoreDatastore,
    ids: &[String],
) -> AppResult<u32> {
    if ids.is_empty() {
        return Ok(0);
    }

    let sql = format!(
        "DELETE FROM media_files WHERE id IN ({})",
        placeholders(ids.len())
    );
    let args = ids.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
    execute_housekeeping_delete(datastore, "delete_media_files_by_ids", sql, args).await
}

#[async_trait]
impl HousekeepingRepository for HousekeepingStore {
    async fn delete_stale_workflow_operations(
        &self,
        completed_days: i64,
        warning_failed_days: i64,
    ) -> AppResult<u32> {
        let now = Utc::now();
        execute_housekeeping_delete(
            &self.datastore,
            "delete_stale_workflow_operations",
            "DELETE FROM workflow_operations
              WHERE job_key IS NOT NULL
                AND (
                    (status = 'completed' AND started_at <= {})
                    OR (status IN ('warning', 'failed') AND started_at <= {})
                )
                AND id NOT IN (
                    SELECT id
                      FROM (
                            SELECT id,
                                   ROW_NUMBER() OVER (
                                       PARTITION BY job_key
                                       ORDER BY started_at DESC NULLS LAST, id DESC
                                   ) AS retention_rank
                              FROM workflow_operations
                             WHERE job_key IS NOT NULL
                               AND status IN ('completed', 'warning', 'failed')
                      ) terminal_workflow_operations
                     WHERE retention_rank = 1
                )",
            vec![
                SqlArg::Timestamp(now - Duration::days(completed_days)),
                SqlArg::Timestamp(now - Duration::days(warning_failed_days)),
            ],
        )
        .await
    }

    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_release_decisions_older_than",
            "DELETE FROM release_decisions WHERE created_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_title_history_older_than(&self, _days: i64) -> AppResult<u32> {
        // Legacy title_history rows are retired by migration 0085; nothing remains to prune.
        Ok(0)
    }

    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_release_attempts_older_than",
            "DELETE FROM release_download_attempts
              WHERE attempted_at < {}
                AND outcome != 'pending'",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_history_events_older_than",
            "DELETE FROM history_events WHERE occurred_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_domain_events_older_than_for_types(
        &self,
        days: i64,
        event_types: &[DomainEventType],
    ) -> AppResult<u32> {
        if event_types.is_empty() {
            return Ok(0);
        }

        let sql = format!(
            "DELETE FROM domain_events
              WHERE occurred_at <= {{}}
                AND event_type IN ({})",
            placeholders(event_types.len())
        );
        let mut args = vec![housekeeping_cutoff_arg(days)];
        args.extend(
            event_types
                .iter()
                .map(|event_type| SqlArg::Text(event_type.as_str().to_string())),
        );
        execute_housekeeping_delete(
            &self.datastore,
            "delete_domain_events_older_than_for_types",
            sql,
            args,
        )
        .await
    }

    async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_download_import_artifacts_older_than",
            "DELETE FROM download_import_artifacts
              WHERE created_at < {}
                AND (
                    import_id IS NULL
                    OR NOT EXISTS (
                        SELECT 1
                          FROM imports
                         WHERE imports.id = download_import_artifacts.import_id
                    )
                    OR EXISTS (
                        SELECT 1
                          FROM imports
                         WHERE imports.id = download_import_artifacts.import_id
                           AND imports.status IN ('completed', 'failed', 'skipped')
                    )
                )",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_terminal_imports_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_terminal_imports_older_than",
            "DELETE FROM imports
              WHERE status IN ('completed', 'failed', 'skipped')
                AND updated_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        days: i64,
    ) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_terminal_download_queue_commands_older_than",
            "DELETE FROM download_queue_commands
              WHERE action = 'delete'
                AND status IN ('completed', 'failed')
                AND updated_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "delete_rule_set_history_older_than",
            "DELETE FROM rule_set_history WHERE created_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_history_events_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        delete_for_title_ids_shared(
            &self.datastore,
            "delete_history_events_for_title_ids",
            "history_events",
            title_ids,
        )
        .await
    }

    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<u32> {
        delete_for_title_ids_shared(
            &self.datastore,
            "delete_download_import_artifacts_for_title_ids",
            "download_import_artifacts",
            title_ids,
        )
        .await
    }

    async fn delete_release_attempts_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        delete_for_title_ids_shared(
            &self.datastore,
            "delete_release_attempts_for_title_ids",
            "release_download_attempts",
            title_ids,
        )
        .await
    }

    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, file_path FROM media_files",
            &[],
        )
        .await?;
        rows.iter()
            .map(|row| Ok((row.text("id")?, row.text("file_path")?)))
            .collect()
    }

    async fn list_media_files_with_roots(&self) -> AppResult<Vec<HousekeepingMediaFileRootRow>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT media_files.id AS media_file_id,
                    media_files.title_id AS title_id,
                    media_files.file_path AS file_path,
                    titles.library_id AS library_id,
                    library_roots.path AS root_path
               FROM media_files
          LEFT JOIN titles ON titles.id = media_files.title_id
          LEFT JOIN library_roots ON library_roots.library_id = titles.library_id
           ORDER BY media_files.id ASC, library_roots.is_default DESC, library_roots.path ASC",
            &[],
        )
        .await?;

        let mut rows_by_file = BTreeMap::<String, HousekeepingMediaFileRootRow>::new();
        for row in rows {
            let media_file_id = row.text("media_file_id")?;
            let entry =
                rows_by_file
                    .entry(media_file_id.clone())
                    .or_insert(HousekeepingMediaFileRootRow {
                        media_file_id,
                        title_id: row.text("title_id")?,
                        file_path: row.text("file_path")?,
                        library_id: row.opt_text("library_id")?.unwrap_or_default(),
                        root_paths: Vec::new(),
                    });
            if let Some(root_path) = row.opt_text("root_path")?
                && !entry
                    .root_paths
                    .iter()
                    .any(|existing| existing == &root_path)
            {
                entry.root_paths.push(root_path);
            }
        }

        Ok(rows_by_file.into_values().collect())
    }

    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32> {
        delete_media_files_by_ids_shared(&self.datastore, ids).await
    }

    async fn prune_unreferenced_title_image_blobs(&self, limit: u32) -> AppResult<u32> {
        execute_housekeeping_delete(
            &self.datastore,
            "prune_unreferenced_title_image_blobs",
            "DELETE FROM title_image_blobs
              WHERE digest IN (
                    SELECT blob.digest
                      FROM title_image_blobs blob
                     WHERE NOT EXISTS (
                            SELECT 1
                              FROM title_image_variants variant
                             WHERE variant.blob_digest = blob.digest
                     )
                     ORDER BY blob.digest
                     LIMIT {}
              )",
            vec![SqlArg::I64(i64::from(limit))],
        )
        .await
    }

    async fn run_database_maintenance(&self) -> AppResult<()> {
        match &self.datastore {
            StoreDatastore::Sqlite { .. } => {
                SqlRuntime::run_serialized_sqlite(
                    &self.datastore,
                    "sqlite_database_maintenance",
                    |pool| async move {
                        sqlx::query("PRAGMA optimize")
                            .execute(&pool)
                            .await
                            .map_err(repo_err)?;
                        // Throttled full VACUUM: only reclaim when
                        // the free-page ratio shows meaningful bloat, so the daily
                        // maintenance tick does not pay the VACUUM cost every run.
                        let freelist_pages: i64 = sqlx::query_scalar("PRAGMA freelist_count")
                            .fetch_one(&pool)
                            .await
                            .map_err(repo_err)?;
                        let total_pages: i64 = sqlx::query_scalar("PRAGMA page_count")
                            .fetch_one(&pool)
                            .await
                            .map_err(repo_err)?;
                        let freelist_fraction = if total_pages > 0 {
                            freelist_pages as f64 / total_pages as f64
                        } else {
                            0.0
                        };
                        if freelist_pages >= SQLITE_VACUUM_MIN_FREELIST_PAGES
                            && freelist_fraction >= SQLITE_VACUUM_MIN_FREELIST_FRACTION
                        {
                            tracing::info!(
                                freelist_pages,
                                total_pages,
                                freelist_fraction,
                                "running throttled sqlite VACUUM to reclaim free pages"
                            );
                            sqlx::query("VACUUM")
                                .execute(&pool)
                                .await
                                .map_err(repo_err)?;
                        }
                        Ok(())
                    },
                )
                .await?;

                let (busy, log_frames, checkpointed_frames) = SqlRuntime::run_serialized_sqlite(
                    &self.datastore,
                    "sqlite_database_checkpoint",
                    |pool| async move {
                        sqlx::query_as::<_, (i64, i64, i64)>("PRAGMA wal_checkpoint(TRUNCATE)")
                            .fetch_one(&pool)
                            .await
                            .map_err(repo_err)
                    },
                )
                .await?;
                tracing::info!(
                    busy,
                    log_frames,
                    checkpointed_frames,
                    "completed sqlite WAL checkpoint"
                );
                validate_sqlite_checkpoint_result(busy, log_frames, checkpointed_frames)
            }
            StoreDatastore::Postgres { pool } => sqlx::query("VACUUM (ANALYZE)")
                .execute(pool)
                .await
                .map(|_| ())
                .map_err(repo_err),
        }
    }
}

const PENDING_RELEASE_COLUMNS: &str =
    "id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
    source_kind, release_score, scoring_log_json, indexer_source, indexer_id, release_guid,
    added_at, last_observed_at, delay_until, status, grabbed_at, source_password, published_at, info_hash,
    minimum_seed_ratio, minimum_seed_time_minutes, season_pack_seed_ratio,
    season_pack_seed_time_minutes, seeders, release_identity, coverage_identity, role,
    last_decision_code, release_age_unknown";

/// Same columns as [`PENDING_RELEASE_COLUMNS`] but qualified with the `pr` alias
/// so the paged read can JOIN `titles` for library scoping without ambiguous
/// column names. The output column names are unchanged.
const PENDING_RELEASE_COLUMNS_PR: &str =
    "pr.id, pr.wanted_item_id, pr.title_id, pr.release_title, pr.release_url, pr.release_size_bytes,
    pr.source_kind, pr.release_score, pr.scoring_log_json, pr.indexer_source, pr.indexer_id, pr.release_guid,
    pr.added_at, pr.last_observed_at, pr.delay_until, pr.status, pr.grabbed_at, pr.source_password, pr.published_at, pr.info_hash,
    pr.minimum_seed_ratio, pr.minimum_seed_time_minutes, pr.season_pack_seed_ratio,
    pr.season_pack_seed_time_minutes, pr.seeders, pr.release_identity, pr.coverage_identity,
    pr.role, pr.last_decision_code, pr.release_age_unknown";

fn pending_release_row_to_item(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<PendingRelease> {
    let status = row.text("status")?;
    Ok(PendingRelease {
        id: row.text("id")?,
        wanted_item_id: row.text("wanted_item_id")?,
        title_id: row.text("title_id")?,
        release_title: row.text("release_title")?,
        release_url: row.opt_text("release_url")?,
        release_size_bytes: row.opt_i64("release_size_bytes")?,
        source_kind: row
            .opt_text("source_kind")?
            .and_then(|value| DownloadSourceKind::parse(&value)),
        release_score: row.i32("release_score")?,
        scoring_log_json: json_text_from_row(row, "scoring_log_json")?,
        indexer_source: row.opt_text("indexer_source")?,
        indexer_id: row.opt_text("indexer_id")?,
        release_guid: row.opt_text("release_guid")?,
        added_at: required_timestamp_text(row, "added_at")?,
        last_observed_at: required_timestamp_text(row, "last_observed_at")?,
        delay_until: required_timestamp_text(row, "delay_until")?,
        status: PendingReleaseStatus::parse(&status).ok_or_else(|| {
            scryer_application::AppError::Repository("invalid pending release status".into())
        })?,
        grabbed_at: opt_timestamp_text(row, "grabbed_at")?,
        source_password: decrypt_pending_release_source_password(
            encryption_key,
            row.opt_text("source_password")?,
        )?,
        published_at: opt_timestamp_text(row, "published_at")?,
        info_hash: row.opt_text("info_hash")?,
        // Rows parked before migration 0165 read back as all-`None`; the grab
        // then falls back to the profile's own goals with no tracker clamp.
        seed_minimums: ReleaseSeedMinimums {
            min_seed_ratio: row.opt_f64("minimum_seed_ratio")?,
            min_seed_time_minutes: row.opt_i64("minimum_seed_time_minutes")?,
            season_pack_seed_ratio: row.opt_f64("season_pack_seed_ratio")?,
            season_pack_seed_time_minutes: row.opt_i64("season_pack_seed_time_minutes")?,
        },
        // Rows parked before migration 0169 read back as `None`, which the
        // promotion re-judge treats as unknown — and unknown stays eligible.
        seeders: row.opt_i64("seeders")?,
        release_identity: row.text("release_identity")?,
        coverage_identity: row.text("coverage_identity")?,
        role: PendingReleaseRole::parse(&row.text("role")?).ok_or_else(|| {
            scryer_application::AppError::Repository("invalid pending release role".into())
        })?,
        last_decision_code: row.opt_text("last_decision_code")?,
        release_age_unknown: row.bool("release_age_unknown")?,
    })
}

async fn fetch_pending_releases(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<PendingRelease>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(|row| pending_release_row_to_item(row, encryption_key))
        .collect()
}

fn pending_release_insert_args(
    datastore: &StoreDatastore,
    release: &PendingRelease,
    encryption_key: Option<&EncryptionKey>,
    observation: &PendingReleaseObservation,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(release.id.clone()),
        SqlArg::Text(release.wanted_item_id.clone()),
        SqlArg::Text(release.title_id.clone()),
        SqlArg::Text(release.release_title.clone()),
        SqlArg::OptText(release.release_url.clone()),
        SqlArg::OptI64(release.release_size_bytes),
        SqlArg::OptText(release.source_kind.map(|value| value.as_str().to_string())),
        SqlArg::I32(release.release_score),
        opt_json_arg_for_datastore(datastore, release.scoring_log_json.as_deref())?,
        SqlArg::OptText(release.indexer_source.clone()),
        SqlArg::OptText(release.indexer_id.clone()),
        SqlArg::OptText(release.release_guid.clone()),
        timestamp_arg_for_datastore(datastore, &release.added_at)?,
        timestamp_arg_for_datastore(datastore, &observation.last_observed_at)?,
        timestamp_arg_for_datastore(datastore, &observation.eligible_at)?,
        SqlArg::Text(release.status.as_str().to_string()),
        opt_timestamp_arg_for_datastore(datastore, release.grabbed_at.as_deref())?,
        SqlArg::OptText(encrypt_pending_release_source_password(
            encryption_key,
            release.source_password.as_ref(),
        )?),
        opt_timestamp_arg_for_datastore(datastore, release.published_at.as_deref())?,
        SqlArg::OptText(release.info_hash.clone()),
        SqlArg::OptF64(release.seed_minimums.min_seed_ratio),
        SqlArg::OptI64(release.seed_minimums.min_seed_time_minutes),
        SqlArg::OptF64(release.seed_minimums.season_pack_seed_ratio),
        SqlArg::OptI64(release.seed_minimums.season_pack_seed_time_minutes),
        SqlArg::OptI64(release.seeders),
        SqlArg::Text(observation.release_identity.clone()),
        SqlArg::Text(observation.coverage_identity.clone()),
        SqlArg::Text(observation.role.as_str().to_string()),
        SqlArg::OptText(observation.latest_decision_code.clone()),
        SqlArg::Bool(observation.release_age_unknown),
    ])
}

fn encrypt_pending_release_source_password(
    key: Option<&EncryptionKey>,
    value: Option<&String>,
) -> AppResult<Option<String>> {
    encrypt_optional_value(key, value, "pending release source_password", true)
}

fn decrypt_pending_release_source_password(
    key: Option<&EncryptionKey>,
    value: Option<String>,
) -> AppResult<Option<String>> {
    decrypt_optional_value(key, value, "pending release source_password", true)
}

#[async_trait]
impl PendingReleaseRepository for PendingReleaseStore {
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String> {
        let observation = PendingReleaseObservation::derived(release, PendingReleaseRole::Primary);
        self.insert_pending_release_observation(release, &observation)
            .await
    }

    async fn insert_pending_release_with_role(
        &self,
        release: &PendingRelease,
        role: PendingReleaseRole,
    ) -> AppResult<String> {
        let observation = PendingReleaseObservation::derived(release, role);
        self.insert_pending_release_observation(release, &observation)
            .await
    }

    async fn insert_pending_release_observation(
        &self,
        release: &PendingRelease,
        observation: &PendingReleaseObservation,
    ) -> AppResult<String> {
        let encryption_key = self.encryption_key()?;

        // A row first seen without a publish timestamp retains its original
        // clock. A later observation may carry a GUID or info hash, so match
        // the unknown row by the listing facts rather than its old identity.
        if release
            .published_at
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            let provisional_sql = "SELECT id FROM pending_releases
                WHERE coverage_identity = {}
                  AND release_age_unknown = {}
                  AND status IN ('waiting', 'standby', 'processing', 'needs_review')
                  AND lower(trim(release_title)) = {}
                  AND lower(trim(COALESCE(indexer_id, indexer_source, 'unknown'))) = {}
                ORDER BY added_at ASC, id ASC
                LIMIT 1";
            let normalized_title = release.release_title.trim().to_ascii_lowercase();
            let normalized_indexer = release
                .indexer_id
                .as_deref()
                .or(release.indexer_source.as_deref())
                .unwrap_or("unknown")
                .trim()
                .to_ascii_lowercase();
            if let Some(existing) = SqlRuntime::fetch_all(
                self.datastore.read_exec(),
                provisional_sql,
                &[
                    SqlArg::Text(observation.coverage_identity.clone()),
                    SqlArg::Bool(true),
                    SqlArg::Text(normalized_title),
                    SqlArg::Text(normalized_indexer),
                ],
            )
            .await?
            .into_iter()
            .next()
            {
                let existing_id = existing.text("id")?;
                execute_datastore_write(
                    &self.datastore,
                    "fill_pending_release_published_at",
                    "UPDATE pending_releases
                        SET release_url = {},
                            source_kind = {},
                            release_size_bytes = {},
                            release_score = {},
                            scoring_log_json = {},
                            source_password = COALESCE({}, source_password),
                            indexer_source = {},
                            indexer_id = {},
                            release_guid = {},
                            info_hash = {},
                            minimum_seed_ratio = {},
                            minimum_seed_time_minutes = {},
                            season_pack_seed_ratio = {},
                            season_pack_seed_time_minutes = {},
                            seeders = {},
                            delay_until = {},
                            status = 'waiting',
                            role = {},
                            last_decision_code = COALESCE({}, last_decision_code),
                            published_at = {},
                            release_identity = {},
                            release_age_unknown = {},
                            last_observed_at = {}
                      WHERE id = {}",
                    vec![
                        SqlArg::OptText(release.release_url.clone()),
                        SqlArg::OptText(
                            release.source_kind.map(|value| value.as_str().to_string()),
                        ),
                        SqlArg::OptI64(release.release_size_bytes),
                        SqlArg::I32(release.release_score),
                        opt_json_arg_for_datastore(
                            &self.datastore,
                            release.scoring_log_json.as_deref(),
                        )?,
                        SqlArg::OptText(encrypt_pending_release_source_password(
                            encryption_key.as_ref(),
                            release.source_password.as_ref(),
                        )?),
                        SqlArg::OptText(release.indexer_source.clone()),
                        SqlArg::OptText(release.indexer_id.clone()),
                        SqlArg::OptText(release.release_guid.clone()),
                        SqlArg::OptText(release.info_hash.clone()),
                        SqlArg::OptF64(release.seed_minimums.min_seed_ratio),
                        SqlArg::OptI64(release.seed_minimums.min_seed_time_minutes),
                        SqlArg::OptF64(release.seed_minimums.season_pack_seed_ratio),
                        SqlArg::OptI64(release.seed_minimums.season_pack_seed_time_minutes),
                        SqlArg::OptI64(release.seeders),
                        timestamp_arg_for_datastore(&self.datastore, &observation.eligible_at)?,
                        SqlArg::Text(observation.role.as_str().to_string()),
                        SqlArg::OptText(observation.latest_decision_code.clone()),
                        opt_timestamp_arg_for_datastore(
                            &self.datastore,
                            release.published_at.as_deref(),
                        )?,
                        SqlArg::Text(observation.release_identity.clone()),
                        SqlArg::Bool(false),
                        timestamp_arg_for_datastore(
                            &self.datastore,
                            &observation.last_observed_at,
                        )?,
                        SqlArg::Text(existing_id.clone()),
                    ],
                )
                .await?;
                return Ok(existing_id);
            }
        }

        execute_datastore_write(
            &self.datastore,
            "insert_pending_release",
            "INSERT INTO pending_releases
             (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
              source_kind, release_score, scoring_log_json, indexer_source, indexer_id, release_guid,
              added_at, last_observed_at, delay_until, status, grabbed_at, source_password, published_at, info_hash,
              minimum_seed_ratio, minimum_seed_time_minutes, season_pack_seed_ratio,
              season_pack_seed_time_minutes, seeders, release_identity, coverage_identity, role,
              last_decision_code, release_age_unknown)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {},
                     {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
             ON CONFLICT(release_identity)
             WHERE status IN ('waiting', 'standby', 'processing', 'needs_review')
             DO UPDATE SET
                release_url = excluded.release_url,
                source_kind = excluded.source_kind,
                release_size_bytes = excluded.release_size_bytes,
                release_score = excluded.release_score,
                scoring_log_json = excluded.scoring_log_json,
                indexer_source = excluded.indexer_source,
                indexer_id = excluded.indexer_id,
                release_guid = excluded.release_guid,
                source_password = COALESCE(excluded.source_password, pending_releases.source_password),
                info_hash = excluded.info_hash,
                minimum_seed_ratio = excluded.minimum_seed_ratio,
                minimum_seed_time_minutes = excluded.minimum_seed_time_minutes,
                season_pack_seed_ratio = excluded.season_pack_seed_ratio,
                season_pack_seed_time_minutes = excluded.season_pack_seed_time_minutes,
                seeders = excluded.seeders,
                delay_until = excluded.delay_until,
                published_at = COALESCE(pending_releases.published_at, excluded.published_at),
                release_age_unknown = excluded.release_age_unknown
                    AND pending_releases.published_at IS NULL,
                coverage_identity = excluded.coverage_identity,
                role = excluded.role,
                last_decision_code = COALESCE(excluded.last_decision_code, pending_releases.last_decision_code),
                last_observed_at = excluded.last_observed_at",
            pending_release_insert_args(
                &self.datastore,
                release,
                encryption_key.as_ref(),
                observation,
            )?,
        )
        .await?;
        let persisted = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id FROM pending_releases
              WHERE release_identity = {}
                AND status IN ('waiting', 'standby', 'processing', 'needs_review')
              ORDER BY added_at ASC, id ASC
              LIMIT 1",
            &[SqlArg::Text(observation.release_identity.clone())],
        )
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| {
            scryer_application::AppError::Repository(
                "pending release observation was not persisted".to_string(),
            )
        })?;
        persisted.text("id")
    }

    async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE status = 'waiting' AND delay_until <= {{}}
              ORDER BY delay_until ASC"
        );
        let encryption_key = self.encryption_key()?;
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[timestamp_arg_for_datastore(&self.datastore, now)?],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        // Merge candidates remain actionable; manual-review rows are deliberately
        // excluded so automatic processing cannot grab them.
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE status IN ('waiting', 'standby')
              ORDER BY delay_until ASC"
        );
        let encryption_key = self.encryption_key()?;
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn list_active_release_age_unknown_pending_releases(
        &self,
    ) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
              FROM pending_releases
              WHERE release_age_unknown = {{}}
                AND status IN ('waiting', 'standby', 'processing', 'needs_review')
              ORDER BY indexer_id ASC, added_at ASC, id ASC"
        );
        let encryption_key = self.encryption_key()?;
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Bool(true)],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>> {
        let sql = format!("SELECT {PENDING_RELEASE_COLUMNS} FROM pending_releases WHERE id = {{}}");
        let encryption_key = self.encryption_key()?;
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .as_ref()
        .map(|row| pending_release_row_to_item(row, encryption_key.as_ref()))
        .transpose()
    }

    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE wanted_item_id = {{}} AND status = 'waiting'
              ORDER BY release_score DESC"
        );
        let encryption_key = self.encryption_key()?;
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(wanted_item_id.to_string())],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE title_id = {{}}
              ORDER BY added_at DESC"
        );
        let encryption_key = self.encryption_key()?;
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn list_pending_releases_page(
        &self,
        query: PendingReleasesPageQuery,
    ) -> AppResult<(Vec<PendingRelease>, i64)> {
        let limit = query.limit.max(0);
        let offset = query.offset.max(0);

        // Only JOIN titles when a library scope is supplied; the per-wanted-item
        // caller has already authorized its single scope and passes no libraries.
        let use_library_filter = !query.library_ids.is_empty();
        let from_clause = if use_library_filter {
            "FROM pending_releases pr JOIN titles t ON t.id = pr.title_id"
        } else {
            "FROM pending_releases pr"
        };

        // An explicit status filter owns the base set. Without one, preserve the
        // historical open-for-review default (`waiting` plus `needs_review`).
        let mut where_sql = if query.statuses.is_empty() {
            String::from(" WHERE pr.status IN ('waiting', 'needs_review')")
        } else {
            String::from(" WHERE 1 = 1")
        };
        let mut filter_args: Vec<SqlArg> = Vec::new();
        if let Some(title_id) = query.title_id.as_deref() {
            where_sql.push_str(" AND pr.title_id = {}");
            filter_args.push(SqlArg::Text(title_id.to_string()));
        }
        if let Some(wanted_item_id) = query.wanted_item_id.as_deref() {
            where_sql.push_str(" AND pr.wanted_item_id = {}");
            filter_args.push(SqlArg::Text(wanted_item_id.to_string()));
        }
        append_in_filter(
            &mut where_sql,
            &mut filter_args,
            "pr.status",
            &query.statuses,
        );
        if use_library_filter {
            append_in_filter(
                &mut where_sql,
                &mut filter_args,
                "t.library_id",
                &query.library_ids,
            );
        }

        let count_sql = format!("SELECT COUNT(*) AS cnt {from_clause}{where_sql}");
        let total =
            SqlRuntime::fetch_optional(self.datastore.read_exec(), &count_sql, &filter_args)
                .await?
                .map(|row| row.i64("cnt"))
                .transpose()?
                .unwrap_or_default();

        if limit == 0 || total == 0 {
            return Ok((Vec::new(), total));
        }

        let order_sql = match query.sort {
            PendingReleasePageSort::DelayUntilAsc => " ORDER BY pr.delay_until ASC, pr.id ASC",
            PendingReleasePageSort::ReleaseScoreDesc => {
                " ORDER BY pr.release_score DESC, pr.added_at ASC, pr.id ASC"
            }
        };
        let page_sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS_PR} {from_clause}{where_sql}{order_sql} LIMIT {{}} OFFSET {{}}"
        );
        let mut page_args = filter_args;
        page_args.push(SqlArg::I64(limit));
        page_args.push(SqlArg::I64(offset));
        let encryption_key = self.encryption_key()?;
        let items = fetch_pending_releases(
            self.datastore.read_exec(),
            &page_sql,
            &page_args,
            encryption_key.as_ref(),
        )
        .await?;
        Ok((items, total))
    }

    async fn update_pending_release_status(
        &self,
        id: &str,
        status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "update_pending_release_status",
            "UPDATE pending_releases
                SET status = {}, grabbed_at = {}
              WHERE id = {}",
            vec![
                SqlArg::Text(status.as_str().to_string()),
                opt_timestamp_arg_for_datastore(&self.datastore, grabbed_at)?,
                SqlArg::Text(id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn expire_pending_release(&self, id: &str, decision_code: &str) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "expire_pending_release",
            "UPDATE pending_releases
                SET status = 'expired', last_decision_code = {}
              WHERE id = {}",
            vec![
                SqlArg::Text(decision_code.to_string()),
                SqlArg::Text(id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn mark_release_age_unknown_pending_release_needs_review(
        &self,
        id: &str,
        decision_code: &str,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "mark_release_age_unknown_pending_release_needs_review",
            "UPDATE pending_releases
                SET status = 'needs_review', last_decision_code = {}
              WHERE id = {}
                AND release_age_unknown = 1
                AND status IN ('waiting', 'standby', 'processing')",
            vec![
                SqlArg::Text(decision_code.to_string()),
                SqlArg::Text(id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn update_pending_release_delay_until(
        &self,
        id: &str,
        delay_until: &str,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "update_pending_release_delay_until",
            "UPDATE pending_releases
                SET delay_until = {}
              WHERE id = {}",
            vec![
                opt_timestamp_arg_for_datastore(&self.datastore, Some(delay_until))?,
                SqlArg::Text(id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE wanted_item_id = {{}} AND status = 'standby'
              ORDER BY release_score DESC, added_at ASC"
        );
        let encryption_key = self.encryption_key()?;
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(wanted_item_id.to_string())],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn count_standby_pending_releases_for_wanted_items(
        &self,
        wanted_item_ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, i64>> {
        if wanted_item_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let placeholders = placeholders(wanted_item_ids.len());
        let sql = format!(
            "SELECT wanted_item_id, COUNT(*) AS cnt
               FROM pending_releases
              WHERE status = 'standby' AND wanted_item_id IN ({placeholders})
              GROUP BY wanted_item_id"
        );
        let args = wanted_item_ids
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        rows.iter()
            .map(|row| Ok((row.text("wanted_item_id")?, row.i64("cnt")?)))
            .collect()
    }

    async fn list_standby_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE title_id = {{}} AND status = 'standby'
              ORDER BY release_score DESC, added_at ASC"
        );
        let encryption_key = self.encryption_key()?;
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_standby_pending_releases_for_wanted_item",
            "DELETE FROM pending_releases
              WHERE wanted_item_id = {} AND status = 'standby'",
            vec![SqlArg::Text(wanted_item_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS}
               FROM pending_releases
              WHERE status = 'standby'
              ORDER BY wanted_item_id ASC, release_score DESC, added_at ASC"
        );
        let encryption_key = self.encryption_key()?;
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: PendingReleaseStatus,
        next_status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        let rows = execute_datastore_write(
            &self.datastore,
            "compare_and_set_pending_release_status",
            "UPDATE pending_releases
                SET status = {}, grabbed_at = {}
              WHERE id = {} AND status = {}",
            vec![
                SqlArg::Text(next_status.as_str().to_string()),
                opt_timestamp_arg_for_datastore(&self.datastore, grabbed_at)?,
                SqlArg::Text(id.to_string()),
                SqlArg::Text(current_status.as_str().to_string()),
            ],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn retire_lower_or_equal_overlapping_pending_releases(
        &self,
        lower_or_equal_ids: &[String],
    ) -> AppResult<()> {
        for id in lower_or_equal_ids {
            execute_datastore_write(
                &self.datastore,
                "retire_lower_or_equal_overlapping_pending_release",
                "UPDATE pending_releases
                    SET status = 'superseded',
                        last_decision_code = 'grabbed_overlap_retired'
                  WHERE id = {}
                    AND status IN ('waiting', 'standby', 'processing', 'needs_review')",
                vec![SqlArg::Text(id.clone())],
            )
            .await?;
        }
        Ok(())
    }

    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_pending_releases_for_title",
            "DELETE FROM pending_releases WHERE title_id = {}",
            vec![SqlArg::Text(title_id.to_string())],
        )
        .await?;
        Ok(())
    }
}

const BLOCKLIST_COLUMNS: &str = "id, title_id, release_name, normalized_release_name, indexer_id, info_hash, reason, created_at";

fn blocklist_row_to_entry_sql(row: &SqlRow) -> AppResult<BlocklistEntry> {
    Ok(BlocklistEntry {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        release_name: row.text("release_name")?,
        normalized_release_name: row.text("normalized_release_name")?,
        indexer_id: row.text("indexer_id")?,
        info_hash: row.opt_text("info_hash")?,
        reason: row.opt_text("reason")?,
        created_at: required_timestamp_text(row, "created_at")?,
    })
}

async fn fetch_blocklist_entries(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<BlocklistEntry>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(blocklist_row_to_entry_sql)
        .collect()
}

async fn fetch_exists(exec: SqlExec<'_, '_>, sql: &str, args: &[SqlArg]) -> AppResult<bool> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .map(|row| row.bool("matched"))
        .transpose()
        .map(|value| value.unwrap_or(false))
}

#[async_trait]
impl BlocklistRepository for BlocklistStore {
    async fn block(&self, entry: &NewBlocklistEntry) -> AppResult<bool> {
        let Some(normalized_release_name) = normalize_release_name(Some(&entry.release_name))
        else {
            return Ok(false);
        };
        // `ON CONFLICT DO NOTHING` against the two unique indexes, rather than a
        // read-then-write in application code: two writers recording the same
        // failure cannot both insert, and neither needs a lock to find out.
        let rows_written = execute_datastore_write(
            &self.datastore,
            "insert_blocklist_entry",
            "INSERT INTO blocklist
             (id, title_id, release_name, normalized_release_name, indexer_id,
              info_hash, reason, created_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {})
             ON CONFLICT DO NOTHING",
            vec![
                SqlArg::Text(Id::new().0),
                SqlArg::Text(entry.title_id.clone()),
                SqlArg::Text(entry.release_name.trim().to_string()),
                SqlArg::Text(normalized_release_name),
                SqlArg::Text(entry.indexer_id.trim().to_string()),
                SqlArg::OptText(normalize_info_hash(entry.info_hash.as_deref())),
                SqlArg::OptText(entry.reason.clone()),
                timestamp_arg_for_datastore(&self.datastore, &Utc::now().to_rfc3339())?,
            ],
        )
        .await?;
        Ok(rows_written > 0)
    }

    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>> {
        let sql = format!(
            "SELECT {BLOCKLIST_COLUMNS}
               FROM blocklist
              WHERE title_id = {{}}
              ORDER BY created_at DESC
              LIMIT {{}}"
        );
        fetch_blocklist_entries(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::I64(limit as i64),
            ],
        )
        .await
    }

    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        let total = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS cnt FROM blocklist",
            &[],
        )
        .await?
        .map(|row| row.i64("cnt"))
        .transpose()?
        .unwrap_or_default();
        let sql = format!(
            "SELECT {BLOCKLIST_COLUMNS}
               FROM blocklist
              ORDER BY created_at DESC
              LIMIT {{}} OFFSET {{}}"
        );
        let entries = fetch_blocklist_entries(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::I64(limit as i64), SqlArg::I64(offset as i64)],
        )
        .await?;
        Ok((entries, total))
    }

    async fn is_blocked(
        &self,
        title_id: &str,
        indexer_id: &str,
        release_name: &str,
        info_hash: Option<&str>,
    ) -> AppResult<bool> {
        // Infohash first: content identity is the same wherever the torrent
        // came from, so it answers without consulting the indexer at all.
        if let Some(info_hash) = normalize_info_hash(info_hash)
            && fetch_exists(
                self.datastore.read_exec(),
                "SELECT EXISTS(
                     SELECT 1 FROM blocklist
                      WHERE title_id = {} AND info_hash = {}
                 ) AS matched",
                &[SqlArg::Text(title_id.to_string()), SqlArg::Text(info_hash)],
            )
            .await?
        {
            return Ok(true);
        }
        let Some(normalized_release_name) = normalize_release_name(Some(release_name)) else {
            return Ok(false);
        };
        // An empty stored indexer blocks the name on every indexer.
        fetch_exists(
            self.datastore.read_exec(),
            "SELECT EXISTS(
                 SELECT 1 FROM blocklist
                  WHERE title_id = {}
                    AND normalized_release_name = {}
                    AND (indexer_id = '' OR indexer_id = {})
             ) AS matched",
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(normalized_release_name),
                SqlArg::Text(indexer_id.trim().to_string()),
            ],
        )
        .await
    }

    async fn get(&self, id: &str) -> AppResult<Option<BlocklistEntry>> {
        let sql = format!(
            "SELECT {BLOCKLIST_COLUMNS}
               FROM blocklist
              WHERE id = {{}}
              LIMIT 1"
        );
        Ok(fetch_blocklist_entries(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .into_iter()
        .next())
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_blocklist_entry",
            "DELETE FROM blocklist WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_blocklist_for_title",
            "DELETE FROM blocklist WHERE title_id = {}",
            vec![SqlArg::Text(title_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn delete_for_indexer(&self, indexer_id: &str) -> AppResult<()> {
        let indexer_id = indexer_id.trim();
        if indexer_id.is_empty() {
            // The empty indexer is the every-indexer wildcard, not a row a
            // deleted indexer owns.
            return Ok(());
        }
        execute_datastore_write(
            &self.datastore,
            "delete_blocklist_for_indexer",
            "DELETE FROM blocklist WHERE indexer_id = {}",
            vec![SqlArg::Text(indexer_id.to_string())],
        )
        .await?;
        Ok(())
    }
}

const SUBTITLE_DOWNLOAD_COLUMNS: &str =
    "id, media_file_id, title_id, episode_id, source_kind, language, provider,
    provider_file_id, file_path, score, hearing_impaired, forced, ai_translated,
    machine_translated, uploader, release_info, synced, downloaded_at";

const SUBTITLE_PROBE_CACHE_COLUMNS: &str =
    "media_file_id, file_path, size_bytes, modified_at, language,
    hearing_impaired, detection_source_language, detection_source_hi, probe_version, updated_at";

const SUBTITLE_BLOCKLIST_COLUMNS: &str =
    "id, media_file_id, provider, provider_file_id, language, reason, created_at";

fn subtitle_download_row_to_item(row: &SqlRow) -> AppResult<SubtitleDownload> {
    let source_kind = row.text("source_kind")?;
    let provider = row.opt_text("provider")?.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    });
    Ok(SubtitleDownload {
        id: row.text("id")?,
        media_file_id: row.text("media_file_id")?,
        title_id: row.text("title_id")?,
        episode_id: row.opt_text("episode_id")?,
        source_kind: ExternalSubtitleSourceKind::parse(&source_kind).ok_or_else(|| {
            scryer_application::AppError::Repository("invalid external subtitle source kind".into())
        })?,
        language: row.text("language")?,
        provider,
        provider_file_id: row.opt_text("provider_file_id")?,
        file_path: row.text("file_path")?,
        score: row.opt_i32("score")?,
        hearing_impaired: row.bool("hearing_impaired")?,
        forced: row.bool("forced")?,
        ai_translated: row.bool("ai_translated")?,
        machine_translated: row.bool("machine_translated")?,
        uploader: row.opt_text("uploader")?,
        release_info: row.opt_text("release_info")?,
        synced: row.bool("synced")?,
        downloaded_at: required_timestamp_text(row, "downloaded_at")?,
    })
}

fn subtitle_probe_cache_row_to_entry(row: &SqlRow) -> AppResult<ExternalSubtitleProbeCacheEntry> {
    let detection_source_language =
        ExternalSubtitleDetectionSource::parse(&row.text("detection_source_language")?)
            .ok_or_else(|| {
                scryer_application::AppError::Repository(
                    "invalid subtitle probe language detection source".into(),
                )
            })?;
    let detection_source_hi = ExternalSubtitleDetectionSource::parse(
        &row.text("detection_source_hi")?,
    )
    .ok_or_else(|| {
        scryer_application::AppError::Repository(
            "invalid subtitle probe hi detection source".into(),
        )
    })?;

    Ok(ExternalSubtitleProbeCacheEntry {
        media_file_id: row.text("media_file_id")?,
        file_path: row.text("file_path")?,
        size_bytes: row.i64("size_bytes")?,
        modified_at: opt_timestamp_text(row, "modified_at")?,
        language: row.opt_text("language")?,
        hearing_impaired: row.opt_bool("hearing_impaired")?,
        detection_source_language,
        detection_source_hi,
        probe_version: row.i32("probe_version")?,
        updated_at: required_timestamp_text(row, "updated_at")?,
    })
}

fn subtitle_blocklist_row_to_entry(row: &SqlRow) -> AppResult<SubtitleBlocklistEntry> {
    Ok(SubtitleBlocklistEntry {
        id: row.text("id")?,
        media_file_id: row.text("media_file_id")?,
        provider: row.text("provider")?,
        provider_file_id: row.text("provider_file_id")?,
        language: row.text("language")?,
        reason: row.opt_text("reason")?,
        created_at: required_timestamp_text(row, "created_at")?,
    })
}

async fn fetch_subtitle_downloads(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<SubtitleDownload>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(subtitle_download_row_to_item)
        .collect()
}

#[async_trait]
impl SubtitleDownloadRepository for SubtitleDownloadStore {
    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<SubtitleDownload>> {
        let sql = format!(
            "SELECT {SUBTITLE_DOWNLOAD_COLUMNS}
               FROM subtitle_downloads
              WHERE title_id = {{}}
              ORDER BY downloaded_at DESC"
        );
        fetch_subtitle_downloads(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
        )
        .await
    }

    async fn get(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        let sql =
            format!("SELECT {SUBTITLE_DOWNLOAD_COLUMNS} FROM subtitle_downloads WHERE id = {{}}");
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .as_ref()
        .map(subtitle_download_row_to_item)
        .transpose()
    }

    async fn list_for_media_file(&self, media_file_id: &str) -> AppResult<Vec<SubtitleDownload>> {
        let sql = format!(
            "SELECT {SUBTITLE_DOWNLOAD_COLUMNS}
               FROM subtitle_downloads
              WHERE media_file_id = {{}}
              ORDER BY downloaded_at DESC"
        );
        fetch_subtitle_downloads(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(media_file_id.to_string())],
        )
        .await
    }

    async fn list_probe_cache_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<ExternalSubtitleProbeCacheEntry>> {
        let sql = format!(
            "SELECT {SUBTITLE_PROBE_CACHE_COLUMNS}
               FROM external_subtitle_probe_cache
              WHERE media_file_id = {{}}
              ORDER BY file_path ASC"
        );
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(media_file_id.to_string())],
        )
        .await?
        .iter()
        .map(subtitle_probe_cache_row_to_entry)
        .collect()
    }

    async fn list_blocklist_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<SubtitleBlocklistEntry>> {
        let sql = format!(
            "SELECT {SUBTITLE_BLOCKLIST_COLUMNS}
               FROM subtitle_blocklist
              WHERE media_file_id = {{}}
              ORDER BY created_at DESC"
        );
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(media_file_id.to_string())],
        )
        .await?
        .iter()
        .map(subtitle_blocklist_row_to_entry)
        .collect()
    }

    async fn insert(&self, download: &SubtitleDownload) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "insert_subtitle_download",
            "INSERT INTO subtitle_downloads
             (id, media_file_id, title_id, episode_id, source_kind, language, provider,
              provider_file_id, file_path, score, hearing_impaired, forced,
              ai_translated, machine_translated, uploader, release_info, synced, downloaded_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
             ON CONFLICT (id) DO UPDATE SET
                media_file_id = excluded.media_file_id,
                title_id = excluded.title_id,
                episode_id = excluded.episode_id,
                source_kind = excluded.source_kind,
                language = excluded.language,
                provider = excluded.provider,
                provider_file_id = excluded.provider_file_id,
                file_path = excluded.file_path,
                score = excluded.score,
                hearing_impaired = excluded.hearing_impaired,
                forced = excluded.forced,
                ai_translated = excluded.ai_translated,
                machine_translated = excluded.machine_translated,
                uploader = excluded.uploader,
                release_info = excluded.release_info,
                synced = excluded.synced,
                downloaded_at = excluded.downloaded_at",
            vec![
                SqlArg::Text(download.id.clone()),
                SqlArg::Text(download.media_file_id.clone()),
                SqlArg::Text(download.title_id.clone()),
                SqlArg::OptText(download.episode_id.clone()),
                SqlArg::Text(download.source_kind.as_str().to_string()),
                SqlArg::Text(download.language.clone()),
                SqlArg::Text(download.provider.clone().unwrap_or_default()),
                SqlArg::OptText(download.provider_file_id.clone()),
                SqlArg::Text(download.file_path.clone()),
                SqlArg::OptI32(download.score),
                SqlArg::Bool(download.hearing_impaired),
                SqlArg::Bool(download.forced),
                SqlArg::Bool(download.ai_translated),
                SqlArg::Bool(download.machine_translated),
                SqlArg::OptText(download.uploader.clone()),
                SqlArg::OptText(download.release_info.clone()),
                SqlArg::Bool(download.synced),
                timestamp_arg_for_datastore(&self.datastore, &download.downloaded_at)?,
            ],
        )
        .await?;
        Ok(())
    }

    async fn upsert_probe_cache_entry(
        &self,
        entry: &ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "upsert_external_subtitle_probe_cache_entry",
            "INSERT INTO external_subtitle_probe_cache
             (media_file_id, file_path, size_bytes, modified_at, language,
              hearing_impaired, detection_source_language, detection_source_hi, probe_version, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})
             ON CONFLICT (media_file_id, file_path) DO UPDATE SET
                size_bytes = excluded.size_bytes,
                modified_at = excluded.modified_at,
                language = excluded.language,
                hearing_impaired = excluded.hearing_impaired,
                detection_source_language = excluded.detection_source_language,
                detection_source_hi = excluded.detection_source_hi,
                probe_version = excluded.probe_version,
                updated_at = excluded.updated_at",
            vec![
                SqlArg::Text(entry.media_file_id.clone()),
                SqlArg::Text(entry.file_path.clone()),
                SqlArg::I64(entry.size_bytes),
                opt_timestamp_arg_for_datastore(&self.datastore, entry.modified_at.as_deref())?,
                SqlArg::OptText(entry.language.clone()),
                SqlArg::OptBool(entry.hearing_impaired),
                SqlArg::Text(entry.detection_source_language.as_str().to_string()),
                SqlArg::Text(entry.detection_source_hi.as_str().to_string()),
                SqlArg::I32(entry.probe_version),
                timestamp_arg_for_datastore(&self.datastore, &entry.updated_at)?,
            ],
        )
        .await?;
        Ok(())
    }

    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "set_subtitle_download_synced",
            "UPDATE subtitle_downloads SET synced = {} WHERE id = {}",
            vec![SqlArg::Bool(synced), SqlArg::Text(id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_subtitle_download", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let sql = format!(
                    "SELECT {SUBTITLE_DOWNLOAD_COLUMNS} FROM subtitle_downloads WHERE id = {{}}"
                );
                let existing =
                    SqlRuntime::fetch_optional(SqlExec::Tx(tx), &sql, &[SqlArg::Text(id.clone())])
                        .await?
                        .as_ref()
                        .map(subtitle_download_row_to_item)
                        .transpose()?;
                if existing.is_some() {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM subtitle_downloads WHERE id = {}",
                        &[SqlArg::Text(id)],
                    )
                    .await?;
                }
                Ok(existing)
            })
        })
        .await
    }

    async fn delete_probe_cache_entry(
        &self,
        media_file_id: &str,
        file_path: &str,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "delete_external_subtitle_probe_cache_entry",
            "DELETE FROM external_subtitle_probe_cache
              WHERE media_file_id = {} AND file_path = {}",
            vec![
                SqlArg::Text(media_file_id.to_string()),
                SqlArg::Text(file_path.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn is_blocklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool> {
        fetch_exists(
            self.datastore.read_exec(),
            "SELECT EXISTS(
                 SELECT 1 FROM subtitle_blocklist
                  WHERE media_file_id = {} AND provider = {} AND provider_file_id = {}
             ) AS matched",
            &[
                SqlArg::Text(media_file_id.to_string()),
                SqlArg::Text(provider.to_string()),
                SqlArg::Text(provider_file_id.to_string()),
            ],
        )
        .await
    }

    async fn blocklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        execute_datastore_write(
            &self.datastore,
            "blocklist_subtitle_download",
            "INSERT INTO subtitle_blocklist
             (id, media_file_id, provider, provider_file_id, language, reason)
             VALUES ({}, {}, {}, {}, {}, {})
             ON CONFLICT (media_file_id, provider, provider_file_id) DO NOTHING",
            vec![
                SqlArg::Text(uuid::Uuid::new_v4().to_string()),
                SqlArg::Text(media_file_id.to_string()),
                SqlArg::Text(provider.to_string()),
                SqlArg::Text(provider_file_id.to_string()),
                SqlArg::Text(language.to_string()),
                SqlArg::OptText(reason.map(str::to_string)),
            ],
        )
        .await?;
        Ok(())
    }
}

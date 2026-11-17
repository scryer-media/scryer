use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use scryer_application::url_redaction::redact_optional_url_credentials;
use scryer_application::{
    AcquisitionScopeCoordinates, AcquisitionScopeKey, AcquisitionScopeState,
    AcquisitionScopeStateRepository, AcquisitionScopeStatesQuery, AcquisitionScopeStatus,
    AcquisitionScopeStatusTally, AppResult, BlocklistRepository, DownloadSourceKind,
    HousekeepingMediaFileRootRow, HousekeepingRepository, LibraryProbeRepository,
    LibraryProbeSignature, NewBlocklistEntry, PendingRelease, PendingReleaseObservation,
    PendingReleasePageSort, PendingReleaseRepository, PendingReleaseRole, PendingReleaseStatus,
    PendingReleasesPageQuery, ReleaseDecision, ReleaseSeedMinimums, SubtitleDownloadRepository,
    normalize_release_name,
    subtitles::{
        ExternalSubtitleDetectionSource, ExternalSubtitleProbeCacheEntry, SubtitleSearchBackoff,
        SubtitleSearchBackoffKey,
    },
};
use scryer_domain::{
    BlocklistEntry, DomainEventType, ExternalSubtitleSourceKind, Id, SubtitleBlocklistEntry,
    SubtitleDownload,
};
use scryer_plugin_sdk::torrent::normalize_info_hash;

use crate::config_store::{current_encryption_key, decrypt_optional_value, encrypt_optional_value};
use crate::encryption::{EncryptionKey, is_encrypted};
use crate::media::id_list_statements::{id_list_statements, max_binds};
use crate::queries::sql_runtime::repo_err;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

use super::decode_release_decision_explanation_record;
use super::release_decision_explanation::ReleaseDecisionExplanationEncoder;

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
    /// The bounded-distance lane for the wanted view's title filter. Absent
    /// in fixtures and while the index rebuilds, which costs typo tolerance
    /// and nothing else.
    fuzzy: Option<Arc<scryer_infrastructure_library_search::TitleFuzzyIndex>>,
}

impl WantedStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self {
            datastore,
            fuzzy: None,
        }
    }

    pub fn with_fuzzy_index(
        mut self,
        index: Arc<scryer_infrastructure_library_search::TitleFuzzyIndex>,
    ) -> Self {
        self.fuzzy = Some(index);
        self
    }
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

    /// Fills the publish time and the latest listing facts into the
    /// provisional row `existing_id`, only while that row is still active.
    /// Returns whether the row was filled.
    async fn fill_provisional_pending_release(
        &self,
        existing_id: &str,
        release: &PendingRelease,
        observation: &PendingReleaseObservation,
    ) -> AppResult<bool> {
        let encryption_key = self.encryption_key()?;
        let update_sql = format!(
            "UPDATE pending_releases
                    SET release_url = {{}},
                        source_kind = {{}},
                        release_size_bytes = {{}},
                        release_score = {{}},
                        scoring_log_json = {{}},
                        release_listing_json = COALESCE({{}}, release_listing_json),
                        source_password = COALESCE({{}}, source_password),
                        indexer_source = {{}},
                        indexer_id = {{}},
                        release_guid = {{}},
                        info_hash = {{}},
                        minimum_seed_ratio = {{}},
                        minimum_seed_time_minutes = {{}},
                        season_pack_seed_ratio = {{}},
                        season_pack_seed_time_minutes = {{}},
                        seeders = {{}},
                        delay_until = {{}},
                        status = 'waiting',
                        role = {{}},
                        last_decision_code = COALESCE({{}}, last_decision_code),
                        published_at = {{}},
                        release_identity = {{}},
                        release_age_unknown = {{}},
                        last_observed_at = {{}}
                  WHERE id = {{}}
                    AND status IN {PENDING_RELEASE_ACTIVE_STATUSES_SQL}"
        );
        let mut update_args = vec![
            SqlArg::OptText(release.release_url.clone()),
            SqlArg::OptText(release.source_kind.map(|value| value.as_str().to_string())),
            SqlArg::OptI64(release.release_size_bytes),
            SqlArg::I32(release.release_score),
            opt_json_arg_for_datastore(&self.datastore, release.scoring_log_json.as_deref())?,
            SqlArg::OptText(release.release_listing_json.clone()),
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
            self.datastore
                .timestamp_text_arg(&observation.eligible_at)?,
            SqlArg::Text(observation.role.as_str().to_string()),
            SqlArg::OptText(observation.latest_decision_code.clone()),
            self.datastore
                .opt_timestamp_text_arg(release.published_at.as_deref())?,
            SqlArg::Text(observation.release_identity.clone()),
            SqlArg::Bool(false),
            self.datastore
                .timestamp_text_arg(&observation.last_observed_at)?,
        ];
        update_args.push(SqlArg::Text(existing_id.to_string()));
        let updated = execute_datastore_write(
            &self.datastore,
            "fill_pending_release_published_at",
            update_sql,
            update_args,
        )
        .await?;
        Ok(updated > 0)
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

/// A scope's `updated_at`, as read back through `timestamp_text`, as an
/// instant, so rows of several statements order the way one statement orders
/// them. Text that is not RFC 3339 has none and orders by its text.
fn wanted_updated_at_instant(value: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
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
        last_search_at: row.opt_timestamp_text("last_search_at")?,
        status: AcquisitionScopeStatus::parse(&status).unwrap_or_default(),
        grabbed_release: row.opt_text("grabbed_release")?,
        // Resolved from the library by the caller, never stored.
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: row.timestamp_text("created_at")?,
        updated_at: row.timestamp_text("updated_at")?,
    })
}

/// The identity a release decision is deduplicated on: which scope decided,
/// what it decided, and which release it decided about.
///
/// `release_url` is the release's own identity when the indexer gave one;
/// title and size carry the rest, and stand alone for a decision recorded
/// without a canonical source. `COALESCE` rather than `IS NULL` handling keeps
/// one predicate that means the same thing on SQLite and Postgres — a SQL
/// `NULL` never equals a `NULL`, so a decision with no URL would otherwise
/// never match itself.
#[derive(Clone)]
struct ReleaseDecisionIdentityArgs {
    wanted_item_id: String,
    decision_code: String,
    release_url: String,
    release_title: String,
    release_size_bytes: i64,
}

/// The release URL as the decision ledger stores it. Indexer download links
/// carry the operator's indexer key; the ledger is read-only history that is
/// shown in the UI and never fetched from, so the key is dropped on write and
/// the dedupe identity compares the same redacted form.
fn stored_release_decision_url(decision: &ReleaseDecision) -> Option<String> {
    redact_optional_url_credentials(decision.release_url.clone())
}

/// Stands in for "no size recorded" in the identity comparison. Negative, so
/// it cannot collide with a real byte count.
const RELEASE_DECISION_UNKNOWN_SIZE: i64 = -1;

impl ReleaseDecisionIdentityArgs {
    fn of(decision: &ReleaseDecision) -> Self {
        Self {
            wanted_item_id: decision.wanted_item_id.clone(),
            decision_code: decision.decision_code.clone(),
            release_url: stored_release_decision_url(decision).unwrap_or_default(),
            release_title: decision.release_title.clone(),
            release_size_bytes: decision
                .release_size_bytes
                .unwrap_or(RELEASE_DECISION_UNKNOWN_SIZE),
        }
    }

    fn args(&self) -> Vec<SqlArg> {
        vec![
            SqlArg::Text(self.wanted_item_id.clone()),
            SqlArg::Text(self.decision_code.clone()),
            SqlArg::Text(self.release_url.clone()),
            SqlArg::Text(self.release_title.clone()),
            SqlArg::I64(self.release_size_bytes),
        ]
    }
}

/// Newest first, so a table that still holds pre-dedupe duplicates collapses
/// onto the row a reader would have seen anyway.
///
/// The sort key is wrapped so the planner cannot read it from
/// `idx_release_decisions_wanted`: that plan walks every decision of the
/// scope, newest first, until one matches, and all of them when none does.
/// With the wrapped key the equalities are sought in
/// `idx_release_decisions_identity` and the few rows found are sorted. The
/// order is the same either way.
const RELEASE_DECISION_IDENTITY_SELECT_SQL: &str = "SELECT id
       FROM release_decisions
      WHERE wanted_item_id = {}
        AND decision_code = {}
        AND COALESCE(release_url, '') = {}
        AND release_title = {}
        AND COALESCE(release_size_bytes, -1) = {}
      ORDER BY COALESCE(created_at, created_at) DESC
      LIMIT 1";

fn release_decision_row_to_item(row: &SqlRow) -> AppResult<ReleaseDecision> {
    release_decision_from_row(row, "", row.text("id")?)
}

/// A `release_decisions` row whose columns are named `{prefix}{column}`, with
/// its id already read. A missing candidate score reads as 0.
fn release_decision_from_row(row: &SqlRow, prefix: &str, id: String) -> AppResult<ReleaseDecision> {
    let column = |name: &str| format!("{prefix}{name}");
    Ok(ReleaseDecision {
        explanation_json: release_decision_explanation_from_row(
            row,
            &column("explanation_json"),
            &id,
        )?,
        id,
        wanted_item_id: row.text(&column("wanted_item_id"))?,
        title_id: row.text(&column("title_id"))?,
        release_title: row.text(&column("release_title"))?,
        release_url: row.opt_text(&column("release_url"))?,
        release_size_bytes: row.opt_i64(&column("release_size_bytes"))?,
        decision_code: row.text(&column("decision_code"))?,
        candidate_score: row.opt_i32(&column("candidate_score"))?.unwrap_or_default(),
        current_score: row.opt_i32(&column("current_score"))?,
        score_delta: row.opt_i32(&column("score_delta"))?,
        created_at: row.timestamp_text(&column("created_at"))?,
    })
}

fn release_decision_explanation_from_row(
    row: &SqlRow,
    column: &str,
    decision_id: &str,
) -> AppResult<Option<scryer_application::ReleaseDecisionExplanation>> {
    let encoded = row.opt_bytes(column)?;
    match decode_release_decision_explanation_record(encoded.as_deref()) {
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
    wanted_projected_row_to_item(row, WantedProjection::WithLatestDecision)
}

fn wanted_projected_row_to_item(
    row: &SqlRow,
    projection: WantedProjection,
) -> AppResult<AcquisitionScopeState> {
    let latest_decision_id = match projection {
        WantedProjection::WithLatestDecision => row.opt_text("latest_decision_id")?,
        WantedProjection::WithoutLatestDecision => None,
    };
    let latest_release_decision = latest_decision_id
        .map(|id| release_decision_from_row(row, "latest_decision_", id))
        .transpose()?;

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
        last_search_at: row.opt_timestamp_text("last_search_at")?,
        status: AcquisitionScopeStatus::parse(&status).unwrap_or_default(),
        grabbed_release: row.opt_text("grabbed_release")?,
        // Resolved from the library by the caller, never stored.
        landed_bar: None,
        latest_release_decision,
        mismatch_recovery_eligible: row.bool("mismatch_recovery_eligible")?,
        created_at: row.timestamp_text("created_at")?,
        updated_at: row.timestamp_text("updated_at")?,
    })
}

/// Which columns a wanted read decodes: the full row the wanted list shows, or
/// the same row without the latest release decision, for callers that never
/// read it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum WantedProjection {
    WithLatestDecision,
    WithoutLatestDecision,
}

/// The newest decision recorded for each wanted row, joined as
/// `latest_decision`.
const WANTED_LATEST_DECISION_JOIN_SQL: &str = "
       LEFT JOIN release_decisions latest_decision ON latest_decision.id = (
           SELECT rd.id
             FROM release_decisions rd
            WHERE rd.wanted_item_id = w.id
            ORDER BY rd.created_at DESC
            LIMIT 1
       )";

/// The columns of the joined `latest_decision`, each prefixed
/// `latest_decision_` for [`release_decision_from_row`]. Ends with a comma.
const WANTED_LATEST_DECISION_COLUMNS_SQL: &str = "
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
            latest_decision.created_at AS latest_decision_created_at,";

/// Whether the wanted row `w` is eligible for title-mismatch recovery: it is
/// wanted, has a release decision, and every one of its decisions is a title
/// mismatch. Both probes are served by the `(wanted_item_id, created_at)`
/// index on `release_decisions`.
const WANTED_MISMATCH_RECOVERY_ELIGIBLE_SQL: &str = "(w.status = 'wanted'
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
                 ))";

/// Title ids per statement once a title-id read is too long for one.
const ID_LIST_CHUNK: usize = 1_000;

fn wanted_item_select_sql() -> String {
    wanted_item_projection_sql(WantedProjection::WithLatestDecision)
}

fn wanted_item_projection_sql(projection: WantedProjection) -> String {
    let with_decision = projection == WantedProjection::WithLatestDecision;
    format!(
        "SELECT w.id, w.title_id, t.name AS title_name, t.slug AS title_slug,
            t.facet AS title_facet, t.library_id AS library_id,
            libraries.name AS library_name, libraries.slug AS library_slug,
            w.episode_id, w.collection_id, w.series_movie_link_id,
            e.season_number, e.episode_number, w.media_type,
            w.last_search_at, w.status, w.grabbed_release,{}
            CASE WHEN {WANTED_MISMATCH_RECOVERY_ELIGIBLE_SQL} THEN TRUE ELSE FALSE END
                AS mismatch_recovery_eligible,
            w.created_at, w.updated_at
       FROM wanted_items w
       LEFT JOIN titles t ON t.id = w.title_id
       LEFT JOIN libraries ON libraries.id = t.library_id
       LEFT JOIN episodes e ON e.id = w.episode_id{}",
        if with_decision {
            WANTED_LATEST_DECISION_COLUMNS_SQL
        } else {
            ""
        },
        if with_decision {
            WANTED_LATEST_DECISION_JOIN_SQL
        } else {
            ""
        }
    )
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
    title_search: Option<&crate::queries::title_search::ResolvedTitleSearch>,
) {
    append_in_filter(sql, args, "w.status", &query.statuses);
    append_in_filter(sql, args, "w.media_type", &query.media_types);
    if let Some(title_id) = query.title_id.as_deref() {
        sql.push_str(" AND w.title_id = {}");
        args.push(SqlArg::Text(title_id.to_string()));
    }
    append_in_filter(sql, args, "t.library_id", &query.library_ids);
    if let Some(title_search) = title_search {
        let (predicate, search_args) = title_search.predicate("w.title_id");
        sql.push_str(" AND ");
        sql.push_str(&predicate);
        args.extend(search_args.into_iter().map(SqlArg::Text));
    }
    append_in_filter(
        sql,
        args,
        "latest_decision.decision_code",
        &query.latest_decision_codes,
    );
    if query.mismatch_recovery_eligible_only {
        sql.push_str(" AND ");
        sql.push_str(WANTED_MISMATCH_RECOVERY_ELIGIBLE_SQL);
    }
}

fn wanted_upsert_sql(datastore: &StoreDatastore, item: &AcquisitionScopeState) -> String {
    // Episode scope before collection scope. An episode row carries its owning
    // `collection_id` for attribution as well as its `episode_id`, so matching
    // the collection first would fold every episode of one season onto the same
    // row — the shape `find_wanted_state_for_scope` and the in-memory repository
    // already dispatch on.
    let conflict_target = if item.series_movie_link_id.is_some() {
        "(series_movie_link_id) WHERE series_movie_link_id IS NOT NULL"
    } else if item.episode_id.is_some() {
        match datastore {
            StoreDatastore::Sqlite { .. } => "(title_id, episode_id)",
            StoreDatastore::Postgres { .. } => {
                "(title_id, episode_id) WHERE episode_id IS NOT NULL"
            }
        }
    } else if item.collection_id.is_some() {
        // Matches `idx_wanted_items_collection_id`, which 0255 narrowed to the
        // collection-scoped rows so episode rows may carry the same id.
        "(collection_id) WHERE collection_id IS NOT NULL AND episode_id IS NULL"
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
        datastore.opt_timestamp_text_arg(item.last_search_at.as_deref())?,
        SqlArg::Text(item.status.as_str().to_string()),
        SqlArg::OptText(item.grabbed_release.clone()),
        datastore.timestamp_text_arg(&now)?,
        datastore.timestamp_text_arg(&now)?,
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

/// The statuses of an active pending release, as an SQL list. It is the
/// predicate of the active-identity unique index, so an `ON CONFLICT` target
/// that names it matches that index.
const PENDING_RELEASE_ACTIVE_STATUSES_SQL: &str =
    "('waiting', 'standby', 'processing', 'needs_review')";

/// The grabbed scopes of `count` titles, newest first.
///
/// The status is compared through an expression so no index can be sought on
/// it. Written as a bare column, sqlite reads `idx_wanted_items_status_updated_at`
/// for its ready-made order, which is every grabbed scope of the library; this
/// way the titles are sought and their rows filtered and sorted. A scope
/// always has a status, so the rows are the same.
fn grabbed_scopes_for_titles_sql(select: &str, count: usize) -> String {
    format!(
        "{select} WHERE w.title_id IN ({}) AND COALESCE(w.status, '') = 'grabbed'
                 ORDER BY w.updated_at DESC, w.id",
        placeholders(count)
    )
}

/// One page of a title's mismatch-recovery scopes in id order. The title
/// filter, the id cursor and the id order are all read from
/// `idx_wanted_items_title_id_id`, so no page sorts.
fn mismatch_recovery_scopes_for_title_statement(
    title_id: &str,
    after_id: Option<&str>,
    limit: i64,
) -> (String, Vec<SqlArg>) {
    let mut sql = format!(
        "{} WHERE w.title_id = {{}} AND {WANTED_MISMATCH_RECOVERY_ELIGIBLE_SQL}",
        wanted_item_projection_sql(WantedProjection::WithoutLatestDecision)
    );
    let mut args = vec![SqlArg::Text(title_id.to_string())];
    if let Some(after_id) = after_id {
        sql.push_str(" AND w.id > {}");
        args.push(SqlArg::Text(after_id.to_string()));
    }
    sql.push_str(" ORDER BY w.id LIMIT {}");
    args.push(SqlArg::I64(limit));
    (sql, args)
}

/// The active pending releases whose age is unknown. The flag is written as
/// each engine's own literal, exactly as the partial index
/// `idx_pending_releases_active_unknown_age` states it: a bound value there
/// keeps the planner from proving the index applies.
fn active_release_age_unknown_pending_releases_sql(datastore: &StoreDatastore) -> String {
    let age_unknown = match datastore {
        StoreDatastore::Sqlite { .. } => "release_age_unknown = 1",
        StoreDatastore::Postgres { .. } => "release_age_unknown",
    };
    format!(
        "SELECT {PENDING_RELEASE_COLUMNS}
              FROM pending_releases
              WHERE {age_unknown}
                AND status IN {PENDING_RELEASE_ACTIVE_STATUSES_SQL}
              ORDER BY indexer_id ASC, added_at ASC, id ASC"
    )
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

/// The `WHERE` predicate that selects one scope's row of `wanted_items` under a
/// title, with its bound arguments. `prefix` qualifies the columns (`"w."` when
/// the table is aliased). An episode row also carries its collection id, so the
/// collection arm excludes episode rows; otherwise it would hand every episode
/// of a season the same sibling row. Each arm is served by an index: the
/// `(title_id, episode_id)` unique key for the episode and title arms,
/// `idx_wanted_items_collection_id` (or `idx_wanted_items_collection`) and
/// `idx_wanted_items_series_movie_link`.
fn wanted_scope_predicate(
    prefix: &str,
    title_id: &str,
    scope: AcquisitionScopeKey<'_>,
) -> (String, Vec<SqlArg>) {
    let title_arg = SqlArg::Text(title_id.to_string());
    match scope {
        AcquisitionScopeKey::Episode(episode_id) => (
            format!("{prefix}title_id = {{}} AND {prefix}episode_id = {{}}"),
            vec![title_arg, SqlArg::Text(episode_id.to_string())],
        ),
        AcquisitionScopeKey::Collection(collection_id) => (
            format!(
                "{prefix}title_id = {{}} AND {prefix}collection_id = {{}} AND {prefix}episode_id IS NULL"
            ),
            vec![title_arg, SqlArg::Text(collection_id.to_string())],
        ),
        AcquisitionScopeKey::SeriesMovieLink(series_movie_link_id) => (
            format!("{prefix}title_id = {{}} AND {prefix}series_movie_link_id = {{}}"),
            vec![title_arg, SqlArg::Text(series_movie_link_id.to_string())],
        ),
        AcquisitionScopeKey::Title => (
            format!(
                "{prefix}title_id = {{}} AND {prefix}episode_id IS NULL AND {prefix}collection_id IS NULL AND {prefix}series_movie_link_id IS NULL"
            ),
            vec![title_arg],
        ),
    }
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
    let scope = AcquisitionScopeKey::of(item);
    let (predicate, args) = wanted_scope_predicate("", &item.title_id, scope);
    let sql = format!("{columns} WHERE {predicate}");

    SqlRuntime::fetch_optional(SqlExec::Tx(tx), &sql, &args)
        .await?
        .as_ref()
        .map(wanted_seed_row_to_item)
        .transpose()
}

/// A decision's bound arguments, built once so a batch and its per-row
/// replay write exactly the same values.
struct PreparedReleaseDecision {
    identity: ReleaseDecisionIdentityArgs,
    insert_args: Vec<SqlArg>,
    update_args: Vec<SqlArg>,
    record_id: String,
}

/// Decisions prepared per blocking call. A search can return thousands of
/// candidates; bounding the call keeps one batch from holding a blocking
/// thread for its whole length, and lets a cancelled write stop between
/// chunks.
const RELEASE_DECISION_PREPARE_CHUNK: usize = 256;

impl PreparedReleaseDecision {
    fn of(datastore: &StoreDatastore, decision: &ReleaseDecision) -> AppResult<Self> {
        Self::with_encoder(
            datastore,
            decision,
            &mut ReleaseDecisionExplanationEncoder::new(),
        )
    }

    /// Prepares every decision, in order, off the async runtime: parsing,
    /// canonicalizing and compressing explanations is CPU work that grows with
    /// the number of candidates a search returned. One zstd context serves a
    /// whole chunk.
    /// The decisions are moved, a chunk at a time, to the blocking thread
    /// that prepares them; none is copied.
    async fn of_batch(
        datastore: &StoreDatastore,
        decisions: Vec<ReleaseDecision>,
    ) -> Vec<AppResult<Self>> {
        let mut prepared = Vec::with_capacity(decisions.len());
        let mut remaining = decisions.into_iter();
        loop {
            let chunk_decisions = remaining
                .by_ref()
                .take(RELEASE_DECISION_PREPARE_CHUNK)
                .collect::<Vec<_>>();
            if chunk_decisions.is_empty() {
                return prepared;
            }
            let chunk_len = chunk_decisions.len();
            let datastore = datastore.clone();
            let joined = tokio::task::spawn_blocking(move || {
                let mut encoder = ReleaseDecisionExplanationEncoder::new();
                chunk_decisions
                    .iter()
                    .map(|decision| Self::with_encoder(&datastore, decision, &mut encoder))
                    .collect::<Vec<_>>()
            })
            .await;
            match joined {
                Ok(chunk_prepared) => prepared.extend(chunk_prepared),
                Err(error) => {
                    // Every row of the chunk reports the failure as its own
                    // error, exactly as a row that could not be prepared does.
                    tracing::warn!(
                        decisions = chunk_len,
                        error = %error,
                        "release decision preparation task failed"
                    );
                    prepared.extend((0..chunk_len).map(|_| {
                        Err(scryer_application::AppError::Repository(format!(
                            "release decision preparation task failed: {error}"
                        )))
                    }));
                }
            }
        }
    }

    fn with_encoder(
        datastore: &StoreDatastore,
        decision: &ReleaseDecision,
        encoder: &mut ReleaseDecisionExplanationEncoder,
    ) -> AppResult<Self> {
        let explanation_json = match encoder.encode_explanation(decision.explanation_json.as_ref())
        {
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
        // Held once: the insert and update argument lists, and every copy of
        // this prepared decision a transaction attempt takes, share the bytes.
        let explanation_json: Option<Arc<[u8]>> = explanation_json.map(Arc::from);
        // The same verdict, on the same release, for the same scope is one
        // ledger row that ages forward — not a new row per re-evaluation.
        // RSS re-sees the same feed every cadence, so an idle instance was
        // appending thousands of byte-identical `queued_better_or_equal` rows
        // (4,720 rows / 2.7 MB per 110 minutes on the 12k-title load test) for
        // 19 scopes. A changed verdict still appends: `decision_code` is part
        // of the identity, so the ledger keeps the history that means
        // something and drops the history that repeats.
        let identity = ReleaseDecisionIdentityArgs::of(decision);
        let created_at = datastore.timestamp_text_arg(&decision.created_at)?;
        let insert_args = vec![
            SqlArg::Text(decision.id.clone()),
            SqlArg::Text(decision.wanted_item_id.clone()),
            SqlArg::Text(decision.title_id.clone()),
            SqlArg::Text(decision.release_title.clone()),
            SqlArg::OptText(stored_release_decision_url(decision)),
            SqlArg::OptI64(decision.release_size_bytes),
            SqlArg::Text(decision.decision_code.clone()),
            SqlArg::I32(decision.candidate_score),
            SqlArg::OptI32(decision.current_score),
            SqlArg::OptI32(decision.score_delta),
            SqlArg::OptSharedBytes(explanation_json.clone()),
            created_at.clone(),
        ];
        let update_args = vec![
            SqlArg::Text(decision.title_id.clone()),
            SqlArg::I32(decision.candidate_score),
            SqlArg::OptI32(decision.current_score),
            SqlArg::OptI32(decision.score_delta),
            SqlArg::OptSharedBytes(explanation_json),
            created_at,
        ];
        Ok(Self {
            identity,
            insert_args,
            update_args,
            record_id: decision.id.clone(),
        })
    }
}

/// Insert the decision, or age the row already holding its identity forward.
/// Returns the id of the row written.
async fn write_release_decision_tx(
    tx: &mut crate::queries::sql_runtime::SqlTx<'_>,
    prepared: &PreparedReleaseDecision,
) -> AppResult<String> {
    let existing = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        RELEASE_DECISION_IDENTITY_SELECT_SQL,
        &prepared.identity.args(),
    )
    .await?
    .as_ref()
    .map(|row| row.text("id"))
    .transpose()?;

    let Some(existing_id) = existing else {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO release_decisions
             (id, wanted_item_id, title_id, release_title, release_url,
              release_size_bytes, decision_code, candidate_score, current_score,
              score_delta, explanation_json, created_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            &prepared.insert_args,
        )
        .await?;
        return Ok(prepared.record_id.clone());
    };

    let mut args = prepared.update_args.clone();
    args.push(SqlArg::Text(existing_id.clone()));
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "UPDATE release_decisions
            SET title_id = {},
                candidate_score = {},
                current_score = {},
                score_delta = {},
                explanation_json = {},
                created_at = {}
          WHERE id = {}",
        &args,
    )
    .await?;
    Ok(existing_id)
}

impl WantedStore {
    async fn list_states_for_title_ids(
        &self,
        title_ids: &[String],
        projection: WantedProjection,
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }
        let select = wanted_item_projection_sql(projection);
        let exec = self.datastore.read_exec();
        let (ids, per_statement) =
            id_list_statements(title_ids, 1, 0, max_binds(&exec), ID_LIST_CHUNK);
        let mut states = Vec::new();
        for chunk in ids.chunks(per_statement) {
            let sql = format!(
                "{select} WHERE w.title_id IN ({})",
                placeholders(chunk.len())
            );
            let args = chunk.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
            for row in SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await? {
                states.push(wanted_projected_row_to_item(&row, projection)?);
            }
        }
        Ok(states)
    }

    async fn write_prepared_release_decision(
        &self,
        prepared: Arc<PreparedReleaseDecision>,
    ) -> AppResult<String> {
        SqlRuntime::run_in_transaction(&self.datastore, "insert_release_decision", move |tx| {
            let prepared = Arc::clone(&prepared);
            Box::pin(async move { write_release_decision_tx(tx, &prepared).await })
        })
        .await
    }
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

    async fn ensure_acquisition_scopes_grabbed(
        &self,
        candidates: &[AcquisitionScopeState],
        grabbed_release: &str,
    ) -> AppResult<()> {
        if candidates.is_empty() {
            return Ok(());
        }
        // Shared, not copied, by every transaction attempt.
        let items: Arc<[AcquisitionScopeState]> = Arc::from(candidates);
        let release = grabbed_release.to_string();
        let datastore = self.datastore.clone();
        let batch = SqlRuntime::run_in_transaction(
            &self.datastore,
            "ensure_acquisition_scopes_grabbed",
            move |tx| {
                let datastore = datastore.clone();
                let items = Arc::clone(&items);
                let grabbed_release = release.clone();
                Box::pin(async move {
                    for item in items.iter() {
                        // The search time kept is the one already in hand: the
                        // existing row's, read in this transaction, or the one a
                        // new row was just written with.
                        let (id, last_search_at) = match fetch_seed_target_tx(tx, item).await? {
                            Some(existing) => (existing.id, existing.last_search_at),
                            None => (
                                execute_wanted_upsert_tx(tx, &datastore, item).await?,
                                item.last_search_at.clone(),
                            ),
                        };
                        let now = Utc::now().to_rfc3339();
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            crate::workflow::stores::UPDATE_WANTED_ITEM_STATUS_SQL,
                            &[
                                SqlArg::Text(AcquisitionScopeStatus::Grabbed.as_str().to_string()),
                                datastore.opt_timestamp_text_arg(last_search_at.as_deref())?,
                                SqlArg::OptText(Some(grabbed_release.clone())),
                                datastore.timestamp_text_arg(&now)?,
                                SqlArg::Text(id),
                            ],
                        )
                        .await?;
                    }
                    Ok(())
                })
            },
        )
        .await;
        if batch.is_ok() {
            return Ok(());
        }
        // Nothing from the failed batch was kept; replaying row by row stops
        // at the same failing row with the earlier rows written.
        scryer_application::ensure_acquisition_scopes_grabbed_per_row(
            self,
            candidates,
            grabbed_release,
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
            crate::workflow::stores::UPDATE_WANTED_ITEM_STATUS_SQL,
            vec![
                SqlArg::Text(status.to_string()),
                self.datastore.opt_timestamp_text_arg(last_search_at)?,
                SqlArg::OptText(grabbed_release.map(str::to_string)),
                self.datastore.timestamp_text_arg(&now)?,
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
                self.datastore.timestamp_text_arg(last_search_at)?,
                self.datastore.timestamp_text_arg(&now)?,
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
        let scope = match episode_id {
            Some(episode_id) => AcquisitionScopeKey::Episode(episode_id),
            None => AcquisitionScopeKey::Title,
        };
        self.get_acquisition_scope_state_for_scope(title_id, scope)
            .await
    }

    async fn get_acquisition_scope_state_for_scope(
        &self,
        title_id: &str,
        scope: AcquisitionScopeKey<'_>,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        let (predicate, args) = wanted_scope_predicate("w.", title_id, scope);
        let sql = format!("{} WHERE {predicate}", wanted_item_select_sql());
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
        self.list_states_for_title_ids(title_ids, WantedProjection::WithLatestDecision)
            .await
    }

    async fn list_acquisition_scope_states_for_title_ids_without_decisions(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        self.list_states_for_title_ids(title_ids, WantedProjection::WithoutLatestDecision)
            .await
    }

    async fn list_mismatch_recovery_scopes_for_title(
        &self,
        title_id: &str,
        after_id: Option<&str>,
        limit: i64,
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let (sql, args) = mismatch_recovery_scopes_for_title_statement(title_id, after_id, limit);
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| wanted_projected_row_to_item(row, WantedProjection::WithoutLatestDecision))
            .collect()
    }

    async fn list_grabbed_acquisition_scopes_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }
        // The titles are sought through an index that leads with `title_id`;
        // the status is checked on those titles' rows and the order is a sort
        // of what is left.
        let projection = WantedProjection::WithoutLatestDecision;
        let select = wanted_item_projection_sql(projection);
        let exec = self.datastore.read_exec();
        let (ids, per_statement) =
            id_list_statements(title_ids, 1, 0, max_binds(&exec), ID_LIST_CHUNK);
        let mut states = Vec::new();
        let mut statements = 0usize;
        for chunk in ids.chunks(per_statement) {
            statements += 1;
            let sql = grabbed_scopes_for_titles_sql(&select, chunk.len());
            let args = chunk.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
            for row in SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await? {
                states.push(wanted_projected_row_to_item(&row, projection)?);
            }
        }
        if statements > 1 {
            // Each statement is ordered; a list read in several is merged
            // into the one order here.
            states.sort_by(|left, right| {
                wanted_updated_at_instant(&right.updated_at)
                    .cmp(&wanted_updated_at_instant(&left.updated_at))
                    .then_with(|| right.updated_at.cmp(&left.updated_at))
                    .then_with(|| left.id.cmp(&right.id))
            });
        }
        Ok(states)
    }

    async fn list_acquisition_scope_coordinates_for_statuses(
        &self,
        statuses: &[AcquisitionScopeStatus],
    ) -> AppResult<Vec<AcquisitionScopeCoordinates>> {
        if statuses.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT title_id, episode_id, collection_id, series_movie_link_id
               FROM wanted_items
              WHERE status IN ({})",
            placeholders(statuses.len())
        );
        let args = statuses
            .iter()
            .map(|status| SqlArg::Text(status.as_str().to_string()))
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok(AcquisitionScopeCoordinates {
                    title_id: row.text("title_id")?,
                    episode_id: row.opt_text("episode_id")?,
                    collection_id: row.opt_text("collection_id")?,
                    series_movie_link_id: row.opt_text("series_movie_link_id")?,
                })
            })
            .collect()
    }

    async fn complete_acquisition_scope_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
        last_search_at: Option<&str>,
        landed_import: bool,
    ) -> AppResult<bool> {
        let scope = episode_id.map_or(AcquisitionScopeKey::Title, AcquisitionScopeKey::Episode);
        self.complete_acquisition_scope_for_key(title_id, scope, last_search_at, landed_import)
            .await
    }

    async fn complete_acquisition_scope_for_key(
        &self,
        title_id: &str,
        scope: AcquisitionScopeKey<'_>,
        last_search_at: Option<&str>,
        landed_import: bool,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        let (predicate, key_args) = wanted_scope_predicate("", title_id, scope);
        let sql = format!(
            "UPDATE wanted_items
                SET status = {{}},
                    last_search_at = {{}},
                    grabbed_release = CASE WHEN {{}} THEN NULL ELSE grabbed_release END,
                    updated_at = {{}}
              WHERE {predicate}"
        );
        let mut args = vec![
            SqlArg::Text(AcquisitionScopeStatus::Completed.as_str().to_string()),
            self.datastore.opt_timestamp_text_arg(last_search_at)?,
            SqlArg::Bool(landed_import),
            self.datastore.timestamp_text_arg(&now)?,
        ];
        args.extend(key_args);
        let rows = execute_datastore_write(
            &self.datastore,
            "complete_acquisition_scope_for_title",
            sql,
            args,
        )
        .await?;
        Ok(rows > 0)
    }

    async fn complete_acquisition_scopes_for_episodes(
        &self,
        title_id: &str,
        episode_ids: &[String],
        last_search_at: Option<&str>,
        landed_import: bool,
    ) -> AppResult<()> {
        // Keeps each statement well under both engines' bind limits.
        const MAX_COMPLETION_BATCH_EPISODES: usize = 1_000;
        if episode_ids.is_empty() {
            return Ok(());
        }
        // The per-episode statement applied to every episode at once, one
        // statement per chunk, all in one transaction.
        let now = Utc::now().to_rfc3339();
        let head = vec![
            SqlArg::Text(AcquisitionScopeStatus::Completed.as_str().to_string()),
            self.datastore.opt_timestamp_text_arg(last_search_at)?,
            SqlArg::Bool(landed_import),
            self.datastore.timestamp_text_arg(&now)?,
            SqlArg::Text(title_id.to_string()),
        ];
        let statements = episode_ids
            .chunks(MAX_COMPLETION_BATCH_EPISODES)
            .map(|chunk| {
                let sql = format!(
                    "UPDATE wanted_items
                        SET status = {{}},
                            last_search_at = {{}},
                            grabbed_release = CASE WHEN {{}} THEN NULL ELSE grabbed_release END,
                            updated_at = {{}}
                      WHERE title_id = {{}} AND episode_id IN ({})",
                    std::iter::repeat_n("{}", chunk.len())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                let mut args = head.clone();
                args.extend(chunk.iter().cloned().map(SqlArg::Text));
                (sql, args)
            })
            .collect::<Vec<_>>();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "complete_acquisition_scopes_for_episodes",
            move |tx| {
                let statements = statements.clone();
                Box::pin(async move {
                    for (sql, args) in &statements {
                        SqlRuntime::execute(SqlExec::Tx(tx), sql, args).await?;
                    }
                    Ok(())
                })
            },
        )
        .await
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
        let prepared = PreparedReleaseDecision::of(&self.datastore, decision)?;
        self.write_prepared_release_decision(Arc::new(prepared))
            .await
    }

    async fn insert_release_decisions(
        &self,
        decisions: Vec<ReleaseDecision>,
    ) -> Vec<AppResult<String>> {
        // Shared, not copied: the batch transaction, each of its attempts, and
        // the per-row replay all read the same prepared values.
        let prepared = PreparedReleaseDecision::of_batch(&self.datastore, decisions)
            .await
            .into_iter()
            .map(|prepared| prepared.map(Arc::new))
            .collect::<Vec<_>>();
        let batch = prepared
            .iter()
            .map(|prepared| prepared.as_ref().ok().map(Arc::clone))
            .collect::<Option<Vec<_>>>()
            .map(Arc::<[Arc<PreparedReleaseDecision>]>::from);
        if let Some(batch) = batch.filter(|batch| batch.len() > 1) {
            let written =
                SqlRuntime::run_in_transaction(&self.datastore, "insert_release_decisions", {
                    move |tx| {
                        let batch = Arc::clone(&batch);
                        Box::pin(async move {
                            let mut ids = Vec::with_capacity(batch.len());
                            for prepared in batch.iter() {
                                ids.push(write_release_decision_tx(tx, prepared).await?);
                            }
                            Ok(ids)
                        })
                    }
                })
                .await;
            if let Ok(ids) = written {
                return ids.into_iter().map(Ok).collect();
            }
        }
        // One transaction per decision, so each failure stays with its own row.
        let mut results = Vec::with_capacity(prepared.len());
        for prepared in prepared {
            results.push(match prepared {
                Ok(prepared) => self.write_prepared_release_decision(prepared).await,
                Err(error) => Err(error),
            });
        }
        results
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

    async fn list_latest_release_decisions_for_acquisition_scope_states(
        &self,
        ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, ReleaseDecision>> {
        let mut decisions = std::collections::HashMap::new();
        if ids.is_empty() {
            return Ok(decisions);
        }
        let exec = self.datastore.read_exec();
        let (ids, per_statement) = id_list_statements(ids, 1, 0, max_binds(&exec), ID_LIST_CHUNK);
        for chunk in ids.chunks(per_statement) {
            let sql = format!(
                "SELECT{WANTED_LATEST_DECISION_COLUMNS_SQL}
                        w.id AS scope_state_id
                   FROM wanted_items w{WANTED_LATEST_DECISION_JOIN_SQL}
                  WHERE w.id IN ({})
                    AND latest_decision.id IS NOT NULL",
                placeholders(chunk.len())
            );
            let args = chunk.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
            for row in SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await? {
                let decision_id = row.text("latest_decision_id")?;
                let decision = release_decision_from_row(&row, "latest_decision_", decision_id)?;
                decisions.insert(row.text("scope_state_id")?, decision);
            }
        }
        Ok(decisions)
    }

    async fn list_acquisition_scope_statuses_by_ids(
        &self,
        ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, AcquisitionScopeStatus>> {
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let sql = format!(
            "SELECT id, status FROM wanted_items WHERE id IN ({})",
            placeholders(ids.len())
        );
        let args = ids.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(|row| {
                Ok((
                    row.text("id")?,
                    AcquisitionScopeStatus::parse(&row.text("status")?).unwrap_or_default(),
                ))
            })
            .collect()
    }

    async fn list_acquisition_scope_states(
        &self,
        query: AcquisitionScopeStatesQuery,
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let mut sql = wanted_item_select_sql();
        sql.push_str(" WHERE 1=1");
        let mut args = Vec::new();
        let title_search = crate::queries::title_search::ResolvedTitleSearch::resolve(
            self.fuzzy.as_deref(),
            None,
            query.title_search.as_deref(),
        )
        .await?;
        append_wanted_query_filters(&mut sql, &mut args, &query, title_search.as_ref());
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
        // The latest-decision join adds at most one row per scope, so it
        // changes the count only through a filter on its code; without one it
        // is left out.
        let latest_decision_join = if query.latest_decision_codes.is_empty() {
            ""
        } else {
            WANTED_LATEST_DECISION_JOIN_SQL
        };
        let mut sql = format!(
            "SELECT COUNT(*) AS cnt
               FROM wanted_items w
               LEFT JOIN titles t ON t.id = w.title_id{latest_decision_join}
              WHERE 1=1"
        );
        let mut args = Vec::new();
        let title_search = crate::queries::title_search::ResolvedTitleSearch::resolve(
            self.fuzzy.as_deref(),
            None,
            query.title_search.as_deref(),
        )
        .await?;
        append_wanted_query_filters(&mut sql, &mut args, &query, title_search.as_ref());
        SqlRuntime::fetch_optional(self.datastore.read_exec(), &sql, &args)
            .await?
            .map(|row| row.i64("cnt"))
            .transpose()
            .map(|value| value.unwrap_or_default())
    }

    async fn tally_acquisition_scope_states_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<AcquisitionScopeStatusTally>> {
        let sql = format!(
            "SELECT w.status,
                    COUNT(*) AS scope_count,
                    COUNT(CASE WHEN {WANTED_MISMATCH_RECOVERY_ELIGIBLE_SQL} THEN 1 END)
                        AS mismatch_recovery_eligible,
                    MAX(w.last_search_at) AS latest_search_at
               FROM wanted_items w
              WHERE w.title_id = {{}}
              GROUP BY w.status"
        );
        // A stored status the domain does not know reads as the default, as
        // it does on a row; such groups fold into that status here.
        let mut tallies: Vec<AcquisitionScopeStatusTally> = Vec::new();
        for row in SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?
        {
            let status = AcquisitionScopeStatus::parse(&row.text("status")?).unwrap_or_default();
            let count = row.i64("scope_count")?;
            let eligible = row.i64("mismatch_recovery_eligible")?;
            let latest_search_at = row.opt_timestamp_text("latest_search_at")?;
            match tallies.iter_mut().find(|tally| tally.status == status) {
                Some(tally) => {
                    tally.count += count;
                    tally.mismatch_recovery_eligible += eligible;
                    if latest_search_at > tally.latest_search_at {
                        tally.latest_search_at = latest_search_at;
                    }
                }
                None => tallies.push(AcquisitionScopeStatusTally {
                    status,
                    count,
                    mismatch_recovery_eligible: eligible,
                    latest_search_at,
                }),
            }
        }
        Ok(tallies)
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

/// Most rows one retention batch deletes, so a large backlog never holds the
/// single SQLite writer for one long statement.
const HOUSEKEEPING_DELETE_BATCH: i64 = 5_000;

/// Ids per statement when a title id list is too long for one statement.
const HOUSEKEEPING_TITLE_ID_CHUNK: usize = 900;

/// Deletes every `table` row matching `predicate`, the same rows one
/// unbounded `DELETE ... WHERE predicate` removes, in batches of at most
/// [`HOUSEKEEPING_DELETE_BATCH`] keyed by `key`, one write transaction each.
///
/// A read asks first whether any row matches, so a prune with nothing to do
/// takes no write transaction. `predicate` binds `args` with `{}`.
async fn execute_housekeeping_delete_batched(
    datastore: &StoreDatastore,
    op_name: &'static str,
    table: &'static str,
    key: &'static str,
    predicate: &str,
    args: Vec<SqlArg>,
) -> AppResult<u32> {
    let probe_sql = format!("SELECT 1 AS present FROM {table} WHERE {predicate} LIMIT 1");
    let delete_sql = format!(
        "DELETE FROM {table}
          WHERE {key} IN (SELECT {key} FROM {table} WHERE {predicate} LIMIT {{}})"
    );
    let mut delete_args = args.clone();
    delete_args.push(SqlArg::I64(HOUSEKEEPING_DELETE_BATCH));
    let mut deleted_total = 0u64;
    loop {
        if SqlRuntime::fetch_optional(datastore.read_exec(), &probe_sql, &args)
            .await?
            .is_none()
        {
            break;
        }
        let deleted = SqlRuntime::run_in_transaction(datastore, op_name, |tx| {
            let sql = delete_sql.clone();
            let args = delete_args.clone();
            Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await })
        })
        .await?;
        deleted_total = deleted_total.saturating_add(deleted);
        if deleted < HOUSEKEEPING_DELETE_BATCH as u64 {
            break;
        }
    }
    Ok(deleted_total as u32)
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

    // One transaction as before; a list longer than one statement accepts is
    // split across statements inside it.
    let title_ids = title_ids.to_vec();
    let rows_affected = SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let title_ids = title_ids.clone();
        Box::pin(async move {
            let limit = crate::media::id_list_statements::max_binds(&SqlExec::Tx(tx));
            let (title_ids, per_statement) = crate::media::id_list_statements::id_list_statements(
                &title_ids,
                1,
                0,
                limit,
                HOUSEKEEPING_TITLE_ID_CHUNK,
            );
            let mut deleted = 0u64;
            for chunk in title_ids.chunks(per_statement) {
                let sql = format!(
                    "DELETE FROM {table} WHERE title_id IN ({})",
                    placeholders(chunk.len())
                );
                let args = chunk.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
                deleted += SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await?;
            }
            Ok(deleted)
        })
    })
    .await?;
    Ok(rows_affected as u32)
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
        // The newest terminal run of each job is kept whatever its age. Every
        // user-defined job shares the `custom_job` key, so those runs keep
        // one per operation type (one per script) instead of one in total.
        let deleted = execute_housekeeping_delete_batched(
            &self.datastore,
            "delete_stale_workflow_operations",
            "workflow_operations",
            "id",
            "NOT EXISTS (SELECT 1 FROM workflow_private_inputs WHERE workflow_private_inputs.run_id = workflow_operations.id AND workflow_private_inputs.input_encrypted IS NOT NULL)
               AND job_key IS NOT NULL
                AND (
                    (status = 'completed' AND started_at <= {})
                    OR (status IN ('warning', 'failed') AND started_at <= {})
                )
                AND id NOT IN (
                    SELECT id
                      FROM (
                            SELECT id,
                                   ROW_NUMBER() OVER (
                                       PARTITION BY job_key,
                                           CASE WHEN job_key = 'custom_job'
                                                THEN operation_type
                                                ELSE ''
                                           END
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
        .await?;
        // A request receipt whose run is gone can no longer resolve a replay.
        execute_housekeeping_delete_batched(
            &self.datastore,
            "delete_orphaned_workflow_private_inputs",
            "workflow_private_inputs",
            "run_id",
            "NOT EXISTS (
                 SELECT 1 FROM workflow_operations
                  WHERE workflow_operations.id = workflow_private_inputs.run_id
             )",
            Vec::new(),
        )
        .await?;
        Ok(deleted)
    }

    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete_batched(
            &self.datastore,
            "delete_release_decisions_older_than",
            "release_decisions",
            "id",
            "created_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_title_history_older_than(&self, _days: i64) -> AppResult<u32> {
        // Legacy title_history rows are retired by migration 0085; nothing remains to prune.
        Ok(0)
    }

    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete_batched(
            &self.datastore,
            "delete_release_attempts_older_than",
            "release_download_attempts",
            "id",
            "attempted_at < {}
                AND outcome != 'pending'",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete_batched(
            &self.datastore,
            "delete_history_events_older_than",
            "history_events",
            "id",
            "occurred_at < {}",
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

        let predicate = format!(
            "occurred_at <= {{}}
                AND event_type IN ({})",
            placeholders(event_types.len())
        );
        let mut args = vec![housekeeping_cutoff_arg(days)];
        args.extend(
            event_types
                .iter()
                .map(|event_type| SqlArg::Text(event_type.as_str().to_string())),
        );
        execute_housekeeping_delete_batched(
            &self.datastore,
            "delete_domain_events_older_than_for_types",
            "domain_events",
            "sequence",
            &predicate,
            args,
        )
        .await
    }

    async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete_batched(
            &self.datastore,
            "delete_download_import_artifacts_older_than",
            "download_import_artifacts",
            "id",
            "created_at < {}
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
        execute_housekeeping_delete_batched(
            &self.datastore,
            "delete_terminal_imports_older_than",
            "imports",
            "id",
            "status IN ('completed', 'failed', 'skipped')
                AND updated_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        days: i64,
    ) -> AppResult<u32> {
        execute_housekeeping_delete_batched(
            &self.datastore,
            "delete_terminal_download_queue_commands_older_than",
            "download_queue_commands",
            "id",
            "action = 'delete'
                AND status IN ('completed', 'failed')
                AND updated_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete_batched(
            &self.datastore,
            "delete_rule_set_history_older_than",
            "rule_set_history",
            "id",
            "created_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_lifecycle_evaluation_runs_older_than(&self, days: i64) -> AppResult<u32> {
        execute_housekeeping_delete_batched(
            &self.datastore,
            "delete_lifecycle_evaluation_runs_older_than",
            "lifecycle_evaluation_runs",
            "id",
            "started_at < {}",
            vec![housekeeping_cutoff_arg(days)],
        )
        .await
    }

    async fn delete_held_lifecycle_action_runs_older_than(&self, days: i64) -> AppResult<u32> {
        // A held run whose detail carries a scoped-deletion journal may be the
        // checkpoint an interrupted deletion resumes from, even when later
        // holds of the same generation follow it, so journal rows stay. The
        // newest run of each candidate generation is kept whatever it is:
        // a newer run of the same generation must exist for a row to go.
        execute_housekeeping_delete_batched(
            &self.datastore,
            "delete_held_lifecycle_action_runs_older_than",
            "lifecycle_action_runs",
            "id",
            "status = {}
                AND finished_at IS NOT NULL
                AND finished_at < {}
                AND detail NOT LIKE {} ESCAPE '!'
                AND EXISTS (
                    SELECT 1
                      FROM lifecycle_action_runs newer
                     WHERE newer.candidate_id = lifecycle_action_runs.candidate_id
                       AND newer.match_generation = lifecycle_action_runs.match_generation
                       AND (
                           newer.started_at > lifecycle_action_runs.started_at
                           OR (
                               newer.started_at = lifecycle_action_runs.started_at
                               AND newer.id > lifecycle_action_runs.id
                           )
                       )
                )",
            vec![
                SqlArg::Text(
                    scryer_domain::LifecycleActionRunStatus::Held
                        .as_storage_str()
                        .to_string(),
                ),
                housekeeping_cutoff_arg(days),
                SqlArg::Text("%\"scoped!_deletion\":%".to_string()),
            ],
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
    last_decision_code, release_age_unknown, release_listing_json, http_release_json";

/// Same columns as [`PENDING_RELEASE_COLUMNS`] but qualified with the `pr` alias
/// so the paged read can JOIN `titles` for library scoping without ambiguous
/// column names. The output column names are unchanged.
const PENDING_RELEASE_COLUMNS_PR: &str =
    "pr.id, pr.wanted_item_id, pr.title_id, pr.release_title, pr.release_url, pr.release_size_bytes,
    pr.source_kind, pr.release_score, pr.scoring_log_json, pr.indexer_source, pr.indexer_id, pr.release_guid,
    pr.added_at, pr.last_observed_at, pr.delay_until, pr.status, pr.grabbed_at, pr.source_password, pr.published_at, pr.info_hash,
    pr.minimum_seed_ratio, pr.minimum_seed_time_minutes, pr.season_pack_seed_ratio,
    pr.season_pack_seed_time_minutes, pr.seeders, pr.release_identity, pr.coverage_identity,
    pr.role, pr.last_decision_code, pr.release_age_unknown, pr.release_listing_json,
    pr.http_release_json";

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
        added_at: row.timestamp_text("added_at")?,
        last_observed_at: row.timestamp_text("last_observed_at")?,
        delay_until: row.timestamp_text("delay_until")?,
        status: PendingReleaseStatus::parse(&status).ok_or_else(|| {
            scryer_application::AppError::Repository("invalid pending release status".into())
        })?,
        grabbed_at: row.opt_timestamp_text("grabbed_at")?,
        source_password: decrypt_pending_release_source_password(
            encryption_key,
            row.opt_text("source_password")?,
        )?,
        published_at: row.opt_timestamp_text("published_at")?,
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
        // Rows parked before the column existed read back as `None`.
        release_listing_json: row.opt_text("release_listing_json")?,
        // Rows parked before the column, and every non-HTTP release, read
        // back as `None`; an unreadable payload is reported, not swallowed.
        http_release: decode_pending_release_http_release(row.opt_text("http_release_json")?)?,
    })
}

fn decode_pending_release_http_release(
    json: Option<String>,
) -> AppResult<Option<scryer_plugin_sdk::PluginHttpRelease>> {
    json.map(|json| {
        serde_json::from_str(&json).map_err(|error| {
            scryer_application::AppError::Repository(format!(
                "invalid pending release http_release_json: {error}"
            ))
        })
    })
    .transpose()
}

fn encode_pending_release_http_release(
    release: Option<&scryer_plugin_sdk::PluginHttpRelease>,
) -> AppResult<Option<String>> {
    release
        .map(|release| {
            serde_json::to_string(release).map_err(|error| {
                scryer_application::AppError::Repository(format!(
                    "failed to encode pending release http_release_json: {error}"
                ))
            })
        })
        .transpose()
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
        datastore.timestamp_text_arg(&release.added_at)?,
        datastore.timestamp_text_arg(&observation.last_observed_at)?,
        datastore.timestamp_text_arg(&observation.eligible_at)?,
        SqlArg::Text(release.status.as_str().to_string()),
        datastore.opt_timestamp_text_arg(release.grabbed_at.as_deref())?,
        SqlArg::OptText(encrypt_pending_release_source_password(
            encryption_key,
            release.source_password.as_ref(),
        )?),
        datastore.opt_timestamp_text_arg(release.published_at.as_deref())?,
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
        SqlArg::OptText(release.release_listing_json.clone()),
        SqlArg::OptText(encode_pending_release_http_release(
            release.http_release.as_ref(),
        )?),
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
            let provisional_sql = format!(
                "SELECT id FROM pending_releases
                WHERE coverage_identity = {{}}
                  AND release_age_unknown = {{}}
                  AND status IN {PENDING_RELEASE_ACTIVE_STATUSES_SQL}
                  AND lower(trim(release_title)) = {{}}
                  AND lower(trim(COALESCE(indexer_id, indexer_source, 'unknown'))) = {{}}
                ORDER BY added_at ASC, id ASC
                LIMIT 1"
            );
            let normalized_title = release.release_title.trim().to_ascii_lowercase();
            let normalized_indexer = release
                .indexer_id
                .as_deref()
                .or(release.indexer_source.as_deref())
                .unwrap_or("unknown")
                .trim()
                .to_ascii_lowercase();
            let provisional_args = vec![
                SqlArg::Text(observation.coverage_identity.clone()),
                SqlArg::Bool(true),
                SqlArg::Text(normalized_title),
                SqlArg::Text(normalized_indexer),
            ];
            // The match is a plain read, so a sighting that matches nothing
            // takes no write. The fill re-checks that the row is still
            // active: a row grabbed, expired or superseded after the read is
            // never put back to waiting, and the observation is stored by the
            // insert below instead.
            if let Some(existing) = SqlRuntime::fetch_optional(
                self.datastore.read_exec(),
                &provisional_sql,
                &provisional_args,
            )
            .await?
            {
                let existing_id = existing.text("id")?;
                if self
                    .fill_provisional_pending_release(&existing_id, release, observation)
                    .await?
                {
                    return Ok(existing_id);
                }
            }
        }

        execute_datastore_write(
            &self.datastore,
            "insert_pending_release",
            format!(
                "INSERT INTO pending_releases
             (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
              source_kind, release_score, scoring_log_json, indexer_source, indexer_id, release_guid,
              added_at, last_observed_at, delay_until, status, grabbed_at, source_password, published_at, info_hash,
              minimum_seed_ratio, minimum_seed_time_minutes, season_pack_seed_ratio,
              season_pack_seed_time_minutes, seeders, release_identity, coverage_identity, role,
              last_decision_code, release_age_unknown, release_listing_json, http_release_json)
             VALUES ({{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}},
                     {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}})
             ON CONFLICT(release_identity)
             WHERE status IN {PENDING_RELEASE_ACTIVE_STATUSES_SQL}
             DO UPDATE SET
                release_url = excluded.release_url,
                source_kind = excluded.source_kind,
                release_size_bytes = excluded.release_size_bytes,
                release_score = excluded.release_score,
                scoring_log_json = excluded.scoring_log_json,
                release_listing_json = COALESCE(excluded.release_listing_json, pending_releases.release_listing_json),
                http_release_json = COALESCE(excluded.http_release_json, pending_releases.http_release_json),
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
                last_observed_at = excluded.last_observed_at"
            ),
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
            &format!(
                "SELECT id FROM pending_releases
                  WHERE release_identity = {{}}
                    AND status IN {PENDING_RELEASE_ACTIVE_STATUSES_SQL}
                  ORDER BY added_at ASC, id ASC
                  LIMIT 1"
            ),
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
            &[self.datastore.timestamp_text_arg(now)?],
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
        let sql = active_release_age_unknown_pending_releases_sql(&self.datastore);
        let encryption_key = self.encryption_key()?;
        fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[],
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

    async fn list_active_pending_releases_for_title(
        &self,
        title_id: &str,
        replay_ids: &[String],
    ) -> AppResult<Vec<PendingRelease>> {
        let encryption_key = self.encryption_key()?;
        let sql = format!(
            "SELECT {PENDING_RELEASE_COLUMNS} FROM pending_releases
             WHERE title_id = {{}} AND status IN {PENDING_RELEASE_ACTIVE_STATUSES_SQL}"
        );
        let mut rows = fetch_pending_releases(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_owned())],
            encryption_key.as_ref(),
        )
        .await?;
        let mut seen: std::collections::HashSet<String> =
            rows.iter().map(|row| row.id.clone()).collect();
        let ids: Vec<_> = replay_ids
            .iter()
            .filter(|id| seen.insert((*id).clone()))
            .collect();
        // Keep bind counts bounded even when a title has a large replay backlog.
        for chunk in ids.chunks(400) {
            let binds = vec!["{}"; chunk.len()].join(", ");
            let sql = format!(
                "SELECT {PENDING_RELEASE_COLUMNS} FROM pending_releases
                WHERE title_id = {{}} AND id IN ({binds})"
            );
            let mut args = vec![SqlArg::Text(title_id.to_owned())];
            args.extend(chunk.iter().map(|id| SqlArg::Text(id.as_str().to_owned())));
            rows.extend(
                fetch_pending_releases(
                    self.datastore.read_exec(),
                    &sql,
                    &args,
                    encryption_key.as_ref(),
                )
                .await?,
            );
        }
        rows.sort_by(|left, right| right.added_at.cmp(&left.added_at));
        Ok(rows)
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
            crate::workflow::stores::UPDATE_PENDING_RELEASE_STATUS_SQL,
            vec![
                SqlArg::Text(status.as_str().to_string()),
                self.datastore.opt_timestamp_text_arg(grabbed_at)?,
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
            // Only an active row expires: one grabbed or superseded since the
            // caller read it keeps its status.
            format!(
                "UPDATE pending_releases
                    SET status = 'expired', last_decision_code = {{}}
                  WHERE id = {{}}
                    AND status IN {PENDING_RELEASE_ACTIVE_STATUSES_SQL}"
            ),
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
                self.datastore.opt_timestamp_text_arg(Some(delay_until))?,
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

    async fn list_standby_wanted_item_ids(&self) -> AppResult<Vec<String>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT DISTINCT wanted_item_id
               FROM pending_releases
              WHERE status = 'standby'
              ORDER BY wanted_item_id ASC",
            &[],
        )
        .await?
        .iter()
        .map(|row| row.text("wanted_item_id"))
        .collect()
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
            crate::workflow::stores::COMPARE_AND_SET_PENDING_RELEASE_STATUS_SQL,
            vec![
                SqlArg::Text(next_status.as_str().to_string()),
                self.datastore.opt_timestamp_text_arg(grabbed_at)?,
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
        if lower_or_equal_ids.is_empty() {
            return Ok(());
        }
        let (ids, per_statement) = id_list_statements(
            lower_or_equal_ids,
            1,
            0,
            max_binds(&self.datastore.read_exec()),
            ID_LIST_CHUNK,
        );
        for chunk in ids.chunks(per_statement) {
            let placeholders = vec!["{}"; chunk.len()].join(", ");
            execute_datastore_write(
                &self.datastore,
                "retire_lower_or_equal_overlapping_pending_release",
                format!(
                    "UPDATE pending_releases
                        SET status = 'superseded',
                            last_decision_code = 'grabbed_overlap_retired'
                      WHERE id IN ({placeholders})
                        AND status IN {PENDING_RELEASE_ACTIVE_STATUSES_SQL}"
                ),
                chunk.iter().cloned().map(SqlArg::Text).collect(),
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
        created_at: row.timestamp_text("created_at")?,
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
                self.datastore
                    .timestamp_text_arg(&Utc::now().to_rfc3339())?,
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
        downloaded_at: row.timestamp_text("downloaded_at")?,
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
        modified_at: row.opt_timestamp_text("modified_at")?,
        language: row.opt_text("language")?,
        hearing_impaired: row.opt_bool("hearing_impaired")?,
        detection_source_language,
        detection_source_hi,
        probe_version: row.i32("probe_version")?,
        updated_at: row.timestamp_text("updated_at")?,
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
        created_at: row.timestamp_text("created_at")?,
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
                self.datastore.timestamp_text_arg(&download.downloaded_at)?,
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
                self.datastore.opt_timestamp_text_arg(entry.modified_at.as_deref())?,
                SqlArg::OptText(entry.language.clone()),
                SqlArg::OptBool(entry.hearing_impaired),
                SqlArg::Text(entry.detection_source_language.as_str().to_string()),
                SqlArg::Text(entry.detection_source_hi.as_str().to_string()),
                SqlArg::I32(entry.probe_version),
                self.datastore.timestamp_text_arg(&entry.updated_at)?,
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

    async fn list_search_backoffs_for_media_files(
        &self,
        media_file_ids: &[String],
    ) -> AppResult<Vec<SubtitleSearchBackoff>> {
        if media_file_ids.is_empty() {
            return Ok(Vec::new());
        }
        let exec = self.datastore.read_exec();
        let (ids, per_statement) = id_list_statements(
            media_file_ids,
            1,
            0,
            max_binds(&exec),
            SEARCH_BACKOFF_ID_CHUNK,
        );
        let mut rows = Vec::new();
        for chunk in ids.chunks(per_statement) {
            let sql = format!(
                "SELECT {SUBTITLE_SEARCH_BACKOFF_COLUMNS}
                   FROM subtitle_search_backoffs
                  WHERE media_file_id IN ({})",
                placeholders(chunk.len())
            );
            let args = chunk.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
            for row in SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await? {
                rows.push(subtitle_search_backoff_row_to_entry(&row)?);
            }
        }
        Ok(rows)
    }

    async fn apply_search_backoffs(
        &self,
        media_file_id: &str,
        upserts: &[SubtitleSearchBackoff],
        cleared: &[SubtitleSearchBackoffKey],
    ) -> AppResult<()> {
        if upserts.is_empty() && cleared.is_empty() {
            return Ok(());
        }
        let mut statements = Vec::with_capacity(upserts.len() + cleared.len());
        for key in cleared {
            statements.push((
                "DELETE FROM subtitle_search_backoffs
                  WHERE media_file_id = {} AND language = {}
                    AND hearing_impaired = {} AND forced = {}",
                vec![
                    SqlArg::Text(media_file_id.to_string()),
                    SqlArg::Text(key.language.clone()),
                    SqlArg::Bool(key.hearing_impaired),
                    SqlArg::Bool(key.forced),
                ],
            ));
        }
        for row in upserts
            .iter()
            .filter(|row| row.media_file_id == media_file_id)
        {
            statements.push((
                "INSERT INTO subtitle_search_backoffs
                 (media_file_id, language, hearing_impaired, forced, file_path, size_bytes,
                  wanted_languages, enabled_providers, empty_searches, searched_at, retry_after)
                 VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                 ON CONFLICT (media_file_id, language, hearing_impaired, forced) DO UPDATE SET
                    file_path = excluded.file_path,
                    size_bytes = excluded.size_bytes,
                    wanted_languages = excluded.wanted_languages,
                    enabled_providers = excluded.enabled_providers,
                    empty_searches = excluded.empty_searches,
                    searched_at = excluded.searched_at,
                    retry_after = excluded.retry_after",
                vec![
                    SqlArg::Text(row.media_file_id.clone()),
                    SqlArg::Text(row.language.clone()),
                    SqlArg::Bool(row.hearing_impaired),
                    SqlArg::Bool(row.forced),
                    SqlArg::Text(row.file_path.clone()),
                    SqlArg::I64(row.size_bytes),
                    SqlArg::Text(row.wanted_languages.clone()),
                    SqlArg::Text(row.enabled_providers.clone()),
                    SqlArg::I32(row.empty_searches),
                    self.datastore.timestamp_text_arg(&row.searched_at)?,
                    self.datastore.timestamp_text_arg(&row.retry_after)?,
                ],
            ));
        }
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "apply_subtitle_search_backoffs",
            move |tx| {
                let statements = statements.clone();
                Box::pin(async move {
                    for (sql, args) in statements {
                        SqlRuntime::execute(SqlExec::Tx(tx), sql, &args).await?;
                    }
                    Ok(())
                })
            },
        )
        .await
    }
}

/// Media file ids per `IN (...)` read of the search backoff table, used only
/// when one statement could not bind the whole list.
const SEARCH_BACKOFF_ID_CHUNK: usize = 900;

const SUBTITLE_SEARCH_BACKOFF_COLUMNS: &str =
    "media_file_id, language, hearing_impaired, forced, file_path, size_bytes,
    wanted_languages, enabled_providers, empty_searches, searched_at, retry_after";

fn subtitle_search_backoff_row_to_entry(row: &SqlRow) -> AppResult<SubtitleSearchBackoff> {
    Ok(SubtitleSearchBackoff {
        media_file_id: row.text("media_file_id")?,
        language: row.text("language")?,
        hearing_impaired: row.bool("hearing_impaired")?,
        forced: row.bool("forced")?,
        file_path: row.text("file_path")?,
        size_bytes: row.i64("size_bytes")?,
        wanted_languages: row.text("wanted_languages")?,
        enabled_providers: row.text("enabled_providers")?,
        empty_searches: row.i32("empty_searches")?,
        searched_at: row.timestamp_text("searched_at")?,
        retry_after: row.timestamp_text("retry_after")?,
    })
}

#[cfg(test)]
mod title_id_delete_tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::delete_for_title_ids_shared;
    use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

    async fn seeded() -> StoreDatastore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        let datastore = StoreDatastore::Sqlite {
            pool,
            writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        };
        SqlRuntime::execute(
            datastore.read_exec(),
            "CREATE TABLE title_rows (id INTEGER PRIMARY KEY, title_id TEXT NOT NULL)",
            &[],
        )
        .await
        .expect("create table");
        for title_id in ["t-a", "t-a", "t-b", "t-c", "t-d", "t-e", "t-e"] {
            SqlRuntime::execute(
                datastore.read_exec(),
                "INSERT INTO title_rows (title_id) VALUES ({})",
                &[SqlArg::Text(title_id.to_string())],
            )
            .await
            .expect("seed row");
        }
        datastore
    }

    async fn remaining(datastore: &StoreDatastore) -> Vec<String> {
        SqlRuntime::fetch_all(
            datastore.read_exec(),
            "SELECT title_id FROM title_rows ORDER BY id",
            &[],
        )
        .await
        .expect("read rows")
        .iter()
        .map(|row| row.text("title_id").expect("title_id"))
        .collect()
    }

    /// A list longer than one statement can bind removes the same rows, and
    /// reports the same count, as the single statement does.
    #[tokio::test]
    async fn a_title_id_list_past_the_bind_limit_deletes_what_one_statement_would() {
        let ids: Vec<String> = ["t-e", "t-a", "t-e", "t-c", "t-missing"]
            .iter()
            .map(|id| id.to_string())
            .collect();

        let single = seeded().await;
        let single_count = delete_for_title_ids_shared(&single, "test", "title_rows", &ids)
            .await
            .expect("single statement");

        let chunked = seeded().await;
        let chunked_count = {
            let _limit = crate::media::id_list_statements::lower_max_binds_for_tests(2);
            delete_for_title_ids_shared(&chunked, "test", "title_rows", &ids)
                .await
                .expect("chunked statements")
        };

        assert_eq!(single_count, 5);
        assert_eq!(chunked_count, single_count);
        assert_eq!(remaining(&chunked).await, remaining(&single).await);
        assert_eq!(remaining(&single).await, vec!["t-b", "t-d"]);
    }
}

#[cfg(test)]
mod subtitle_search_backoff_tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::{SubtitleDownloadStore, SubtitleSearchBackoff, SubtitleSearchBackoffKey};
    use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};
    use scryer_application::SubtitleDownloadRepository;

    async fn store() -> (SubtitleDownloadStore, StoreDatastore) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("PRAGMA foreign_keys = ON")
                        .execute(connection)
                        .await
                        .map(|_| ())
                })
            })
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        let datastore = StoreDatastore::Sqlite {
            pool,
            writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        };
        SqlRuntime::execute(
            datastore.read_exec(),
            "CREATE TABLE media_files (id TEXT PRIMARY KEY)",
            &[],
        )
        .await
        .expect("create media files");
        SqlRuntime::execute(
            datastore.read_exec(),
            include_str!(
                "../../../../../scryer/src/db/migrations/0340_subtitle_search_backoffs.sql"
            ),
            &[],
        )
        .await
        .expect("apply backoff migration");
        for id in ["mf-emberfall", "mf-cinderwake"] {
            SqlRuntime::execute(
                datastore.read_exec(),
                "INSERT INTO media_files (id) VALUES ({})",
                &[SqlArg::Text(id.to_string())],
            )
            .await
            .expect("seed media file");
        }
        (SubtitleDownloadStore::new(datastore.clone()), datastore)
    }

    fn row(media_file_id: &str, language: &str, empty_searches: i32) -> SubtitleSearchBackoff {
        SubtitleSearchBackoff {
            media_file_id: media_file_id.to_string(),
            language: language.to_string(),
            hearing_impaired: false,
            forced: false,
            file_path: format!("/library/{media_file_id}.mkv"),
            size_bytes: 5,
            wanted_languages: "eng:0:0,swe:0:0".to_string(),
            enabled_providers: "cinderwake-subtitles".to_string(),
            empty_searches,
            searched_at: "2026-01-01T00:00:00+00:00".to_string(),
            retry_after: "2026-01-02T00:00:00+00:00".to_string(),
        }
    }

    fn sorted(mut rows: Vec<SubtitleSearchBackoff>) -> Vec<SubtitleSearchBackoff> {
        rows.sort_by(|a, b| (&a.media_file_id, &a.language).cmp(&(&b.media_file_id, &b.language)));
        rows
    }

    #[tokio::test]
    async fn backoff_rows_upsert_clear_and_follow_their_media_file() {
        let (store, datastore) = store().await;
        let ids = vec!["mf-emberfall".to_string(), "mf-cinderwake".to_string()];
        store
            .apply_search_backoffs(
                "mf-emberfall",
                &[row("mf-emberfall", "eng", 1), row("mf-emberfall", "swe", 1)],
                &[],
            )
            .await
            .unwrap();
        store
            .apply_search_backoffs("mf-cinderwake", &[row("mf-cinderwake", "eng", 1)], &[])
            .await
            .unwrap();

        // An upsert replaces the row for the same file and language.
        store
            .apply_search_backoffs("mf-emberfall", &[row("mf-emberfall", "eng", 2)], &[])
            .await
            .unwrap();
        assert_eq!(
            sorted(
                store
                    .list_search_backoffs_for_media_files(&ids)
                    .await
                    .unwrap()
            ),
            vec![
                row("mf-cinderwake", "eng", 1),
                row("mf-emberfall", "eng", 2),
                row("mf-emberfall", "swe", 1),
            ]
        );

        // A list past the bind limit reads the same rows as one statement.
        let one_statement = sorted(
            store
                .list_search_backoffs_for_media_files(&ids)
                .await
                .unwrap(),
        );
        {
            let _limit = crate::media::id_list_statements::lower_max_binds_for_tests(1);
            assert_eq!(
                sorted(
                    store
                        .list_search_backoffs_for_media_files(&ids)
                        .await
                        .unwrap()
                ),
                one_statement
            );
        }

        // Clearing removes only that file's language.
        store
            .apply_search_backoffs(
                "mf-emberfall",
                &[],
                &[SubtitleSearchBackoffKey {
                    language: "eng".to_string(),
                    hearing_impaired: false,
                    forced: false,
                }],
            )
            .await
            .unwrap();
        assert_eq!(
            sorted(
                store
                    .list_search_backoffs_for_media_files(&ids)
                    .await
                    .unwrap()
            ),
            vec![
                row("mf-cinderwake", "eng", 1),
                row("mf-emberfall", "swe", 1)
            ]
        );

        // Removing the media file removes its rows and no other file's.
        SqlRuntime::execute(
            datastore.read_exec(),
            "DELETE FROM media_files WHERE id = {}",
            &[SqlArg::Text("mf-emberfall".to_string())],
        )
        .await
        .unwrap();
        assert_eq!(
            store
                .list_search_backoffs_for_media_files(&ids)
                .await
                .unwrap(),
            vec![row("mf-cinderwake", "eng", 1)]
        );
    }
}

#[cfg(test)]
mod pending_release_fill_tests {
    use std::sync::{Arc, RwLock};

    use chrono::{Duration, Utc};
    use scryer_application::{
        DownloadSourceKind, PendingRelease, PendingReleaseObservation, PendingReleaseRepository,
        PendingReleaseRole, PendingReleaseStatus,
    };

    use super::PendingReleaseStore;
    use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

    async fn store() -> (StoreDatastore, PendingReleaseStore) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
            &pool, None, true,
        )
        .await
        .unwrap();
        let datastore = StoreDatastore::sqlite(pool.clone(), Arc::new(tokio::sync::Mutex::new(())));
        // The guard under test is the status check on the fill; the title and
        // scope rows the foreign keys name are not part of it.
        SqlRuntime::execute(datastore.read_exec(), "PRAGMA foreign_keys = OFF", &[])
            .await
            .unwrap();
        let store = PendingReleaseStore::new(datastore.clone(), Arc::new(RwLock::new(None)));
        (datastore, store)
    }

    /// A release first seen without a publish time, and that observation.
    fn provisional(name: &str) -> (PendingRelease, PendingReleaseObservation) {
        let now = Utc::now();
        let release = PendingRelease {
            http_release: None,
            id: format!("pr-fill-{name}"),
            wanted_item_id: "w-fill".to_string(),
            title_id: "t-fill".to_string(),
            release_title: format!("Synthetic.Fill.{name}.1080p.WEB-DL"),
            release_url: Some(format!("https://example.com/{name}.nzb")),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: Some(1024),
            release_score: 100,
            scoring_log_json: None,
            indexer_source: Some("Synthetic Indexer".to_string()),
            indexer_id: Some("synthetic-indexer".to_string()),
            release_guid: Some(format!("guid-{name}")),
            added_at: now.to_rfc3339(),
            last_observed_at: now.to_rfc3339(),
            delay_until: (now + Duration::minutes(5)).to_rfc3339(),
            status: PendingReleaseStatus::Waiting,
            grabbed_at: None,
            source_password: None,
            published_at: None,
            info_hash: None,
            seed_minimums: Default::default(),
            seeders: None,
            release_identity: String::new(),
            coverage_identity: String::new(),
            role: PendingReleaseRole::Primary,
            last_decision_code: None,
            release_age_unknown: true,
            release_listing_json: None,
        };
        let observation = PendingReleaseObservation {
            eligible_at: release.delay_until.clone(),
            last_observed_at: now.to_rfc3339(),
            latest_decision_code: None,
            release_identity: format!("guid:synthetic-indexer:guid-{name}"),
            coverage_identity: "scope:w-fill".to_string(),
            role: PendingReleaseRole::Primary,
            release_age_unknown: true,
        };
        (release, observation)
    }

    fn with_publish_time(
        release: &PendingRelease,
        observation: &PendingReleaseObservation,
    ) -> (PendingRelease, PendingReleaseObservation) {
        let mut hydrated = release.clone();
        hydrated.published_at = Some((Utc::now() - Duration::minutes(30)).to_rfc3339());
        let observation = PendingReleaseObservation {
            release_age_unknown: false,
            ..observation.clone()
        };
        (hydrated, observation)
    }

    async fn status_and_unknown(datastore: &StoreDatastore, id: &str) -> (String, bool) {
        let row = SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT status, release_age_unknown FROM pending_releases WHERE id = {}",
            &[SqlArg::Text(id.to_string())],
        )
        .await
        .unwrap()
        .expect("row exists");
        (
            row.text("status").unwrap(),
            row.bool("release_age_unknown").unwrap(),
        )
    }

    /// The fill runs after the provisional read. A row that left the active
    /// set between the read and the fill is the race the status check
    /// covers; calling the fill with that row's id reproduces it without
    /// timing.
    #[tokio::test]
    async fn the_fill_never_revives_a_row_grabbed_after_it_was_matched() {
        let (datastore, store) = store().await;

        let (active, active_observation) = provisional("active");
        let active_id = store
            .insert_pending_release_observation(&active, &active_observation)
            .await
            .unwrap();
        let (hydrated, hydrated_observation) = with_publish_time(&active, &active_observation);
        assert!(
            store
                .fill_provisional_pending_release(&active_id, &hydrated, &hydrated_observation)
                .await
                .unwrap()
        );
        assert_eq!(
            status_and_unknown(&datastore, &active_id).await,
            ("waiting".to_string(), false)
        );

        // Matched while waiting, then grabbed before the fill lands.
        let (matched, matched_observation) = provisional("matched");
        let matched_id = store
            .insert_pending_release_observation(&matched, &matched_observation)
            .await
            .unwrap();
        store
            .update_pending_release_status(
                &matched_id,
                PendingReleaseStatus::Grabbed,
                Some(&Utc::now().to_rfc3339()),
            )
            .await
            .unwrap();
        let (hydrated, hydrated_observation) = with_publish_time(&matched, &matched_observation);
        assert!(
            !store
                .fill_provisional_pending_release(&matched_id, &hydrated, &hydrated_observation)
                .await
                .unwrap()
        );
        assert_eq!(
            status_and_unknown(&datastore, &matched_id).await,
            ("grabbed".to_string(), true)
        );

        // The sighting itself is stored as its own active row.
        let sighting = PendingRelease {
            id: "pr-fill-matched-again".to_string(),
            ..hydrated
        };
        let stored_id = store
            .insert_pending_release_observation(&sighting, &hydrated_observation)
            .await
            .unwrap();
        assert_ne!(stored_id, matched_id);
        assert_eq!(
            status_and_unknown(&datastore, &stored_id).await,
            ("waiting".to_string(), false)
        );
        assert_eq!(
            status_and_unknown(&datastore, &matched_id).await,
            ("grabbed".to_string(), true)
        );
    }
}

#[cfg(test)]
mod keyed_scope_completion_tests {
    use std::sync::Arc;

    use chrono::Utc;
    use scryer_application::{
        AcquisitionScopeKey, AcquisitionScopeState, AcquisitionScopeStateRepository,
        AcquisitionScopeStatus,
    };

    use super::WantedStore;
    use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

    const TITLE_ID: &str = "t-link-completion";
    const RELEASE: &str = "{\"title\":\"Synthetic.Feature.2031.1080p-GROUP\"}";
    const SEEDED_AT: &str = "2031-02-01T00:00:00+00:00";

    async fn store() -> (StoreDatastore, WantedStore) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
            &pool, None, true,
        )
        .await
        .unwrap();
        let datastore = StoreDatastore::sqlite(pool.clone(), Arc::new(tokio::sync::Mutex::new(())));
        // The statement under test is keyed on the scope columns; the title
        // and link rows the foreign keys name are not part of it.
        SqlRuntime::execute(datastore.read_exec(), "PRAGMA foreign_keys = OFF", &[])
            .await
            .unwrap();
        (datastore.clone(), WantedStore::new(datastore))
    }

    async fn seed_link_scope(
        datastore: &StoreDatastore,
        id: &str,
        link_id: &str,
        status: AcquisitionScopeStatus,
    ) {
        SqlRuntime::execute(
            datastore.read_exec(),
            "INSERT INTO wanted_items
             (id, title_id, series_movie_link_id, media_type, last_search_at, status,
              grabbed_release, created_at, updated_at)
             VALUES ({}, {}, {}, 'series_movie', '2031-01-01T00:00:00+00:00', {}, {}, {}, {})",
            &[
                SqlArg::Text(id.to_string()),
                SqlArg::Text(TITLE_ID.to_string()),
                SqlArg::Text(link_id.to_string()),
                SqlArg::Text(status.as_str().to_string()),
                SqlArg::Text(RELEASE.to_string()),
                SqlArg::Text(SEEDED_AT.to_string()),
                SqlArg::Text(SEEDED_AT.to_string()),
            ],
        )
        .await
        .unwrap();
    }

    async fn link_scope(store: &WantedStore, link_id: &str) -> AcquisitionScopeState {
        store
            .get_acquisition_scope_state_for_scope(
                TITLE_ID,
                AcquisitionScopeKey::SeriesMovieLink(link_id),
            )
            .await
            .unwrap()
            .expect("the link scope exists")
    }

    /// The fields the completion writes, and nothing it leaves alone.
    fn written(scope: &AcquisitionScopeState) -> impl std::fmt::Debug + PartialEq {
        (
            scope.status,
            scope.last_search_at.clone(),
            scope.grabbed_release.clone(),
            scope.episode_id.clone(),
            scope.collection_id.clone(),
            scope.series_movie_link_id.is_some(),
            scope.media_type.clone(),
            scope.created_at.clone(),
        )
    }

    #[tokio::test]
    async fn a_keyed_link_completion_writes_what_the_read_then_write_path_wrote_in_every_status() {
        let (datastore, store) = store().await;
        let now = Utc::now().to_rfc3339();
        for status in [
            AcquisitionScopeStatus::Wanted,
            AcquisitionScopeStatus::Grabbed,
            AcquisitionScopeStatus::Paused,
            AcquisitionScopeStatus::Completed,
        ] {
            for landed_import in [true, false] {
                let case = format!("{}-{landed_import}", status.as_str());
                let old_link = format!("old-link-{case}");
                let new_link = format!("new-link-{case}");
                seed_link_scope(&datastore, &format!("old-{case}"), &old_link, status).await;
                seed_link_scope(&datastore, &format!("new-{case}"), &new_link, status).await;

                // The path the import used: read the row, then complete it by id.
                let read = link_scope(&store, &old_link).await;
                store
                    .update_acquisition_scope_status(
                        &read.id,
                        AcquisitionScopeStatus::Completed.as_str(),
                        Some(&now),
                        if landed_import {
                            None
                        } else {
                            read.grabbed_release.as_deref()
                        },
                    )
                    .await
                    .unwrap();

                let found = store
                    .complete_acquisition_scope_for_key(
                        TITLE_ID,
                        AcquisitionScopeKey::SeriesMovieLink(&new_link),
                        Some(&now),
                        landed_import,
                    )
                    .await
                    .unwrap();
                assert!(found, "{case}: the row is there");

                let old = link_scope(&store, &old_link).await;
                let new = link_scope(&store, &new_link).await;
                assert_eq!(written(&new), written(&old), "{case}");
                assert_eq!(new.status, AcquisitionScopeStatus::Completed, "{case}");
                assert_eq!(
                    new.grabbed_release.as_deref(),
                    (!landed_import).then_some(RELEASE),
                    "{case}"
                );
                assert_ne!(new.updated_at, SEEDED_AT, "{case}: the row is stamped");
            }
        }
    }

    #[tokio::test]
    async fn a_keyed_link_completion_without_a_row_writes_nothing() {
        let (datastore, store) = store().await;
        seed_link_scope(
            &datastore,
            "kept",
            "kept-link",
            AcquisitionScopeStatus::Grabbed,
        )
        .await;
        let before = link_scope(&store, "kept-link").await;

        let found = store
            .complete_acquisition_scope_for_key(
                TITLE_ID,
                AcquisitionScopeKey::SeriesMovieLink("link-without-row"),
                Some(&Utc::now().to_rfc3339()),
                true,
            )
            .await
            .unwrap();

        assert!(!found);
        assert_eq!(
            format!("{:?}", link_scope(&store, "kept-link").await),
            format!("{before:?}")
        );
        let rows = SqlRuntime::fetch_all(datastore.read_exec(), "SELECT id FROM wanted_items", &[])
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "no row is created");
    }
}

#[cfg(test)]
mod index_plan_tests {
    use std::sync::Arc;

    use super::{
        RELEASE_DECISION_IDENTITY_SELECT_SQL, WantedProjection,
        active_release_age_unknown_pending_releases_sql, grabbed_scopes_for_titles_sql,
        mismatch_recovery_scopes_for_title_statement, wanted_item_projection_sql,
    };
    use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

    /// The plans are sqlite's, over the full migration catalog.
    async fn datastore() -> StoreDatastore {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
            &pool, None, true,
        )
        .await
        .unwrap();
        StoreDatastore::sqlite(pool, Arc::new(tokio::sync::Mutex::new(())))
    }

    async fn query_plan(datastore: &StoreDatastore, sql: &str, args: &[SqlArg]) -> Vec<String> {
        SqlRuntime::fetch_all(
            datastore.read_exec(),
            &format!("EXPLAIN QUERY PLAN {sql}"),
            args,
        )
        .await
        .expect("query plan should read")
        .iter()
        .map(|row| row.text("detail").expect("plan detail"))
        .collect()
    }

    #[tokio::test]
    async fn unknown_age_pending_releases_are_read_through_their_partial_index() {
        let datastore = datastore().await;
        let sql = active_release_age_unknown_pending_releases_sql(&datastore);
        let plan = query_plan(&datastore, &sql, &[]).await;
        assert!(
            plan.iter().any(
                |line| line.starts_with("SEARCH pending_releases USING INDEX ")
                    && line.contains("idx_pending_releases_active_unknown_age ")
            ),
            "unknown-age pending releases must be sought in the partial index: {plan:#?}"
        );
        assert!(
            !plan.iter().any(|line| line.starts_with("SCAN")),
            "the pending releases table must not be scanned: {plan:#?}"
        );
    }

    #[tokio::test]
    async fn release_decision_identity_lookup_seeks_the_identity_index() {
        let datastore = datastore().await;
        let args = [
            SqlArg::Text("scope-1".to_string()),
            SqlArg::Text("title_mismatch".to_string()),
            SqlArg::Text(String::new()),
            SqlArg::Text("Synthetic.Release".to_string()),
            SqlArg::I64(-1),
        ];
        let plan = query_plan(&datastore, RELEASE_DECISION_IDENTITY_SELECT_SQL, &args).await;
        assert!(
            plan.iter().any(|line| line
                == "SEARCH release_decisions USING INDEX idx_release_decisions_identity \
                    (wanted_item_id=? AND release_title=? AND decision_code=?)"),
            "a decision's identity must be sought on scope, release and code: {plan:#?}"
        );
    }

    /// The literal status and the newest-first order must not pull the read
    /// onto the status index, which holds every grabbed scope of the library.
    #[tokio::test]
    async fn grabbed_scopes_for_titles_are_sought_by_title() {
        let datastore = datastore().await;
        let select = wanted_item_projection_sql(WantedProjection::WithoutLatestDecision);
        for count in [1, 3, 1_000] {
            let sql = grabbed_scopes_for_titles_sql(&select, count);
            let args = (0..count)
                .map(|n| SqlArg::Text(format!("title-{n}")))
                .collect::<Vec<_>>();
            let plan = query_plan(&datastore, &sql, &args).await;
            assert!(
                plan.iter().any(|line| {
                    // The `(title_id, episode_id)` unique key or the
                    // `(title_id, id)` index: both lead with the title.
                    line == "SEARCH w USING INDEX sqlite_autoindex_wanted_items_2 (title_id=?)"
                        || line == "SEARCH w USING INDEX idx_wanted_items_title_id_id (title_id=?)"
                }),
                "{count} titles: grabbed scopes must be sought by title id: {plan:#?}"
            );
            assert!(
                !plan
                    .iter()
                    .any(|line| line.contains("idx_wanted_items_status_updated_at")),
                "{count} titles: the status index reads every grabbed scope: {plan:#?}"
            );
            assert!(
                !plan.iter().any(|line| line.starts_with("SCAN w")),
                "{count} titles: the wanted table must not be scanned: {plan:#?}"
            );
        }
    }

    #[tokio::test]
    async fn mismatch_recovery_pages_read_a_title_in_id_order_without_sorting() {
        let datastore = datastore().await;
        for after_id in [None, Some("scope-1")] {
            let (sql, args) = mismatch_recovery_scopes_for_title_statement("title-1", after_id, 50);
            let plan = query_plan(&datastore, &sql, &args).await;
            assert!(
                plan.iter()
                    .any(|line| line.starts_with("SEARCH w USING INDEX ")
                        && line.contains("idx_wanted_items_title_id_id ")),
                "a title's page must be sought in the title and id index: {plan:#?}"
            );
            assert!(
                !plan.iter().any(|line| line.contains("TEMP B-TREE")),
                "a title's page must come back in id order without a sort: {plan:#?}"
            );
            assert!(
                !plan.iter().any(|line| line.starts_with("SCAN w")),
                "the wanted table must not be scanned: {plan:#?}"
            );
        }
    }
}

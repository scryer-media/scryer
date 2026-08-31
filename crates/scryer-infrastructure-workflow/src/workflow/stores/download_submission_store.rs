use super::*;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, CanonicalDownloadIdentityDisposition, ClientJobLocator, DownloadOrigin,
    DownloadSubmission, DownloadSubmissionActorSnapshot, DownloadSubmissionIdentity,
    DownloadSubmissionRepository, IdentityTrackedStateTarget, PersistedSeedGoals,
    SeedGoalResolutionSource, TerminalDownloadHistoryRow,
};
use scryer_domain::{Id, TrackedDownloadState, download_identity::DownloadId};

use super::unique_violation::run_in_transaction_retrying_unique_violation;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

#[derive(Clone)]
pub struct DownloadSubmissionStore {
    datastore: StoreDatastore,
}

impl DownloadSubmissionStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    async fn find_by_canonical_download_id(
        &self,
        canonical_download_id: &DownloadId,
    ) -> AppResult<Option<DownloadSubmission>> {
        let sql = download_submission_select_sql(&self.datastore, "WHERE id = {} LIMIT 1");
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(canonical_download_id.to_string())],
        )
        .await?;
        row.map(|row| download_submission_from_row(&row))
            .transpose()
    }

    async fn active_binding_download_id(
        &self,
        locator: &ClientJobLocator,
    ) -> AppResult<Option<DownloadId>> {
        active_binding_download_id(self.datastore.read_exec(), locator).await
    }

    async fn get_seed_goals_by_canonical_download_id(
        &self,
        canonical_download_id: &DownloadId,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SEED_GOAL_COLUMNS}
                 FROM download_submissions
                 WHERE id = {{}}
                 LIMIT 1"
            ),
            &[SqlArg::Text(canonical_download_id.to_string())],
        )
        .await?;
        row.map(|row| seed_goals_from_row(&row))
            .transpose()
            .map(Option::flatten)
    }
}

async fn active_binding_download_id(
    exec: SqlExec<'_, '_>,
    locator: &ClientJobLocator,
) -> AppResult<Option<DownloadId>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT download_id
         FROM download_client_bindings
         WHERE ended_at IS NULL
           AND native_item_id IS NOT NULL
           AND COALESCE(client_config_id, '') = {}
           AND LOWER(TRIM(COALESCE(client_type_snapshot, ''))) = {}
           AND native_item_id = {}
         ORDER BY created_at, download_id
         LIMIT 1",
        &[
            SqlArg::Text(locator.client_id.clone().unwrap_or_default()),
            SqlArg::Text(locator.client_type.clone()),
            SqlArg::Text(locator.item_id.clone()),
        ],
    )
    .await?;
    row.map(|row| {
        let value = row.text("download_id")?;
        DownloadId::parse(&value).ok_or_else(|| {
            AppError::Repository(format!(
                "active client binding has invalid download id {value:?}"
            ))
        })
    })
    .transpose()
}

/// Resolve a locator to its active canonical download, or bind a new canonical
/// row to it. A caller-provided id denotes an accepted Scryer submission;
/// existing locator bindings are adopted and their parent is upgraded to
/// `scryer_submission`. Without one, this preserves the tracked-state stub
/// behavior by creating a foreign-observation parent.
///
/// Adoption covers the live job: dedup-by-hash clients hand back the same
/// native job for a re-submitted release, and one active binding per locator is
/// the identity invariant. A binding whose download already reached a terminal
/// outcome is *not* live — the job left the client (or was deleted out from
/// under us) and only the binding row lagged behind. Adopting it would hand a
/// fresh grab a download id that already carries an import or failure history,
/// so the whole grab reads as already finished. Such a binding is ended here
/// and the claim proceeds as if the locator were unbound — for an accepted
/// grab whatever the parent's origin, and on the tracked-state stub path only
/// for foreign parents (see below).
///
/// The read-then-insert is not atomic under a datastore with concurrent
/// writers: two claimants can both find the locator unbound (or both end the
/// same stale binding) and the loser's insert then hits the 0180 active-locator
/// unique index. Nothing is recovered here — the transaction is already lost.
/// Every public store method whose transaction reaches this helper is wrapped
/// in [`run_in_transaction_retrying_unique_violation`] instead, so the whole
/// method re-runs once in a fresh transaction and the first branch above adopts
/// the winner's committed binding through the ordinary path.
pub(super) async fn claim_or_create_binding_download_id_tx(
    tx: &mut SqlTx<'_>,
    locator: &ClientJobLocator,
    requested_download_id: Option<DownloadId>,
) -> AppResult<DownloadId> {
    if let Some(download_id) = active_binding_download_id(SqlExec::Tx(tx), locator).await? {
        // Re-claiming the same identity is not a stale adopt: there is no other
        // download's history to inherit, and the binding row is that identity's
        // own (`download_id` is the bindings primary key).
        let reclaims_bound_identity = requested_download_id == Some(download_id);
        let stale = !reclaims_bound_identity
            && bound_download_is_terminal_tx(tx, &download_id).await?
            // On the stub path the guard is for foreign re-adds only. A
            // Scryer-owned binding ends through the queue-delete /
            // authoritative-absence lifecycle, and a duplicate terminal-state
            // write for a job that is still in the client must not detach it
            // from its submission and seed goals.
            && (requested_download_id.is_some()
                || !bound_download_is_scryer_submission_tx(tx, &download_id).await?);
        if !stale {
            if requested_download_id.is_some() {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE downloads
                     SET origin = 'scryer_submission'
                     WHERE id = {}",
                    &[SqlArg::Text(download_id.to_string())],
                )
                .await?;
            }
            return Ok(download_id);
        }
        end_binding_tx(tx, &download_id).await?;
    }

    let now = Utc::now();
    let download_id = requested_download_id.unwrap_or_else(DownloadId::new);
    if requested_download_id.is_some() {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO downloads (id, origin, created_at)
             VALUES ({}, 'scryer_submission', {})
             ON CONFLICT(id) DO UPDATE SET origin = 'scryer_submission'",
            &[
                SqlArg::Text(download_id.to_string()),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    } else {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO downloads (id, origin, created_at, first_observed_at, last_observed_at)
             VALUES ({}, 'foreign_observation', {}, {}, {})",
            &[
                SqlArg::Text(download_id.to_string()),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    let client_name_snapshot = client_name_snapshot_tx(tx, locator).await?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO download_client_bindings (
            download_id, client_config_id, client_type_snapshot, client_name_snapshot,
            native_item_id, created_at, last_seen_at, ended_at
         ) VALUES ({}, {}, {}, {}, {}, {}, {}, NULL)",
        &[
            SqlArg::Text(download_id.to_string()),
            SqlArg::OptText(locator.client_id.clone()),
            SqlArg::Text(locator.client_type.clone()),
            SqlArg::OptText(client_name_snapshot),
            SqlArg::Text(locator.item_id.clone()),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await?;
    Ok(download_id)
}

/// Did this download already reach a terminal outcome?
///
/// Read from the same durable rows the tracking layer reconstructs state from
/// on first see (`TrackedDownloadService::reconstruct_state`): the canonical
/// identity state first, then the canonical submission's `tracked_state`.
/// Terminality itself is [`TrackedDownloadState::is_terminal`], so this cannot
/// drift into a parallel notion of "done".
async fn bound_download_is_terminal_tx(
    tx: &mut SqlTx<'_>,
    download_id: &DownloadId,
) -> AppResult<bool> {
    let identity_state = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT tracked_state
         FROM download_identity_states
         WHERE canonical_download_id = {}
         ORDER BY updated_at DESC, id DESC
         LIMIT 1",
        &[SqlArg::Text(download_id.to_string())],
    )
    .await?
    .map(|row| row.text("tracked_state"))
    .transpose()?;
    let tracked_state = match identity_state {
        Some(tracked_state) => Some(tracked_state),
        None => SqlRuntime::fetch_optional(
            SqlExec::Tx(tx),
            "SELECT tracked_state
             FROM download_submissions
             WHERE id = {}
             LIMIT 1",
            &[SqlArg::Text(download_id.to_string())],
        )
        .await?
        .map(|row| row.opt_text("tracked_state"))
        .transpose()?
        .flatten(),
    };
    Ok(tracked_state
        .as_deref()
        .and_then(TrackedDownloadState::from_str_opt)
        .is_some_and(TrackedDownloadState::is_terminal))
}

/// Is the bound parent a Scryer-owned download rather than a foreign
/// observation? Only the no-requested-id (tracked-state stub) path asks.
async fn bound_download_is_scryer_submission_tx(
    tx: &mut SqlTx<'_>,
    download_id: &DownloadId,
) -> AppResult<bool> {
    Ok(SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT origin
         FROM downloads
         WHERE id = {}
         LIMIT 1",
        &[SqlArg::Text(download_id.to_string())],
    )
    .await?
    .map(|row| row.text("origin"))
    .transpose()?
    .is_some_and(|origin| origin == "scryer_submission"))
}

/// End a binding inside the caller's transaction, with the same `ended_at`
/// mechanics as the registry store's `end_binding`.
async fn end_binding_tx(tx: &mut SqlTx<'_>, download_id: &DownloadId) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "UPDATE download_client_bindings
         SET ended_at = {}
         WHERE download_id = {}
           AND ended_at IS NULL",
        &[
            SqlArg::Timestamp(Utc::now()),
            SqlArg::Text(download_id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

async fn client_name_snapshot_tx(
    tx: &mut SqlTx<'_>,
    locator: &ClientJobLocator,
) -> AppResult<Option<String>> {
    let configured_name = match locator.client_id.as_deref() {
        Some(client_id) if !client_id.trim().is_empty() => SqlRuntime::fetch_optional(
            SqlExec::Tx(tx),
            "SELECT name FROM download_clients WHERE id = {} LIMIT 1",
            &[SqlArg::Text(client_id.to_string())],
        )
        .await?
        .map(|row| row.text("name"))
        .transpose()?,
        _ => None,
    };
    Ok(configured_name
        .or_else(|| (!locator.client_type.trim().is_empty()).then(|| locator.client_type.clone())))
}

fn canonical_tracked_state_key(canonical_download_id: &DownloadId) -> String {
    format!("download:{canonical_download_id}")
}

/// The durable tracked states that end a grab, spelled the way
/// [`TrackedDownloadState::as_str`] spells them.
///
/// [`TERMINAL_DOWNLOAD_HISTORY_SQL`] filters on exactly these, inline, so its
/// `LIMIT` cuts an already-filtered set;
/// `terminal_history_states_match_the_domain_definition` pins the list to
/// [`TrackedDownloadState::is_terminal`].
const TERMINAL_HISTORY_TRACKED_STATES: [&str; 3] = ["imported", "failed", "ignored"];

/// Durable download-history rows.
///
/// The terminal state is read the way `bound_download_is_terminal_tx` reads it:
/// the canonical identity state first, then the submission's own
/// `tracked_state`. Client identity prefers the canonical binding's snapshot and
/// falls back to the submission's legacy locator columns, so a row still names
/// its client after the binding was ended.
static TERMINAL_DOWNLOAD_HISTORY_SQL: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| {
        let states = TERMINAL_HISTORY_TRACKED_STATES
            .iter()
            .map(|state| format!("'{state}'"))
            .collect::<Vec<_>>()
            .join(", ");
        TERMINAL_DOWNLOAD_HISTORY_SQL_TEMPLATE.replace("{terminal_states}", &states)
    });

const TERMINAL_DOWNLOAD_HISTORY_SQL_TEMPLATE: &str = "SELECT * FROM (
                SELECT
                    d.id AS download_id,
                    d.origin AS origin,
                    COALESCE(
                        (SELECT st.tracked_state
                           FROM download_identity_states st
                          WHERE st.canonical_download_id = d.id
                          ORDER BY st.updated_at DESC, st.id DESC
                          LIMIT 1),
                        s.tracked_state
                    ) AS tracked_state,
                    (SELECT st.reason
                       FROM download_identity_states st
                      WHERE st.canonical_download_id = d.id
                      ORDER BY st.updated_at DESC, st.id DESC
                      LIMIT 1) AS tracked_reason,
                    (SELECT st.detail
                       FROM download_identity_states st
                      WHERE st.canonical_download_id = d.id
                      ORDER BY st.updated_at DESC, st.id DESC
                      LIMIT 1) AS tracked_detail,
                    s.title_id AS title_id,
                    s.episode_id AS episode_id,
                    s.facet AS facet,
                    s.source_title AS source_title,
                    s.source_provider_name AS source_provider_name,
                    s.release_size_bytes AS size_bytes,
                    s.submitted_at AS submitted_at,
                    COALESCE(b.client_config_id, s.download_client_id) AS client_id,
                    COALESCE(b.client_type_snapshot, s.download_client_type) AS client_type,
                    COALESCE(b.client_name_snapshot, cl.name) AS client_name,
                    COALESCE(b.native_item_id, s.download_client_item_id)
                        AS download_client_item_id,
                    COALESCE(
                        d.terminal_at,
                        s.tracked_state_at,
                        b.last_seen_at,
                        d.last_observed_at,
                        d.created_at
                    ) AS last_state_at
                FROM downloads d
                LEFT JOIN download_submissions s ON s.id = d.id
                LEFT JOIN download_client_bindings b ON b.download_id = d.id
                LEFT JOIN download_clients cl
                       ON cl.id = COALESCE(b.client_config_id, s.download_client_id)
            ) terminal_downloads
            WHERE terminal_downloads.tracked_state IN ({terminal_states})
            ORDER BY terminal_downloads.last_state_at DESC, terminal_downloads.download_id
            LIMIT {}";

fn terminal_download_history_row_from_row(row: &SqlRow) -> AppResult<TerminalDownloadHistoryRow> {
    let raw_id = row.text("download_id")?;
    let download_id = DownloadId::parse(&raw_id).ok_or_else(|| {
        AppError::Repository(format!(
            "invalid canonical download id {raw_id:?} in download history projection"
        ))
    })?;
    let origin = match row.text("origin")?.as_str() {
        "foreign_observation" => DownloadOrigin::ForeignObservation,
        // `downloads.origin` is CHECK-constrained to the two known values, so a
        // history row never needs to fail the whole page over an unknown one.
        _ => DownloadOrigin::ScryerSubmission,
    };
    Ok(TerminalDownloadHistoryRow {
        download_id,
        origin,
        tracked_state: row.text("tracked_state")?,
        tracked_reason: blank_to_none(row.opt_text("tracked_reason")?),
        tracked_detail: blank_to_none(row.opt_text("tracked_detail")?),
        title_id: blank_to_none(row.opt_text("title_id")?),
        episode_id: blank_to_none(row.opt_text("episode_id")?),
        facet: blank_to_none(row.opt_text("facet")?),
        source_title: blank_to_none(row.opt_text("source_title")?),
        client_id: blank_to_none(row.opt_text("client_id")?),
        client_type: blank_to_none(row.opt_text("client_type")?),
        client_name: blank_to_none(row.opt_text("client_name")?),
        download_client_item_id: blank_to_none(row.opt_text("download_client_item_id")?),
        source_provider_name: blank_to_none(row.opt_text("source_provider_name")?),
        size_bytes: row.opt_i64("size_bytes")?,
        submitted_at: lenient_timestamp(row, "submitted_at")?,
        last_state_at: lenient_timestamp(row, "last_state_at")?,
    })
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Timestamps here are sort values, never identity. A legacy row stored in a
/// form RFC3339 cannot read must not fail the whole history page, so an
/// unparseable value simply sorts as absent.
fn lenient_timestamp(
    row: &SqlRow,
    column: &str,
) -> AppResult<Option<chrono::DateTime<chrono::Utc>>> {
    Ok(opt_timestamp_string(row, column)?.and_then(|value| {
        chrono::DateTime::parse_from_rfc3339(value.trim())
            .ok()
            .map(|value| value.with_timezone(&Utc))
    }))
}

#[async_trait]
impl DownloadSubmissionRepository for DownloadSubmissionStore {
    async fn find_info_hash_for_title_release(
        &self,
        title_id: &str,
        normalized_release_name: &str,
    ) -> AppResult<Option<String>> {
        // The name comparison happens here rather than in SQL: SQLite's LOWER
        // is ASCII-only while Postgres' lower() is locale-aware, and the
        // blocklist key this feeds is normalized in Rust. Any matching
        // submission serves — one release name is one release is one hash.
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT source_title, info_hash
               FROM download_submissions
              WHERE title_id = {} AND info_hash IS NOT NULL AND source_title IS NOT NULL
              LIMIT 200",
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?;
        for row in rows {
            let source_title = row.opt_text("source_title")?;
            if scryer_application::normalize_release_name(source_title.as_deref())
                .is_some_and(|name| name == normalized_release_name)
            {
                return row.opt_text("info_hash");
            }
        }
        Ok(None)
    }

    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        run_in_transaction_retrying_unique_violation(
            &self.datastore,
            "record_download_submission",
            move |tx| {
                let submission = submission.clone();
                Box::pin(async move {
                    record_download_submission_tx(tx, &submission).await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn record_ambiguous_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_ambiguous_download_submission",
            move |tx| {
                let submission = submission.clone();
                Box::pin(
                    async move { record_ambiguous_download_submission_tx(tx, &submission).await },
                )
            },
        )
        .await
    }

    async fn record_submission_with_identity(
        &self,
        submission: DownloadSubmission,
        submission_identity: DownloadSubmissionIdentity,
        seed_goals: Option<PersistedSeedGoals>,
    ) -> AppResult<CanonicalDownloadIdentityDisposition> {
        let requested_download_id = submission.download_id;
        let locator = ClientJobLocator::from_submission(&submission);
        run_in_transaction_retrying_unique_violation(
            &self.datastore,
            "record_download_submission_with_identity_disposition",
            move |tx| {
                let submission = submission.clone();
                let submission_identity = submission_identity.clone();
                let seed_goals = seed_goals.clone();
                let locator = locator.clone();
                Box::pin(async move {
                    if let Some(download_id) =
                        active_binding_download_id(SqlExec::Tx(tx), &locator).await?
                        && download_id != requested_download_id
                        && !bound_download_is_terminal_tx(tx, &download_id).await?
                    {
                        return Ok(CanonicalDownloadIdentityDisposition::AdoptedExisting {
                            download_id,
                        });
                    }
                    let effective_download_id = record_download_submission_with_identity_tx(
                        tx,
                        &submission,
                        &submission_identity,
                    )
                    .await?
                    .ok_or_else(|| {
                        AppError::Repository(format!(
                            "canonical submission {requested_download_id} was not persisted"
                        ))
                    })?;
                    if effective_download_id == requested_download_id
                        && let Some(goals) = seed_goals.as_ref()
                    {
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "UPDATE download_submissions
                             SET seeding_profile_id = {}, seed_goal_ratio = {},
                                 seed_goal_seconds = {}, seed_never_remove = {},
                                 seed_goal_met_action = {}, seed_post_import_tracking = {},
                                 seed_goal_source = {}, seed_info_hash = {}
                             WHERE id = {}",
                            &[
                                SqlArg::OptText(goals.seeding_profile_id.clone()),
                                SqlArg::OptF64(goals.seed_goal_ratio),
                                SqlArg::OptI64(goals.seed_goal_seconds),
                                SqlArg::OptBool(Some(goals.never_remove)),
                                SqlArg::OptText(
                                    goals
                                        .goal_met_action
                                        .map(|action| action.as_str().to_string()),
                                ),
                                SqlArg::OptText(Some(
                                    goals.post_import_tracking.as_str().to_string(),
                                )),
                                SqlArg::OptText(Some(goals.resolution_source.as_str().to_string())),
                                SqlArg::OptText(normalized_info_hash(goals.info_hash.as_deref())),
                                SqlArg::Text(effective_download_id.to_string()),
                            ],
                        )
                        .await?;
                    }
                    Ok(if effective_download_id == requested_download_id {
                        CanonicalDownloadIdentityDisposition::Requested
                    } else {
                        CanonicalDownloadIdentityDisposition::AdoptedExisting {
                            download_id: effective_download_id,
                        }
                    })
                })
            },
        )
        .await
    }

    async fn record_submission_identity(
        &self,
        identity: &ClientJobLocator,
        submission_identity: &DownloadSubmissionIdentity,
    ) -> AppResult<()> {
        let identity = identity.clone();
        let submission_identity = submission_identity.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_download_submission_identity",
            move |tx| {
                let identity = identity.clone();
                let submission_identity = submission_identity.clone();
                Box::pin(async move {
                    let Some(canonical_download_id) =
                        active_binding_download_id(SqlExec::Tx(tx), &identity).await?
                    else {
                        return Ok(());
                    };
                    record_download_submission_identity_tx(
                        tx,
                        &canonical_download_id,
                        &submission_identity,
                    )
                    .await
                })
            },
        )
        .await
    }

    async fn record_submission_actor_snapshot(
        &self,
        identity: &ClientJobLocator,
        actor: DownloadSubmissionActorSnapshot,
    ) -> AppResult<()> {
        let identity = identity.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_download_submission_actor_snapshot",
            move |tx| {
                let identity = identity.clone();
                let actor = actor.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE download_submissions
                         SET actor_kind = {},
                             actor_user_id = {},
                             actor_display_name = {}
                         WHERE download_client_type = {}
                           AND download_client_item_id = {}
                           AND download_client_id = {}",
                        &[
                            SqlArg::Text(actor.kind.as_str().to_string()),
                            SqlArg::OptText(actor.user_id),
                            SqlArg::Text(actor.display_name),
                            SqlArg::Text(identity.client_type),
                            SqlArg::Text(identity.item_id),
                            SqlArg::Text(normalize_download_client_id(
                                identity.client_id.as_deref(),
                            )),
                        ],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn get_submission_actor_snapshot(
        &self,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<DownloadSubmissionActorSnapshot>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT actor_kind, actor_user_id, actor_display_name
             FROM download_submissions
             WHERE download_client_type = {}
               AND download_client_item_id = {}
               AND download_client_id = {}
             LIMIT 1",
            &[
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            ],
        )
        .await?;
        row.map(|row| download_submission_actor_snapshot_from_row(&row))
            .transpose()
            .map(Option::flatten)
    }

    async fn find_by_client_item_id(
        &self,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<DownloadSubmission>> {
        let sql = download_submission_select_sql(
            &self.datastore,
            "WHERE download_client_type = {} AND download_client_item_id = {} AND download_client_id = {} LIMIT 1",
        );
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            ],
        )
        .await?;
        row.map(|row| download_submission_from_row(&row))
            .transpose()
    }

    async fn find_by_client_item_id_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<DownloadSubmission>> {
        let canonical_download_id = match canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match self.active_binding_download_id(identity).await? {
                Some(canonical_download_id) => canonical_download_id,
                None => return Ok(None),
            },
        };
        self.find_by_canonical_download_id(&canonical_download_id)
            .await
    }

    async fn find_by_canonical_download_id(
        &self,
        download_id: &DownloadId,
    ) -> AppResult<Option<DownloadSubmission>> {
        DownloadSubmissionStore::find_by_canonical_download_id(self, download_id).await
    }

    async fn list_by_download_id(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_id: &str,
    ) -> AppResult<Vec<DownloadSubmission>> {
        let sql = download_submission_select_sql(
            &self.datastore,
            "WHERE download_client_type = {} AND download_client_id = {} AND download_id = {} AND download_client_item_id IS NOT NULL ORDER BY submitted_at DESC, id DESC",
        );
        fetch_download_submissions(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(client_type.trim().to_ascii_lowercase()),
                SqlArg::Text(normalize_download_client_id(client_id)),
                SqlArg::Text(download_id.trim().to_string()),
            ],
        )
        .await
    }

    async fn list_by_download_id_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        _client_id: Option<&str>,
        _client_type: &str,
        _download_id: &str,
    ) -> AppResult<Vec<DownloadSubmission>> {
        let Some(canonical_download_id) = canonical_download_id else {
            return Ok(Vec::new());
        };
        Ok(self
            .find_by_canonical_download_id(canonical_download_id)
            .await?
            .into_iter()
            .collect())
    }

    async fn get_submission_identity(
        &self,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<DownloadSubmissionIdentity>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT download_id
             FROM download_submissions
             WHERE download_client_type = {}
               AND download_client_item_id = {}
               AND download_client_id = {}
             LIMIT 1",
            &[
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            ],
        )
        .await?;
        row.map(|row| {
            Ok(DownloadSubmissionIdentity {
                download_id: row.opt_text("download_id")?,
            })
        })
        .transpose()
    }

    async fn record_identity_tracked_state(
        &self,
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
        _tracked_state: &str,
        _reason: Option<&str>,
        _detail: Option<&str>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn record_identity_tracked_state_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
        tracked_state: &str,
        reason: Option<&str>,
        detail: Option<&str>,
    ) -> AppResult<()> {
        let canonical_download_id = match canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match source_identity {
                Some(source_identity) => {
                    let Some(canonical_download_id) =
                        self.active_binding_download_id(source_identity).await?
                    else {
                        return Ok(());
                    };
                    canonical_download_id
                }
                None => return Ok(()),
            },
        };
        let identity_key = canonical_tracked_state_key(&canonical_download_id);
        let canonical_download_id = canonical_download_id.to_string();
        let identity = identity.clone();
        let source_identity = source_identity.cloned();
        let tracked_state = tracked_state.to_string();
        let reason = reason.map(str::to_string);
        let detail = detail.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_download_identity_tracked_state",
            move |tx| {
                let identity_key = identity_key.clone();
                let canonical_download_id = canonical_download_id.clone();
                let identity = identity.clone();
                let source_identity = source_identity.clone();
                let tracked_state = tracked_state.clone();
                let reason = reason.clone();
                let detail = detail.clone();
                Box::pin(async move {
                    let now = Utc::now();
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO download_identity_states
                         (id, identity_key, canonical_download_id, download_id,
                          client_id, client_type, download_client_item_id,
                          tracked_state, reason, detail, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                         ON CONFLICT(identity_key) DO UPDATE
                         SET canonical_download_id = excluded.canonical_download_id,
                             download_id = excluded.download_id,
                             client_id = excluded.client_id,
                             client_type = excluded.client_type,
                             download_client_item_id = excluded.download_client_item_id,
                             tracked_state = excluded.tracked_state,
                             reason = excluded.reason,
                             detail = excluded.detail,
                             updated_at = excluded.updated_at",
                        &[
                            SqlArg::Text(Id::new().0),
                            SqlArg::Text(identity_key),
                            SqlArg::OptText(Some(canonical_download_id)),
                            SqlArg::OptText(identity.download_id),
                            SqlArg::OptText(source_identity.as_ref().map(|source| {
                                normalize_download_client_id(source.client_id.as_deref())
                            })),
                            SqlArg::OptText(
                                source_identity
                                    .as_ref()
                                    .map(|source| source.client_type.clone()),
                            ),
                            SqlArg::OptText(
                                source_identity
                                    .as_ref()
                                    .map(|source| source.item_id.clone()),
                            ),
                            SqlArg::Text(tracked_state),
                            SqlArg::OptText(reason),
                            SqlArg::OptText(detail),
                            SqlArg::Timestamp(now),
                            SqlArg::Timestamp(now),
                        ],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn get_identity_tracked_state(
        &self,
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn get_identity_tracked_state_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        _identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        let canonical_download_id = match canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match source_identity {
                Some(source_identity) => {
                    let Some(canonical_download_id) =
                        self.active_binding_download_id(source_identity).await?
                    else {
                        return Ok(None);
                    };
                    canonical_download_id
                }
                None => return Ok(None),
            },
        };
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT tracked_state
             FROM download_identity_states
             WHERE canonical_download_id = {}
             ORDER BY updated_at DESC, id DESC
             LIMIT 1",
            &[SqlArg::Text(canonical_download_id.to_string())],
        )
        .await?;
        row.map(|row| row.text("tracked_state")).transpose()
    }

    async fn get_identity_tracked_state_reason(
        &self,
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn get_identity_tracked_state_reason_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        _identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        let canonical_download_id = match canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match source_identity {
                Some(source_identity) => {
                    let Some(canonical_download_id) =
                        self.active_binding_download_id(source_identity).await?
                    else {
                        return Ok(None);
                    };
                    canonical_download_id
                }
                None => return Ok(None),
            },
        };
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT reason
             FROM download_identity_states
             WHERE canonical_download_id = {}
             ORDER BY updated_at DESC, id DESC
             LIMIT 1",
            &[SqlArg::Text(canonical_download_id.to_string())],
        )
        .await?;
        row.map(|row| row.opt_text("reason"))
            .transpose()
            .map(Option::flatten)
    }

    async fn get_identity_tracked_state_detail(
        &self,
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn get_identity_tracked_state_detail_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        _identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        let canonical_download_id = match canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match source_identity {
                Some(source_identity) => {
                    let Some(canonical_download_id) =
                        self.active_binding_download_id(source_identity).await?
                    else {
                        return Ok(None);
                    };
                    canonical_download_id
                }
                None => return Ok(None),
            },
        };
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT detail
             FROM download_identity_states
             WHERE canonical_download_id = {}
             ORDER BY updated_at DESC, id DESC
             LIMIT 1",
            &[SqlArg::Text(canonical_download_id.to_string())],
        )
        .await?;
        row.map(|row| row.opt_text("detail"))
            .transpose()
            .map(Option::flatten)
    }

    async fn upsert_identity_tracked_state_returning_previous(
        &self,
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
        _tracked_state: &str,
        _preserve_previous: &[&str],
        _reason: Option<&str>,
        _detail: Option<&str>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn upsert_identity_tracked_state_for_download_returning_previous(
        &self,
        target: IdentityTrackedStateTarget<'_>,
        tracked_state: &str,
        preserve_previous: &[&str],
        reason: Option<&str>,
        detail: Option<&str>,
    ) -> AppResult<Option<String>> {
        let canonical_download_id = match target.canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match target.source_identity {
                Some(source_identity) => {
                    let Some(canonical_download_id) =
                        self.active_binding_download_id(source_identity).await?
                    else {
                        return Ok(None);
                    };
                    canonical_download_id
                }
                None => return Ok(None),
            },
        };
        let identity_key = canonical_tracked_state_key(&canonical_download_id);
        let canonical_download_id = canonical_download_id.to_string();
        let identity = target.identity.clone();
        let source_identity = target.source_identity.cloned();
        let tracked_state = tracked_state.to_string();
        let preserve_previous = preserve_previous
            .iter()
            .map(|state| state.to_string())
            .collect::<Vec<_>>();
        let reason = reason.map(str::to_string);
        let detail = detail.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "upsert_download_identity_tracked_state",
            move |tx| {
                let identity_key = identity_key.clone();
                let canonical_download_id = canonical_download_id.clone();
                let identity = identity.clone();
                let source_identity = source_identity.clone();
                let tracked_state = tracked_state.clone();
                let preserve_previous = preserve_previous.clone();
                let reason = reason.clone();
                let detail = detail.clone();
                Box::pin(async move {
                    let previous = SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "SELECT tracked_state
                         FROM download_identity_states
                         WHERE canonical_download_id = {}
                         ORDER BY updated_at DESC, id DESC
                         LIMIT 1",
                        &[SqlArg::Text(canonical_download_id.clone())],
                    )
                    .await?
                    .map(|row| row.text("tracked_state"))
                    .transpose()?;
                    if let Some(previous) = previous.as_deref().filter(|previous| {
                        preserve_previous
                            .iter()
                            .any(|preserved| preserved == previous)
                    }) {
                        // The read and this early return share the transaction,
                        // so a terminal outcome can never be flipped by a
                        // concurrent ignore.
                        return Ok(Some(previous.to_string()));
                    }
                    let now = Utc::now();
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO download_identity_states
                         (id, identity_key, canonical_download_id, download_id,
                          client_id, client_type, download_client_item_id,
                          tracked_state, reason, detail, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                         ON CONFLICT(identity_key) DO UPDATE
                         SET canonical_download_id = excluded.canonical_download_id,
                             download_id = excluded.download_id,
                             client_id = excluded.client_id,
                             client_type = excluded.client_type,
                             download_client_item_id = excluded.download_client_item_id,
                             tracked_state = excluded.tracked_state,
                             reason = excluded.reason,
                             detail = excluded.detail,
                             updated_at = excluded.updated_at",
                        &[
                            SqlArg::Text(Id::new().0),
                            SqlArg::Text(identity_key),
                            SqlArg::OptText(Some(canonical_download_id)),
                            SqlArg::OptText(identity.download_id),
                            SqlArg::OptText(source_identity.as_ref().map(|source| {
                                normalize_download_client_id(source.client_id.as_deref())
                            })),
                            SqlArg::OptText(
                                source_identity
                                    .as_ref()
                                    .map(|source| source.client_type.clone()),
                            ),
                            SqlArg::OptText(
                                source_identity
                                    .as_ref()
                                    .map(|source| source.item_id.clone()),
                            ),
                            SqlArg::Text(tracked_state),
                            SqlArg::OptText(reason),
                            SqlArg::OptText(detail),
                            SqlArg::Timestamp(now),
                            SqlArg::Timestamp(now),
                        ],
                    )
                    .await?;
                    Ok(previous)
                })
            },
        )
        .await
    }

    async fn list_identity_tracked_states_for_client_items(
        &self,
        client_items: &[ClientJobLocator],
    ) -> AppResult<Vec<(ClientJobLocator, String)>> {
        let chunks = chunk_download_submission_client_items(client_items);
        if chunks.is_empty() {
            return Ok(Vec::new());
        }

        let mut states = Vec::new();
        for chunk in chunks {
            let mut args = Vec::with_capacity(chunk.len() * 3);
            let clauses = chunk
                .iter()
                .map(|identity| {
                    args.push(SqlArg::Text(identity.client_type.clone()));
                    args.push(SqlArg::Text(identity.item_id.clone()));
                    args.push(SqlArg::Text(normalize_download_client_id(
                        identity.client_id.as_deref(),
                    )));
                    "(states.client_type = {} AND states.download_client_item_id = {} AND COALESCE(states.client_id, '') = {})"
                })
                .collect::<Vec<_>>()
                .join(" OR ");
            let rows = SqlRuntime::fetch_all(
                self.datastore.read_exec(),
                &format!(
                    "SELECT client_id, client_type, download_client_item_id, tracked_state
                     FROM (
                         SELECT states.client_id, states.client_type,
                                states.download_client_item_id, states.tracked_state,
                                states.updated_at, states.id AS state_id,
                                ROW_NUMBER() OVER (
                                    PARTITION BY COALESCE(states.client_id, ''),
                                                 states.client_type,
                                                 states.download_client_item_id
                                    ORDER BY submissions.submitted_at DESC, submissions.id DESC
                                ) AS row_number
                         FROM download_identity_states states
                         JOIN download_submissions submissions
                           ON submissions.id = states.canonical_download_id
                         WHERE {clauses}
                     ) ranked
                     WHERE row_number = 1
                     ORDER BY updated_at ASC, state_id ASC"
                ),
                &args,
            )
            .await?;
            for row in rows {
                states.push((
                    ClientJobLocator::new(
                        row.opt_text("client_id")?.as_deref(),
                        row.text("client_type")?,
                        row.text("download_client_item_id")?,
                    ),
                    row.text("tracked_state")?,
                ));
            }
        }
        Ok(states)
    }

    async fn list_for_client_items(
        &self,
        client_items: &[ClientJobLocator],
    ) -> AppResult<Vec<DownloadSubmission>> {
        let chunks = chunk_download_submission_client_items(client_items);
        if chunks.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for chunk in chunks {
            let mut args = Vec::with_capacity(chunk.len() * 3);
            let clauses = chunk
                .iter()
                .map(|identity| {
                    args.push(SqlArg::Text(identity.client_type.clone()));
                    args.push(SqlArg::Text(identity.item_id.clone()));
                    args.push(SqlArg::Text(normalize_download_client_id(
                        identity.client_id.as_deref(),
                    )));
                    "(download_client_type = {} AND download_client_item_id = {} AND download_client_id = {})"
                })
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = download_submission_select_sql(
                &self.datastore,
                &format!(
                    "JOIN (
                         SELECT id AS submission_id,
                                ROW_NUMBER() OVER (
                                    PARTITION BY download_client_id,
                                                 download_client_type,
                                                 download_client_item_id
                                    ORDER BY submitted_at DESC, id DESC
                                ) AS row_number
                         FROM download_submissions
                         WHERE {clauses}
                     ) ranked ON ranked.submission_id = download_submissions.id
                     WHERE ranked.row_number = 1"
                ),
            );
            out.extend(fetch_download_submissions(self.datastore.read_exec(), &sql, &args).await?);
        }
        Ok(out)
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
        let sql = download_submission_select_sql(
            &self.datastore,
            "WHERE title_id = {} AND download_client_item_id IS NOT NULL",
        );
        fetch_download_submissions(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
        )
        .await
    }

    async fn list_active_unbound_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<DownloadSubmission>> {
        let sql = download_submission_select_sql(
            &self.datastore,
            "WHERE title_id = {}
               AND download_client_item_id IS NULL
               AND EXISTS (
                   SELECT 1
                     FROM download_client_bindings binding
                    WHERE binding.download_id = download_submissions.id
                      AND binding.native_item_id IS NULL
                      AND binding.ended_at IS NULL
               )
             ORDER BY submitted_at, id",
        );
        fetch_download_submissions(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
        )
        .await
    }

    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
        purpose: scryer_application::DownloadSubmissionPurpose,
        scope: &scryer_application::SubmissionScope,
    ) -> AppResult<Option<DownloadSubmission>> {
        let recent_cutoff = Utc::now() - chrono::Duration::seconds(30);
        let sql = download_submission_select_sql(
            &self.datastore,
            "WHERE title_id = {} AND request_signature = {} AND purpose = {} AND download_client_item_id IS NOT NULL AND COALESCE(tracked_state, '') = '' AND submitted_at >= {} ORDER BY submitted_at DESC, id DESC",
        );
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(request_signature.to_string()),
                SqlArg::Text(purpose.as_str().to_string()),
                SqlArg::Timestamp(recent_cutoff),
            ],
        )
        .await?;
        for row in rows {
            let submission = download_submission_from_row(&row)?;
            if &submission.scope == scope {
                return Ok(Some(submission));
            }
        }
        Ok(None)
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_download_submissions_for_title",
            move |tx| {
                let title_id = title_id.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_submission_episode_links
                         WHERE download_id IN (
                             SELECT id
                               FROM download_submissions
                              WHERE title_id = {}
                         )",
                        &[SqlArg::Text(title_id.clone())],
                    )
                    .await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_submissions WHERE title_id = {}",
                        &[SqlArg::Text(title_id)],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn delete_by_client_item_id(&self, identity: &ClientJobLocator) -> AppResult<()> {
        let identity = identity.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_download_submission_by_client_item_id",
            move |tx| {
                let identity = identity.clone();
                Box::pin(async move {
                    if let Some(download_id) =
                        active_binding_download_id(SqlExec::Tx(tx), &identity).await?
                    {
                        end_binding_tx(tx, &download_id).await?;
                    }
                    let normalized_client_id =
                        normalize_download_client_id(identity.client_id.as_deref());
                    let args = [
                        SqlArg::Text(normalized_client_id.clone()),
                        SqlArg::Text(identity.client_type.clone()),
                        SqlArg::Text(identity.item_id.clone()),
                    ];
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_submission_episode_links
                         WHERE download_id IN (
                             SELECT id
                               FROM download_submissions
                              WHERE download_client_id = {}
                                AND download_client_type = {}
                                AND download_client_item_id = {}
                         )",
                        &args,
                    )
                    .await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_submissions
                         WHERE download_client_id = {}
                           AND download_client_type = {}
                           AND download_client_item_id = {}",
                        &args,
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn list_terminal_download_history_rows(
        &self,
        limit: usize,
    ) -> AppResult<Vec<TerminalDownloadHistoryRow>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            TERMINAL_DOWNLOAD_HISTORY_SQL.as_str(),
            &[SqlArg::I64(limit as i64)],
        )
        .await?
        .into_iter()
        .map(|row| terminal_download_history_row_from_row(&row))
        .collect()
    }

    async fn update_tracked_state(
        &self,
        identity: &ClientJobLocator,
        tracked_state: &str,
    ) -> AppResult<()> {
        let identity = identity.clone();
        let tracked_state = tracked_state.to_string();
        run_in_transaction_retrying_unique_violation(
            &self.datastore,
            "update_tracked_state",
            move |tx| {
                let identity = identity.clone();
                let tracked_state = tracked_state.clone();
                Box::pin(async move {
                    let canonical_download_id =
                        claim_or_create_binding_download_id_tx(tx, &identity, None).await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO download_submissions
                         (id, title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_kind, source_title, request_signature, purpose, episode_id, collection_id, tracked_state, tracked_state_at)
                         VALUES ({}, '', '', {}, {}, {}, NULL, NULL, NULL, NULL, 'standard', NULL, NULL, {}, {})
                         ON CONFLICT(id) DO UPDATE
                         SET tracked_state = excluded.tracked_state,
                             tracked_state_at = excluded.tracked_state_at",
                        &[
                            SqlArg::Text(canonical_download_id.to_string()),
                            SqlArg::Text(normalize_download_client_id(
                                identity.client_id.as_deref(),
                            )),
                            SqlArg::Text(identity.client_type.clone()),
                            SqlArg::Text(identity.item_id.clone()),
                            SqlArg::Text(tracked_state),
                            SqlArg::Timestamp(Utc::now()),
                        ],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn get_tracked_state(&self, identity: &ClientJobLocator) -> AppResult<Option<String>> {
        let Some(canonical_download_id) = self.active_binding_download_id(identity).await? else {
            return Ok(None);
        };
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT tracked_state FROM download_submissions
             WHERE id = {}
             LIMIT 1",
            &[SqlArg::Text(canonical_download_id.to_string())],
        )
        .await?;
        row.map(|row| row.opt_text("tracked_state"))
            .transpose()
            .map(Option::flatten)
    }

    async fn get_seed_goals(
        &self,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SEED_GOAL_COLUMNS}
                 FROM download_submissions
                 WHERE download_client_type = {{}}
                   AND download_client_item_id = {{}}
                   AND download_client_id = {{}}
                 LIMIT 1"
            ),
            &[
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            ],
        )
        .await?;
        row.map(|row| seed_goals_from_row(&row))
            .transpose()
            .map(Option::flatten)
    }

    async fn get_seed_goals_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        let canonical_download_id = match canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match self.active_binding_download_id(identity).await? {
                Some(canonical_download_id) => canonical_download_id,
                None => return Ok(None),
            },
        };
        self.get_seed_goals_by_canonical_download_id(&canonical_download_id)
            .await
    }

    async fn list_seed_goals_for_client_items(
        &self,
        client_items: &[ClientJobLocator],
    ) -> AppResult<Vec<(ClientJobLocator, PersistedSeedGoals)>> {
        let chunks = chunk_download_submission_client_items(client_items);
        if chunks.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for chunk in chunks {
            let mut args = Vec::with_capacity(chunk.len() * 3);
            let clauses = chunk
                .iter()
                .map(|identity| {
                    args.push(SqlArg::Text(identity.client_type.clone()));
                    args.push(SqlArg::Text(identity.item_id.clone()));
                    args.push(SqlArg::Text(normalize_download_client_id(
                        identity.client_id.as_deref(),
                    )));
                    "(download_client_type = {} AND download_client_item_id = {} AND download_client_id = {})"
                })
                .collect::<Vec<_>>()
                .join(" OR ");
            let rows = SqlRuntime::fetch_all(
                self.datastore.read_exec(),
                &format!(
                    "WITH ranked AS (
                         SELECT download_client_id, download_client_type,
                                download_client_item_id, {SEED_GOAL_COLUMNS},
                                ROW_NUMBER() OVER (
                                    PARTITION BY download_client_id,
                                                 download_client_type,
                                                 download_client_item_id
                                    ORDER BY submitted_at DESC, id DESC
                                ) AS row_number
                         FROM download_submissions
                         WHERE {clauses}
                     )
                     SELECT download_client_id, download_client_type, download_client_item_id,
                            {SEED_GOAL_COLUMNS}
                     FROM ranked
                     WHERE row_number = 1"
                ),
                &args,
            )
            .await?;
            for row in rows {
                let Some(goals) = seed_goals_from_row(&row)? else {
                    continue;
                };
                let client_id = row.opt_text("download_client_id")?;
                out.push((
                    ClientJobLocator::new(
                        client_id.as_deref(),
                        &row.text("download_client_type")?,
                        &row.text("download_client_item_id")?,
                    ),
                    goals,
                ));
            }
        }
        Ok(out)
    }

    async fn find_seed_goals_by_info_hash(
        &self,
        info_hash: &str,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        let Some(info_hash) = normalized_info_hash(Some(info_hash)) else {
            return Ok(None);
        };
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SEED_GOAL_COLUMNS}
                 FROM download_submissions
                 WHERE seed_info_hash = {{}}
                 ORDER BY submitted_at DESC
                 LIMIT 1"
            ),
            &[SqlArg::Text(info_hash)],
        )
        .await?;
        row.map(|row| seed_goals_from_row(&row))
            .transpose()
            .map(Option::flatten)
    }
}

const SEED_GOAL_COLUMNS: &str = "seeding_profile_id, seed_goal_ratio, seed_goal_seconds, \
     seed_never_remove, seed_goal_met_action, seed_post_import_tracking, seed_goal_source, \
     seed_info_hash";

/// Info hashes are compared case-insensitively across clients; store and look
/// them up in one canonical form.
fn normalized_info_hash(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

/// `None` when the row predates any seeding resolution (or the grab resolved to
/// no profile at all), so callers can tell "not evaluated" from "evaluated to
/// nothing".
fn seed_goals_from_row(row: &SqlRow) -> AppResult<Option<PersistedSeedGoals>> {
    let Some(source) = row
        .opt_text("seed_goal_source")?
        .as_deref()
        .and_then(SeedGoalResolutionSource::parse)
    else {
        return Ok(None);
    };
    if source == SeedGoalResolutionSource::None {
        return Ok(None);
    }
    Ok(Some(PersistedSeedGoals {
        seeding_profile_id: row.opt_text("seeding_profile_id")?,
        seed_goal_ratio: row.opt_f64("seed_goal_ratio")?,
        seed_goal_seconds: row.opt_i64("seed_goal_seconds")?,
        never_remove: row.opt_bool("seed_never_remove")?.unwrap_or(false),
        goal_met_action: row
            .opt_text("seed_goal_met_action")?
            .as_deref()
            .and_then(scryer_domain::SeedGoalMetAction::parse),
        // Absent (rows written before migration 0166) or unparseable reads as
        // `Park`: Scryer keeps managing the torrent, which is the direction
        // that cannot lose a seeding obligation.
        post_import_tracking: row
            .opt_text("seed_post_import_tracking")?
            .as_deref()
            .and_then(scryer_domain::PostImportTracking::parse)
            .unwrap_or_default(),
        resolution_source: source,
        info_hash: row.opt_text("seed_info_hash")?,
    }))
}

#[cfg(test)]
mod seed_goal_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use scryer_application::{DownloadSubmissionPurpose, SubmissionScope};
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn store() -> DownloadSubmissionStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        sqlx::query(
            "CREATE TABLE download_submissions (
                 id TEXT PRIMARY KEY,
                 title_id TEXT NOT NULL,
                 facet TEXT NOT NULL,
                 download_client_id TEXT NOT NULL DEFAULT '',
                 download_client_type TEXT NOT NULL,
                 download_client_item_id TEXT,
                 source_title TEXT,
                 info_hash TEXT,
                 release_size_bytes INTEGER,
                 submitted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 collection_id TEXT,
                 tracked_state TEXT,
                 tracked_state_at TEXT,
                 source_hint TEXT,
                 source_provider_id TEXT,
                 source_provider_name TEXT,
                 source_kind TEXT,
                 request_signature TEXT,
                 episode_id TEXT,
                 download_id TEXT,
                 purpose TEXT NOT NULL DEFAULT 'standard',
                 series_movie_link_id TEXT,
                 actor_kind TEXT,
                 actor_user_id TEXT,
                 actor_display_name TEXT
             )",
        )
        .execute(&pool)
        .await
        .expect("submission table should be created");
        sqlx::query(
            "CREATE TABLE download_submission_episode_links (
                 download_id TEXT NOT NULL,
                 episode_id TEXT NOT NULL,
                 PRIMARY KEY (download_id, episode_id)
             )",
        )
        .execute(&pool)
        .await
        .expect("episode link table should be created");
        sqlx::query(
            "CREATE TABLE download_clients (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL
             );
             CREATE TABLE downloads (
                 id TEXT PRIMARY KEY,
                 origin TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 first_observed_at TEXT,
                 last_observed_at TEXT,
                 terminal_at TEXT
             );
             CREATE TABLE download_client_bindings (
                 download_id TEXT PRIMARY KEY,
                 client_config_id TEXT,
                 client_type_snapshot TEXT,
                 client_name_snapshot TEXT,
                 native_item_id TEXT,
                 created_at TEXT NOT NULL,
                 last_seen_at TEXT,
                 ended_at TEXT
             );
             CREATE TABLE download_identity_states (
                 id TEXT PRIMARY KEY,
                 identity_key TEXT NOT NULL UNIQUE,
                 canonical_download_id TEXT,
                 download_id TEXT,
                 client_id TEXT,
                 client_type TEXT,
                 download_client_item_id TEXT,
                 tracked_state TEXT NOT NULL,
                 reason TEXT,
                 detail TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .expect("canonical download fixture tables should be created");
        for statement in include_str!(
            "../../../../scryer/src/db/migrations/0164_download_submission_seed_goals.sql"
        )
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("seed goal migration should apply");
        }
        // 0166 also touches `seeding_profiles`, which this fixture does not
        // create; apply only its download-submission statements.
        for statement in include_str!(
            "../../../../scryer/src/db/migrations/0166_seeding_profile_post_import_tracking.sql"
        )
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty() && statement.contains("download_submissions"))
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("post-import tracking migration should apply");
        }
        DownloadSubmissionStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    fn identity() -> ClientJobLocator {
        ClientJobLocator::new(Some("primary"), "qbittorrent", "job-1")
    }

    fn submission(download_id: DownloadId, item_id: &str, title_id: &str) -> DownloadSubmission {
        DownloadSubmission {
            download_id,
            title_id: title_id.to_string(),
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "qbittorrent".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: Some(format!("https://indexer.invalid/{item_id}.torrent")),
            source_provider_id: Some("indexer-1".to_string()),
            source_provider_name: Some("Indexer".to_string()),
            source_kind: Some(scryer_application::DownloadSourceKind::TorrentFile),
            source_title: Some(format!("Release {item_id}")),
            info_hash: None,
            release_size_bytes: Some(123),
            request_signature: Some(format!("signature-{item_id}")),
            scope: SubmissionScope::Title,
            purpose: DownloadSubmissionPurpose::Standard,
        }
    }

    fn submission_identity(download_id: DownloadId) -> DownloadSubmissionIdentity {
        DownloadSubmissionIdentity {
            download_id: Some(download_id.to_wire()),
        }
    }

    fn goals(ratio: f64, info_hash: Option<&str>) -> PersistedSeedGoals {
        PersistedSeedGoals {
            seeding_profile_id: Some("profile-1".to_string()),
            seed_goal_ratio: Some(ratio),
            seed_goal_seconds: Some(7200),
            never_remove: true,
            goal_met_action: Some(scryer_domain::SeedGoalMetAction::StopSeeding),
            post_import_tracking: scryer_domain::PostImportTracking::HandOff,
            resolution_source: SeedGoalResolutionSource::Indexer,
            info_hash: info_hash.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn seed_goals_load_in_one_batch_for_the_queue_projection() {
        let store = store().await;
        let first_id = DownloadId::new();
        store
            .record_submission_with_identity(
                submission(first_id, "job-1", "title-1"),
                submission_identity(first_id),
                Some(goals(2.5, Some("ABCDEF0123456789ABCDEF0123456789ABCDEF01"))),
            )
            .await
            .expect("submission and seed goals should persist");
        let second_id = DownloadId::new();
        store
            .record_submission_with_identity(
                submission(second_id, "job-2", "title-1"),
                submission_identity(second_id),
                Some(goals(1.25, None)),
            )
            .await
            .expect("submission and seed goals should persist");

        let loaded = store
            .list_seed_goals_for_client_items(&[
                identity(),
                ClientJobLocator::new(Some("primary"), "qbittorrent", "job-2"),
                // A row with no resolution at all must simply be absent, not a
                // default-valued entry that would read as "no goals resolved".
                ClientJobLocator::new(Some("primary"), "qbittorrent", "job-missing"),
            ])
            .await
            .expect("batch read should succeed");

        let mut by_item = loaded
            .into_iter()
            .map(|(identity, goals)| (identity.item_id, goals))
            .collect::<Vec<_>>();
        by_item.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(by_item.len(), 2);
        assert_eq!(by_item[0].0, "job-1");
        assert_eq!(by_item[0].1.seed_goal_ratio, Some(2.5));
        assert!(by_item[0].1.never_remove);
        assert_eq!(by_item[1].0, "job-2");
        assert_eq!(by_item[1].1.seed_goal_ratio, Some(1.25));
    }

    #[tokio::test]
    async fn active_locator_adopts_the_first_submission_without_replacing_frozen_seed_goals() {
        let store = store().await;
        let first_id = DownloadId::parse("00000000-0000-4000-8000-000000000001")
            .expect("first fixed download id should parse");
        let second_id = DownloadId::parse("00000000-0000-4000-8000-000000000002")
            .expect("second fixed download id should parse");
        let first_effective_download_id = store
            .record_submission_with_identity(
                submission(first_id, "job-1", "title-1"),
                submission_identity(first_id),
                Some(goals(1.0, None)),
            )
            .await
            .expect("first submission should persist");
        let second_effective_download_id = store
            .record_submission_with_identity(
                submission(second_id, "job-1", "title-2"),
                submission_identity(second_id),
                Some(goals(2.0, None)),
            )
            .await
            .expect("second submission should reuse the active binding");
        assert_eq!(
            first_effective_download_id,
            CanonicalDownloadIdentityDisposition::Requested
        );
        assert_eq!(
            second_effective_download_id,
            CanonicalDownloadIdentityDisposition::AdoptedExisting {
                download_id: first_id,
            }
        );

        store
            .record_identity_tracked_state_for_download(
                Some(&first_id),
                &DownloadSubmissionIdentity {
                    download_id: Some(format!("legacy-{first_id}")),
                },
                Some(&identity()),
                "new",
                None,
                None,
            )
            .await
            .expect("identity state should persist");

        let submissions = store
            .list_for_client_items(&[identity()])
            .await
            .expect("submission projection should load");
        assert_eq!(submissions.len(), 1);
        assert_eq!(submissions[0].download_id, first_id);

        let seed_goals = store
            .list_seed_goals_for_client_items(&[identity()])
            .await
            .expect("seed-goal projection should load");
        assert_eq!(seed_goals.len(), 1);
        assert_eq!(seed_goals[0].0, identity());
        assert_eq!(seed_goals[0].1.seed_goal_ratio, Some(1.0));

        let states = store
            .list_identity_tracked_states_for_client_items(&[identity()])
            .await
            .expect("tracked-state projection should load");
        assert_eq!(states, vec![(identity(), "new".to_string())]);
    }

    #[tokio::test]
    async fn deleting_a_submission_releases_its_locator_for_a_fresh_identity() {
        let store = store().await;
        let first_id = DownloadId::new();
        store
            .record_submission_with_identity(
                submission(first_id, "job-1", "title-1"),
                submission_identity(first_id),
                None,
            )
            .await
            .expect("first submission should persist");

        store
            .delete_by_client_item_id(&identity())
            .await
            .expect("authoritative absence should release the old locator");

        let second_id = DownloadId::new();
        let disposition = store
            .record_submission_with_identity(
                submission(second_id, "job-1", "title-1"),
                submission_identity(second_id),
                None,
            )
            .await
            .expect("fresh submission should claim the released locator");

        assert_eq!(disposition, CanonicalDownloadIdentityDisposition::Requested);
        assert_eq!(
            active_binding_download_id(store.datastore.read_exec(), &identity())
                .await
                .expect("active binding should load"),
            Some(second_id)
        );
        assert!(
            store
                .find_by_canonical_download_id(&first_id)
                .await
                .expect("old submission lookup should succeed")
                .is_none()
        );
    }

    #[tokio::test]
    async fn seed_goals_read_by_canonical_download_id() {
        let store = store().await;
        let canonical_download_id = DownloadId::new();
        let mut expected = goals(2.5, Some("ABCDEF0123456789ABCDEF0123456789ABCDEF01"));
        expected.info_hash = expected.info_hash.map(|hash| hash.to_ascii_lowercase());
        store
            .record_submission_with_identity(
                submission(canonical_download_id, "job-1", "title-1"),
                submission_identity(canonical_download_id),
                Some(expected.clone()),
            )
            .await
            .expect("submission and seed goals should persist");

        let loaded = store
            .get_seed_goals_for_download(Some(&canonical_download_id), &identity())
            .await
            .expect("canonical read should succeed");

        assert_eq!(loaded, Some(expected));
    }

    fn ambiguous_submission(
        download_id: scryer_domain::download_identity::DownloadId,
    ) -> DownloadSubmission {
        DownloadSubmission {
            download_id,
            scope: SubmissionScope::Title,
            title_id: "title-ambiguous".to_string(),
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "sabnzbd".to_string(),
            // `record_ambiguous_submission` deliberately persists NULL here.
            download_client_item_id: String::new(),
            source_hint: Some("https://indexer.invalid/release.nzb".to_string()),
            source_provider_id: Some("indexer-1".to_string()),
            source_provider_name: Some("Indexer".to_string()),
            source_kind: Some(scryer_application::DownloadSourceKind::NzbUrl),
            source_title: Some("Ambiguous Release".to_string()),
            info_hash: None,
            release_size_bytes: Some(123),
            request_signature: Some("ambiguous-signature".to_string()),
            purpose: DownloadSubmissionPurpose::Standard,
        }
    }

    #[tokio::test]
    async fn canonical_persistence_reports_adoption_without_overwriting_the_owner() {
        let store = store().await;
        let first_id = DownloadId::parse("00000000-0000-4000-8000-000000000001")
            .expect("first fixed download id should parse");
        let second_id = DownloadId::parse("00000000-0000-4000-8000-000000000002")
            .expect("second fixed download id should parse");
        let submission_identity = DownloadSubmissionIdentity {
            download_id: Some(first_id.to_wire()),
        };
        let mut first = ambiguous_submission(first_id);
        first.download_client_item_id = "existing-job".to_string();
        store
            .record_submission_with_identity(
                first.clone(),
                submission_identity.clone(),
                Some(goals(1.0, None)),
            )
            .await
            .expect("first submission should persist");
        let mut second = first;
        second.download_id = second_id;
        second.title_id = "different-title".to_string();

        let disposition = store
            .record_submission_with_identity(second, submission_identity, Some(goals(2.0, None)))
            .await
            .expect("canonical persistence should report the existing owner");

        assert_eq!(
            disposition,
            CanonicalDownloadIdentityDisposition::AdoptedExisting {
                download_id: first_id,
            }
        );
        let existing = store
            .find_by_canonical_download_id(&first_id)
            .await
            .expect("existing submission should load")
            .expect("existing submission should remain");
        assert_eq!(existing.title_id, "title-ambiguous");
        assert_eq!(
            store
                .get_seed_goals_by_canonical_download_id(&first_id)
                .await
                .expect("existing seed goals should load")
                .expect("existing seed goals should remain")
                .seed_goal_ratio,
            Some(1.0)
        );
        assert!(
            store
                .find_by_canonical_download_id(&second_id)
                .await
                .expect("requested identity lookup should succeed")
                .is_none()
        );
    }

    #[tokio::test]
    async fn ambiguous_submissions_are_unbound_durable_rows_and_hidden_from_legacy_readers() {
        let store = store().await;
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO download_clients (id, name) VALUES ({}, {})",
            &[
                SqlArg::Text("primary".to_string()),
                SqlArg::Text("Primary SAB".to_string()),
            ],
        )
        .await
        .expect("client fixture should insert");
        let first = scryer_domain::download_identity::DownloadId::parse(
            "00000000-0000-4000-8000-000000000001",
        )
        .expect("fixed UUID should parse");
        let second = scryer_domain::download_identity::DownloadId::parse(
            "00000000-0000-4000-8000-000000000002",
        )
        .expect("fixed UUID should parse");

        store
            .record_ambiguous_submission(ambiguous_submission(first))
            .await
            .expect("first ambiguous mutation should persist");
        store
            .record_ambiguous_submission(ambiguous_submission(first))
            .await
            .expect("retrying the same ambiguous mutation should be idempotent");
        store
            .record_ambiguous_submission(ambiguous_submission(second))
            .await
            .expect("a later ambiguous mutation must coexist");

        let rows = SqlRuntime::fetch_all(
            store.datastore.read_exec(),
            "SELECT id, download_client_item_id FROM download_submissions
             WHERE title_id = {} ORDER BY id",
            &[SqlArg::Text("title-ambiguous".to_string())],
        )
        .await
        .expect("ambiguous rows should be readable directly");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text("id").expect("first id"), first.to_string());
        assert_eq!(rows[1].text("id").expect("second id"), second.to_string());
        assert!(rows.iter().all(|row| {
            row.opt_text("download_client_item_id")
                .expect("nullable item id")
                .is_none()
        }));

        let bindings = SqlRuntime::fetch_all(
            store.datastore.read_exec(),
            "SELECT download_id, client_config_id, client_type_snapshot,
                    client_name_snapshot, native_item_id, ended_at
             FROM download_client_bindings ORDER BY download_id",
            &[],
        )
        .await
        .expect("unbound bindings should persist with the rows");
        assert_eq!(bindings.len(), 2);
        for binding in &bindings {
            assert_eq!(
                binding.opt_text("client_config_id").expect("config"),
                Some("primary".to_string())
            );
            assert_eq!(
                binding.opt_text("client_type_snapshot").expect("type"),
                Some("sabnzbd".to_string())
            );
            assert_eq!(
                binding.opt_text("client_name_snapshot").expect("name"),
                Some("Primary SAB".to_string())
            );
            assert_eq!(binding.opt_text("native_item_id").expect("native id"), None);
            assert_eq!(binding.opt_text("ended_at").expect("ended at"), None);
        }
        let downloads = SqlRuntime::fetch_all(
            store.datastore.read_exec(),
            "SELECT id, origin FROM downloads ORDER BY id",
            &[],
        )
        .await
        .expect("canonical downloads should persist with the rows");
        assert_eq!(downloads.len(), 2);
        assert!(
            downloads
                .iter()
                .all(|row| { row.text("origin").expect("origin") == "scryer_submission" })
        );

        assert!(
            store
                .list_for_title("title-ambiguous")
                .await
                .expect("legacy title reader should succeed")
                .is_empty()
        );
        let unresolved = store
            .list_active_unbound_for_title("title-ambiguous")
            .await
            .expect("canonical ambiguity reader should succeed");
        assert_eq!(unresolved.len(), 2);
        assert_eq!(unresolved[0].download_id, first);
        assert_eq!(unresolved[1].download_id, second);
        assert!(
            store
                .find_by_title_and_request_signature(
                    "title-ambiguous",
                    "ambiguous-signature",
                    DownloadSubmissionPurpose::Standard,
                    &SubmissionScope::Title,
                )
                .await
                .expect("legacy dedupe reader should succeed")
                .is_none()
        );
    }

    #[tokio::test]
    async fn accepted_sab_style_submission_keeps_its_preallocated_row_id() {
        let store = store().await;
        let download_id = scryer_domain::download_identity::DownloadId::parse(
            "00000000-0000-4000-8000-000000000003",
        )
        .expect("fixed UUID should parse");
        let mut submission = ambiguous_submission(download_id);
        submission.download_client_item_id = "SABnzbd_nzo_1".to_string();
        store
            .record_submission_with_identity(
                submission,
                DownloadSubmissionIdentity {
                    download_id: Some("SABnzbd_nzo_1".to_string()),
                },
                None,
            )
            .await
            .expect("accepted SAB-style submission should persist");
        let stored = store
            .find_by_client_item_id(&ClientJobLocator::new(
                Some("primary"),
                "sabnzbd",
                "SABnzbd_nzo_1",
            ))
            .await
            .expect("stored submission should load")
            .expect("accepted submission should be present");
        assert_eq!(stored.download_id, download_id);
    }

    #[tokio::test]
    async fn download_id_submission_lookup_reads_the_canonical_row() {
        let store = store().await;
        let canonical_download_id = DownloadId::new();
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO download_submissions (
                id, title_id, facet, download_client_id, download_client_type,
                download_client_item_id, download_id
             ) VALUES ({}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(canonical_download_id.to_string()),
                SqlArg::Text("title-canonical".to_string()),
                SqlArg::Text("series".to_string()),
                SqlArg::Text("primary".to_string()),
                SqlArg::Text("nzbget".to_string()),
                SqlArg::Text("canonical-bound-job".to_string()),
                SqlArg::Text("canonical-overloaded-id".to_string()),
            ],
        )
        .await
        .expect("canonical submission fixture should insert");

        let resolved = store
            .list_by_download_id_for_download(
                Some(&canonical_download_id),
                Some("primary"),
                "nzbget",
                "legacy-overloaded-id",
            )
            .await
            .expect("canonical-first lookup should succeed");

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].download_id, canonical_download_id);
        assert_eq!(resolved[0].title_id, "title-canonical");
    }

    /// The import-rejection blocklist writer resolves the grab-time infohash
    /// through this lookup. The name comparison is Rust normalization, never
    /// SQL, so a differently-cased release title still matches — and a title
    /// with no matching submission degrades to `None`, a name-only block.
    #[tokio::test]
    async fn title_release_lookup_resolves_the_grab_time_infohash() {
        let store = store().await;
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO download_submissions (
                id, title_id, facet, download_client_id, download_client_type,
                source_title, info_hash
             ) VALUES ({}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text("submission-hash".to_string()),
                SqlArg::Text("title-hash".to_string()),
                SqlArg::Text("series".to_string()),
                SqlArg::Text("primary".to_string()),
                SqlArg::Text("qbittorrent".to_string()),
                SqlArg::Text("Show.S01E01.1080p.WEB-DL".to_string()),
                SqlArg::Text("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            ],
        )
        .await
        .expect("submission fixture should insert");

        let matched = store
            .find_info_hash_for_title_release("title-hash", "show.s01e01.1080p.web-dl")
            .await
            .expect("lookup should succeed");
        assert_eq!(
            matched.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );

        let other_title = store
            .find_info_hash_for_title_release("title-other", "show.s01e01.1080p.web-dl")
            .await
            .expect("lookup should succeed");
        assert_eq!(other_title, None, "the hash is scoped to its title");

        let other_name = store
            .find_info_hash_for_title_release("title-hash", "other.release.name")
            .await
            .expect("lookup should succeed");
        assert_eq!(other_name, None, "an unmatched name is a name-only block");
    }

    #[tokio::test]
    async fn tracked_state_stub_creation_mints_registry_row_and_active_binding() {
        let store = store().await;
        let locator = ClientJobLocator::new(Some("primary"), "nzbget", "unseen-job");

        store
            .update_tracked_state(&locator, "ignored")
            .await
            .expect("tracked-state stub should persist");

        let row = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT d.id, d.origin, b.download_id AS binding_download_id, s.tracked_state
             FROM downloads d
             JOIN download_client_bindings b ON b.download_id = d.id
             JOIN download_submissions s ON s.id = d.id
             WHERE b.ended_at IS NULL
               AND COALESCE(b.client_config_id, '') = {}
               AND b.client_type_snapshot = {}
               AND b.native_item_id = {}",
            &[
                SqlArg::Text("primary".to_string()),
                SqlArg::Text("nzbget".to_string()),
                SqlArg::Text("unseen-job".to_string()),
            ],
        )
        .await
        .expect("registry row should load")
        .expect("registry row should exist");

        assert_eq!(row.text("origin").expect("origin"), "foreign_observation");
        assert_eq!(
            row.text("id").expect("download id"),
            row.text("binding_download_id")
                .expect("binding download id")
        );
        assert_eq!(row.text("tracked_state").expect("tracked state"), "ignored");
    }

    #[tokio::test]
    async fn failed_identity_state_uses_the_canonical_key_and_preserves_compatibility_columns() {
        let store = store().await;
        let canonical_download_id = DownloadId::new();
        let identity = DownloadSubmissionIdentity {
            download_id: Some("legacy-failure-id".to_string()),
        };
        let source_identity = ClientJobLocator::new(Some("primary"), "nzbget", "legacy-failed-job");

        store
            .record_identity_tracked_state_for_download(
                Some(&canonical_download_id),
                &identity,
                Some(&source_identity),
                "failed",
                Some("import_gate_rejected"),
                Some("failure detail"),
            )
            .await
            .expect("failed state should persist");

        let row = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT identity_key, canonical_download_id, download_id
             FROM download_identity_states
             LIMIT 1",
            &[],
        )
        .await
        .expect("failed state row should load")
        .expect("failed state row should exist");

        assert_eq!(
            row.text("identity_key").expect("canonical identity key"),
            format!("download:{canonical_download_id}")
        );
        assert_eq!(
            row.opt_text("canonical_download_id")
                .expect("canonical column"),
            Some(canonical_download_id.to_string())
        );
        assert_eq!(
            row.opt_text("download_id").expect("legacy download id"),
            Some("legacy-failure-id".to_string())
        );
    }

    #[tokio::test]
    async fn a_token_less_identity_state_round_trips_on_the_canonical_download_id() {
        // Plugin download clients omit the legacy wire token entirely. The row
        // is keyed by the canonical download id, so it is written with a NULL
        // `download_id` and still reads back on the restart path.
        let store = store().await;
        let canonical_download_id = DownloadId::new();
        let identity = DownloadSubmissionIdentity { download_id: None };
        let source_identity = ClientJobLocator::new(Some("primary"), "plugin", "plugin-job");

        store
            .record_identity_tracked_state_for_download(
                Some(&canonical_download_id),
                &identity,
                Some(&source_identity),
                "failed",
                Some("import_gate_rejected"),
                Some("failure detail"),
            )
            .await
            .expect("token-less state should persist");

        let row = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT identity_key, canonical_download_id, download_id
             FROM download_identity_states
             LIMIT 1",
            &[],
        )
        .await
        .expect("token-less state row should load")
        .expect("token-less state row should exist");
        assert_eq!(
            row.text("identity_key").expect("canonical identity key"),
            format!("download:{canonical_download_id}")
        );
        assert_eq!(
            row.opt_text("canonical_download_id")
                .expect("canonical column"),
            Some(canonical_download_id.to_string())
        );
        assert_eq!(
            row.opt_text("download_id").expect("legacy download id"),
            None
        );

        assert_eq!(
            store
                .get_identity_tracked_state_for_download(
                    Some(&canonical_download_id),
                    &identity,
                    Some(&source_identity),
                )
                .await
                .expect("token-less state should read back"),
            Some("failed".to_string())
        );
        assert_eq!(
            store
                .get_identity_tracked_state_reason_for_download(
                    Some(&canonical_download_id),
                    &identity,
                    Some(&source_identity),
                )
                .await
                .expect("token-less reason should read back"),
            Some("import_gate_rejected".to_string())
        );
        assert_eq!(
            store
                .get_identity_tracked_state_detail_for_download(
                    Some(&canonical_download_id),
                    &identity,
                    Some(&source_identity),
                )
                .await
                .expect("token-less detail should read back"),
            Some("failure detail".to_string())
        );
    }

    /// Verbatim sqlite text for the 0180 active-locator index, as `repo_err`
    /// flattens sqlx failures into `AppError::Repository`.
    const ACTIVE_LOCATOR_VIOLATION: &str = "error returned from database: (code: 2067) UNIQUE \
         constraint failed: index 'idx_download_client_bindings_active_locator_unique'";
    const FOREIGN_KEY_VIOLATION: &str = "error returned from database: (code: 787) FOREIGN KEY \
         constraint failed";

    /// The 0180 index the claim's binding insert races on. The shared fixture
    /// omits it, so the retry tests add it verbatim.
    async fn create_active_locator_unique_index(store: &DownloadSubmissionStore) {
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "CREATE UNIQUE INDEX idx_download_client_bindings_active_locator_unique
                 ON download_client_bindings(client_config_id, client_type_snapshot, native_item_id)
                 WHERE native_item_id IS NOT NULL
                   AND ended_at IS NULL",
            &[],
        )
        .await
        .expect("active-locator unique index should be created");
    }

    async fn count_rows(store: &DownloadSubmissionStore, table: &str) -> i64 {
        SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            &format!("SELECT COUNT(*) AS row_count FROM {table}"),
            &[],
        )
        .await
        .expect("count should read")
        .expect("count should return a row")
        .i64("row_count")
        .expect("count should decode")
    }

    async fn insert_committed_winner(
        store: &DownloadSubmissionStore,
        download_id: &DownloadId,
        locator: &ClientJobLocator,
    ) {
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO downloads (id, origin, created_at) VALUES ({}, 'foreign_observation', {})",
            &[
                SqlArg::Text(download_id.to_string()),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await
        .expect("winner download should insert");
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO download_client_bindings (
                download_id, client_config_id, client_type_snapshot, client_name_snapshot,
                native_item_id, created_at, last_seen_at, ended_at
             ) VALUES ({}, {}, {}, NULL, {}, {}, {}, NULL)",
            &[
                SqlArg::Text(download_id.to_string()),
                SqlArg::OptText(locator.client_id.clone()),
                SqlArg::Text(locator.client_type.clone()),
                SqlArg::Text(locator.item_id.clone()),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await
        .expect("winner binding should insert");
    }

    /// The losing claimant's whole transaction is gone by the time the
    /// violation surfaces, so the wrapper must re-run the operation from the
    /// top rather than patch anything up in place.
    ///
    /// Sqlite serializes writers behind the store's writer gate, so the race
    /// cannot be lost there for real; the first attempt's failure is injected
    /// after a genuine claim so the rollback is observable.
    #[tokio::test]
    async fn a_claim_that_loses_the_locator_race_reruns_in_a_fresh_transaction() {
        let store = store().await;
        create_active_locator_unique_index(&store).await;
        let locator = ClientJobLocator::new(Some("primary"), "qbittorrent", "raced-job");
        let attempts = Arc::new(AtomicUsize::new(0));

        let claimed = run_in_transaction_retrying_unique_violation(
            &store.datastore,
            "claim_retry_test",
            |tx| {
                let locator = locator.clone();
                let attempts = Arc::clone(&attempts);
                Box::pin(async move {
                    let claimed =
                        claim_or_create_binding_download_id_tx(tx, &locator, None).await?;
                    if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        return Err(AppError::Repository(ACTIVE_LOCATOR_VIOLATION.to_string()));
                    }
                    Ok(claimed)
                })
            },
        )
        .await
        .expect("the retry should claim the locator");

        assert_eq!(attempts.load(Ordering::SeqCst), 2, "exactly one retry");
        // The losing attempt minted its own download and binding; both rolled
        // back, so only the retry's rows survive.
        assert_eq!(count_rows(&store, "downloads").await, 1);
        assert_eq!(count_rows(&store, "download_client_bindings").await, 1);
        assert_eq!(
            active_binding_download_id(store.datastore.read_exec(), &locator)
                .await
                .expect("active binding should load"),
            Some(claimed)
        );
    }

    /// The retry adopts the winner through the claim's ordinary
    /// already-bound-locator branch — there is no special-case recovery path,
    /// and no second identity is minted.
    #[tokio::test]
    async fn a_claim_retry_adopts_the_winner_that_committed_the_locator() {
        let store = store().await;
        create_active_locator_unique_index(&store).await;
        let locator = ClientJobLocator::new(Some("primary"), "qbittorrent", "raced-job");
        let winner = DownloadId::new();
        insert_committed_winner(&store, &winner, &locator).await;
        let attempts = Arc::new(AtomicUsize::new(0));

        let claimed = run_in_transaction_retrying_unique_violation(
            &store.datastore,
            "claim_retry_test",
            |tx| {
                let locator = locator.clone();
                let attempts = Arc::clone(&attempts);
                Box::pin(async move {
                    if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        // This claimant read the locator as unbound before the
                        // winner committed, so its insert lost the race.
                        return Err(AppError::Repository(ACTIVE_LOCATOR_VIOLATION.to_string()));
                    }
                    claim_or_create_binding_download_id_tx(tx, &locator, None).await
                })
            },
        )
        .await
        .expect("the retry should adopt the winner");

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(claimed, winner);
        assert_eq!(count_rows(&store, "downloads").await, 1);
        assert_eq!(count_rows(&store, "download_client_bindings").await, 1);
    }

    #[tokio::test]
    async fn a_second_unique_violation_surfaces_the_error_instead_of_spinning() {
        let store = store().await;
        let attempts = Arc::new(AtomicUsize::new(0));

        let error = run_in_transaction_retrying_unique_violation::<DownloadId, _>(
            &store.datastore,
            "claim_retry_test",
            |_tx| {
                let attempts = Arc::clone(&attempts);
                Box::pin(async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Err(AppError::Repository(ACTIVE_LOCATOR_VIOLATION.to_string()))
                })
            },
        )
        .await
        .expect_err("a persistent conflict must not be swallowed");

        assert_eq!(attempts.load(Ordering::SeqCst), 2, "exactly one retry");
        let AppError::Repository(message) = error else {
            panic!("the original repository error should surface");
        };
        assert_eq!(message, ACTIVE_LOCATOR_VIOLATION);
    }

    #[tokio::test]
    async fn a_failure_that_is_not_a_unique_violation_is_never_retried() {
        let store = store().await;
        let attempts = Arc::new(AtomicUsize::new(0));

        let error = run_in_transaction_retrying_unique_violation::<DownloadId, _>(
            &store.datastore,
            "claim_retry_test",
            |_tx| {
                let attempts = Arc::clone(&attempts);
                Box::pin(async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    // Deliberately not "database is locked": the sqlite busy
                    // retries inside `run_in_transaction` own that one.
                    Err(AppError::Repository(FOREIGN_KEY_VIOLATION.to_string()))
                })
            },
        )
        .await
        .expect_err("an unrelated failure must propagate");

        assert_eq!(attempts.load(Ordering::SeqCst), 1, "no retry");
        let AppError::Repository(message) = error else {
            panic!("the original repository error should surface");
        };
        assert_eq!(message, FOREIGN_KEY_VIOLATION);
    }

    /// The history query inlines its terminal states so `LIMIT` cuts an
    /// already-filtered set. Every variant is listed here on purpose: a new
    /// `TrackedDownloadState` that is terminal has to be added to both this
    /// list and the SQL, and this test is what says so.
    #[test]
    fn terminal_history_states_match_the_domain_definition() {
        let mut expected = [
            TrackedDownloadState::Downloading,
            TrackedDownloadState::ImportPending,
            TrackedDownloadState::Importing,
            TrackedDownloadState::Imported,
            TrackedDownloadState::ImportedSeeding,
            TrackedDownloadState::ImportBlocked,
            TrackedDownloadState::FailedPending,
            TrackedDownloadState::Failed,
            TrackedDownloadState::Ignored,
        ]
        .into_iter()
        .filter(|state| state.is_terminal())
        .map(TrackedDownloadState::as_str)
        .collect::<Vec<_>>();
        expected.sort_unstable();

        let mut actual = TERMINAL_HISTORY_TRACKED_STATES.to_vec();
        actual.sort_unstable();
        assert_eq!(actual, expected);

        for state in TERMINAL_HISTORY_TRACKED_STATES {
            assert!(
                TERMINAL_DOWNLOAD_HISTORY_SQL.contains(&format!("'{state}'")),
                "the history query must filter on {state}"
            );
        }
    }

    #[tokio::test]
    async fn terminal_history_rows_project_finished_downloads_only() {
        let store = store().await;
        let imported_id = DownloadId::new();
        store
            .record_submission_with_identity(
                submission(imported_id, "job-1", "title-1"),
                submission_identity(imported_id),
                None,
            )
            .await
            .expect("imported submission should persist");
        let live_id = DownloadId::new();
        store
            .record_submission_with_identity(
                submission(live_id, "job-2", "title-1"),
                submission_identity(live_id),
                None,
            )
            .await
            .expect("live submission should persist");

        store
            .update_tracked_state(
                &ClientJobLocator::new(Some("primary"), "qbittorrent", "job-1"),
                TrackedDownloadState::Imported.as_str(),
            )
            .await
            .expect("terminal state should persist");
        store
            .update_tracked_state(
                &ClientJobLocator::new(Some("primary"), "qbittorrent", "job-2"),
                TrackedDownloadState::Downloading.as_str(),
            )
            .await
            .expect("live state should persist");

        let rows = store
            .list_terminal_download_history_rows(50)
            .await
            .expect("terminal history rows should read");

        assert_eq!(
            rows.iter().map(|row| row.download_id).collect::<Vec<_>>(),
            vec![imported_id],
            "only the finished download is durable history"
        );
        let row = &rows[0];
        assert_eq!(row.tracked_state, TrackedDownloadState::Imported.as_str());
        assert_eq!(row.title_id.as_deref(), Some("title-1"));
        assert_eq!(row.download_client_item_id.as_deref(), Some("job-1"));
        assert_eq!(row.client_type.as_deref(), Some("qbittorrent"));
        assert_eq!(row.source_title.as_deref(), Some("Release job-1"));
        assert_eq!(row.size_bytes, Some(123));
    }

    #[tokio::test]
    async fn a_zero_limit_reads_nothing() {
        let store = store().await;
        assert!(
            store
                .list_terminal_download_history_rows(0)
                .await
                .expect("a zero limit should not error")
                .is_empty()
        );
    }
}

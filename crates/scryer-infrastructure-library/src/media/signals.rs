//! Dual-dialect storage for media-server watch signals (RFC 137 section 7.3).
//!
//! Two tables: the normalized per-user observations, and one sync-state row per
//! connection.
//!
//! # The generation swap
//!
//! [`MediaServerSignalStore::replace_participant_signals`] is the only write
//! that matters, and it is the reason `sync_generation` exists. Providers
//! answer "what has this person played", never "what has this person unplayed",
//! so the absence of an item in a sweep is the only evidence that it is no
//! longer played. The swap turns that absence into a deletion:
//!
//! 1. take the participant's next generation number,
//! 2. upsert every observed row at that generation,
//! 3. delete the participant's rows still sitting at an older one.
//!
//! Upsert rather than delete-then-insert, so a row that survives a sweep keeps
//! its identity and `created_at`. All three steps run in one transaction: a
//! crash between them would otherwise leave a participant with a half-deleted
//! history that reads as "stopped watching".
//!
//! # Provider neutrality
//!
//! Nothing here is Jellyfin-specific. `provider` is a stored column, so the
//! Emby and Plex adapters write through this same store unchanged.

use async_trait::async_trait;
use scryer_application::{AppError, AppResult, MediaServerSignalRepository};
use scryer_domain::{
    MediaServerProvider, MediaServerSignalKind, MediaServerSignalSyncState, NewUserMediaSignal,
    UserMediaSignal,
};
use std::collections::{HashMap, HashSet};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

#[derive(Clone)]
pub struct MediaServerSignalStore {
    datastore: StoreDatastore,
}

impl MediaServerSignalStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

const SIGNAL_COLUMNS: &str = "id, connection_id, provider, external_user_id, scryer_user_id,
    provider_item_id, kind, scryer_title_id, scryer_episode_id, played, play_count,
    last_played_at, observed_at, sync_generation, created_at, updated_at";

const SIGNAL_INSERT_PREFIX: &str = "INSERT INTO media_server_user_media_signals
        (id, connection_id, provider, external_user_id, scryer_user_id, provider_item_id, kind,
         scryer_title_id, scryer_episode_id, played, play_count, last_played_at, observed_at,
         sync_generation, created_at, updated_at)";

/// The conflict target is the table's natural key. `id` and `created_at` are
/// deliberately absent from the update list: a row that survives a sweep is the
/// same observation, not a new one.
const SIGNAL_UPSERT_SUFFIX: &str =
    "ON CONFLICT (connection_id, external_user_id, provider_item_id) DO UPDATE SET
         provider = excluded.provider,
         scryer_user_id = excluded.scryer_user_id,
         kind = excluded.kind,
         scryer_title_id = excluded.scryer_title_id,
         scryer_episode_id = excluded.scryer_episode_id,
         played = excluded.played,
         play_count = excluded.play_count,
         last_played_at = excluded.last_played_at,
         observed_at = excluded.observed_at,
         sync_generation = excluded.sync_generation,
         updated_at = excluded.updated_at";

const SIGNAL_ROW_WIDTH: usize = 16;

const SYNC_STATE_COLUMNS: &str = "connection_id, provider, enabled, last_started_at,
    last_success_at, last_error, participant_count, signal_count, updated_at";

#[async_trait]
impl MediaServerSignalRepository for MediaServerSignalStore {
    async fn replace_participant_signals(
        &self,
        connection_id: &str,
        external_user_id: &str,
        signals: &[NewUserMediaSignal],
    ) -> AppResult<u64> {
        // A provider that lists one item twice would make the batched upsert
        // touch a row twice in one statement, which Postgres refuses outright.
        // Last write wins, matching what a per-row loop would have done.
        let mut seen = HashSet::new();
        let mut deduped: Vec<&NewUserMediaSignal> = Vec::with_capacity(signals.len());
        for signal in signals.iter().rev() {
            if seen.insert(signal.provider_item_id.clone()) {
                deduped.push(signal);
            }
        }
        deduped.reverse();

        let connection_id = connection_id.to_string();
        let external_user_id = external_user_id.to_string();
        let owned = deduped
            .into_iter()
            .cloned()
            .collect::<Vec<NewUserMediaSignal>>();

        SqlRuntime::run_in_transaction(
            &self.datastore,
            "replace_media_server_participant_signals",
            move |tx| {
                let connection_id = connection_id.clone();
                let external_user_id = external_user_id.clone();
                let owned = owned.clone();
                Box::pin(async move {
                    let participant_args = vec![
                        SqlArg::Text(connection_id.clone()),
                        SqlArg::Text(external_user_id.clone()),
                    ];

                    // Read inside the transaction: two sweeps of the same
                    // participant must not pick the same generation and leave
                    // each other's rows looking current.
                    let generation = SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "SELECT MAX(sync_generation) AS newest
                           FROM media_server_user_media_signals
                          WHERE connection_id = {} AND external_user_id = {}",
                        &participant_args,
                    )
                    .await?
                    .map(|row| row.opt_i64("newest"))
                    .transpose()?
                    .flatten()
                    .unwrap_or(0)
                        + 1;

                    let now = chrono::Utc::now();
                    let rows = owned
                        .iter()
                        .map(|signal| {
                            signal_args(&connection_id, &external_user_id, signal, generation, now)
                        })
                        .collect::<Vec<_>>();

                    SqlRuntime::execute_batch_insert(
                        tx,
                        SIGNAL_INSERT_PREFIX,
                        SIGNAL_ROW_WIDTH,
                        rows,
                        SIGNAL_UPSERT_SUFFIX,
                    )
                    .await?;

                    // Everything the sweep did not just write is gone. This is
                    // the step that makes "no longer played" expressible, and
                    // it is why an empty `signals` is a real write rather than
                    // a no-op.
                    let mut delete_args = participant_args.clone();
                    delete_args.push(SqlArg::I64(generation));
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM media_server_user_media_signals
                          WHERE connection_id = {} AND external_user_id = {}
                            AND sync_generation < {}",
                        &delete_args,
                    )
                    .await?;

                    let remaining = SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "SELECT COUNT(*) AS signal_count
                           FROM media_server_user_media_signals
                          WHERE connection_id = {} AND external_user_id = {}",
                        &participant_args,
                    )
                    .await?
                    .map(|row| row.i64("signal_count"))
                    .transpose()?
                    .unwrap_or(0);

                    Ok(remaining.max(0) as u64)
                })
            },
        )
        .await
    }

    async fn movie_signals_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<UserMediaSignal>>> {
        self.signals_by_title(title_ids, MediaServerSignalKind::Movie)
            .await
    }

    async fn episode_signals_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<UserMediaSignal>>> {
        self.signals_by_title(title_ids, MediaServerSignalKind::Episode)
            .await
    }

    async fn signal_sync_states(&self) -> AppResult<Vec<MediaServerSignalSyncState>> {
        let sql = format!(
            "SELECT {SYNC_STATE_COLUMNS}
               FROM media_server_signal_sync_state
              ORDER BY connection_id"
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &[])
            .await?
            .iter()
            .map(row_to_sync_state)
            .collect()
    }

    async fn upsert_signal_sync_state(&self, state: &MediaServerSignalSyncState) -> AppResult<()> {
        let args = vec![
            SqlArg::Text(state.connection_id.clone()),
            SqlArg::Text(state.provider.as_str().to_string()),
            SqlArg::Bool(state.enabled),
            SqlArg::OptTimestamp(state.last_started_at),
            SqlArg::OptTimestamp(state.last_success_at),
            SqlArg::OptText(state.last_error.clone()),
            SqlArg::I64(state.participant_count),
            SqlArg::I64(state.signal_count),
            SqlArg::Timestamp(state.updated_at),
        ];
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "upsert_media_server_signal_sync_state",
            move |tx| {
                let args = args.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO media_server_signal_sync_state
                             (connection_id, provider, enabled, last_started_at, last_success_at,
                              last_error, participant_count, signal_count, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})
                         ON CONFLICT (connection_id) DO UPDATE SET
                             provider = excluded.provider,
                             enabled = excluded.enabled,
                             last_started_at = excluded.last_started_at,
                             last_success_at = excluded.last_success_at,
                             last_error = excluded.last_error,
                             participant_count = excluded.participant_count,
                             signal_count = excluded.signal_count,
                             updated_at = excluded.updated_at",
                        &args,
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }
}

impl MediaServerSignalStore {
    /// Both batched reads are the same query with a different `kind`, grouped
    /// by the title that owns the observation. A title with no rows is simply
    /// absent from the map: an empty vector would claim "asked and nobody
    /// watched it", which is not what a missing row means.
    async fn signals_by_title(
        &self,
        title_ids: &[String],
        kind: MediaServerSignalKind,
    ) -> AppResult<HashMap<String, Vec<UserMediaSignal>>> {
        if title_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let placeholders = title_ids
            .iter()
            .map(|_| "{}")
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {SIGNAL_COLUMNS}
               FROM media_server_user_media_signals
              WHERE kind = {{}}
                AND scryer_title_id IN ({placeholders})
              ORDER BY scryer_title_id, external_user_id, provider_item_id"
        );
        let mut args = vec![SqlArg::Text(kind.as_storage_str().to_string())];
        args.extend(title_ids.iter().cloned().map(SqlArg::Text));

        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        let mut grouped: HashMap<String, Vec<UserMediaSignal>> = HashMap::new();
        for row in rows.iter() {
            let signal = row_to_signal(row)?;
            // The `IN` clause already excludes NULLs; the guard is here so an
            // unmapped row can never be filed under an empty title id.
            if let Some(title_id) = signal.scryer_title_id.clone() {
                grouped.entry(title_id).or_default().push(signal);
            }
        }
        Ok(grouped)
    }
}

fn signal_args(
    connection_id: &str,
    external_user_id: &str,
    signal: &NewUserMediaSignal,
    generation: i64,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(scryer_domain::Id::new().0),
        SqlArg::Text(connection_id.to_string()),
        SqlArg::Text(signal.provider.as_str().to_string()),
        SqlArg::Text(external_user_id.to_string()),
        SqlArg::OptText(signal.scryer_user_id.clone()),
        SqlArg::Text(signal.provider_item_id.clone()),
        SqlArg::Text(signal.kind.as_storage_str().to_string()),
        SqlArg::OptText(signal.scryer_title_id.clone()),
        SqlArg::OptText(signal.scryer_episode_id.clone()),
        SqlArg::Bool(signal.played),
        SqlArg::I64(signal.play_count),
        SqlArg::OptTimestamp(signal.last_played_at),
        SqlArg::Timestamp(signal.observed_at),
        SqlArg::I64(generation),
        SqlArg::Timestamp(now),
        SqlArg::Timestamp(now),
    ]
}

fn row_to_signal(row: &SqlRow) -> AppResult<UserMediaSignal> {
    let provider = MediaServerProvider::parse(&row.text("provider")?)
        .ok_or_else(|| AppError::Repository("invalid media server signal provider".into()))?;
    let kind = MediaServerSignalKind::parse_storage(&row.text("kind")?)
        .ok_or_else(|| AppError::Repository("invalid media server signal kind".into()))?;
    Ok(UserMediaSignal {
        id: row.text("id")?,
        connection_id: row.text("connection_id")?,
        provider,
        external_user_id: row.text("external_user_id")?,
        scryer_user_id: row.opt_text("scryer_user_id")?,
        provider_item_id: row.text("provider_item_id")?,
        kind,
        scryer_title_id: row.opt_text("scryer_title_id")?,
        scryer_episode_id: row.opt_text("scryer_episode_id")?,
        played: row.bool("played")?,
        play_count: row.i64("play_count")?,
        last_played_at: row.opt_timestamp("last_played_at")?,
        observed_at: row.timestamp("observed_at")?,
        sync_generation: row.i64("sync_generation")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn row_to_sync_state(row: &SqlRow) -> AppResult<MediaServerSignalSyncState> {
    let provider = MediaServerProvider::parse(&row.text("provider")?)
        .ok_or_else(|| AppError::Repository("invalid media server signal provider".into()))?;
    Ok(MediaServerSignalSyncState {
        connection_id: row.text("connection_id")?,
        provider,
        enabled: row.bool("enabled")?,
        last_started_at: row.opt_timestamp("last_started_at")?,
        last_success_at: row.opt_timestamp("last_success_at")?,
        last_error: row.opt_text("last_error")?,
        participant_count: row.i64("participant_count")?,
        signal_count: row.i64("signal_count")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

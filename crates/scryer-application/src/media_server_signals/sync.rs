//! Scheduled media-server signal synchronization (RFC 137 sections 4.4, 7.3).
//!
//! One pass over every Jellyfin, Emby, and Plex connection. Per connection: resolve the
//! participant set, read each participant's played items through the adapter,
//! map them onto Scryer subjects, and replace that participant's stored signal
//! set atomically. Per connection the outcome is recorded in
//! `media_server_signal_sync_state`.
//!
//! # Isolation is the whole design
//!
//! Three levels of failure, three containments:
//!
//! * **One participant fails** — recorded, counted, and skipped. Their existing
//!   rows are left alone: a replace is only performed with a set that was
//!   actually read, because replacing with nothing would delete a person's real
//!   watch history because their account was momentarily unreadable.
//! * **One connection fails** — recorded as that connection's `last_error`,
//!   and the sweep moves to the next connection.
//! * **The connection list fails** — the job fails, because there is nothing to
//!   sweep and nothing to record it against.
//!
//! A connection with any participant failure does **not** advance
//! `last_success_at`. Freshness has to mean "everything here was read", or a
//! rule that requires fresh signals would trust a partial answer.
//!
//! # Provider scope
//!
//! Jellyfin, Emby, and Plex. The tables, the normalized item, the store, and
//! this orchestration are all provider-neutral; each provider is one arm in the
//! adapter plus an entry in [`SIGNAL_SYNC_PROVIDERS`].
//!
//! A provider in that list is also part of the lifecycle-rule freshness gate:
//! until every enabled connection of every listed provider has swept cleanly,
//! watch facts stay unobserved rather than answering from a partial roster.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use scryer_domain::{
    ExternalAccountProvider, Id, MediaServerConnection, MediaServerProvider, MediaServerSignalKind,
    MediaServerSignalSyncState, NewUserMediaSignal, UserExternalAccount,
};
use serde::Serialize;
use tracing::{info, warn};

use crate::media_server_signals::mapping::{
    EpisodeNumberIndex, MappedSignalSubject, SIGNAL_EXTERNAL_ID_SOURCES, ScryerLibraryPaths,
    TitleExternalIdIndex, is_foreign_item, resolve_subject,
};
use crate::media_server_signals::reasons;
use crate::ports::{
    AudiencePlay, MediaServerSignalReader, ProviderPlayedItem, SignalReadFailure, SignalRosterLink,
    TitleExternalIdLookup,
};
use crate::{AppResult, AppUseCase};

/// Providers the sweep synchronizes. Every provider here has a reading arm in
/// the signal source; listing one without it would report an error per sweep
/// and hold every watch fact unobserved.
pub const SIGNAL_SYNC_PROVIDERS: [MediaServerProvider; 3] = [
    MediaServerProvider::Jellyfin,
    MediaServerProvider::Emby,
    MediaServerProvider::Plex,
];

/// One connection's sweep state as an operator reads it: the stored row plus
/// whether its last clean sweep still counts for the watch gate.
#[derive(Clone, Debug)]
pub struct MediaServerSignalSyncStatus {
    pub state: MediaServerSignalSyncState,
    /// A clean sweep is on record but the watch gate no longer accepts it:
    /// it is older than the freshness window, or it was recorded while the
    /// connection was off and nothing has swept since it was turned back on.
    pub clean_sweep_stale: bool,
}

impl MediaServerSignalSyncStatus {
    pub fn at(state: MediaServerSignalSyncState, now: chrono::DateTime<Utc>) -> Self {
        let clean_sweep_stale = state.last_success_at.is_some_and(|last_success_at| {
            !state.enabled
                || !crate::maintenance_rules::facts::watch_signal_sweep_is_fresh(
                    last_success_at,
                    now,
                )
        });
        Self {
            state,
            clean_sweep_stale,
        }
    }
}

/// What one sweep did, for the job summary.
///
/// `participants_failed` is reported separately from `connections_failed`
/// because they mean different things to an operator: one person's Jellyfin
/// account being unreadable is not the same as a whole server being down.
#[derive(Clone, Debug, Default, Serialize)]
pub struct MediaServerSignalSyncReport {
    pub connections_considered: usize,
    pub connections_synced: usize,
    pub connections_skipped_disabled: usize,
    pub connections_failed: usize,
    pub participants_considered: usize,
    pub participants_synced: usize,
    pub participants_failed: usize,
    pub signals_written: u64,
    pub signals_unmapped: u64,
}

/// One connection's outcome, folded into the report and the stored state.
#[derive(Default)]
struct ConnectionOutcome {
    participants: usize,
    participants_synced: usize,
    participants_failed: usize,
    signals: u64,
    unmapped: u64,
    /// First failure, kept for the state row as its code and its sentence. A
    /// later failure does not overwrite it: the first one is usually the cause
    /// and the rest the consequence.
    error: Option<SignalReadFailure>,
}

impl ConnectionOutcome {
    fn record_error(&mut self, failure: SignalReadFailure) {
        if self.error.is_none() {
            self.error = Some(failure);
        }
    }
}

/// One participant's mapped read.
struct ParticipantOutcome {
    written: u64,
    unmapped: u64,
    /// A played movie stayed unmapped without being provably foreign, so a
    /// played Scryer title may be among the unmapped rows.
    incomplete: bool,
}

/// A failure of Scryer's own store, as a sweep failure.
fn store_failure(context: &str, error: impl std::fmt::Display) -> SignalReadFailure {
    SignalReadFailure::new(reasons::STORE_FAILED, format!("{context}: {error}"))
}

impl AppUseCase {
    /// Job body for [`crate::jobs::JobKey::MediaServerSignalSync`].
    pub(crate) async fn run_media_server_signal_sync_job(
        &self,
    ) -> AppResult<MediaServerSignalSyncReport> {
        let mut report = MediaServerSignalSyncReport::default();

        // The one error that fails the whole job: with no connection list there
        // is nothing to sweep, and no state row to record the failure against.
        let mut connections = Vec::new();
        for provider in SIGNAL_SYNC_PROVIDERS {
            connections.extend(
                self.services
                    .integrations
                    .media_server_connections
                    .list(Some(provider))
                    .await?,
            );
        }

        // Prior state is carried forward field by field, so a sweep that fails
        // never rewrites the timestamp of the last one that succeeded.
        let previous = self
            .services
            .integrations
            .media_server_signals
            .signal_sync_states()
            .await?
            .into_iter()
            .map(|state| (state.connection_id.clone(), state))
            .collect::<HashMap<_, _>>();

        // Where Scryer keeps files, read once for the sweep. Without it no
        // played file can be shown to lie outside Scryer's libraries, so an
        // unmatched movie with a file path then keeps the read incomplete.
        let library_roots = match self.services.catalog.libraries.list(None).await {
            Ok(libraries) => Some(
                libraries
                    .into_iter()
                    .flat_map(|library| library.roots.into_iter().map(|root| root.path))
                    .collect::<Vec<_>>(),
            ),
            Err(error) => {
                warn!(
                    error = %error,
                    "could not read library roots for a signal sync; unmatched played files count as Scryer's"
                );
                None
            }
        };

        for connection in connections {
            report.connections_considered += 1;
            let started_at = Utc::now();
            let prior = previous.get(&connection.id);

            if !connection.enabled {
                report.connections_skipped_disabled += 1;
                // Still recorded: "off" is an answer, and without it a reader
                // cannot tell a disabled connection from a silent one.
                self.record_signal_sync_state(MediaServerSignalSyncState {
                    connection_id: connection.id.clone(),
                    provider: connection.provider.clone(),
                    enabled: false,
                    last_started_at: prior.and_then(|state| state.last_started_at),
                    last_success_at: prior.and_then(|state| state.last_success_at),
                    last_error: None,
                    last_error_code: None,
                    participant_count: prior.map_or(0, |state| state.participant_count),
                    signal_count: prior.map_or(0, |state| state.signal_count),
                    updated_at: started_at,
                })
                .await;
                continue;
            }

            let library = ScryerLibraryPaths::new(library_roots.clone(), &connection.path_mappings);
            let outcome = self
                .sync_one_connection(&connection, &library, started_at)
                .await;
            report.participants_considered += outcome.participants;
            report.participants_synced += outcome.participants_synced;
            report.participants_failed += outcome.participants_failed;
            report.signals_written += outcome.signals;
            report.signals_unmapped += outcome.unmapped;

            let clean = outcome.error.is_none();
            if clean {
                report.connections_synced += 1;
            } else {
                report.connections_failed += 1;
            }

            self.record_signal_sync_state(MediaServerSignalSyncState {
                connection_id: connection.id.clone(),
                provider: connection.provider.clone(),
                enabled: true,
                last_started_at: Some(started_at),
                // Only a fully clean sweep is a success. A partial read that
                // advanced this would let a freshness check pass over data that
                // was never actually refreshed.
                last_success_at: if clean {
                    Some(Utc::now())
                } else {
                    prior.and_then(|state| state.last_success_at)
                },
                last_error: outcome
                    .error
                    .as_ref()
                    .map(|failure| failure.message.clone()),
                last_error_code: outcome
                    .error
                    .as_ref()
                    .map(|failure| failure.reason.to_string()),
                participant_count: outcome.participants as i64,
                signal_count: outcome.signals as i64,
                updated_at: Utc::now(),
            })
            .await;
        }

        Ok(report)
    }

    /// Sweep one enabled connection. Never returns an error: everything that
    /// can go wrong belongs in this connection's state row.
    async fn sync_one_connection(
        &self,
        connection: &MediaServerConnection,
        library: &ScryerLibraryPaths,
        started_at: chrono::DateTime<Utc>,
    ) -> ConnectionOutcome {
        let mut outcome = ConnectionOutcome::default();

        if !connection.api_key_present() {
            outcome.record_error(SignalReadFailure::new(
                reasons::CREDENTIAL_MISSING,
                "no credential stored",
            ));
            return outcome;
        }

        let Some(account_provider) = connection.provider.external_account_provider() else {
            outcome.record_error(SignalReadFailure::new(
                reasons::PROVIDER_UNSUPPORTED,
                "provider has no linked-account identity",
            ));
            return outcome;
        };

        let participants = match self
            .signal_participants(account_provider, &connection.id)
            .await
        {
            Ok(participants) => participants,
            Err(error) => {
                outcome.record_error(store_failure("could not resolve participants", error));
                return outcome;
            }
        };
        outcome.participants = participants.len();
        let identities: Vec<_> = participants
            .iter()
            .filter_map(|participant| {
                participant
                    .external_user_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(|id| (id.to_string(), participant.user_id.clone()))
            })
            .collect();
        if let Err(error) = self
            .services
            .integrations
            .media_server_signals
            .retain_connection_participants(&connection.id, &identities)
            .await
        {
            outcome.record_error(store_failure("could not retire participant signals", error));
            return outcome;
        }
        // No early return when nobody is linked: the audience read below is
        // every account's on the server, and the anonymous facts it feeds
        // stand without a linked user.

        // Opened once for every participant: whatever the provider needs per
        // connection (the server to talk to, who owns it) is read once a sweep.
        let reader = match self
            .services
            .integrations
            .media_server_signal_source
            .open(connection)
            .await
        {
            Ok(reader) => reader,
            Err(failure) => {
                outcome.participants_failed += identities.len();
                warn!(
                    connection_id = connection.id.as_str(),
                    reason = failure.reason,
                    error = %failure,
                    "media-server connection could not be opened for a signal sync; stored signals are unchanged"
                );
                outcome.record_error(failure);
                return outcome;
            }
        };

        for participant in participants {
            let Some(external_user_id) = participant
                .external_user_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                // The repository query already excludes these; this is the
                // belt-and-braces arm, not an expected path.
                continue;
            };

            match self
                .sync_one_participant(
                    connection,
                    reader.as_ref(),
                    library,
                    external_user_id,
                    &participant,
                    started_at,
                )
                .await
            {
                Ok(read) => {
                    outcome.participants_synced += 1;
                    outcome.signals += read.written;
                    outcome.unmapped += read.unmapped;
                    if read.incomplete {
                        outcome.record_error(SignalReadFailure::new(
                            reasons::UNMAPPED_PLAYED_MOVIES,
                            "played movies could not be matched to a Scryer title; watch history is incomplete",
                        ));
                    }
                }
                Err(failure) => {
                    outcome.participants_failed += 1;
                    // The participant's existing rows are deliberately left in
                    // place: a failed read is not evidence that they stopped
                    // watching everything.
                    warn!(
                        connection_id = connection.id.as_str(),
                        reason = failure.reason,
                        error = %failure,
                        "media-server signal sync failed for one participant; their stored signals are unchanged"
                    );
                    outcome.record_error(SignalReadFailure::new(
                        failure.reason,
                        format!("participant sync failed: {}", failure.message),
                    ));
                }
            }
        }

        if let Some(failure) = self
            .sync_connection_audience(connection, reader.as_ref(), library, started_at)
            .await
        {
            outcome.record_error(failure);
        }

        outcome
    }

    /// Read every account's plays on the connection and replace its audience
    /// rows with them.
    ///
    /// Only the anonymous "anyone watched it" facts read these rows, so a
    /// failed read is not the connection's failure: it is logged, and the
    /// connection's read marker is removed so those two facts go unknown
    /// until a read succeeds. The rows already stored are kept, exactly as a
    /// failed participant read keeps that participant's rows.
    ///
    /// A marker that cannot be removed would leave the failed read looking
    /// like the last good one, so that failure is returned for the
    /// connection's state: the sweep is then incomplete, and every watch
    /// fact holds until a sweep succeeds.
    async fn sync_connection_audience(
        &self,
        connection: &MediaServerConnection,
        reader: &dyn MediaServerSignalReader,
        library: &ScryerLibraryPaths,
        started_at: chrono::DateTime<Utc>,
    ) -> Option<SignalReadFailure> {
        let result = self
            .read_connection_audience(connection, reader, library, started_at)
            .await;
        let Err(failure) = result else {
            return None;
        };
        warn!(
            connection_id = connection.id.as_str(),
            reason = failure.reason,
            error = %failure,
            "media-server audience read failed; the anonymous watch facts are unknown until one succeeds"
        );
        if let Err(error) = self
            .services
            .integrations
            .media_server_signals
            .clear_audience_read(&connection.id)
            .await
        {
            warn!(
                connection_id = connection.id.as_str(),
                error = %error,
                "could not clear the audience read marker; the sweep is recorded as incomplete"
            );
            return Some(store_failure(
                "could not clear the audience read marker after a failed audience read",
                error,
            ));
        }
        None
    }

    async fn read_connection_audience(
        &self,
        connection: &MediaServerConnection,
        reader: &dyn MediaServerSignalReader,
        library: &ScryerLibraryPaths,
        started_at: chrono::DateTime<Utc>,
    ) -> Result<u64, SignalReadFailure> {
        let (accounts, items): (Vec<String>, Vec<ProviderPlayedItem>) = reader
            .fetch_audience_plays()
            .await?
            .into_iter()
            .filter(|play| play.item.played)
            .map(|play| (play.external_account_id, play.item))
            .unzip();
        let (subjects, titles) = self
            .map_played_items(&items)
            .await
            .map_err(|error| store_failure("could not map audience plays", error))?;

        let mut plays = Vec::new();
        let mut incomplete = false;
        for ((account, item), subject) in accounts.into_iter().zip(&items).zip(subjects) {
            if !subject.is_mapped() {
                // As for a participant: a played movie nothing proves foreign
                // could be a Scryer title the server never identified, and
                // "nobody watched it" would then be wrong about it.
                incomplete |= item.kind == MediaServerSignalKind::Movie
                    && !is_foreign_item(item, &titles, library);
                continue;
            }
            plays.push(AudiencePlay {
                connection_id: connection.id.clone(),
                external_account_id: account,
                provider_item_id: item.provider_item_id.clone(),
                kind: item.kind,
                scryer_title_id: subject.title_id,
                scryer_episode_id: subject.episode_id,
                play_count: item.play_count,
                last_played_at: item.last_played_at,
            });
        }
        if incomplete {
            return Err(SignalReadFailure::new(
                reasons::UNMAPPED_PLAYED_MOVIES,
                "played movies could not be matched to a Scryer title; the audience is incomplete",
            ));
        }
        self.services
            .integrations
            .media_server_signals
            .replace_connection_audience(&connection.id, &plays, started_at)
            .await
            .map_err(|error| store_failure("could not store audience plays", error))
    }

    /// The participant set: verified linked accounts on this connection
    /// (RFC 137, "Participant sets and multi-user identity").
    async fn signal_participants(
        &self,
        provider: ExternalAccountProvider,
        connection_id: &str,
    ) -> AppResult<Vec<UserExternalAccount>> {
        self.services
            .identity
            .external_accounts
            .list_verified_by_connection(provider, connection_id)
            .await
    }

    /// Read, map, and replace one participant's signals.
    ///
    /// A played movie that maps to no Scryer title makes the read incomplete
    /// only when nothing proves it is foreign: one with no usable external id
    /// could be a Scryer title the server never identified, and one whose file
    /// lies in a Scryer library could be Scryer's film identified as another.
    /// One whose ids no Scryer title claims, with no file inside a library, is
    /// simply something Scryer does not manage (a home video, a film outside
    /// every library). See [`is_foreign_item`].
    async fn sync_one_participant(
        &self,
        connection: &MediaServerConnection,
        reader: &dyn MediaServerSignalReader,
        library: &ScryerLibraryPaths,
        external_user_id: &str,
        participant: &UserExternalAccount,
        started_at: chrono::DateTime<Utc>,
    ) -> Result<ParticipantOutcome, SignalReadFailure> {
        let scryer_user_id = participant.user_id.as_str();
        let items = reader.fetch_played_items(external_user_id).await?;

        let observed_at = Utc::now();
        let (subjects, titles) = self
            .map_played_items(&items)
            .await
            .map_err(|error| store_failure("could not map played items", error))?;
        let mut unmapped = 0_u64;
        let mut incomplete = false;

        let signals = items
            .iter()
            .zip(subjects)
            .map(|(item, subject)| {
                if !subject.is_mapped() {
                    unmapped += 1;
                    incomplete |= item.played
                        && item.kind == MediaServerSignalKind::Movie
                        && !is_foreign_item(item, &titles, library);
                }
                NewUserMediaSignal {
                    provider: connection.provider.clone(),
                    scryer_user_id: Some(scryer_user_id.to_string()),
                    provider_item_id: item.provider_item_id.clone(),
                    kind: item.kind,
                    scryer_title_id: subject.title_id,
                    scryer_episode_id: subject.episode_id,
                    played: item.played,
                    play_count: item.play_count,
                    last_played_at: item.last_played_at,
                    is_favorite: item.is_favorite,
                    observed_at,
                }
            })
            .collect::<Vec<_>>();

        let written = self
            .services
            .integrations
            .media_server_signals
            .replace_participant_signals(
                &connection.id,
                external_user_id,
                &signals,
                // The link this read went through joins the roster in the
                // same write as its signals: the watch gate observes exactly
                // the links whose plays are stored.
                Some(&SignalRosterLink {
                    link_id: participant.id.clone(),
                    scryer_user_id: scryer_user_id.to_string(),
                    link_verified_at: participant.verified_at,
                    sweep_started_at: started_at,
                }),
            )
            .await
            .map_err(|error| store_failure("could not store signals", error))?;

        Ok(ParticipantOutcome {
            written,
            unmapped,
            incomplete,
        })
    }

    /// Map a whole participant's items in two batched reads: one external-id
    /// lookup for every item, then one episode read per distinct series that
    /// actually resolved.
    ///
    /// The returned vector is positionally aligned with `items`; the title
    /// index it was resolved against comes back beside it.
    async fn map_played_items(
        &self,
        items: &[ProviderPlayedItem],
    ) -> AppResult<(Vec<MappedSignalSubject>, TitleExternalIdIndex)> {
        if items.is_empty() {
            return Ok((Vec::new(), TitleExternalIdIndex::new()));
        }

        let titles = self.signal_title_index(items).await?;

        // Only series that resolved need their episodes loaded, and each one is
        // loaded once no matter how many of its episodes were watched.
        let mut series_ids = items
            .iter()
            .filter(|item| item.kind == MediaServerSignalKind::Episode)
            .filter_map(|item| crate::media_server_signals::mapping::resolve_title(item, &titles))
            .collect::<Vec<_>>();
        series_ids.sort();
        series_ids.dedup();

        let episodes = self.signal_episode_index(&series_ids).await?;

        let subjects = items
            .iter()
            .map(|item| resolve_subject(item, &titles, &episodes))
            .collect();
        Ok((subjects, titles))
    }

    /// One batched `title_external_ids` join covering every id every item
    /// carries.
    async fn signal_title_index(
        &self,
        items: &[ProviderPlayedItem],
    ) -> AppResult<TitleExternalIdIndex> {
        let mut seen = HashSet::new();
        let mut lookups: Vec<TitleExternalIdLookup> = Vec::new();
        for item in items {
            let ids = match item.kind {
                MediaServerSignalKind::Movie => &item.external_ids,
                MediaServerSignalKind::Episode => &item.series_external_ids,
            };
            for source in SIGNAL_EXTERNAL_ID_SOURCES {
                let Some(external_id) = ids.get(source).map(|value| value.trim()) else {
                    continue;
                };
                if external_id.is_empty() {
                    continue;
                }
                let key = (source.to_string(), external_id.to_string());
                if !seen.insert(key.clone()) {
                    continue;
                }
                lookups.push(TitleExternalIdLookup {
                    lookup_index: lookups.len(),
                    source: key.0,
                    external_id: key.1,
                });
            }
        }

        if lookups.is_empty() {
            return Ok(TitleExternalIdIndex::new());
        }

        let matches = self
            .services
            .catalog
            .titles
            .list_by_external_id_lookups(&lookups)
            .await?;

        let mut index = TitleExternalIdIndex::new();
        for matched in matches {
            let Some(lookup) = lookups.get(matched.lookup_index) else {
                continue;
            };
            index
                .entry((lookup.source.clone(), lookup.external_id.clone()))
                .or_default()
                .push((matched.title.id.clone(), matched.title.facet.clone()));
        }
        Ok(index)
    }

    /// Season/episode coordinates for the resolved series.
    ///
    /// Scryer stores both numbers as text; anything that does not parse as an
    /// integer is left out of the index rather than compared as a string, so a
    /// label like `"Special"` can never collide with episode 0.
    async fn signal_episode_index(&self, title_ids: &[String]) -> AppResult<EpisodeNumberIndex> {
        let mut index = EpisodeNumberIndex::new();
        for title_id in title_ids {
            let episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_title(title_id)
                .await?;
            for episode in episodes {
                let Some(season) = episode
                    .season_number
                    .as_deref()
                    .and_then(|value| value.trim().parse::<i64>().ok())
                else {
                    continue;
                };
                let Some(number) = episode
                    .episode_number
                    .as_deref()
                    .and_then(|value| value.trim().parse::<i64>().ok())
                else {
                    continue;
                };
                index
                    .entry((title_id.clone(), season, number))
                    .or_default()
                    .push(episode.id.clone());
            }
        }
        Ok(index)
    }

    /// Write the per-connection state row, logging rather than failing if the
    /// store refuses it. The sweep's real work is already done at this point;
    /// losing the bookkeeping row must not lose the signals with it.
    async fn record_signal_sync_state(&self, state: MediaServerSignalSyncState) {
        let connection_id = state.connection_id.clone();
        if let Err(error) = self
            .services
            .integrations
            .media_server_signals
            .upsert_signal_sync_state(&state)
            .await
        {
            warn!(
                connection_id = connection_id.as_str(),
                error = %error,
                "could not record media-server signal sync state"
            );
        }
    }
}

/// Fresh identifier for a signal row. Kept here so the store and any in-memory
/// double agree on where ids come from.
pub fn new_signal_id() -> String {
    Id::new().0
}

/// Human-readable one-line summary of a sweep, for the job run record.
pub fn signal_sync_summary(report: &MediaServerSignalSyncReport) -> String {
    format!(
        "Synced {} of {} media-server connection(s): {} participant(s), {} signal(s) stored ({} unmapped)",
        report.connections_synced,
        report.connections_considered,
        report.participants_synced,
        report.signals_written,
        report.signals_unmapped,
    )
}

/// Emitted once per sweep at info level so an operator can see the job ran even
/// when it found nothing.
pub fn log_signal_sync_report(report: &MediaServerSignalSyncReport) {
    info!(
        connections_considered = report.connections_considered,
        connections_synced = report.connections_synced,
        connections_failed = report.connections_failed,
        participants_synced = report.participants_synced,
        participants_failed = report.participants_failed,
        signals_written = report.signals_written,
        signals_unmapped = report.signals_unmapped,
        "media-server signal sync finished"
    );
}

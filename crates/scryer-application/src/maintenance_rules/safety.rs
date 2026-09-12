//! Safety preconditions for maintenance actions (RFC 137 §9.10, §17 D3).
//!
//! Two read-only observations the action executor must consult before a due
//! maintenance action touches media:
//!
//! 1. **Live playback.** RFC 9.10: "Any current playback session for the
//!    subject on a supported Plex/Jellyfin/Emby source holds every due
//!    action... An unavailable required session source is unknown and fails
//!    closed for destructive work."
//! 2. **Active acquisition.** A title with a grab or import in flight must not
//!    be deleted, unmonitored, or otherwise disturbed underneath the pipeline
//!    that is currently writing to it.
//!
//! # The playback hold is global in this MVP
//!
//! The RFC scopes the hold to *the subject*: only a session playing the title
//! under evaluation holds that title's action. Mapping a live session back to a
//! Scryer title needs per-provider session→item→title resolution, which this
//! wave does not build. Until it exists the hold is **global**: if any playback
//! session is active on any enabled media-server connection, every due
//! maintenance action holds.
//!
//! That is deliberately stricter than the RFC and never less strict — a global
//! hold is a superset of the per-subject hold, so no action the RFC would hold
//! can escape it. The cost is over-holding (someone watching anything defers
//! all maintenance for that cycle), which is the acceptable direction for
//! destructive work. Per-subject session mapping replaces this later.
//!
//! # Fail closed
//!
//! An enabled connection that cannot be queried is *unknown*, not idle. Unknown
//! holds destructive work. The one case that is genuinely `Clear` rather than
//! unknown is having no enabled connections at all: nothing can be playing on
//! servers Scryer does not know about.

use crate::contracts::AcquisitionScopeStatesQuery;
use crate::ports::{PlaybackActivitySnapshot, PlaybackProbeStatus};
use crate::types::AcquisitionScopeStatus;
use crate::{AppResult, AppUseCase};
use scryer_domain::ImportRecord;

/// Client parameter Scryer stamps on its own grabs so a completed download can
/// be attributed back to the title it was grabbed for. Mirrors the private
/// `SCRYER_TITLE_ID_PARAM` in `import::workflow::completed`; kept as a literal
/// here rather than widening that module's visibility for a read-only probe.
const SCRYER_TITLE_ID_PARAM: &str = "*scryer_title_id";

/// Whether live playback holds the due maintenance actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaintenancePlaybackHold {
    /// Every enabled connection answered, and none of them is streaming. (Also
    /// the answer when there are no enabled connections to ask.)
    Clear,
    /// At least one enabled connection is streaming. `active_sessions` is the
    /// total across every connection that answered.
    Hold { active_sessions: u32 },
    /// Nothing is known to be playing, but at least one enabled connection
    /// could not be asked, so playback could not be ruled out. Holds
    /// destructive work.
    Unknown { reason: String },
}

impl MaintenancePlaybackHold {
    /// Whether a destructive action must be held. `Unknown` holds — that is the
    /// fail-closed rule from RFC §17 D3.
    pub fn holds_destructive_work(&self) -> bool {
        !matches!(self, Self::Clear)
    }
}

/// Whether a title currently has acquisition work in flight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaintenanceActivityCheck {
    /// A grab or an import for this title is in flight.
    Active,
    /// Every signal answered, and none of them shows work in flight.
    Clear,
    /// A signal could not be read, or in-flight work could not be attributed to
    /// a title. Holds destructive work.
    Unknown { reason: String },
}

impl MaintenanceActivityCheck {
    /// Whether a destructive action must be held. Both `Active` and `Unknown`
    /// hold; only `Clear` releases.
    pub fn holds_destructive_work(&self) -> bool {
        !matches!(self, Self::Clear)
    }
}

/// Folds a playback snapshot into the global hold decision.
///
/// Precedence, highest first:
///
/// 1. any connection streaming → [`MaintenancePlaybackHold::Hold`] with the
///    summed session count (a live session is a fact; it does not matter that
///    another connection was unreachable);
/// 2. any connection unreachable → [`MaintenancePlaybackHold::Unknown`];
/// 3. every connection idle, or no connections at all →
///    [`MaintenancePlaybackHold::Clear`].
///
/// `ActiveSessions(0)` is treated as idle: a server that answered "zero
/// sessions" said the same thing as one that answered "idle".
pub fn fold_playback_hold(snapshot: &PlaybackActivitySnapshot) -> MaintenancePlaybackHold {
    let active_sessions: u32 = snapshot
        .connections
        .iter()
        .filter_map(|connection| match &connection.status {
            PlaybackProbeStatus::ActiveSessions(count) => Some(*count),
            PlaybackProbeStatus::Idle | PlaybackProbeStatus::Unreachable(_) => None,
        })
        .sum();
    if active_sessions > 0 {
        return MaintenancePlaybackHold::Hold { active_sessions };
    }

    let unreachable = snapshot
        .connections
        .iter()
        .filter_map(|connection| match &connection.status {
            PlaybackProbeStatus::Unreachable(reason) => {
                Some(format!("{} ({reason})", connection.connection_id))
            }
            PlaybackProbeStatus::ActiveSessions(_) | PlaybackProbeStatus::Idle => None,
        })
        .collect::<Vec<_>>();
    if unreachable.is_empty() {
        return MaintenancePlaybackHold::Clear;
    }

    MaintenancePlaybackHold::Unknown {
        reason: format!(
            "playback could not be ruled out on {}: {}",
            unreachable.len(),
            unreachable.join("; ")
        ),
    }
}

/// The title a queued import will land in, when the queued row says so.
///
/// `imports` carries no title column, and its payload is one of three shapes,
/// so attribution is a payload read rather than a query:
///
/// * a manual import stores the operator's chosen `title_id`;
/// * a completed-download import stores `target_title_id` (legacy alias
///   `manual_title_id`) when the release evidence carries no Scryer identity;
/// * a download Scryer grabbed carries `*scryer_title_id` in the completed
///   download's client parameters, at `completed.parameters` in the current
///   payload shape and at `parameters` in the legacy one.
///
/// `None` means the row exists but names no title — a foreign import whose
/// title is resolved from release evidence at execution time. The caller must
/// treat that as unknown, never as "not this title".
fn queued_import_title_id(record: &ImportRecord) -> Option<String> {
    let payload = serde_json::from_str::<serde_json::Value>(&record.payload_json).ok()?;

    let direct = ["title_id", "target_title_id", "manual_title_id"]
        .into_iter()
        .filter_map(|key| payload.get(key))
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .find(|value| !value.is_empty());
    if let Some(title_id) = direct {
        return Some(title_id.to_string());
    }

    [
        payload.pointer("/completed/parameters"),
        payload.get("parameters"),
    ]
    .into_iter()
    .flatten()
    .filter_map(serde_json::Value::as_array)
    .flatten()
    .filter_map(serde_json::Value::as_array)
    .filter_map(|pair| {
        // Client parameters serialize as two-element `[key, value]` arrays.
        let mut entry = pair.iter();
        let key = entry.next()?.as_str()?;
        if key != SCRYER_TITLE_ID_PARAM {
            return None;
        }
        entry.next()?.as_str()
    })
    .map(str::trim)
    .find(|value| !value.is_empty())
    .map(str::to_string)
}

impl AppUseCase {
    /// Observes live playback and folds it into the global maintenance hold.
    ///
    /// A failure to read the connection list itself is reported as
    /// [`MaintenancePlaybackHold::Unknown`] rather than an `Err`: the caller's
    /// question is "may I touch media?", and the fail-closed answer to a broken
    /// probe is the same as to an unreachable server.
    pub async fn maintenance_playback_hold(&self) -> AppResult<MaintenancePlaybackHold> {
        match self
            .services
            .integrations
            .media_server_playback_probe
            .active_playback()
            .await
        {
            Ok(snapshot) => Ok(fold_playback_hold(&snapshot)),
            Err(error) => Ok(MaintenancePlaybackHold::Unknown {
                reason: format!("playback probe unavailable: {error}"),
            }),
        }
    }

    /// Whether `title_id` currently has a download, import, or acquisition
    /// workflow in flight.
    ///
    /// # Signals, and why these
    ///
    /// * **Acquisition scope states in `grabbed`**
    ///   ([`crate::ports::AcquisitionScopeStateRepository::count_acquisition_scope_states`]
    ///   filtered by `title_id`). This is the authoritative per-title in-flight
    ///   marker: a scope enters `grabbed` when a release is grabbed for it and
    ///   leaves only when the import lands (or an operator completes/reopens
    ///   it), so one indexed count covers the whole grab → download → import
    ///   lifetime of every scope under the title, movie and episode alike.
    /// * **The queued import set** ([`crate::ports::ImportRepository::list_pending_imports`],
    ///   i.e. rows in `queued`/`pending`/`running`/`processing`). This catches
    ///   the import of a download Scryer never grabbed, which has no scope
    ///   state to observe. It is a bounded read of the live work queue, not a
    ///   history scan.
    ///
    /// Ports deliberately *not* used: `DownloadRegistryRepository` and
    /// `DownloadSubmissionRepository` expose no per-title lookup at all
    /// (`DownloadRecord` has no title column, and the submission lookups key on
    /// an info hash or a normalized release name), and
    /// `WorkflowOperationRepository` exposes only `create_workflow_operation` —
    /// there is nothing to read. Answering from those would mean an unbounded
    /// table scan for a check that runs per candidate.
    ///
    /// # Fail closed
    ///
    /// Any repository read that fails is [`MaintenanceActivityCheck::Unknown`],
    /// never `Clear`. A queued import row that names no title is also
    /// `Unknown`, not "not this title": `imports` has no title column, so an
    /// unattributed row could be for this title. That over-holds while a
    /// foreign import is in flight — the safe direction, and the reason a
    /// per-title import index is the follow-up this check wants.
    pub async fn title_has_active_acquisition(
        &self,
        title_id: &str,
    ) -> AppResult<MaintenanceActivityCheck> {
        let title_id = title_id.trim();
        if title_id.is_empty() {
            return Ok(MaintenanceActivityCheck::Unknown {
                reason: "activity check requires a title id".to_string(),
            });
        }

        let grabbed_scopes = self
            .services
            .workflow
            .acquisition_scope_states
            .count_acquisition_scope_states(AcquisitionScopeStatesQuery {
                statuses: vec![AcquisitionScopeStatus::Grabbed.as_str().to_string()],
                title_id: Some(title_id.to_string()),
                ..AcquisitionScopeStatesQuery::default()
            })
            .await;

        // Only read the import queue when the grab signal left the answer
        // open: a grab in flight already decides `Active`, and an unreadable
        // scope table already decides `Unknown`. The decision itself stays in
        // the fold — this match only skips work the fold would ignore.
        let queued_import_titles = match &grabbed_scopes {
            Ok(count) if *count == 0 => self
                .services
                .workflow
                .imports
                .list_pending_imports()
                .await
                .map(|records| records.iter().map(queued_import_title_id).collect()),
            Ok(_) | Err(_) => Ok(Vec::new()),
        };

        Ok(fold_acquisition_activity(
            title_id,
            grabbed_scopes,
            queued_import_titles,
        ))
    }
}

/// Folds the two acquisition signals into the activity answer.
///
/// Precedence, highest first:
///
/// 1. a grabbed scope for this title, or a queued import naming it → `Active`;
/// 2. a signal that could not be read, or a queued import naming no title at
///    all → `Unknown`;
/// 3. both signals answered and neither shows work → `Clear`.
///
/// `queued_import_titles` is one entry per queued import row: `Some(title_id)`
/// when the row names the title it will land in, `None` when it names none.
fn fold_acquisition_activity(
    title_id: &str,
    grabbed_scopes: AppResult<i64>,
    queued_import_titles: AppResult<Vec<Option<String>>>,
) -> MaintenanceActivityCheck {
    match grabbed_scopes {
        Ok(count) if count > 0 => return MaintenanceActivityCheck::Active,
        Ok(_) => {}
        Err(error) => {
            return MaintenanceActivityCheck::Unknown {
                reason: format!("acquisition scope states unreadable: {error}"),
            };
        }
    }

    let queued = match queued_import_titles {
        Ok(queued) => queued,
        Err(error) => {
            return MaintenanceActivityCheck::Unknown {
                reason: format!("queued imports unreadable: {error}"),
            };
        }
    };

    let mut unattributed = 0usize;
    for queued_title_id in &queued {
        match queued_title_id.as_deref() {
            Some(queued_title_id) if queued_title_id == title_id => {
                return MaintenanceActivityCheck::Active;
            }
            Some(_) => {}
            None => unattributed += 1,
        }
    }
    if unattributed > 0 {
        return MaintenanceActivityCheck::Unknown {
            reason: format!("{unattributed} queued import(s) name no title"),
        };
    }

    MaintenanceActivityCheck::Clear
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::ConnectionPlaybackActivity;
    use chrono::Utc;
    use scryer_domain::{ImportStatus, ImportType, MediaServerProvider};

    fn connection(id: &str, status: PlaybackProbeStatus) -> ConnectionPlaybackActivity {
        ConnectionPlaybackActivity {
            connection_id: id.to_string(),
            provider: MediaServerProvider::Plex,
            status,
        }
    }

    fn snapshot(connections: Vec<ConnectionPlaybackActivity>) -> PlaybackActivitySnapshot {
        PlaybackActivitySnapshot {
            connections,
            observed_at: Utc::now(),
        }
    }

    fn import_record(payload_json: &str) -> ImportRecord {
        ImportRecord {
            id: "import-1".to_string(),
            source_client_id: None,
            source_system: "test".to_string(),
            source_ref: "ref".to_string(),
            import_type: ImportType::MovieDownload,
            status: ImportStatus::Pending,
            payload_json: payload_json.to_string(),
            result_json: None,
            download_id: None,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            started_at: None,
            finished_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    // ── The fold: every status combination ──────────────────────────────────

    #[test]
    fn no_configured_connections_is_clear() {
        // Nothing can be playing on servers Scryer does not know about.
        assert_eq!(
            fold_playback_hold(&snapshot(Vec::new())),
            MaintenancePlaybackHold::Clear
        );
    }

    #[test]
    fn every_connection_idle_is_clear() {
        let folded = fold_playback_hold(&snapshot(vec![
            connection("plex", PlaybackProbeStatus::Idle),
            connection("jellyfin", PlaybackProbeStatus::Idle),
        ]));
        assert_eq!(folded, MaintenancePlaybackHold::Clear);
        assert!(!folded.holds_destructive_work());
    }

    #[test]
    fn a_zero_session_answer_is_idle_not_a_hold() {
        assert_eq!(
            fold_playback_hold(&snapshot(vec![connection(
                "plex",
                PlaybackProbeStatus::ActiveSessions(0)
            )])),
            MaintenancePlaybackHold::Clear
        );
    }

    #[test]
    fn a_single_active_session_holds() {
        let folded = fold_playback_hold(&snapshot(vec![connection(
            "plex",
            PlaybackProbeStatus::ActiveSessions(1),
        )]));
        assert_eq!(folded, MaintenancePlaybackHold::Hold { active_sessions: 1 });
        assert!(folded.holds_destructive_work());
    }

    #[test]
    fn active_sessions_sum_across_connections() {
        assert_eq!(
            fold_playback_hold(&snapshot(vec![
                connection("plex", PlaybackProbeStatus::ActiveSessions(2)),
                connection("jellyfin", PlaybackProbeStatus::ActiveSessions(3)),
                connection("emby", PlaybackProbeStatus::Idle),
            ])),
            MaintenancePlaybackHold::Hold { active_sessions: 5 }
        );
    }

    #[test]
    fn one_unreachable_connection_is_unknown() {
        let folded = fold_playback_hold(&snapshot(vec![connection(
            "plex",
            PlaybackProbeStatus::Unreachable("connect timeout".to_string()),
        )]));
        let MaintenancePlaybackHold::Unknown { reason } = &folded else {
            panic!("expected unknown, got {folded:?}");
        };
        assert!(reason.contains("plex"), "{reason}");
        assert!(reason.contains("connect timeout"), "{reason}");
        assert!(folded.holds_destructive_work());
    }

    #[test]
    fn unreachable_while_others_are_idle_is_unknown() {
        let folded = fold_playback_hold(&snapshot(vec![
            connection("plex", PlaybackProbeStatus::Idle),
            connection(
                "jellyfin",
                PlaybackProbeStatus::Unreachable("status 401".to_string()),
            ),
        ]));
        assert!(matches!(folded, MaintenancePlaybackHold::Unknown { .. }));
    }

    #[test]
    fn every_connection_unreachable_names_all_of_them() {
        let folded = fold_playback_hold(&snapshot(vec![
            connection("plex", PlaybackProbeStatus::Unreachable("dns".to_string())),
            connection(
                "jellyfin",
                PlaybackProbeStatus::Unreachable("status 500".to_string()),
            ),
        ]));
        let MaintenancePlaybackHold::Unknown { reason } = folded else {
            panic!("expected unknown");
        };
        assert!(reason.contains("plex"), "{reason}");
        assert!(reason.contains("jellyfin"), "{reason}");
    }

    #[test]
    fn an_active_session_beats_an_unreachable_peer() {
        // A live session is a fact; it does not become less certain because a
        // different server was down. Hold is already the strictest answer.
        assert_eq!(
            fold_playback_hold(&snapshot(vec![
                connection("plex", PlaybackProbeStatus::ActiveSessions(1)),
                connection(
                    "jellyfin",
                    PlaybackProbeStatus::Unreachable("timeout".to_string())
                ),
            ])),
            MaintenancePlaybackHold::Hold { active_sessions: 1 }
        );
    }

    #[test]
    fn every_status_triple_folds_as_specified() {
        // Exhaustive over the three statuses across three connections.
        let statuses = [
            PlaybackProbeStatus::Idle,
            PlaybackProbeStatus::ActiveSessions(1),
            PlaybackProbeStatus::Unreachable("down".to_string()),
        ];
        for first in &statuses {
            for second in &statuses {
                for third in &statuses {
                    let connections = vec![
                        connection("a", first.clone()),
                        connection("b", second.clone()),
                        connection("c", third.clone()),
                    ];
                    let active = connections
                        .iter()
                        .filter(|connection| {
                            matches!(connection.status, PlaybackProbeStatus::ActiveSessions(n) if n > 0)
                        })
                        .count() as u32;
                    let any_unreachable = connections.iter().any(|connection| {
                        matches!(connection.status, PlaybackProbeStatus::Unreachable(_))
                    });
                    let expected = if active > 0 {
                        MaintenancePlaybackHold::Hold {
                            active_sessions: active,
                        }
                    } else if any_unreachable {
                        match fold_playback_hold(&snapshot(connections.clone())) {
                            unknown @ MaintenancePlaybackHold::Unknown { .. } => unknown,
                            other => panic!("expected unknown, got {other:?}"),
                        }
                    } else {
                        MaintenancePlaybackHold::Clear
                    };
                    assert_eq!(
                        fold_playback_hold(&snapshot(connections)),
                        expected,
                        "{first:?} / {second:?} / {third:?}"
                    );
                }
            }
        }
    }

    // ── Queued-import attribution ───────────────────────────────────────────

    #[test]
    fn manual_import_payload_names_its_title() {
        let record = import_record(r#"{"title_id":"title-1","client_type":"sabnzbd"}"#);
        assert_eq!(queued_import_title_id(&record).as_deref(), Some("title-1"));
    }

    #[test]
    fn completed_import_payload_names_its_target_title() {
        let record =
            import_record(r#"{"completed":{"parameters":[]},"target_title_id":"title-2"}"#);
        assert_eq!(queued_import_title_id(&record).as_deref(), Some("title-2"));
    }

    #[test]
    fn legacy_completed_import_payload_alias_is_read() {
        let record = import_record(r#"{"manual_title_id":"title-3"}"#);
        assert_eq!(queued_import_title_id(&record).as_deref(), Some("title-3"));
    }

    #[test]
    fn a_scryer_grab_is_attributed_through_its_client_parameter() {
        let record = import_record(
            r#"{"completed":{"parameters":[["*scryer_category","movies"],["*scryer_title_id","title-4"]]}}"#,
        );
        assert_eq!(queued_import_title_id(&record).as_deref(), Some("title-4"));
    }

    #[test]
    fn a_legacy_top_level_completed_payload_is_attributed_too() {
        let record = import_record(r#"{"parameters":[["*scryer_title_id","title-5"]]}"#);
        assert_eq!(queued_import_title_id(&record).as_deref(), Some("title-5"));
    }

    #[test]
    fn a_foreign_import_names_no_title() {
        let record = import_record(r#"{"completed":{"parameters":[["*scryer_category","tv"]]}}"#);
        assert_eq!(queued_import_title_id(&record), None);
    }

    #[test]
    fn a_blank_title_id_is_not_an_attribution() {
        let record = import_record(r#"{"title_id":"   "}"#);
        assert_eq!(queued_import_title_id(&record), None);
    }

    #[test]
    fn an_unparseable_payload_names_no_title() {
        let record = import_record("not json");
        assert_eq!(queued_import_title_id(&record), None);
    }

    // ── The acquisition fold ────────────────────────────────────────────────

    #[test]
    fn a_grabbed_scope_is_active() {
        assert_eq!(
            fold_acquisition_activity("title-1", Ok(1), Ok(Vec::new())),
            MaintenanceActivityCheck::Active
        );
    }

    #[test]
    fn no_grab_and_no_queued_imports_is_clear() {
        assert_eq!(
            fold_acquisition_activity("title-1", Ok(0), Ok(Vec::new())),
            MaintenanceActivityCheck::Clear
        );
    }

    #[test]
    fn a_queued_import_for_this_title_is_active() {
        assert_eq!(
            fold_acquisition_activity(
                "title-1",
                Ok(0),
                Ok(vec![
                    Some("title-2".to_string()),
                    Some("title-1".to_string())
                ])
            ),
            MaintenanceActivityCheck::Active
        );
    }

    #[test]
    fn queued_imports_for_other_titles_are_clear() {
        assert_eq!(
            fold_acquisition_activity(
                "title-1",
                Ok(0),
                Ok(vec![
                    Some("title-2".to_string()),
                    Some("title-3".to_string())
                ])
            ),
            MaintenanceActivityCheck::Clear
        );
    }

    #[test]
    fn an_unattributed_queued_import_is_unknown_not_clear() {
        let folded = fold_acquisition_activity(
            "title-1",
            Ok(0),
            Ok(vec![Some("title-2".to_string()), None]),
        );
        let MaintenanceActivityCheck::Unknown { reason } = &folded else {
            panic!("expected unknown, got {folded:?}");
        };
        assert!(reason.contains('1'), "{reason}");
    }

    #[test]
    fn an_attributed_match_beats_an_unattributed_peer() {
        assert_eq!(
            fold_acquisition_activity(
                "title-1",
                Ok(0),
                Ok(vec![None, Some("title-1".to_string())])
            ),
            MaintenanceActivityCheck::Active
        );
    }

    #[test]
    fn an_unreadable_scope_table_is_unknown() {
        let folded = fold_acquisition_activity(
            "title-1",
            Err(crate::AppError::Repository("datastore offline".into())),
            Ok(Vec::new()),
        );
        let MaintenanceActivityCheck::Unknown { reason } = &folded else {
            panic!("expected unknown, got {folded:?}");
        };
        assert!(reason.contains("acquisition scope states"), "{reason}");
    }

    #[test]
    fn an_unreadable_import_queue_is_unknown() {
        let folded = fold_acquisition_activity(
            "title-1",
            Ok(0),
            Err(crate::AppError::Repository("datastore offline".into())),
        );
        let MaintenanceActivityCheck::Unknown { reason } = &folded else {
            panic!("expected unknown, got {folded:?}");
        };
        assert!(reason.contains("queued imports"), "{reason}");
    }

    #[test]
    fn a_grab_beats_an_unreadable_import_queue() {
        // The grab is already conclusive; the queue read cannot make it less so.
        assert_eq!(
            fold_acquisition_activity(
                "title-1",
                Ok(3),
                Err(crate::AppError::Repository("datastore offline".into()))
            ),
            MaintenanceActivityCheck::Active
        );
    }

    #[test]
    fn activity_check_holds_unless_clear() {
        assert!(MaintenanceActivityCheck::Active.holds_destructive_work());
        assert!(
            MaintenanceActivityCheck::Unknown {
                reason: "unreadable".to_string()
            }
            .holds_destructive_work()
        );
        assert!(!MaintenanceActivityCheck::Clear.holds_destructive_work());
    }
}

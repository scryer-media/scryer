//! Wired-up coverage for the maintenance safety preconditions (RFC 137 §9.10,
//! WP-G). The pure folds are unit-tested next to them in
//! `maintenance_rules::safety`; these tests prove the use case reads the ports
//! it claims to read.

use super::*;

use crate::maintenance_rules::{MaintenanceActivityCheck, MaintenancePlaybackHold};
use crate::ports::{
    ConnectionPlaybackActivity, MediaServerPlaybackProbe, PlaybackActivitySnapshot,
    PlaybackProbeStatus,
};
use scryer_domain::MediaServerProvider;

/// Playback probe that replays a fixed snapshot, or fails.
struct StubPlaybackProbe {
    statuses: Vec<(String, PlaybackProbeStatus)>,
    fail: bool,
}

impl StubPlaybackProbe {
    fn reporting(statuses: Vec<(&str, PlaybackProbeStatus)>) -> Self {
        Self {
            statuses: statuses
                .into_iter()
                .map(|(id, status)| (id.to_string(), status))
                .collect(),
            fail: false,
        }
    }

    fn failing() -> Self {
        Self {
            statuses: Vec::new(),
            fail: true,
        }
    }
}

#[async_trait]
impl MediaServerPlaybackProbe for StubPlaybackProbe {
    async fn active_playback(&self) -> AppResult<PlaybackActivitySnapshot> {
        if self.fail {
            return Err(AppError::Repository("connection list unreadable".into()));
        }
        Ok(PlaybackActivitySnapshot {
            connections: self
                .statuses
                .iter()
                .map(|(id, status)| ConnectionPlaybackActivity {
                    connection_id: id.clone(),
                    provider: MediaServerProvider::Jellyfin,
                    status: status.clone(),
                })
                .collect(),
            observed_at: Utc::now(),
        })
    }
}

fn grabbed_scope(title_id: &str, status: AcquisitionScopeStatus) -> AcquisitionScopeState {
    AcquisitionScopeState {
        id: Id::new().0,
        title_id: title_id.to_string(),
        title_name: None,
        title_slug: None,
        title_facet: Some("movie".to_string()),
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: None,
        status,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    }
}

fn queued_import(payload_json: &str) -> ImportRecord {
    ImportRecord {
        id: Id::new().0,
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
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    }
}

struct ActivityFixture {
    app: AppUseCase,
    scopes: Arc<TrackingAcquisitionScopeStateRepo>,
    imports: Arc<TrackingImportRepo>,
}

fn activity_app() -> ActivityFixture {
    let (app, _user) = bootstrap();
    let scopes = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let imports = Arc::new(TrackingImportRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_acquisition_scope_states(scopes.clone())
            .with_imports(imports.clone())
    });
    ActivityFixture {
        app,
        scopes,
        imports,
    }
}

// ── Playback hold ───────────────────────────────────────────────────────────

#[tokio::test]
async fn an_assembly_without_media_servers_reports_a_clear_playback_hold() {
    // The default assembly carries the null probe, which observes nothing.
    let (app, _user) = bootstrap();

    assert_eq!(
        app.maintenance_playback_hold().await.expect("hold"),
        MaintenancePlaybackHold::Clear
    );
}

#[tokio::test]
async fn any_active_session_anywhere_holds_every_action() {
    // MVP semantics: the hold is global, not per subject.
    let (app, _user) = bootstrap();
    let app = app.with_test_overrides(|services| {
        services.with_media_server_playback_probe(Arc::new(StubPlaybackProbe::reporting(vec![
            ("jellyfin", PlaybackProbeStatus::ActiveSessions(1)),
            ("plex", PlaybackProbeStatus::Idle),
        ])))
    });

    assert_eq!(
        app.maintenance_playback_hold().await.expect("hold"),
        MaintenancePlaybackHold::Hold { active_sessions: 1 }
    );
}

#[tokio::test]
async fn an_unreachable_connection_makes_the_hold_unknown() {
    let (app, _user) = bootstrap();
    let app = app.with_test_overrides(|services| {
        services.with_media_server_playback_probe(Arc::new(StubPlaybackProbe::reporting(vec![
            ("jellyfin", PlaybackProbeStatus::Idle),
            (
                "plex",
                PlaybackProbeStatus::Unreachable("status 401".to_string()),
            ),
        ])))
    });

    let hold = app.maintenance_playback_hold().await.expect("hold");
    assert!(
        matches!(hold, MaintenancePlaybackHold::Unknown { .. }),
        "{hold:?}"
    );
    assert!(hold.holds_destructive_work());
}

#[tokio::test]
async fn a_broken_probe_is_unknown_rather_than_an_error() {
    let (app, _user) = bootstrap();
    let app = app.with_test_overrides(|services| {
        services.with_media_server_playback_probe(Arc::new(StubPlaybackProbe::failing()))
    });

    let hold = app.maintenance_playback_hold().await.expect("hold");
    assert!(
        matches!(hold, MaintenancePlaybackHold::Unknown { .. }),
        "{hold:?}"
    );
}

// ── Active acquisition ──────────────────────────────────────────────────────

#[tokio::test]
async fn a_title_with_no_signals_is_clear() {
    let ActivityFixture { app, .. } = activity_app();

    assert_eq!(
        app.title_has_active_acquisition("title-1")
            .await
            .expect("activity"),
        MaintenanceActivityCheck::Clear
    );
}

#[tokio::test]
async fn a_grabbed_scope_makes_the_title_active() {
    let ActivityFixture { app, scopes, .. } = activity_app();
    scopes
        .upsert_acquisition_scope_state(&grabbed_scope("title-1", AcquisitionScopeStatus::Grabbed))
        .await
        .expect("seed scope");

    assert_eq!(
        app.title_has_active_acquisition("title-1")
            .await
            .expect("activity"),
        MaintenanceActivityCheck::Active
    );
}

#[tokio::test]
async fn a_settled_scope_does_not_make_the_title_active() {
    let ActivityFixture { app, scopes, .. } = activity_app();
    for status in [
        AcquisitionScopeStatus::Wanted,
        AcquisitionScopeStatus::Paused,
        AcquisitionScopeStatus::Completed,
    ] {
        scopes
            .upsert_acquisition_scope_state(&grabbed_scope("title-1", status))
            .await
            .expect("seed scope");
    }

    assert_eq!(
        app.title_has_active_acquisition("title-1")
            .await
            .expect("activity"),
        MaintenanceActivityCheck::Clear
    );
}

#[tokio::test]
async fn a_grabbed_scope_for_another_title_does_not_leak() {
    let ActivityFixture { app, scopes, .. } = activity_app();
    scopes
        .upsert_acquisition_scope_state(&grabbed_scope("title-2", AcquisitionScopeStatus::Grabbed))
        .await
        .expect("seed scope");

    assert_eq!(
        app.title_has_active_acquisition("title-1")
            .await
            .expect("activity"),
        MaintenanceActivityCheck::Clear
    );
}

#[tokio::test]
async fn a_queued_import_naming_the_title_makes_it_active() {
    let ActivityFixture { app, imports, .. } = activity_app();
    imports
        .records
        .lock()
        .await
        .push(queued_import(r#"{"title_id":"title-1"}"#));

    assert_eq!(
        app.title_has_active_acquisition("title-1")
            .await
            .expect("activity"),
        MaintenanceActivityCheck::Active
    );
}

#[tokio::test]
async fn a_queued_scryer_grab_is_attributed_through_its_client_parameter() {
    let ActivityFixture { app, imports, .. } = activity_app();
    imports.records.lock().await.push(queued_import(
        r#"{"completed":{"parameters":[["*scryer_title_id","title-1"]]}}"#,
    ));

    assert_eq!(
        app.title_has_active_acquisition("title-1")
            .await
            .expect("activity"),
        MaintenanceActivityCheck::Active
    );
}

#[tokio::test]
async fn a_queued_import_naming_no_title_is_unknown() {
    // Fail closed: `imports` has no title column, so an unattributed row could
    // be for this title.
    let ActivityFixture { app, imports, .. } = activity_app();
    imports
        .records
        .lock()
        .await
        .push(queued_import(r#"{"completed":{"parameters":[]}}"#));

    let check = app
        .title_has_active_acquisition("title-1")
        .await
        .expect("activity");
    assert!(
        matches!(check, MaintenanceActivityCheck::Unknown { .. }),
        "{check:?}"
    );
    assert!(check.holds_destructive_work());
}

#[tokio::test]
async fn an_empty_title_id_is_unknown() {
    let ActivityFixture { app, .. } = activity_app();

    let check = app
        .title_has_active_acquisition("  ")
        .await
        .expect("activity");
    assert!(
        matches!(check, MaintenanceActivityCheck::Unknown { .. }),
        "{check:?}"
    );
}

//! `finalizeExternalImport` accepts the apply as a tracked background session.
//!
//! The apply walks every warmed title (and, for Sonarr, every episode of every
//! series) and rewrites the monitored-status snapshot, which for a real catalog
//! runs far past the GraphQL execution timeout. These tests pin the contract the
//! Summary step depends on: the mutation answers immediately, progress advances
//! through the same warmup session/status machinery the warmup uses, a failure
//! is durable and readable, and a failed apply can be re-run.

use super::*;
use async_graphql::Variables;

use scryer_application::external_import::{ArrMovie, ArrRootFolder};
use scryer_application::{
    ExternalImportArrSourceKind, ExternalImportArrSourceWarmupResult,
    ExternalImportMonitorSnapshotChunk, ExternalImportMonitorSnapshotEntryKind,
    ExternalImportMonitorWarmupPhase, ExternalImportMonitorWarmupStatus,
};

const ARR_ROOT: &str = "/arr-data/films";
const SCRYER_ROOT: &str = "/scryer-data/films";
const SOURCE_KEY: &str = "radarr@films-fixture";

const FINALIZE: &str = r#"
mutation Finalize($input: FinalizeExternalImportInput!) {
  finalizeExternalImport(input: $input) {
    monitorWarmupSessionId
    finalizeSessionId
    progress {
      sessionId
      status
      phase
      snapshotBuildTotalKnown
      snapshotBuildProgress { total completed failed }
      errorMessage
    }
  }
}
"#;

const WARMUP_STATUS: &str = r#"
query WarmupStatus($sessionId: ID!) {
  externalImportWarmupStatus(sessionId: $sessionId) {
    sessionId
    status
    phase
    snapshotBuildProgress { total completed failed }
    errorMessage
  }
}
"#;

async fn schema_exec(ctx: &TestContext, query: &str, variables: Value, user: &User) -> Value {
    let request = async_graphql::Request::new(query)
        .variables(Variables::from_json(variables))
        .data(user.clone());
    let response = ctx.schema.execute(request).await;
    serde_json::to_value(&response).expect("serialize gql response")
}

async fn create_admin(ctx: &TestContext, username: &str) -> User {
    ctx.users
        .create(User::new_admin(username))
        .await
        .expect("test admin should create")
}

/// Point the default movie library at the Scryer-host root the mapping uses.
async fn configure_movie_library(ctx: &TestContext) -> String {
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let library = ctx
        .libraries
        .get_by_id(&library_id)
        .await
        .expect("lookup default movie library")
        .expect("default movie library exists");
    ctx.libraries
        .update(
            &library_id,
            library.name,
            library.slug,
            vec![LibraryRootDraft {
                path: SCRYER_ROOT.to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("configure default movie library root");
    library_id
}

fn fixture_movie(id: i64, slug: &str, root_folder_path: &str) -> ArrMovie {
    ArrMovie {
        id,
        root_folder_path: root_folder_path.to_string(),
        path: Some(format!("{root_folder_path}/{slug}")),
        file_path: Some(format!("{root_folder_path}/{slug}/{slug}.mkv")),
        tmdb_id: Some(format!("90{id}")),
        imdb_id: None,
        monitored: true,
        quality_profile_id: None,
        minimum_availability: None,
        original_language: None,
        tags: Vec::new(),
    }
}

/// Seed a completed Radarr warmup session holding `movies` in its snapshot.
async fn seed_completed_radarr_warmup(
    ctx: &TestContext,
    actor: &User,
    movies: &[ArrMovie],
) -> String {
    // Mirrors the real flow: the one-shot stale-chunk sweep runs when a warmup
    // starts, before any snapshot chunk is written.
    ctx.app
        .maintain_external_import_arr_source_sessions(actor)
        .await
        .expect("maintain import sessions");
    let begin = ctx
        .app
        .begin_external_import_monitor_warmup(actor, "arr-source=films-fixture")
        .await
        .expect("begin warmup session");
    let session_id = begin.snapshot.session_id.clone();

    ctx.app
        .set_external_import_arr_source_warmup_result(
            &session_id,
            ExternalImportArrSourceWarmupResult {
                source_key: SOURCE_KEY.to_string(),
                kind: ExternalImportArrSourceKind::Radarr,
                base_url: "http://films-fixture.invalid".to_string(),
                version: Some("5.0.0".to_string()),
                root_folders: vec![ArrRootFolder {
                    id: 1,
                    path: ARR_ROOT.to_string(),
                }],
                title_root_paths: vec![ARR_ROOT.to_string()],
                naming_config: None,
                media_management_config: None,
                metadata_providers: Vec::new(),
                quality_profiles: Vec::new(),
                signal_warnings: Vec::new(),
                download_clients: Vec::new(),
                indexers: Vec::new(),
            },
        )
        .await;

    write_source_snapshot(ctx, actor, &session_id, movies).await;

    let mut snapshot = begin.snapshot.clone();
    snapshot.status = ExternalImportMonitorWarmupStatus::Completed;
    snapshot.phase = ExternalImportMonitorWarmupPhase::Ready;
    snapshot.movies_total_known = true;
    snapshot.movies_progress.total = movies.len() as i32;
    snapshot.movies_progress.completed = movies.len() as i32;
    ctx.app
        .update_external_import_monitor_warmup_progress(&session_id, snapshot)
        .await;

    session_id
}

async fn write_source_snapshot(
    ctx: &TestContext,
    actor: &User,
    session_id: &str,
    movies: &[ArrMovie],
) {
    let payload_ndjson = movies
        .iter()
        .map(|movie| serde_json::to_string(movie).expect("serialize fixture movie"))
        .collect::<Vec<_>>()
        .join("\n");
    ctx.app
        .append_external_import_monitor_snapshot_chunk(
            actor,
            ExternalImportMonitorSnapshotChunk {
                session_id: session_id.to_string(),
                facet: MediaFacet::Movie,
                entry_kind: ExternalImportMonitorSnapshotEntryKind::Movie,
                chunk_index: 0,
                payload_ndjson,
                created_at: Utc::now().to_rfc3339(),
            },
        )
        .await
        .expect("append source snapshot chunk");
}

fn finalize_input(session_id: &str, library_id: &str) -> Value {
    json!({
        "input": {
            "sourceWarmupSessionIds": [session_id],
            "mappings": [{
                "sourceWarmupSessionId": session_id,
                "sourceKey": SOURCE_KEY,
                "kind": "RADARR",
                "arrRootPath": ARR_ROOT,
                "scryerRootPath": SCRYER_ROOT,
                "libraryId": library_id,
                "facet": "MOVIE"
            }]
        }
    })
}

/// Poll the tracked apply session until it settles, mirroring the Summary step.
async fn await_settled_apply(ctx: &TestContext, actor: &User, session_id: &str) -> Value {
    let deadline = tokio::time::Instant::now() + crate::common::WAIT_UNTIL_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        let body = schema_exec(
            ctx,
            WARMUP_STATUS,
            json!({ "sessionId": session_id }),
            actor,
        )
        .await;
        assert_no_errors(&body);
        let status = body["data"]["externalImportWarmupStatus"].clone();
        match status["status"].as_str() {
            Some("COMPLETED") | Some("FAILED") | Some("CANCELED") => return status,
            _ => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
        }
    }
    panic!("apply session {session_id} never settled");
}

#[tokio::test]
async fn graphql_finalize_external_import_returns_before_the_apply_runs_and_reports_progress() {
    let ctx = TestContext::new().await;
    let admin = create_admin(&ctx, "finalize-async-admin").await;
    let library_id = configure_movie_library(&ctx).await;
    let movies = vec![
        fixture_movie(1, "harbor-lantern-2019", ARR_ROOT),
        fixture_movie(2, "paper-observatory-2021", ARR_ROOT),
        fixture_movie(3, "salt-cartographer-2022", ARR_ROOT),
    ];
    let source_session_id = seed_completed_radarr_warmup(&ctx, &admin, &movies).await;

    // Hold the apply guard so the spawned job cannot start. If finalize ran the
    // apply on the request future this mutation could never return.
    let apply_guard = ctx.app.acquire_external_import_apply_guard().await;

    let accepted = schema_exec(
        &ctx,
        FINALIZE,
        finalize_input(&source_session_id, &library_id),
        &admin,
    )
    .await;
    assert_no_errors(&accepted);
    let payload = &accepted["data"]["finalizeExternalImport"];
    assert_eq!(
        payload["monitorWarmupSessionId"],
        "external-import-monitor-apply"
    );
    let finalize_session_id = payload["finalizeSessionId"]
        .as_str()
        .expect("finalize session id")
        .to_string();
    assert_ne!(finalize_session_id, source_session_id);
    assert_eq!(payload["progress"]["status"], "RUNNING");
    assert_eq!(payload["progress"]["phase"], "BUILDING_SNAPSHOT");
    assert_eq!(payload["progress"]["snapshotBuildTotalKnown"], true);
    // The denominator comes from the warmed source's own title count.
    assert_eq!(payload["progress"]["snapshotBuildProgress"]["total"], 3);
    assert_eq!(payload["progress"]["snapshotBuildProgress"]["completed"], 0);

    // The source session is still intact while the apply is pending: it is only
    // consumed once the apply has fully landed.
    assert!(
        ctx.app
            .get_external_import_monitor_warmup_status(&admin, &source_session_id)
            .await
            .is_ok()
    );

    drop(apply_guard);

    let settled = await_settled_apply(&ctx, &admin, &finalize_session_id).await;
    assert_eq!(settled["status"], "COMPLETED", "settled: {settled}");
    assert_eq!(settled["phase"], "READY");
    assert_eq!(settled["snapshotBuildProgress"]["completed"], 3);
    assert_eq!(settled["snapshotBuildProgress"]["total"], 3);
    assert!(settled["errorMessage"].is_null());

    // The apply rewrote the per-library snapshot the hinted scan reads.
    let applied = ctx
        .app
        .list_external_import_monitor_snapshot_chunks_for_session(
            &admin,
            &scryer_application::external_import_monitor_apply_session_id_for_library(&library_id),
            MediaFacet::Movie,
            ExternalImportMonitorSnapshotEntryKind::Movie,
            None,
            32,
        )
        .await
        .expect("read applied snapshot chunks");
    let applied_lines = applied
        .iter()
        .flat_map(|chunk| chunk.payload_ndjson.lines())
        .filter(|line| !line.trim().is_empty())
        .count();
    assert_eq!(applied_lines, 3);

    // Consumed only after success.
    assert!(
        ctx.app
            .get_external_import_monitor_warmup_status(&admin, &source_session_id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn graphql_finalize_external_import_reports_apply_failure_and_allows_a_re_run() {
    let ctx = TestContext::new().await;
    let admin = create_admin(&ctx, "finalize-async-retry-admin").await;
    let library_id = configure_movie_library(&ctx).await;

    // One entry sits under a root the mapping set does not cover. Validation
    // can't see it (it only knows the source's advertised roots), so the apply
    // is what fails — exactly the failure mode that used to vanish silently.
    let movies = vec![
        fixture_movie(1, "harbor-lantern-2019", ARR_ROOT),
        fixture_movie(2, "paper-observatory-2021", "/arr-data/stray"),
    ];
    let source_session_id = seed_completed_radarr_warmup(&ctx, &admin, &movies).await;

    let accepted = schema_exec(
        &ctx,
        FINALIZE,
        finalize_input(&source_session_id, &library_id),
        &admin,
    )
    .await;
    assert_no_errors(&accepted);
    let first_session_id = accepted["data"]["finalizeExternalImport"]["finalizeSessionId"]
        .as_str()
        .expect("finalize session id")
        .to_string();

    let failed = await_settled_apply(&ctx, &admin, &first_session_id).await;
    assert_eq!(failed["status"], "FAILED", "settled: {failed}");
    let message = failed["errorMessage"]
        .as_str()
        .expect("failed apply should carry an error message");
    assert!(
        message.contains("missing mapping for source"),
        "unexpected failure message: {message}"
    );

    // A failed apply keeps the warmed source, so the operator can fix and retry.
    assert!(
        ctx.app
            .get_external_import_monitor_warmup_status(&admin, &source_session_id)
            .await
            .is_ok()
    );

    // Re-stage the source with only mapped roots, then finalize again: the
    // second run must get its own session and must not wait on a stale guard.
    ctx.app
        .clear_external_import_monitor_snapshot_chunks_for_session(
            &admin,
            &source_session_id,
            MediaFacet::Movie,
        )
        .await
        .expect("clear source snapshot chunks");
    write_source_snapshot(
        &ctx,
        &admin,
        &source_session_id,
        &[fixture_movie(1, "harbor-lantern-2019", ARR_ROOT)],
    )
    .await;

    let retried = schema_exec(
        &ctx,
        FINALIZE,
        finalize_input(&source_session_id, &library_id),
        &admin,
    )
    .await;
    assert_no_errors(&retried);
    let second_session_id = retried["data"]["finalizeExternalImport"]["finalizeSessionId"]
        .as_str()
        .expect("second finalize session id")
        .to_string();
    assert_ne!(second_session_id, first_session_id);

    let settled = await_settled_apply(&ctx, &admin, &second_session_id).await;
    assert_eq!(settled["status"], "COMPLETED", "settled: {settled}");
}

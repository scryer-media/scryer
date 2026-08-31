use super::*;

fn run_large_stack_graphql_test<F>(future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name("graphql-large-stack-test".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build large-stack GraphQL test runtime")
                .block_on(future);
        })
        .expect("spawn large-stack GraphQL test thread")
        .join()
        .expect("large-stack GraphQL test thread should not panic");
}

#[tokio::test]
async fn graphql_download_queue_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ downloadQueue { id titleName } }", json!({})).await;
    assert_no_errors(&body);
    let queue = body["data"]["downloadQueue"].as_array().unwrap();
    assert!(queue.is_empty(), "queue should start empty");
}

#[test]
fn graphql_invalid_nzb_xml_queue_failure_is_blocklisted() {
    run_large_stack_graphql_test(graphql_invalid_nzb_xml_queue_failure_is_blocklisted_impl());
}

async fn graphql_invalid_nzb_xml_queue_failure_is_blocklisted_impl() {
    let ctx = TestContext::new().await;
    let title_id = add_test_title(&ctx, "Broken NZB Movie", "MOVIE").await;
    let source_hint = format!("{}/invalid.nzb", ctx.nzbget_server.uri());
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let candidate_token = ctx
        .app
        .issue_release_candidate_token(
            &admin,
            &title_id,
            &scryer_application::SubmissionScope::Title,
            &scryer_application::QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some(source_hint.clone()),
                source_kind: Some(scryer_application::DownloadSourceKind::NzbFile),
                source_title: Some("Broken.NZB.Movie.2024".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
        )
        .await
        .expect("issue candidate token");

    Mock::given(method("GET"))
        .and(path("/invalid.nzb"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not xml"))
        .mount(&ctx.nzbget_server)
        .await;

    let queue_body = gql(
        &ctx,
        r#"
        mutation($input: QueueDownloadInput!) {
          queueExistingTitleDownload(input: $input) {
            jobId
          }
        }
        "#,
        json!({
            "input": {
                "titleId": title_id,
                "candidateToken": candidate_token,
                "scope": { "title": true },
            }
        }),
    )
    .await;

    assert!(
        queue_body.get("errors").is_some(),
        "expected queue mutation to fail for invalid nzb xml: {queue_body}"
    );
    let error_message = queue_body["errors"][0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(
        error_message.contains("did not look like xml")
            || error_message.contains("root element must be <nzb>")
            || error_message.contains("not valid xml"),
        "expected invalid-xml error message, got: {error_message}"
    );

    let blocklist_body = gql(
        &ctx,
        r#"
        query($titleId: ID!) {
          titleReleaseBlocklist(titleId: $titleId) {
            id
            releaseName
            errorMessage
          }
        }
        "#,
        json!({ "titleId": title_id }),
    )
    .await;

    assert_no_errors(&blocklist_body);
    let entries = blocklist_body["data"]["titleReleaseBlocklist"]
        .as_array()
        .expect("blocklist entries array");
    assert!(
        entries.iter().any(|entry| {
            entry["releaseName"].as_str() == Some("Broken.NZB.Movie.2024")
                && entry["errorMessage"].as_str().is_some_and(|message| {
                    message.contains("did not look like xml")
                        || message.contains("root element must be <nzb>")
                        || message.contains("not valid xml")
                })
        }),
        "expected invalid nzb release to appear in titleReleaseBlocklist: {blocklist_body}"
    );
}

#[test]
fn graphql_title_release_blocklist_entry_can_be_cleared() {
    run_large_stack_graphql_test(graphql_title_release_blocklist_entry_can_be_cleared_impl());
}

async fn graphql_title_release_blocklist_entry_can_be_cleared_impl() {
    let ctx = TestContext::new().await;
    let title_id = add_test_title(&ctx, "Clear Blocklist Movie", "MOVIE").await;
    let source_hint = format!("{}/invalid-clear.nzb", ctx.nzbget_server.uri());
    let release_title = "Clear.Blocklist.Movie.2024";
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let candidate_token = ctx
        .app
        .issue_release_candidate_token(
            &admin,
            &title_id,
            &scryer_application::SubmissionScope::Title,
            &scryer_application::QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some(source_hint.clone()),
                source_kind: Some(scryer_application::DownloadSourceKind::NzbFile),
                source_title: Some("Clear.Blocklist.Movie.2024".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
        )
        .await
        .expect("issue candidate token");

    Mock::given(method("GET"))
        .and(path("/invalid-clear.nzb"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not xml"))
        .mount(&ctx.nzbget_server)
        .await;

    let queue_body = gql(
        &ctx,
        r#"
        mutation($input: QueueDownloadInput!) {
          queueExistingTitleDownload(input: $input) {
            jobId
          }
        }
        "#,
        json!({
            "input": {
                "titleId": title_id,
                "candidateToken": candidate_token,
                "scope": { "title": true },
            }
        }),
    )
    .await;

    assert!(
        queue_body.get("errors").is_some(),
        "expected queue mutation to fail for invalid nzb xml: {queue_body}"
    );

    let blocklist_before = gql(
        &ctx,
        r#"
        query($titleId: ID!) {
          titleReleaseBlocklist(titleId: $titleId) {
            id
            releaseName
          }
        }
        "#,
        json!({ "titleId": title_id }),
    )
    .await;

    assert_no_errors(&blocklist_before);
    let entry_id = blocklist_before["data"]["titleReleaseBlocklist"]
        .as_array()
        .and_then(|entries| {
            entries.iter().find_map(|entry| {
                (entry["releaseName"].as_str() == Some(release_title))
                    .then(|| entry["id"].as_str().map(ToOwned::to_owned))
                    .flatten()
            })
        })
        .expect("blocklist entry id");

    let clear_body = gql(
        &ctx,
        r#"
        mutation($id: ID!) {
          clearTitleReleaseBlocklistEntry(id: $id) {
            id
          }
        }
        "#,
        json!({ "id": entry_id }),
    )
    .await;

    assert_no_errors(&clear_body);
    assert_eq!(
        clear_body["data"]["clearTitleReleaseBlocklistEntry"]["id"],
        entry_id
    );
    let blocklist_after = gql(
        &ctx,
        r#"
        query($titleId: ID!) {
          titleReleaseBlocklist(titleId: $titleId) {
            releaseName
          }
        }
        "#,
        json!({ "titleId": title_id }),
    )
    .await;

    assert_no_errors(&blocklist_after);
    let entries_after = blocklist_after["data"]["titleReleaseBlocklist"]
        .as_array()
        .expect("blocklist entries array");
    assert!(
        !entries_after
            .iter()
            .any(|entry| entry["releaseName"].as_str() == Some(release_title)),
        "expected cleared release to be removed from titleReleaseBlocklist: {blocklist_after}"
    );
}

#[test]
fn graphql_title_release_blocklist_uses_persisted_blocklist_source_title() {
    run_large_stack_graphql_test(
        graphql_title_release_blocklist_uses_persisted_blocklist_source_title_impl(),
    );
}

async fn graphql_title_release_blocklist_uses_persisted_blocklist_source_title_impl() {
    let ctx = TestContext::new().await;
    let title_id = add_test_title(&ctx, "Pals", "SERIES").await;

    scryer_infrastructure_library::media::libraries::state_store::BlocklistStore::new(
        ctx.db.datastore(),
    )
    .block(&scryer_application::NewBlocklistEntry {
        title_id: title_id.clone(),
        release_name: "pals.s05.720p.bluray.dd5.1.x264-ntb".to_string(),
        indexer_id: String::new(),
        info_hash: None,
        reason: Some("download client failure: corrupt archive".to_string()),
    })
    .await
    .expect("seed blocklist entry");

    let release_store = scryer_infrastructure_workflow::workflow::release_store::ReleaseStore::new(
        ctx.db.datastore(),
        ctx.db.encryption_key_state(),
    );
    scryer_application::ReleaseAttemptRepository::record_release_attempt(
        &release_store,
        Some(title_id.clone()),
        Some("weaver://job-1".to_string()),
        Some("pals".to_string()),
        scryer_application::ReleaseDownloadAttemptOutcome::Failed,
        Some("legacy weak title".to_string()),
        None,
    )
    .await
    .expect("seed legacy weak failure attempt");

    let body = gql(
        &ctx,
        r#"
        query($titleId: ID!) {
          titleReleaseBlocklist(titleId: $titleId) {
            releaseName
          }
        }
        "#,
        json!({ "titleId": title_id }),
    )
    .await;

    assert_no_errors(&body);
    let entries = body["data"]["titleReleaseBlocklist"]
        .as_array()
        .expect("blocklist entries array");
    assert!(entries.iter().any(|entry| {
        entry["releaseName"].as_str() == Some("pals.s05.720p.bluray.dd5.1.x264-ntb")
    }));
}

#[tokio::test]
async fn graphql_download_history_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ downloadHistory(limit: 50, offset: 0) { items { id titleName } hasMore } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    let items = body["data"]["downloadHistory"]["items"].as_array().unwrap();
    assert!(items.is_empty(), "history should start empty");
    assert_eq!(body["data"]["downloadHistory"]["hasMore"], json!(false));
}

#[tokio::test]
async fn housekeeping_reports_pruned_staged_nzb_artifacts() {
    let ctx = TestContext::new().await;
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user should initialize");
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");
    let staged = ctx
        .staged_nzb_store
        .stage_nzb_bytes_for_test(nzb_xml.as_bytes())
        .await
        .expect("staged artifact should insert");
    ctx.staged_nzb_store
        .set_staged_nzb_updated_at(&staged, Utc::now() - Duration::hours(2))
        .await
        .expect("staged artifact timestamp should update");

    let report = ctx
        .app
        .run_housekeeping(&admin)
        .await
        .expect("housekeeping should run");

    assert_eq!(report.staged_nzb_artifacts_pruned, 1);
    assert_eq!(
        ctx.staged_nzb_store.count_staged_artifacts().await.unwrap(),
        0
    );
}

#[tokio::test]
async fn housekeeping_respects_configured_history_retention() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user should initialize");
    let baseline_domain_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domain_events")
        .fetch_one(ctx.db.pool())
        .await
        .expect("baseline domain events count");

    let title = create_catalog_title(
        &ctx,
        "Retention Fixture",
        MediaFacet::Series,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "12345".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let now = Utc::now();
    let stale_at = (now - Duration::days(40)).to_rfc3339();
    let very_stale_at = (now - Duration::days(120)).to_rfc3339();
    let fresh_at = (now - Duration::days(5)).to_rfc3339();
    let expired_operational_at = (now - Duration::days(4)).to_rfc3339();
    let fresh_operational_at = (now - Duration::days(2)).to_rfc3339();
    let expired_acquisition_telemetry_at = (now - Duration::days(2)).to_rfc3339();
    let fresh_acquisition_telemetry_at = (now - Duration::hours(12)).to_rfc3339();
    let wanted_item_id = Id::new().0;
    let stale_completed_import_id = Id::new().0;
    let fresh_completed_import_id = Id::new().0;
    let stale_processing_import_id = Id::new().0;

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, episode_id, media_type, status, created_at, updated_at)
         VALUES (?, ?, NULL, 'series', 'wanted', ?, ?)",
    )
    .bind(&wanted_item_id)
    .bind(&title.id)
    .bind(&fresh_at)
    .bind(&fresh_at)
    .execute(ctx.db.pool())
    .await
    .expect("wanted item should insert");

    sqlx::query(
        "INSERT INTO release_decisions
         (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes, decision_code, candidate_score, current_score, score_delta, explanation_json, created_at)
         VALUES (?, ?, ?, 'stale-release', NULL, NULL, 'accepted', 100, NULL, NULL, NULL, ?),
                (?, ?, ?, 'fresh-release', NULL, NULL, 'accepted', 100, NULL, NULL, NULL, ?)",
    )
    .bind(Id::new().0)
    .bind(&wanted_item_id)
    .bind(&title.id)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&wanted_item_id)
    .bind(&title.id)
    .bind(&fresh_at)
    .execute(ctx.db.pool())
    .await
    .expect("release decisions should insert");

    sqlx::query(
        "INSERT INTO release_download_attempts
         (id, title_id, source_hint, source_title, outcome, error_message, attempted_at, created_at, updated_at)
         VALUES (?, ?, NULL, 'stale-attempt', 'grabbed', NULL, ?, ?, ?),
                (?, ?, NULL, 'fresh-attempt', 'grabbed', NULL, ?, ?, ?),
                (?, ?, NULL, 'pending-attempt', 'pending', NULL, ?, ?, ?)",
    )
    .bind(Id::new().0)
    .bind(&title.id)
    .bind(&very_stale_at)
    .bind(&very_stale_at)
    .bind(&very_stale_at)
    .bind(Id::new().0)
    .bind(&title.id)
    .bind(&fresh_at)
    .bind(&fresh_at)
    .bind(&fresh_at)
    .bind(Id::new().0)
    .bind(&title.id)
    .bind(&stale_at)
    .bind(&stale_at)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("release attempts should insert");

    sqlx::query(
        "INSERT INTO history_events
         (id, event_type, actor_user_id, title_id, message, occurred_at, source, created_at, metadata_json)
         VALUES (?, 'test', NULL, NULL, 'stale-history', ?, NULL, ?, NULL),
                (?, 'test', NULL, NULL, 'fresh-history', ?, NULL, ?, NULL)",
    )
    .bind(Id::new().0)
    .bind(&stale_at)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&fresh_at)
    .bind(&fresh_at)
    .execute(ctx.db.pool())
    .await
    .expect("history events should insert");

    sqlx::query(
        "INSERT INTO domain_events
         (event_id, occurred_at, actor_user_id, title_id, facet, correlation_id, causation_id, schema_version, stream_kind, stream_id, event_type, payload_json)
         VALUES (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'title_added', '{}'),
                (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'import_requested', '{}'),
                (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'library_scan_progressed', '{}'),
                (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'job_run_started', '{}'),
                (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'acquisition_search_completed', '{}'),
                (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'acquisition_candidate_rejected', '{}')",
    )
    .bind(Id::new().0)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&fresh_at)
    .bind(Id::new().0)
    .bind(&expired_operational_at)
    .bind(Id::new().0)
    .bind(&fresh_operational_at)
    .bind(Id::new().0)
    .bind(&expired_acquisition_telemetry_at)
    .bind(Id::new().0)
    .bind(&fresh_acquisition_telemetry_at)
    .execute(ctx.db.pool())
    .await
    .expect("domain events should insert");

    sqlx::query(
        "INSERT INTO imports
         (id, source_system, source_ref, import_type, status, payload_json, result_json, started_at, finished_at, created_at, updated_at)
         VALUES (?, 'test', 'stale-completed', 'manual_import', 'completed', '{}', '{}', NULL, ?, ?, ?),
                (?, 'test', 'fresh-completed', 'manual_import', 'completed', '{}', '{}', NULL, ?, ?, ?),
                (?, 'test', 'stale-processing', 'manual_import', 'processing', '{}', NULL, NULL, NULL, ?, ?)",
    )
    .bind(&stale_completed_import_id)
    .bind(&stale_at)
    .bind(&stale_at)
    .bind(&stale_at)
    .bind(&fresh_completed_import_id)
    .bind(&fresh_at)
    .bind(&fresh_at)
    .bind(&fresh_at)
    .bind(&stale_processing_import_id)
    .bind(&stale_at)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("imports should insert");

    sqlx::query(
        "INSERT INTO download_import_artifacts
         (id, source_system, source_ref, import_id, relative_path, normalized_file_name, media_kind, title_id, episode_id, season_number, episode_number, result, reason_code, imported_media_file_id, created_at)
         VALUES (?, 'test', 'stale-completed', ?, NULL, 'stale.mkv', 'episode', ?, NULL, NULL, NULL, 'imported', NULL, NULL, ?),
                (?, 'test', 'fresh-completed', ?, NULL, 'fresh.mkv', 'episode', ?, NULL, NULL, NULL, 'imported', NULL, NULL, ?),
                (?, 'test', 'stale-processing', ?, NULL, 'active.mkv', 'episode', ?, NULL, NULL, NULL, 'imported', NULL, NULL, ?)",
    )
    .bind(Id::new().0)
    .bind(&stale_completed_import_id)
    .bind(&title.id)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&fresh_completed_import_id)
    .bind(&title.id)
    .bind(&fresh_at)
    .bind(Id::new().0)
    .bind(&stale_processing_import_id)
    .bind(&title.id)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("download import artifacts should insert");

    sqlx::query(
        "INSERT INTO rule_set_history (id, rule_set_id, action, rego_source, actor_id, created_at)
         VALUES (?, 'rule-1', 'updated', NULL, NULL, ?),
                (?, 'rule-1', 'updated', NULL, NULL, ?)",
    )
    .bind(Id::new().0)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&fresh_at)
    .execute(ctx.db.pool())
    .await
    .expect("rule set history should insert");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
          }
        }
        "#,
        json!({
          "input": {
            "keepHistoryForever": false,
            "historyRetentionDays": 30,
            "pluginHttpCaBundlePem": ""
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let report = ctx
        .app
        .run_housekeeping(&admin)
        .await
        .expect("housekeeping should run");
    assert_eq!(report.stale_release_decisions, 1);
    assert_eq!(report.stale_release_attempts, 1);
    assert_eq!(report.stale_history_events, 1);
    assert_eq!(report.stale_history_records, 9);

    let remaining_release_decisions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM release_decisions")
            .fetch_one(ctx.db.pool())
            .await
            .expect("release decisions count");
    assert_eq!(remaining_release_decisions, 1);

    let remaining_release_attempts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM release_download_attempts")
            .fetch_one(ctx.db.pool())
            .await
            .expect("release attempts count");
    assert_eq!(remaining_release_attempts, 2);

    let remaining_history_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM history_events")
        .fetch_one(ctx.db.pool())
        .await
        .expect("history events count");
    assert_eq!(remaining_history_events, 1);

    let remaining_domain_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domain_events")
        .fetch_one(ctx.db.pool())
        .await
        .expect("domain events count");
    assert_eq!(remaining_domain_events, baseline_domain_events + 4);

    let remaining_imports: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM imports")
        .fetch_one(ctx.db.pool())
        .await
        .expect("imports count");
    assert_eq!(remaining_imports, 2);

    let remaining_import_artifacts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM download_import_artifacts")
            .fetch_one(ctx.db.pool())
            .await
            .expect("download import artifacts count");
    assert_eq!(remaining_import_artifacts, 2);

    let remaining_rule_set_history: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM rule_set_history")
            .fetch_one(ctx.db.pool())
            .await
            .expect("rule set history count");
    assert_eq!(remaining_rule_set_history, 1);
}

#[tokio::test]
async fn housekeeping_skips_history_retention_when_keep_forever_is_enabled() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user should initialize");
    let baseline_domain_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domain_events")
        .fetch_one(ctx.db.pool())
        .await
        .expect("baseline domain events count");
    let stale_at = (Utc::now() - Duration::days(400)).to_rfc3339();
    let stale_attempt_at = (Utc::now() - Duration::days(120)).to_rfc3339();
    let import_id = Id::new().0;
    let title = create_catalog_title(
        &ctx,
        "Retention Keep Forever Fixture",
        MediaFacet::Series,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "67890".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    let wanted_item_id = Id::new().0;

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, episode_id, media_type, status, created_at, updated_at)
         VALUES (?, ?, NULL, 'series', 'wanted', ?, ?)",
    )
    .bind(&wanted_item_id)
    .bind(&title.id)
    .bind(&stale_at)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("wanted item should insert");

    sqlx::query(
        "INSERT INTO history_events
         (id, event_type, actor_user_id, title_id, message, occurred_at, source, created_at, metadata_json)
         VALUES (?, 'test', NULL, NULL, 'stale-history', ?, NULL, ?, NULL)",
    )
    .bind(Id::new().0)
    .bind(&stale_at)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("history event should insert");

    sqlx::query(
        "INSERT INTO domain_events
         (event_id, occurred_at, actor_user_id, title_id, facet, correlation_id, causation_id, schema_version, stream_kind, stream_id, event_type, payload_json)
         VALUES (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'title_added', '{}'),
                (?, ?, NULL, NULL, NULL, NULL, NULL, 1, 'test', NULL, 'library_scan_progressed', '{}')",
    )
    .bind(Id::new().0)
    .bind(&stale_at)
    .bind(Id::new().0)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("domain events should insert");

    sqlx::query(
        "INSERT INTO imports
         (id, source_system, source_ref, import_type, status, payload_json, result_json, started_at, finished_at, created_at, updated_at)
         VALUES (?, 'test', 'stale-completed', 'manual_import', 'completed', '{}', '{}', NULL, ?, ?, ?)",
    )
    .bind(&import_id)
    .bind(&stale_at)
    .bind(&stale_at)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("import should insert");

    sqlx::query(
        "INSERT INTO download_import_artifacts
         (id, source_system, source_ref, import_id, relative_path, normalized_file_name, media_kind, title_id, episode_id, season_number, episode_number, result, reason_code, imported_media_file_id, created_at)
         VALUES (?, 'test', 'stale-completed', ?, NULL, 'stale.mkv', 'episode', NULL, NULL, NULL, NULL, 'imported', NULL, NULL, ?)",
    )
    .bind(Id::new().0)
    .bind(&import_id)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("download import artifact should insert");

    sqlx::query(
        "INSERT INTO release_decisions
         (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes, decision_code, candidate_score, current_score, score_delta, explanation_json, created_at)
         VALUES (?, ?, ?, 'stale-release', NULL, NULL, 'accepted', 100, NULL, NULL, NULL, ?)",
    )
    .bind(Id::new().0)
    .bind(&wanted_item_id)
    .bind(&title.id)
    .bind(&stale_at)
    .execute(ctx.db.pool())
    .await
    .expect("release decision should insert");

    sqlx::query(
        "INSERT INTO release_download_attempts
         (id, title_id, source_hint, source_title, outcome, error_message, attempted_at, created_at, updated_at)
         VALUES (?, NULL, NULL, 'stale-attempt', 'grabbed', NULL, ?, ?, ?),
                (?, NULL, NULL, 'pending-attempt', 'pending', NULL, ?, ?, ?)",
    )
    .bind(Id::new().0)
    .bind(&stale_attempt_at)
    .bind(&stale_attempt_at)
    .bind(&stale_attempt_at)
    .bind(Id::new().0)
    .bind(&stale_attempt_at)
    .bind(&stale_attempt_at)
    .bind(&stale_attempt_at)
    .execute(ctx.db.pool())
    .await
    .expect("release attempts should insert");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
          }
        }
        "#,
        json!({
          "input": {
            "keepHistoryForever": true,
            "historyRetentionDays": 180,
            "pluginHttpCaBundlePem": ""
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let report = ctx
        .app
        .run_housekeeping(&admin)
        .await
        .expect("housekeeping should run");
    assert_eq!(report.stale_release_decisions, 1);
    assert_eq!(report.stale_release_attempts, 1);
    assert_eq!(report.stale_history_events, 0);
    assert_eq!(report.stale_history_records, 3);

    let remaining_history_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM history_events")
        .fetch_one(ctx.db.pool())
        .await
        .expect("history events count");
    assert_eq!(remaining_history_events, 1);

    let remaining_imports: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM imports")
        .fetch_one(ctx.db.pool())
        .await
        .expect("imports count");
    assert_eq!(remaining_imports, 1);

    let remaining_domain_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domain_events")
        .fetch_one(ctx.db.pool())
        .await
        .expect("domain events count");
    assert_eq!(remaining_domain_events, baseline_domain_events + 2);

    let remaining_import_artifacts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM download_import_artifacts")
            .fetch_one(ctx.db.pool())
            .await
            .expect("download import artifacts count");
    assert_eq!(remaining_import_artifacts, 1);

    let remaining_release_decisions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM release_decisions")
            .fetch_one(ctx.db.pool())
            .await
            .expect("release decisions count");
    assert_eq!(remaining_release_decisions, 0);

    let remaining_release_attempts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM release_download_attempts")
            .fetch_one(ctx.db.pool())
            .await
            .expect("release attempts count");
    assert_eq!(remaining_release_attempts, 1);
}

#[tokio::test]
async fn sqlite_history_retention_indexes_exist_after_migrations() {
    let ctx = TestContext::new().await;

    let history_event_indexes = sqlx::query("PRAGMA index_list('history_events')")
        .fetch_all(ctx.db.pool())
        .await
        .expect("history event indexes");
    let history_event_index_names = history_event_indexes
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(history_event_index_names.contains(&"idx_history_events_occurred_at".to_string()));

    let import_indexes = sqlx::query("PRAGMA index_list('imports')")
        .fetch_all(ctx.db.pool())
        .await
        .expect("import indexes");
    let import_index_names = import_indexes
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(import_index_names.contains(&"idx_imports_status_updated_at".to_string()));

    let rule_set_history_indexes = sqlx::query("PRAGMA index_list('rule_set_history')")
        .fetch_all(ctx.db.pool())
        .await
        .expect("rule set history indexes");
    let rule_set_history_index_names = rule_set_history_indexes
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(rule_set_history_index_names.contains(&"idx_rule_set_history_created_at".to_string()));

    let release_decision_indexes = sqlx::query("PRAGMA index_list('release_decisions')")
        .fetch_all(ctx.db.pool())
        .await
        .expect("release decision indexes");
    let release_decision_index_names = release_decision_indexes
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(release_decision_index_names.contains(&"idx_release_decisions_created_at".to_string()));

    let import_artifact_indexes = sqlx::query("PRAGMA index_list('download_import_artifacts')")
        .fetch_all(ctx.db.pool())
        .await
        .expect("download import artifact indexes");
    let import_artifact_index_names = import_artifact_indexes
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(
        import_artifact_index_names
            .contains(&"idx_download_import_artifacts_retention".to_string())
    );
}

// ---------------------------------------------------------------------------
// System health
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_system_health() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ systemHealth { serviceReady runtimePathStyle totalTitles } runtimeInfo { runtimePathStyle } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(
        body["data"]["systemHealth"]["serviceReady"].is_boolean(),
        "should return serviceReady boolean"
    );
    assert!(
        matches!(
            body["data"]["systemHealth"]["runtimePathStyle"].as_str(),
            Some("UNIX") | Some("WINDOWS")
        ),
        "should return runtime path style"
    );
    assert!(
        matches!(
            body["data"]["runtimeInfo"]["runtimePathStyle"].as_str(),
            Some("UNIX") | Some("WINDOWS")
        ),
        "should return lightweight runtime path style"
    );
}

#[tokio::test]
async fn graphql_smg_version_compatibility_notice_reads_persisted_notice() {
    let ctx = TestContext::new().await;
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
            category: "service".into(),
            scope: "system".into(),
            key_name: "smg.version_compatibility_notice".into(),
            data_type: "json".into(),
            default_value_json: "null".into(),
            is_sensitive: false,
            validation_json: None,
        }])
        .await
        .expect("compatibility notice definition should seed");
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "smg.version_compatibility_notice",
            None,
            json!({
                "status": "deprecated",
                "minimum_version": "0.14.2",
                "your_version": "0.14.1",
                "message": "Upgrade before support ends.",
                "upgrade_deadline": "2026-06-01",
            })
            .to_string(),
            "test",
            None,
        )
        .await
        .expect("compatibility notice should persist");

    let body = gql(
        &ctx,
        r#"{ smgVersionCompatibilityNotice { status minimumVersion yourVersion message upgradeDeadline } }"#,
        json!({}),
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(
        body["data"]["smgVersionCompatibilityNotice"]["status"],
        "deprecated"
    );
    assert_eq!(
        body["data"]["smgVersionCompatibilityNotice"]["minimumVersion"],
        "0.14.2"
    );
    assert_eq!(
        body["data"]["smgVersionCompatibilityNotice"]["yourVersion"],
        "0.14.1"
    );
    assert_eq!(
        body["data"]["smgVersionCompatibilityNotice"]["message"],
        "Upgrade before support ends."
    );
    assert_eq!(
        body["data"]["smgVersionCompatibilityNotice"]["upgradeDeadline"],
        "2026-06-01"
    );
}

#[tokio::test]
async fn graphql_smg_scryer_update_notice_reads_persisted_notice() {
    let ctx = TestContext::new().await;
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
            category: "service".into(),
            scope: "system".into(),
            key_name: "smg.scryer_update_notice".into(),
            data_type: "json".into(),
            default_value_json: "null".into(),
            is_sensitive: false,
            validation_json: None,
        }])
        .await
        .expect("update notice definition should seed");
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "smg.scryer_update_notice",
            None,
            json!({
                "available": true,
                "current_version": "0.16.0",
                "latest_version": "0.16.1",
                "latest_tag": "v0.16.1",
                "release_url": "https://github.com/scryer-media/scryer/releases/tag/v0.16.1",
                "published_at": "2026-06-14T12:00:00Z",
                "checked_at": "2026-06-15T12:00:00Z",
            })
            .to_string(),
            "test",
            None,
        )
        .await
        .expect("update notice should persist");

    let body = gql(
        &ctx,
        r#"{ smgScryerUpdateNotice { available currentVersion latestVersion latestTag releaseUrl publishedAt checkedAt } }"#,
        json!({}),
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(body["data"]["smgScryerUpdateNotice"]["available"], true);
    assert_eq!(
        body["data"]["smgScryerUpdateNotice"]["currentVersion"],
        "0.16.0"
    );
    assert_eq!(
        body["data"]["smgScryerUpdateNotice"]["latestVersion"],
        "0.16.1"
    );
    assert_eq!(
        body["data"]["smgScryerUpdateNotice"]["latestTag"],
        "v0.16.1"
    );
    assert_eq!(
        body["data"]["smgScryerUpdateNotice"]["releaseUrl"],
        "https://github.com/scryer-media/scryer/releases/tag/v0.16.1"
    );
    assert_eq!(
        body["data"]["smgScryerUpdateNotice"]["publishedAt"],
        "2026-06-14T12:00:00+00:00"
    );
    assert_eq!(
        body["data"]["smgScryerUpdateNotice"]["checkedAt"],
        "2026-06-15T12:00:00+00:00"
    );
}

#[tokio::test]
async fn graphql_application_upgrade_status_requires_system_settings_permission() {
    let ctx = TestContext::new().await;
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("find default admin");
    let denied_actor = ctx
        .app
        .create_user(
            &admin,
            "upgrade_status_denied".to_string(),
            "upgrade-status-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create actor without system settings permission");
    let system_actor = ctx
        .app
        .create_user(
            &admin,
            "upgrade_status_system".to_string(),
            "upgrade-status-pass2".to_string(),
            AppPermissionMask::from_permission(scryer_domain::AppPermission::ManageSystemSettings),
            vec![],
        )
        .await
        .expect("create actor with system settings permission");

    let query = r#"
        {
          applicationUpgradeStatus {
            currentVersion
            updateVersion
            updateTag
            updateAvailable
            installationKind
            managementOwner
            eligible
            eligibilityReason
            activeRun { id status }
            latestRun { id status }
          }
        }
    "#;

    let denied = schema_exec(&ctx, query, Some(denied_actor)).await;
    assert_graphql_field_denied(&denied, "applicationUpgradeStatus");

    let allowed = schema_exec(&ctx, query, Some(system_actor)).await;
    assert_no_errors(&allowed);
    let status = &allowed["data"]["applicationUpgradeStatus"];
    assert!(
        status["currentVersion"]
            .as_str()
            .is_some_and(|version| !version.is_empty()),
        "currentVersion must be populated: {status}"
    );
    assert!(status["updateVersion"].is_null() || status["updateVersion"].is_string());
    assert!(status["updateTag"].is_null() || status["updateTag"].is_string());
    assert!(status["updateAvailable"].is_boolean());
    assert!(matches!(
        status["installationKind"].as_str(),
        Some(
            "PORTABLE"
                | "DIRECT_MSI"
                | "DOCKER"
                | "HOMEBREW"
                | "WINGET"
                | "WINDOWS_SUPERVISED"
                | "DISABLED"
                | "UNSUPPORTED"
        )
    ));
    assert!(matches!(
        status["managementOwner"].as_str(),
        Some("IN_APP" | "OPERATOR")
    ));
    assert!(status["eligible"].is_boolean());
    assert!(status["activeRun"].is_null());
    assert!(status["latestRun"].is_null());
    assert!(matches!(
        status["eligibilityReason"].as_str(),
        Some(
            "disabled_by_operator"
                | "managed_by_docker"
                | "managed_by_homebrew"
                | "windows_supervised"
                | "managed_by_winget"
                | "eligible"
                | "unsupported_layout"
                | "install_dir_not_writable"
        )
    ));
}

#[tokio::test]
async fn graphql_start_application_upgrade_requires_config_permission_and_rejects_ineligible_installations()
 {
    let ctx = TestContext::new().await;
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("find default admin");
    let denied_actor = ctx
        .app
        .create_user(
            &admin,
            "upgrade_start_denied".to_string(),
            "upgrade-start-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create actor without system settings permission");
    let system_actor = ctx
        .app
        .create_user(
            &admin,
            "upgrade_start_system".to_string(),
            "upgrade-start-pass2".to_string(),
            AppPermissionMask::from_permission(scryer_domain::AppPermission::ManageSystemSettings),
            vec![],
        )
        .await
        .expect("create actor with system settings permission");
    let mutation = r#"
        mutation {
          startApplicationUpgrade(input: { expectedTag: "v999.0.0", expectedVersion: "999.0.0" }) {
            jobRun { id }
          }
        }
    "#;

    let denied = schema_exec(&ctx, mutation, Some(denied_actor)).await;
    assert_graphql_field_denied(&denied, "startApplicationUpgrade");

    let ineligible = schema_exec(&ctx, mutation, Some(system_actor)).await;
    assert!(
        ineligible.get("errors").is_some(),
        "expected ineligible error: {ineligible}"
    );
    assert!(
        ineligible["errors"][0]["message"]
            .as_str()
            .is_some_and(|message| message
                .contains("application upgrade is not eligible: unsupported_layout")),
        "unexpected ineligible response: {ineligible}"
    );
}

#[tokio::test]
async fn graphql_start_application_upgrade_requires_mfa_step_up() {
    let ctx = TestContext::new().await;
    let (_admin, token, _totp_code) =
        enable_form_login_with_config_step_up(&ctx, "admin", "admin-pass1").await;
    let body = gql_with_token(
        &ctx,
        r#"
            mutation {
              startApplicationUpgrade(input: { expectedTag: "v999.0.0", expectedVersion: "999.0.0" }) {
                jobRun { id }
              }
            }
        "#,
        json!({}),
        &token,
    )
    .await;
    assert_mfa_step_up_required(&body);
}

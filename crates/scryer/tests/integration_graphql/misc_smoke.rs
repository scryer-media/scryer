use super::*;

#[tokio::test]
async fn graphql_indexers_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ indexers { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert!(body["data"]["indexers"].is_array());
}

#[tokio::test]
async fn graphql_download_client_configs_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ downloadClientConfigs { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert!(body["data"]["downloadClientConfigs"].is_array());
}

#[tokio::test]
async fn graphql_runtime_browse_and_download_client_permissions() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let limited = ctx
        .app
        .create_user(
            &admin,
            "runtime_limited".to_string(),
            "limited-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create limited user");
    let manage_library = ctx
        .app
        .create_user(
            &admin,
            "runtime_manage_library".to_string(),
            "library-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![scryer_domain::LibraryGrant {
                user_id: String::new(),
                library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
                permissions: LibraryPermissionMask::from_permission(
                    LibraryPermission::ManageLibrary,
                ),
            }],
        )
        .await
        .expect("create manage-library user");
    let catalog_user = ctx
        .app
        .create_user(
            &admin,
            "runtime_catalog".to_string(),
            "catalog-pass1".to_string(),
            AppPermissionMask::from_permission(scryer_domain::AppPermission::ManageCatalogSettings),
            vec![],
        )
        .await
        .expect("create catalog user");
    let system_user = ctx
        .app
        .create_user(
            &admin,
            "runtime_system".to_string(),
            "system-pass1".to_string(),
            AppPermissionMask::from_permission(scryer_domain::AppPermission::ManageSystemSettings),
            vec![],
        )
        .await
        .expect("create system user");

    let runtime_body = schema_exec(
        &ctx,
        "{ runtimeInfo { runtimePathStyle } }",
        Some(limited.clone()),
    )
    .await;
    assert_no_errors(&runtime_body);
    assert!(
        matches!(
            runtime_body["data"]["runtimeInfo"]["runtimePathStyle"].as_str(),
            Some("UNIX") | Some("WINDOWS")
        ),
        "runtimeInfo should be readable by authenticated non-admin users"
    );

    let browse_path = serde_json::to_string(&std::env::current_dir().unwrap().to_string_lossy())
        .expect("serialize current dir path");
    let browse_query = format!("{{ browsePath(path: {browse_path}) {{ path }} }}");
    let browse_body = schema_exec(&ctx, &browse_query, Some(manage_library.clone())).await;
    assert_no_errors(&browse_body);
    assert!(browse_body["data"]["browsePath"].is_array());

    let relative_browse_body = schema_exec(
        &ctx,
        "{ browsePath(path: \"relative/path\") { path } }",
        Some(catalog_user.clone()),
    )
    .await;
    let (relative_message, relative_code) =
        first_graphql_error_message_and_code(&relative_browse_body);
    assert_eq!(relative_code, "VALIDATION_ERROR");
    assert!(relative_message.contains("Path must be absolute."));

    let browse_validation_root = tempfile::tempdir().expect("browse validation root");
    let missing_path = browse_validation_root.path().join("missing");
    let missing_path_string = missing_path.to_string_lossy().into_owned();
    let missing_path_json =
        serde_json::to_string(&missing_path_string).expect("serialize missing path");
    let missing_browse_query = format!("{{ browsePath(path: {missing_path_json}) {{ path }} }}");
    let missing_browse_body =
        schema_exec(&ctx, &missing_browse_query, Some(catalog_user.clone())).await;
    let (missing_message, missing_code) =
        first_graphql_error_message_and_code(&missing_browse_body);
    assert_eq!(missing_code, "VALIDATION_ERROR");
    assert!(missing_message.contains("Directory does not exist:"));
    let missing_error = missing_browse_body["errors"][0]
        .as_object()
        .expect("missing path graphql error");
    assert!(
        missing_error
            .get("extensions")
            .and_then(|extensions| extensions.get("errorId"))
            .is_none(),
        "missing browse path should not be masked as an internal error: {missing_browse_body}"
    );

    let file_path = browse_validation_root.path().join("not-a-directory.txt");
    std::fs::write(&file_path, b"not a directory").expect("write browse validation file");
    let file_path_string = file_path.to_string_lossy().into_owned();
    let file_path_json = serde_json::to_string(&file_path_string).expect("serialize file path");
    let file_browse_query = format!("{{ browsePath(path: {file_path_json}) {{ path }} }}");
    let file_browse_body = schema_exec(&ctx, &file_browse_query, Some(catalog_user.clone())).await;
    let (file_message, file_code) = first_graphql_error_message_and_code(&file_browse_body);
    assert_eq!(file_code, "VALIDATION_ERROR");
    assert!(file_message.contains("Path is not a directory:"));

    let browse_denied = schema_exec(&ctx, &browse_query, Some(limited)).await;
    assert!(
        browse_denied.get("errors").is_some(),
        "browsePath should reject users without library-settings access: {browse_denied}"
    );

    let configs_denied = schema_exec(
        &ctx,
        "{ downloadClientConfigs { id name } }",
        Some(manage_library),
    )
    .await;
    assert!(
        configs_denied.get("errors").is_some(),
        "downloadClientConfigs should reject ManageLibrary-only users: {configs_denied}"
    );

    let catalog_configs_body = schema_exec(
        &ctx,
        "{ downloadClientConfigs { id name } }",
        Some(catalog_user.clone()),
    )
    .await;
    assert_no_errors(&catalog_configs_body);
    assert!(catalog_configs_body["data"]["downloadClientConfigs"].is_array());

    let routing_bootstrap_body = schema_exec(
        &ctx,
        r#"
        query {
            downloadClientConfigs { id name }
            indexers { id name }
            downloadClientRouting(scope: MOVIE) { clientId category }
            indexerRouting(scope: MOVIE) { indexerId categories }
        }
        "#,
        Some(catalog_user),
    )
    .await;
    assert_no_errors(&routing_bootstrap_body);
    assert!(routing_bootstrap_body["data"]["downloadClientConfigs"].is_array());
    assert!(routing_bootstrap_body["data"]["indexers"].is_array());
    assert!(routing_bootstrap_body["data"]["downloadClientRouting"].is_array());
    assert!(routing_bootstrap_body["data"]["indexerRouting"].is_array());

    let configs_body = schema_exec(
        &ctx,
        "{ downloadClientConfigs { id name } }",
        Some(system_user),
    )
    .await;
    assert_no_errors(&configs_body);
    assert!(configs_body["data"]["downloadClientConfigs"].is_array());
}

// ---------------------------------------------------------------------------
// Wanted items
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_wanted_items_empty() {
    let ctx = TestContext::new().await;
    // `wantedItems` is the derived Missing/Upgrades view selected by
    // `wantedKind`; the state-row status/media-type filters were removed.
    let body = gql(
        &ctx,
        r#"query($wantedKind: WantedKindValue!) {
            wantedItems(wantedKind: $wantedKind) {
                items { id convergenceState indexersCovered indexersRouted recencyLane }
                totalCount
                hasMore
            }
        }"#,
        json!({ "wantedKind": "MISSING" }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["wantedItems"]["totalCount"], 0,
        "should have no wanted items initially"
    );
    assert_eq!(body["data"]["wantedItems"]["hasMore"], false);
}

// ---------------------------------------------------------------------------
// Rule sets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_rule_sets_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ ruleSets { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["ruleSets"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Import history
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_import_history_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ importHistory { id sourceTitle status } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["importHistory"].is_array());
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_calendar_episodes() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($start: Date!, $end: Date!) {
            calendarEpisodes(startDate: $start, endDate: $end) {
                episodeTitle seasonNumber episodeNumber overview imageUrl
            }
        }"#,
        json!({ "start": "2024-01-01", "end": "2024-12-31" }),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["calendarEpisodes"].is_array());
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_unknown_field_returns_error() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ nonExistentField }", json!({})).await;
    assert!(
        body.get("errors").is_some(),
        "unknown field should return errors"
    );
}

#[tokio::test]
async fn graphql_invalid_mutation_input() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation { addTitle(input: { name: "" }) { title { id } } }"#,
        json!({}),
    )
    .await;
    assert!(
        body.get("errors").is_some(),
        "invalid input should return errors"
    );
}

#[tokio::test]
async fn graphql_batch_request_not_supported_via_single() {
    let ctx = TestContext::new().await;
    // Verify single requests work (batch is handled at the middleware level)
    let body = gql(&ctx, "{ titles { items { id } } }", json!({})).await;
    assert_no_errors(&body);
}

// ── Title-less interactive search (spec 0002) ────────────────────────────

#[tokio::test]
async fn graphql_query_subject_interactive_search_starts_and_polls() {
    let ctx = TestContext::new().await;

    let start = gql(
        &ctx,
        r#"mutation($input: SearchReleasesInput!) {
            startInteractiveReleaseSearch(input: $input) {
                id
                state
                results { title indexerId grabs }
                indexers { indexerId name priority status resultCount elapsedMs failureReason }
            }
        }"#,
        json!({ "input": { "query": "paperman", "kind": "RAW" } }),
    )
    .await;
    assert_no_errors(&start);
    let payload = &start["data"]["startInteractiveReleaseSearch"];
    assert_eq!(payload["state"], "RUNNING");
    let job_id = payload["id"].as_str().expect("job id").to_string();

    // No indexers are configured in the shared TestContext, so the job settles
    // almost immediately.
    let mut state = payload["state"].as_str().unwrap_or_default().to_string();
    for _ in 0..50 {
        let poll = gql(
            &ctx,
            r#"query($id: ID!) {
                interactiveReleaseSearch(id: $id) {
                    id
                    state
                    results { title indexerId grabs }
                    indexers { priority elapsedMs }
                }
            }"#,
            json!({ "id": job_id }),
        )
        .await;
        assert_no_errors(&poll);
        let job = &poll["data"]["interactiveReleaseSearch"];
        assert!(!job.is_null(), "job should be pollable: {poll}");
        state = job["state"].as_str().expect("state").to_string();
        if state != "RUNNING" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(state, "COMPLETED");
}

#[tokio::test]
async fn graphql_candidate_token_mutation_refuses_a_release_outside_the_search() {
    let ctx = TestContext::new().await;
    let title_id = add_test_title(&ctx, "Token Round Trip", "MOVIE").await;

    let start = gql(
        &ctx,
        r#"mutation($input: SearchReleasesInput!) {
            startInteractiveReleaseSearch(input: $input) { id state }
        }"#,
        json!({ "input": { "query": "paperman", "kind": "RAW" } }),
    )
    .await;
    assert_no_errors(&start);
    let job_id = start["data"]["startInteractiveReleaseSearch"]["id"]
        .as_str()
        .expect("job id")
        .to_string();

    let issued = gql(
        &ctx,
        r#"mutation($input: IssueInteractiveReleaseCandidateTokenInput!) {
            issueInteractiveReleaseCandidateToken(input: $input) {
                title
                candidateToken
                queueScope { __typename }
            }
        }"#,
        json!({
            "input": {
                "searchId": job_id,
                "downloadUrl": "https://example.invalid/not-in-this-search.nzb",
                "titleId": title_id,
            }
        }),
    )
    .await;
    let errors = issued["errors"]
        .as_array()
        .expect("expected graphql errors");
    assert!(
        errors[0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("release is no longer in this search"),
        "unexpected error: {issued}"
    );
}

#[tokio::test]
async fn graphql_unlinked_grab_refuses_a_release_outside_the_search() {
    let ctx = TestContext::new().await;

    let start = gql(
        &ctx,
        r#"mutation($input: SearchReleasesInput!) {
            startInteractiveReleaseSearch(input: $input) { id state }
        }"#,
        json!({ "input": { "query": "paperman", "kind": "RAW" } }),
    )
    .await;
    assert_no_errors(&start);
    let job_id = start["data"]["startInteractiveReleaseSearch"]["id"]
        .as_str()
        .expect("job id")
        .to_string();

    // The release lookup runs before the client is resolved, so a release the
    // operator never saw is refused whatever download client is named.
    let grabbed = gql(
        &ctx,
        r#"mutation($input: QueueUnlinkedReleaseInput!) {
            queueUnlinkedRelease(input: $input) { downloadId clientName sourceTitle }
        }"#,
        json!({
            "input": {
                "searchId": job_id,
                "downloadUrl": "https://example.invalid/not-in-this-search.nzb",
                "downloadClientId": "dc-unknown",
            }
        }),
    )
    .await;
    let errors = grabbed["errors"]
        .as_array()
        .expect("expected graphql errors");
    assert!(
        errors[0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("release is no longer in this search"),
        "unexpected error: {grabbed}"
    );
}

#[tokio::test]
async fn graphql_one_shot_search_releases_requires_a_title() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($input: SearchReleasesInput!) { searchReleases(input: $input) { title } }"#,
        json!({ "input": { "query": "paperman", "kind": "RAW" } }),
    )
    .await;
    let errors = body["errors"].as_array().expect("expected graphql errors");
    assert!(
        errors[0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("searchReleases requires a title id"),
        "unexpected error: {body}"
    );
}

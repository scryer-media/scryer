use super::*;

const DASHBOARD_ACTIVITY_STATS_QUERY: &str = r#"
query DashboardActivityStats($windowHours: Int!) {
  dashboardActivityStats(windowHours: $windowHours) {
    current { grabbed upgraded imported importFailed downloadFailed }
    previous { grabbed upgraded imported importFailed downloadFailed }
  }
}
"#;

/// Same selection with no variables, for `schema_exec` calls that attach a
/// scoped actor instead of going through the HTTP variable path.
const DEFAULT_DASHBOARD_ACTIVITY_STATS_QUERY: &str = r#"
{
  dashboardActivityStats {
    current { grabbed upgraded imported importFailed downloadFailed }
    previous { grabbed upgraded imported importFailed downloadFailed }
  }
}
"#;

const STORAGE_ROOTS_QUERY: &str = r#"
query StorageRoots {
  storageRoots { path libraryId libraryName facet usedBytes totalBytes }
}
"#;

async fn create_dashboard_library(
    ctx: &TestContext,
    facet: &str,
    name: &str,
    root_path: &str,
) -> Value {
    let body = gql(
        ctx,
        r#"mutation($input: CreateLibraryInput!) {
            createLibrary(input: $input) {
                id
                name
                roots { id path isDefault }
            }
        }"#,
        json!({
            "input": {
                "facet": facet,
                "name": name,
                "roots": [{ "path": root_path, "isDefault": true }]
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["createLibrary"].clone()
}

fn dashboard_library_id(library: &Value) -> String {
    library["id"].as_str().expect("library id").to_string()
}

fn dashboard_library_root_id(library: &Value) -> String {
    library["roots"][0]["id"]
        .as_str()
        .expect("library root id")
        .to_string()
}

/// Actor that can view exactly the named libraries and holds no app
/// permissions, so neither query can fall back to the catalog-wide override.
fn dashboard_view_actor(library_ids: &[&str]) -> User {
    User {
        id: Id::new().0,
        username: "dashboard-viewer".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: AppPermissionMask::NONE,
            libraries: library_ids
                .iter()
                .map(|library_id| {
                    (
                        (*library_id).to_string(),
                        LibraryPermissionMask::from_permissions([LibraryPermission::View]),
                    )
                })
                .collect::<HashMap<_, _>>(),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    }
}

async fn add_dashboard_title(
    ctx: &TestContext,
    name: &str,
    tvdb_id: &str,
    library_id: &str,
    root_folder_id: &str,
) -> String {
    let body = gql(
        ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) { title { id } }
        }"#,
        json!({
            "input": {
                "name": name,
                "facet": "MOVIE",
                "libraryId": library_id,
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": tvdb_id }],
                "options": { "rootFolderId": root_folder_id }
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .expect("dashboard title id")
        .to_string()
}

fn dashboard_title_snapshot(title_name: &str) -> TitleContextSnapshot {
    TitleContextSnapshot {
        title_name: title_name.to_string(),
        facet: MediaFacet::Movie,
        external_ids: DomainExternalIds::default(),
        poster_url: None,
        year: None,
    }
}

async fn append_dashboard_event(
    ctx: &TestContext,
    title_id: &str,
    minutes_ago: i64,
    payload: DomainEventPayload,
) {
    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now() - Duration::minutes(minutes_ago),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: Some(title_id.to_string()),
            facet: Some(MediaFacet::Movie),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title_id.to_string(),
            },
            payload,
        })
        .await
        .expect("append dashboard activity event");
}

fn grabbed_payload(title_name: &str) -> DomainEventPayload {
    DomainEventPayload::ReleaseGrabbed(scryer_domain::ReleaseGrabbedEventData {
        title: dashboard_title_snapshot(title_name),
        source_title: Some("Dashboard.Release.2026.1080p.WEB-DL".to_string()),
        source_hint: Some("Fixture Indexer".to_string()),
        source_provider: Some("Fixture Indexer".to_string()),
        download_id: Some("dashboard-download-1".to_string()),
        episode_ids: vec![],
    })
}

/// Size the upgrade fixture reports for the new file.
const UPGRADED_SIZE_BYTES: i64 = 9_876_543_210;
/// Size the import fixture reports as its imported total.
const IMPORTED_SIZE_BYTES: i64 = 1_234_567_890;

fn upgraded_payload(title_name: &str) -> DomainEventPayload {
    DomainEventPayload::MediaFileUpgraded(scryer_domain::MediaFileUpgradedEventData {
        title: dashboard_title_snapshot(title_name),
        media_updates: vec![MediaPathUpdate {
            path: "/library/Dashboard/upgraded.mkv".to_string(),
            update_type: MediaUpdateType::Modified,
        }],
        episode_ids: vec![],
        previous_file_id: Some("file-old".to_string()),
        current_file_id: Some("file-new".to_string()),
        old_score: Some(100),
        new_score: Some(200),
        size_bytes: Some(UPGRADED_SIZE_BYTES),
    })
}

fn imported_payload(title_name: &str) -> DomainEventPayload {
    DomainEventPayload::ImportCompleted(ImportCompletedEventData {
        title: dashboard_title_snapshot(title_name),
        media_updates: vec![MediaPathUpdate {
            path: "/library/Dashboard/imported.mkv".to_string(),
            update_type: MediaUpdateType::Created,
        }],
        imported_count: 1,
        import_id: Some("dashboard-import-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dashboard-download-1".to_string()),
        source_title: Some("Dashboard.Release.2026.1080p.WEB-DL".to_string()),
        source_path: Some("/downloads/dashboard".to_string()),
        dest_path: Some("/library/Dashboard".to_string()),
        quality: Some("WEB-DL 1080p".to_string()),
        episode_ids: vec![],
        size_bytes: Some(IMPORTED_SIZE_BYTES),
    })
}

/// An import event shaped like one persisted before `size_bytes` existed, to
/// prove such rows still deserialize and read back with a null size.
fn legacy_imported_payload(title_name: &str) -> DomainEventPayload {
    let DomainEventPayload::ImportCompleted(mut data) = imported_payload(title_name) else {
        unreachable!("imported_payload builds an ImportCompleted event");
    };
    data.size_bytes = None;
    DomainEventPayload::ImportCompleted(data)
}

fn import_rejected_payload(
    title_name: &str,
    status: scryer_domain::ImportStatus,
) -> DomainEventPayload {
    DomainEventPayload::ImportRejected(scryer_domain::ImportRejectedEventData {
        title: Some(dashboard_title_snapshot(title_name)),
        status,
        import_id: Some("dashboard-import-2".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dashboard-download-2".to_string()),
        source_title: Some("Dashboard.Reject.2026.1080p.WEB-DL".to_string()),
        source_path: Some("/downloads/dashboard-reject".to_string()),
        dest_path: None,
        quality: None,
        reason: Some("fixture rejection".to_string()),
        skip_reason: None,
        episode_ids: vec![],
    })
}

fn download_failed_payload(title_name: &str) -> DomainEventPayload {
    DomainEventPayload::DownloadFailed(DownloadFailedEventData {
        title: Some(dashboard_title_snapshot(title_name)),
        source_title: Some("Dashboard.Failed.2026.1080p.WEB-DL".to_string()),
        source_hint: Some("Fixture Indexer".to_string()),
        download_id: Some("dashboard-download-3".to_string()),
        client_id: Some("client-1".to_string()),
        client_name: Some("Fixture Client".to_string()),
        client_type: Some("nzbget".to_string()),
        quality: None,
        reason: Some("fixture failure".to_string()),
        episode_ids: vec![],
        collection_id: None,
    })
}

async fn dashboard_activity_stats(ctx: &TestContext, window_hours: i64) -> Value {
    let body = gql(
        ctx,
        DASHBOARD_ACTIVITY_STATS_QUERY,
        json!({ "windowHours": window_hours }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["dashboardActivityStats"].clone()
}

#[tokio::test]
async fn graphql_dashboard_activity_stats_counts_current_and_previous_windows() {
    let ctx = TestContext::new().await;
    let library = create_dashboard_library(
        &ctx,
        "MOVIE",
        "Dashboard Activity Library",
        "/dashboard-activity/root",
    )
    .await;
    let library_id = dashboard_library_id(&library);
    let root_folder_id = dashboard_library_root_id(&library);
    let title_id = add_dashboard_title(
        &ctx,
        "Dashboard Activity Fixture",
        "770001",
        &library_id,
        &root_folder_id,
    )
    .await;
    let title_name = "Dashboard Activity Fixture";

    // Inside the trailing 24h window.
    append_dashboard_event(&ctx, &title_id, 60, grabbed_payload(title_name)).await;
    append_dashboard_event(&ctx, &title_id, 120, imported_payload(title_name)).await;
    append_dashboard_event(&ctx, &title_id, 180, upgraded_payload(title_name)).await;
    append_dashboard_event(
        &ctx,
        &title_id,
        240,
        import_rejected_payload(title_name, scryer_domain::ImportStatus::Failed),
    )
    .await;
    // A skipped rejection is not an import failure and must not be counted.
    append_dashboard_event(
        &ctx,
        &title_id,
        300,
        import_rejected_payload(title_name, scryer_domain::ImportStatus::Skipped),
    )
    .await;

    // Inside the preceding 24h window.
    append_dashboard_event(&ctx, &title_id, 30 * 60, grabbed_payload(title_name)).await;
    append_dashboard_event(
        &ctx,
        &title_id,
        25 * 60,
        download_failed_payload(title_name),
    )
    .await;

    // Older than both windows.
    append_dashboard_event(&ctx, &title_id, 200 * 60, grabbed_payload(title_name)).await;
    append_dashboard_event(&ctx, &title_id, 200 * 60, imported_payload(title_name)).await;

    let stats = dashboard_activity_stats(&ctx, 24).await;
    assert_eq!(
        stats["current"],
        json!({
            "grabbed": 1,
            "upgraded": 1,
            "imported": 1,
            "importFailed": 1,
            "downloadFailed": 0
        }),
        "current window counts: {stats}"
    );
    assert_eq!(
        stats["previous"],
        json!({
            "grabbed": 1,
            "upgraded": 0,
            "imported": 0,
            "importFailed": 0,
            "downloadFailed": 1
        }),
        "previous window counts: {stats}"
    );
}

#[tokio::test]
async fn graphql_dashboard_activity_stats_counts_only_viewable_libraries() {
    let ctx = TestContext::new().await;
    let allowed_library = create_dashboard_library(
        &ctx,
        "MOVIE",
        "Dashboard Allowed Library",
        "/dashboard-rbac/allowed",
    )
    .await;
    let denied_library = create_dashboard_library(
        &ctx,
        "MOVIE",
        "Dashboard Denied Library",
        "/dashboard-rbac/denied",
    )
    .await;
    let allowed_library_id = dashboard_library_id(&allowed_library);
    let denied_library_id = dashboard_library_id(&denied_library);

    let allowed_title_id = add_dashboard_title(
        &ctx,
        "Dashboard Allowed Fixture",
        "770101",
        &allowed_library_id,
        &dashboard_library_root_id(&allowed_library),
    )
    .await;
    let denied_title_id = add_dashboard_title(
        &ctx,
        "Dashboard Denied Fixture",
        "770102",
        &denied_library_id,
        &dashboard_library_root_id(&denied_library),
    )
    .await;

    append_dashboard_event(
        &ctx,
        &allowed_title_id,
        60,
        grabbed_payload("Dashboard Allowed Fixture"),
    )
    .await;
    append_dashboard_event(
        &ctx,
        &denied_title_id,
        60,
        grabbed_payload("Dashboard Denied Fixture"),
    )
    .await;
    append_dashboard_event(
        &ctx,
        &denied_title_id,
        90,
        imported_payload("Dashboard Denied Fixture"),
    )
    .await;

    let body = schema_exec(
        &ctx,
        DEFAULT_DASHBOARD_ACTIVITY_STATS_QUERY,
        Some(dashboard_view_actor(&[&allowed_library_id])),
    )
    .await;
    assert_no_errors(&body);
    let stats = &body["data"]["dashboardActivityStats"];
    assert_eq!(stats["current"]["grabbed"], 1, "scoped counts: {stats}");
    assert_eq!(stats["current"]["imported"], 0, "scoped counts: {stats}");

    // The unscoped admin caller still sees both libraries.
    let admin_stats = dashboard_activity_stats(&ctx, 24).await;
    assert_eq!(admin_stats["current"]["grabbed"], 2);
    assert_eq!(admin_stats["current"]["imported"], 1);
}

#[tokio::test]
async fn graphql_dashboard_activity_stats_clamps_window_hours() {
    let ctx = TestContext::new().await;
    let library = create_dashboard_library(
        &ctx,
        "MOVIE",
        "Dashboard Clamp Library",
        "/dashboard-clamp/root",
    )
    .await;
    let library_id = dashboard_library_id(&library);
    let title_id = add_dashboard_title(
        &ctx,
        "Dashboard Clamp Fixture",
        "770201",
        &library_id,
        &dashboard_library_root_id(&library),
    )
    .await;
    let title_name = "Dashboard Clamp Fixture";

    append_dashboard_event(&ctx, &title_id, 30, grabbed_payload(title_name)).await;
    append_dashboard_event(&ctx, &title_id, 90, grabbed_payload(title_name)).await;
    append_dashboard_event(&ctx, &title_id, 300 * 60, grabbed_payload(title_name)).await;

    // Below the floor: clamped up to a one-hour window, so only the 30 minute
    // old grab is current and the 90 minute old grab lands in the prior hour.
    let clamped_low = dashboard_activity_stats(&ctx, 0).await;
    assert_eq!(clamped_low["current"]["grabbed"], 1, "{clamped_low}");
    assert_eq!(clamped_low["previous"]["grabbed"], 1, "{clamped_low}");

    // Above the ceiling: clamped down to 168 hours, so the 300 hour old grab
    // falls in the previous window instead of the current one.
    let clamped_high = dashboard_activity_stats(&ctx, 1000).await;
    assert_eq!(clamped_high["current"]["grabbed"], 2, "{clamped_high}");
    assert_eq!(clamped_high["previous"]["grabbed"], 1, "{clamped_high}");
}

fn storage_root_rows_for_path<'a>(body: &'a Value, path: &str) -> Vec<&'a Value> {
    body["data"]["storageRoots"]
        .as_array()
        .expect("storageRoots array")
        .iter()
        .filter(|row| row["path"].as_str() == Some(path))
        .collect()
}

/// One row per (library, root) pair, carrying the owning library's identity.
///
/// A single path cannot be shared by two libraries: `library_roots` carries a
/// global unique index on `normalized_path`, and `createLibrary` rejects a root
/// another library already claims. So the fan-out this asserts is one library
/// with several roots, plus distinct libraries with distinct roots.
#[tokio::test]
async fn graphql_storage_roots_returns_one_row_per_library_root() {
    let ctx = TestContext::new().await;
    let movie_root = tempfile::tempdir().expect("movie storage root tempdir");
    let series_primary_root = tempfile::tempdir().expect("series primary storage root tempdir");
    let series_secondary_root = tempfile::tempdir().expect("series secondary storage root tempdir");
    let movie_path = movie_root.path().to_string_lossy().to_string();
    let series_primary_path = series_primary_root.path().to_string_lossy().to_string();
    let series_secondary_path = series_secondary_root.path().to_string_lossy().to_string();

    let movie_library =
        create_dashboard_library(&ctx, "MOVIE", "Dashboard Storage Movies", &movie_path).await;
    let series_body = gql(
        &ctx,
        r#"mutation($input: CreateLibraryInput!) {
            createLibrary(input: $input) { id roots { id path isDefault } }
        }"#,
        json!({
            "input": {
                "facet": "SERIES",
                "name": "Dashboard Storage Series",
                "roots": [
                    { "path": series_primary_path, "isDefault": true },
                    { "path": series_secondary_path, "isDefault": false }
                ]
            }
        }),
    )
    .await;
    assert_no_errors(&series_body);
    let series_library_id = dashboard_library_id(&series_body["data"]["createLibrary"]);
    let movie_library_id = dashboard_library_id(&movie_library);

    let body = gql(&ctx, STORAGE_ROOTS_QUERY, json!({})).await;
    assert_no_errors(&body);

    let movie_rows = storage_root_rows_for_path(&body, &movie_path);
    assert_eq!(movie_rows.len(), 1, "one row per library root: {body}");
    assert_eq!(movie_rows[0]["libraryId"], movie_library_id.as_str());
    assert_eq!(movie_rows[0]["libraryName"], "Dashboard Storage Movies");
    assert_eq!(movie_rows[0]["facet"], "MOVIE");

    let mut series_rows = storage_root_rows_for_path(&body, &series_primary_path);
    series_rows.extend(storage_root_rows_for_path(&body, &series_secondary_path));
    assert_eq!(
        series_rows.len(),
        2,
        "both roots of one library are reported: {body}"
    );
    for row in &series_rows {
        assert_eq!(row["libraryId"], series_library_id.as_str());
        assert_eq!(row["libraryName"], "Dashboard Storage Series");
        assert_eq!(row["facet"], "SERIES");
    }

    #[cfg(unix)]
    {
        let mut all_rows = movie_rows;
        all_rows.extend(series_rows);
        for row in &all_rows {
            let total = row["totalBytes"]
                .as_i64()
                .unwrap_or_else(|| panic!("totalBytes should be present on unix: {row}"));
            let used = row["usedBytes"]
                .as_i64()
                .unwrap_or_else(|| panic!("usedBytes should be present on unix: {row}"));
            assert!(total > 0, "totalBytes should be positive: {row}");
            assert!(used <= total, "usedBytes should not exceed total: {row}");
        }
    }

    // A configured root that does not exist on disk still reports its identity,
    // with null usage rather than a dropped row.
    create_dashboard_library(
        &ctx,
        "ANIME",
        "Dashboard Missing Root",
        "/dashboard-missing/root",
    )
    .await;
    let body = gql(&ctx, STORAGE_ROOTS_QUERY, json!({})).await;
    assert_no_errors(&body);
    let missing_rows = storage_root_rows_for_path(&body, "/dashboard-missing/root");
    assert_eq!(
        missing_rows.len(),
        1,
        "missing root is still listed: {body}"
    );
    assert!(
        missing_rows[0]["usedBytes"].is_null() && missing_rows[0]["totalBytes"].is_null(),
        "unstattable root reports null usage: {body}"
    );
}

#[tokio::test]
async fn graphql_storage_roots_omits_libraries_the_caller_cannot_view() {
    let ctx = TestContext::new().await;
    let allowed_root = tempfile::tempdir().expect("allowed storage root tempdir");
    let denied_root = tempfile::tempdir().expect("denied storage root tempdir");
    let allowed_path = allowed_root.path().to_string_lossy().to_string();
    let denied_path = denied_root.path().to_string_lossy().to_string();
    let allowed_library =
        create_dashboard_library(&ctx, "MOVIE", "Dashboard Visible Library", &allowed_path).await;
    create_dashboard_library(&ctx, "MOVIE", "Dashboard Hidden Library", &denied_path).await;
    let allowed_library_id = dashboard_library_id(&allowed_library);

    let body = schema_exec(
        &ctx,
        STORAGE_ROOTS_QUERY,
        Some(dashboard_view_actor(&[&allowed_library_id])),
    )
    .await;
    assert_no_errors(&body);

    let rows = body["data"]["storageRoots"]
        .as_array()
        .expect("storageRoots array");
    assert!(
        rows.iter()
            .all(|row| row["libraryId"].as_str() == Some(allowed_library_id.as_str())),
        "only viewable libraries may appear: {body}"
    );
    assert_eq!(storage_root_rows_for_path(&body, &allowed_path).len(), 1);
    assert!(
        storage_root_rows_for_path(&body, &denied_path).is_empty(),
        "hidden library roots must not leak: {body}"
    );
}

// ---------------------------------------------------------------------------
// Manual Imports panel: pending-import age, size, and reason class
// ---------------------------------------------------------------------------

const PENDING_IMPORTS_QUERY: &str = r#"
query PendingImports {
  pendingImports(facet: MOVIE, status: PENDING, limit: 50, offset: 0) {
    totalCount
    items { id displayName reason reasonClass sizeBytes createdAt }
  }
}
"#;

/// Seed one unmatched row exactly as the scanner would persist it.
///
/// Going through the store rather than a real scan is what makes the reason
/// classes deterministic: `AMBIGUOUS` in particular needs a metadata search
/// that returns candidates and rejects all of them, which a scan against
/// fixture metadata cannot reliably produce.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the persisted unmatched-item row the scanner writes"
)]
async fn seed_pending_import_row(
    ctx: &TestContext,
    id: &str,
    library_id: &str,
    item_path: &str,
    display_name: &str,
    reason_code: &str,
    size_bytes: Option<i64>,
    created_at: &str,
) -> String {
    let item = scryer_application::LibraryScanUnmatchedItem {
        id: id.to_string(),
        library_id: library_id.to_string(),
        facet: MediaFacet::Movie,
        status: scryer_application::PendingImportStatus::Pending,
        title_id: None,
        scan_session_id: "dashboard-pending-session".to_string(),
        scan_root: "/dashboard-pending".to_string(),
        item_path: item_path.to_string(),
        display_name: display_name.to_string(),
        query: display_name.to_string(),
        year_hint: Some(2026),
        reason_code: reason_code.to_string(),
        error_message: None,
        search_attempts: Vec::new(),
        size_bytes,
        created_at: created_at.to_string(),
        updated_at: created_at.to_string(),
    };
    scryer_application::LibraryScanUnmatchedItemRepository::upsert_library_scan_unmatched_item(
        &ctx.library_scan_unmatched,
        &item,
    )
    .await
    .expect("seed pending import row")
}

fn pending_import_row<'a>(body: &'a Value, id: &str) -> &'a Value {
    body["data"]["pendingImports"]["items"]
        .as_array()
        .expect("pendingImports items")
        .iter()
        .find(|row| row["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("pending import row {id} should be present: {body}"))
}

#[tokio::test]
async fn graphql_pending_imports_expose_created_at_size_and_reason_class() {
    let ctx = TestContext::new().await;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    // A row whose size the scanner recorded.
    let persisted_created_at = "2026-04-07T12:30:00+00:00";
    seed_pending_import_row(
        &ctx,
        "pending-persisted-size",
        &library_id,
        "/dashboard-pending/Persisted.Size.2026.mkv",
        "Persisted.Size.2026",
        "no_metadata_search_results",
        Some(7_654_321),
        persisted_created_at,
    )
    .await;

    // A row recorded before the size column existed, whose file is still on
    // disk: listing still returns only scanner-recorded metadata.
    let fallback_dir = tempfile::tempdir().expect("fallback pending import tempdir");
    let fallback_file = fallback_dir.path().join("Fallback.Size.2026.mkv");
    let fallback_contents = vec![0_u8; 4_096];
    std::fs::write(&fallback_file, &fallback_contents).expect("write fallback pending import file");
    seed_pending_import_row(
        &ctx,
        "pending-fallback-size",
        &library_id,
        &fallback_file.to_string_lossy(),
        "Fallback.Size.2026",
        "no_acceptable_metadata_match",
        None,
        "2026-04-07T13:00:00+00:00",
    )
    .await;

    // A row with neither a recorded size nor a readable file.
    seed_pending_import_row(
        &ctx,
        "pending-missing-file",
        &library_id,
        "/dashboard-pending/Does.Not.Exist.2026.mkv",
        "Does.Not.Exist.2026",
        "skipped_file_metadata_unreadable",
        None,
        "2026-04-07T14:00:00+00:00",
    )
    .await;

    // A folder-ownership conflict, which classifies as OTHER.
    seed_pending_import_row(
        &ctx,
        "pending-other-reason",
        &library_id,
        "/dashboard-pending/Owned.Elsewhere.2026",
        "Owned.Elsewhere.2026",
        "title_already_owns_another_folder",
        None,
        "2026-04-07T15:00:00+00:00",
    )
    .await;

    let body = gql(&ctx, PENDING_IMPORTS_QUERY, json!({})).await;
    assert_no_errors(&body);

    let persisted = pending_import_row(&body, "pending-persisted-size");
    assert_eq!(persisted["sizeBytes"], 7_654_321_i64);
    assert_eq!(persisted["reasonClass"], "UNMATCHED");
    assert_eq!(persisted["reason"], "no_metadata_search_results");
    assert_eq!(
        persisted["createdAt"].as_str(),
        Some(persisted_created_at),
        "createdAt should round-trip the stored timestamp: {body}"
    );

    let fallback = pending_import_row(&body, "pending-fallback-size");
    assert!(
        fallback["sizeBytes"].is_null(),
        "a row with no stored size stays unknown even when the file exists: {body}"
    );
    assert_eq!(fallback["reasonClass"], "AMBIGUOUS");

    let missing = pending_import_row(&body, "pending-missing-file");
    assert!(
        missing["sizeBytes"].is_null(),
        "an unreadable file leaves sizeBytes null: {body}"
    );
    assert_eq!(missing["reasonClass"], "QUALITY_UNKNOWN");

    let other = pending_import_row(&body, "pending-other-reason");
    assert_eq!(other["reasonClass"], "OTHER");
    assert_eq!(other["reason"], "title_already_owns_another_folder");

    // Listing must leave the scanner's stored metadata unchanged.
    let stored =
        scryer_application::LibraryScanUnmatchedItemRepository::get_library_scan_unmatched_item(
            &ctx.library_scan_unmatched,
            "pending-fallback-size",
        )
        .await
        .expect("reload seeded pending import")
        .expect("seeded pending import should exist");
    assert!(
        stored.size_bytes.is_none(),
        "listing must not backfill the stored row"
    );
}

// ---------------------------------------------------------------------------
// Indexer table: trailing-24h grab counts
// ---------------------------------------------------------------------------

const INDEXER_STATS_QUERY: &str = r#"
query IndexerStats {
  systemHealth {
    indexerStats {
      indexerId
      indexerName
      queriesLast24H
      grabsLast24H
      grabCurrent
      grabMax
    }
  }
}
"#;

/// The GRAB column reads Scryer's own windowed count, not the provider-reported
/// quota counters that sit beside it in the same payload.
#[tokio::test]
async fn graphql_system_health_exposes_windowed_indexer_grab_counts() {
    let ctx = TestContext::new().await;
    ctx.indexer_stats.record_grab("idx-alpha", "Alpha Indexer");
    ctx.indexer_stats.record_grab("idx-alpha", "Alpha Indexer");
    ctx.indexer_stats.record_grab("idx-beta", "Beta Indexer");
    ctx.indexer_stats
        .record_query("idx-gamma", "Gamma Indexer", true);

    let body = gql(&ctx, INDEXER_STATS_QUERY, json!({})).await;
    assert_no_errors(&body);

    let rows = body["data"]["systemHealth"]["indexerStats"]
        .as_array()
        .expect("indexerStats array");
    let row_for = |indexer_id: &str| {
        rows.iter()
            .find(|row| row["indexerId"].as_str() == Some(indexer_id))
            .unwrap_or_else(|| panic!("indexer {indexer_id} should appear in stats: {body}"))
    };

    let alpha = row_for("idx-alpha");
    assert_eq!(alpha["indexerName"], "Alpha Indexer");
    assert_eq!(alpha["grabsLast24H"], 2);
    assert_eq!(alpha["queriesLast24H"], 0);
    // Provider quota counters stay independent of Scryer's own count.
    assert!(alpha["grabCurrent"].is_null());
    assert!(alpha["grabMax"].is_null());

    assert_eq!(row_for("idx-beta")["grabsLast24H"], 1);

    let gamma = row_for("idx-gamma");
    assert_eq!(gamma["queriesLast24H"], 1);
    assert_eq!(
        gamma["grabsLast24H"], 0,
        "an indexer that was only queried reports zero grabs: {body}"
    );
}

// ---------------------------------------------------------------------------
// Recently Imported panel: history library attribution and sizes
// ---------------------------------------------------------------------------

const RECENT_IMPORT_HISTORY_QUERY: &str = r#"
query RecentImportHistory($titleId: ID!) {
  titleHistory(
    filter: { titleIds: [$titleId], eventTypes: [IMPORTED, FILE_UPGRADED], limit: 20 }
  ) {
    totalCount
    items { id eventType libraryId sizeBytes }
  }
}
"#;

fn history_row_by_event_type<'a>(body: &'a Value, event_type: &str) -> &'a Value {
    body["data"]["titleHistory"]["items"]
        .as_array()
        .expect("titleHistory items")
        .iter()
        .find(|row| row["eventType"].as_str() == Some(event_type))
        .unwrap_or_else(|| panic!("history row {event_type} should be present: {body}"))
}

#[tokio::test]
async fn graphql_title_history_exposes_library_id_and_size_bytes() {
    let ctx = TestContext::new().await;
    let library = create_dashboard_library(
        &ctx,
        "MOVIE",
        "Dashboard Recent Imports",
        "/dashboard-recent/root",
    )
    .await;
    let library_id = dashboard_library_id(&library);
    let title_id = add_dashboard_title(
        &ctx,
        "Dashboard Recent Fixture",
        "770301",
        &library_id,
        &dashboard_library_root_id(&library),
    )
    .await;
    let title_name = "Dashboard Recent Fixture";

    append_dashboard_event(&ctx, &title_id, 30, imported_payload(title_name)).await;
    append_dashboard_event(&ctx, &title_id, 60, upgraded_payload(title_name)).await;

    let body = gql(
        &ctx,
        RECENT_IMPORT_HISTORY_QUERY,
        json!({ "titleId": title_id }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titleHistory"]["totalCount"], 2);

    let imported = history_row_by_event_type(&body, "imported");
    assert_eq!(imported["libraryId"], library_id.as_str());
    assert_eq!(imported["sizeBytes"], IMPORTED_SIZE_BYTES);

    let upgraded = history_row_by_event_type(&body, "file_upgraded");
    assert_eq!(upgraded["libraryId"], library_id.as_str());
    assert_eq!(
        upgraded["sizeBytes"], UPGRADED_SIZE_BYTES,
        "an upgrade reports the new file's size: {body}"
    );
}

#[tokio::test]
async fn graphql_title_history_reads_legacy_events_without_size_as_null() {
    let ctx = TestContext::new().await;
    let library = create_dashboard_library(
        &ctx,
        "MOVIE",
        "Dashboard Legacy Imports",
        "/dashboard-legacy/root",
    )
    .await;
    let library_id = dashboard_library_id(&library);
    let title_id = add_dashboard_title(
        &ctx,
        "Dashboard Legacy Fixture",
        "770401",
        &library_id,
        &dashboard_library_root_id(&library),
    )
    .await;

    // Shaped like an import persisted before the payload carried a size.
    append_dashboard_event(
        &ctx,
        &title_id,
        30,
        legacy_imported_payload("Dashboard Legacy Fixture"),
    )
    .await;

    let body = gql(
        &ctx,
        RECENT_IMPORT_HISTORY_QUERY,
        json!({ "titleId": title_id }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titleHistory"]["totalCount"], 1);

    let imported = history_row_by_event_type(&body, "imported");
    assert!(
        imported["sizeBytes"].is_null(),
        "a legacy event reads back with a null size rather than erroring: {body}"
    );
    assert_eq!(
        imported["libraryId"],
        library_id.as_str(),
        "library attribution still resolves for legacy events: {body}"
    );
}
